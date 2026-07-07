use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use parking_lot::Mutex; // non-poisoning; lock() returns the guard directly

use super::upload_gdrive;

/// Pool size per date folder (2 = one ready + one backup while replenishing)
const POOL_SIZE_PER_DAY: usize = 2;

/// How many days ahead to pre-create (2 = today + tomorrow, so midnight transition is seamless)
const DAYS_AHEAD: i64 = 2;

/// Embedded 1x1 white JPEG placeholder (~630 bytes)
const PLACEHOLDER_JPEG: &[u8] = include_bytes!("../../assets/placeholder.jpg");

// ── Data structures ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaceholderFile {
    pub file_id: String,
    pub url: String,
    pub date: String,       // "YYYY-MM-DD"
    pub size_bytes: u64,
    pub claimed: bool,
}

/// Persistent state saved to gdrive_pool.json
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PoolState {
    pub placeholders: Vec<PlaceholderFile>,
    /// date-path → GDrive folder ID, e.g. "public-images/2026/02/20" → "abc123"
    pub folder_cache: HashMap<String, String>,
    /// The folder_name from settings — invalidate cache if it changes
    pub folder_name: String,
}

impl PoolState {
    /// Number of unclaimed placeholders available for a given date ("YYYY-MM-DD").
    pub fn available_for(&self, date: &str) -> usize {
        self.placeholders.iter().filter(|p| p.date == date && !p.claimed).count()
    }

    /// Index of the first unclaimed placeholder for `date`, if any.
    pub fn find_claimable(&self, date: &str) -> Option<usize> {
        self.placeholders.iter().position(|p| p.date == date && !p.claimed)
    }

    /// Drop placeholders and folder-cache entries for dates before `today`
    /// (keeps claimed placeholders and unparseable cache paths). Pure so the
    /// pruning rule is unit-tested independently of disk/HTTP.
    pub fn prune_expired(&mut self, today: &str) {
        self.placeholders.retain(|p| p.date.as_str() >= today || p.claimed);
        self.folder_cache.retain(|path, _| {
            extract_date_from_path(path).map(|d| d.as_str() >= today).unwrap_or(true)
        });
    }
}

/// How many placeholders still need creating for a date. Pure/testable.
pub fn needed_placeholders(available: usize) -> usize {
    POOL_SIZE_PER_DAY.saturating_sub(available)
}

/// What to do with a past-date, unclaimed placeholder given its live Drive
/// size/name. `Delete` only when it's provably still an untouched placeholder;
/// otherwise its content was swapped in (claimed late) and must be kept. Pure.
#[derive(Debug, PartialEq, Eq)]
pub enum CleanupAction {
    Delete,
    Keep,
}

pub fn decide_cleanup(recorded_size: u64, actual_size: u64, name: &str) -> CleanupAction {
    if actual_size == recorded_size && name.starts_with("_placeholder_") {
        CleanupAction::Delete
    } else {
        CleanupAction::Keep
    }
}

pub struct PoolInner {
    pub state: Mutex<PoolState>,
    pub daemon_running: AtomicBool,
    /// Prevents concurrent maintain_pool executions
    pub maintenance_running: AtomicBool,
    /// Notified on disconnect so the background daemon wakes from its long sleep
    /// and terminates instead of polling forever against a disconnected account
    /// (BUGS#9).
    pub stop_signal: tokio::sync::Notify,
}

/// Tauri-managed state for the pre-allocation pool
pub struct PoolRuntime {
    pub inner: Arc<PoolInner>,
}

// ── Persistence ──────────────────────────────────────────────────

fn get_pool_path() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("ClipToAll");
    fs::create_dir_all(&path).ok();
    path.push("gdrive_pool.json");
    path
}

fn save_pool_state(state: &PoolState) {
    let path = get_pool_path();
    if let Ok(json) = serde_json::to_string_pretty(state) {
        let _ = fs::write(&path, json);
    }
}

fn load_pool_state() -> PoolState {
    let path = get_pool_path();
    if let Ok(content) = fs::read_to_string(&path) {
        if let Ok(state) = serde_json::from_str(&content) {
            return state;
        }
    }
    PoolState::default()
}

// ── Init ─────────────────────────────────────────────────────────

/// Load pool from disk, prune old entries, return managed state.
pub fn init_pool() -> PoolRuntime {
    let mut state = load_pool_state();

    // Remove entries for dates older than today
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    state.prune_expired(&today);

    save_pool_state(&state);

    PoolRuntime {
        inner: Arc::new(PoolInner {
            state: Mutex::new(state),
            daemon_running: AtomicBool::new(false),
            maintenance_running: AtomicBool::new(false),
            stop_signal: tokio::sync::Notify::new(),
        }),
    }
}

/// Extract "YYYY-MM-DD" from a path like "folder/2026/02/20"
fn extract_date_from_path(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() >= 4 {
        let len = parts.len();
        let year = parts[len - 3];
        let month = parts[len - 2];
        let day = parts[len - 1];
        if year.len() == 4 && month.len() == 2 && day.len() == 2 {
            return Some(format!("{}-{}-{}", year, month, day));
        }
    }
    None
}

// ── Daemon ───────────────────────────────────────────────────────

/// Start the background daemon if not already running.
pub fn start_daemon(pool: Arc<PoolInner>) {
    if pool.daemon_running.swap(true, Ordering::SeqCst) {
        return; // Already running
    }
    tauri::async_runtime::spawn(async move {
        daemon_loop(pool).await;
    });
}

/// Signal the daemon to stop and prevent it being considered "running". Called
/// on gdrive_disconnect so the pre-allocation loop doesn't keep polling against
/// a disconnected account (BUGS#9). A later gdrive_authorize re-spawns it.
pub fn stop_daemon(pool: &PoolInner) {
    pool.daemon_running.store(false, Ordering::SeqCst);
    pool.stop_signal.notify_waiters();
}

async fn daemon_loop(pool: Arc<PoolInner>) {
    loop {
        // Stop was requested (e.g. gdrive_disconnect) — exit the loop.
        if !pool.daemon_running.load(Ordering::SeqCst) {
            break;
        }
        if let Err(e) = maintain_pool(&pool).await {
            crate::log(&format!("[gdrive_pool] maintain error: {}", e));
        }
        // Re-check after the (possibly long) maintenance work.
        if !pool.daemon_running.load(Ordering::SeqCst) {
            break;
        }
        // Adaptive sleep: retry sooner when pool is depleted
        let available = {
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            let state = pool.state.lock();
            state.available_for(&today)
        };
        let sleep_secs = if available < POOL_SIZE_PER_DAY { 300 } else { 3600 };
        // Wake early if a stop is signalled, so disconnect doesn't leave the
        // daemon sleeping for up to an hour before it notices (BUGS#9).
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(sleep_secs)) => {}
            _ = pool.stop_signal.notified() => { break; }
        }
    }
}

/// Core maintenance: create folders ahead, fill placeholders, clean old ones.
/// Only one instance runs at a time (guarded by maintenance_running flag).
async fn maintain_pool(pool: &PoolInner) -> Result<(), String> {
    // Prevent concurrent maintenance (e.g. daemon + replenish trigger)
    if pool.maintenance_running.swap(true, Ordering::SeqCst) {
        return Ok(()); // Another maintenance is already running
    }
    // RAII guard: reset the flag even if maintain_pool_inner panics/unwinds,
    // otherwise a single API-response panic would stop replenishment forever (3.10).
    struct RunningGuard<'a>(&'a AtomicBool);
    impl Drop for RunningGuard<'_> {
        fn drop(&mut self) { self.0.store(false, Ordering::SeqCst); }
    }
    let _guard = RunningGuard(&pool.maintenance_running);
    maintain_pool_inner(pool).await
}

async fn maintain_pool_inner(pool: &PoolInner) -> Result<(), String> {
    let settings = crate::commands::settings::load_settings_sync();
    if settings.storage_type != "gdrive" {
        return Ok(());
    }

    let access_token = match upload_gdrive::get_valid_token().await {
        Ok(t) => t,
        Err(_) => return Ok(()), // No token — silently skip
    };

    let client = upload_gdrive::http_client();
    let folder_name = &settings.google_drive_folder;

    // Invalidate cache if folder_name changed
    {
        let mut state = pool.state.lock();
        if state.folder_name != *folder_name {
            state.folder_cache.clear();
            state.folder_name = folder_name.clone();
        }
    }

    let today = chrono::Local::now().date_naive();

    // Phase 1+2: Ensure folders exist and fill placeholders for each day
    for day_offset in 0..DAYS_AHEAD {
        let date = today + chrono::Duration::days(day_offset);
        let date_str = date.format("%Y-%m-%d").to_string();

        // Ensure folder hierarchy
        let folder_id = match ensure_date_folder_cached(
            pool, &client, &access_token, folder_name, date,
        ).await {
            Ok(id) => id,
            Err(e) => {
                crate::log(&format!("[gdrive_pool] folder error for {}: {}", date_str, e));
                break; // Likely network issue, stop trying
            }
        };

        // Count unclaimed placeholders for this date
        let existing_count = {
            let state = pool.state.lock();
            state.available_for(&date_str)
        };

        let needed = needed_placeholders(existing_count);
        for _ in 0..needed {
            match create_placeholder(&client, &access_token, &folder_id).await {
                Ok(ph) => {
                    let mut state = pool.state.lock();
                    state.placeholders.push(PlaceholderFile {
                        file_id: ph.0,
                        url: ph.1,
                        date: date_str.clone(),
                        size_bytes: PLACEHOLDER_JPEG.len() as u64,
                        claimed: false,
                    });
                }
                Err(e) => {
                    crate::log(&format!("[gdrive_pool] create placeholder failed: {}", e));
                    break; // Stop on first failure
                }
            }
        }
    }

    // Phase 3: Cleanup old placeholders
    cleanup_old_placeholders(pool, &client, &access_token, &today).await;

    // Persist — clone state, release lock, then write to disk
    let state_snapshot = pool.state.lock().clone();
    save_pool_state(&state_snapshot);

    Ok(())
}

// ── Folder caching ───────────────────────────────────────────────

/// Resolve TODAY's `folder_name/YYYY/MM/DD` folder id, reusing the pool's
/// folder cache. Public so the direct-upload path shares the same cache instead
/// of doing four uncached folder lookups per upload (folder-cache unification).
pub async fn ensure_today_folder_cached(
    pool: &PoolInner,
    client: &reqwest::Client,
    token: &str,
    folder_name: &str,
) -> Result<String, String> {
    // Keep the cache coherent if the user changed the target folder name. Cache
    // keys are namespaced by folder_name so a stale key never collides, but we
    // still clear so old entries don't linger for the whole day.
    {
        let mut state = pool.state.lock();
        if state.folder_name != *folder_name {
            state.folder_cache.clear();
            state.folder_name = folder_name.to_string();
        }
    }
    let today = chrono::Local::now().date_naive();
    ensure_date_folder_cached(pool, client, token, folder_name, today).await
}

async fn ensure_date_folder_cached(
    pool: &PoolInner,
    client: &reqwest::Client,
    token: &str,
    folder_name: &str,
    date: chrono::NaiveDate,
) -> Result<String, String> {
    let year = date.format("%Y").to_string();
    let month = date.format("%m").to_string();
    let day = date.format("%d").to_string();
    let path_key = format!("{}/{}/{}/{}", folder_name, year, month, day);

    // Check cache first
    {
        let state = pool.state.lock();
        if let Some(folder_id) = state.folder_cache.get(&path_key) {
            return Ok(folder_id.clone());
        }
    }

    // Not cached — create folder hierarchy
    let root_key = folder_name.to_string();
    let year_key = format!("{}/{}", folder_name, year);
    let month_key = format!("{}/{}/{}", folder_name, year, month);

    // Try to use cached intermediate folders
    let root_id = get_or_create_folder(pool, client, token, &root_key, folder_name, None).await?;
    let year_id = get_or_create_folder(pool, client, token, &year_key, &year, Some(&root_id)).await?;
    let month_id = get_or_create_folder(pool, client, token, &month_key, &month, Some(&year_id)).await?;
    let day_id = get_or_create_folder(pool, client, token, &path_key, &day, Some(&month_id)).await?;

    Ok(day_id)
}

/// Get folder ID from cache or create it, caching the result.
async fn get_or_create_folder(
    pool: &PoolInner,
    client: &reqwest::Client,
    token: &str,
    cache_key: &str,
    name: &str,
    parent_id: Option<&str>,
) -> Result<String, String> {
    // Check cache
    {
        let state = pool.state.lock();
        if let Some(id) = state.folder_cache.get(cache_key) {
            return Ok(id.clone());
        }
    }

    // Create/find via API
    let id = upload_gdrive::find_or_create_folder(client, token, name, parent_id).await?;

    // Cache it
    {
        let mut state = pool.state.lock();
        state.folder_cache.insert(cache_key.to_string(), id.clone());
    }

    Ok(id)
}

// ── Placeholder creation ─────────────────────────────────────────

/// Create a single placeholder file on GDrive with public permissions.
/// Returns (file_id, url). If permissions fail, the file is deleted.
async fn create_placeholder(
    client: &reqwest::Client,
    token: &str,
    folder_id: &str,
) -> Result<(String, String), String> {
    let placeholder_name = format!("_placeholder_{}.jpg", uuid::Uuid::new_v4());

    let metadata = serde_json::json!({
        "name": placeholder_name,
        "mimeType": "image/jpeg",
        "parents": [folder_id]
    });

    let boundary = upload_gdrive::multipart_boundary();
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    body.extend_from_slice(b"Content-Type: application/json; charset=UTF-8\r\n\r\n");
    body.extend_from_slice(metadata.to_string().as_bytes());
    body.extend_from_slice(format!("\r\n--{}\r\n", boundary).as_bytes());
    body.extend_from_slice(b"Content-Type: image/jpeg\r\n\r\n");
    body.extend_from_slice(PLACEHOLDER_JPEG);
    body.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());

    let resp: serde_json::Value = client
        .post("https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart&fields=id")
        .bearer_auth(token)
        .header("Content-Type", format!("multipart/related; boundary={}", boundary))
        .body(body)
        .send()
        .await
        .map_err(|e| format!("Placeholder upload failed: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Parse placeholder resp: {}", e))?;

    let file_id = resp["id"]
        .as_str()
        .ok_or_else(|| format!("No file ID in placeholder response: {}", resp))?
        .to_string();

    // Set public permissions — if this fails (transport OR non-2xx status), the
    // file would stay private, so delete the orphan and report failure (3.11).
    let permission = serde_json::json!({ "role": "reader", "type": "anyone" });
    let perm_result = client
        .post(format!(
            "https://www.googleapis.com/drive/v3/files/{}/permissions",
            file_id
        ))
        .bearer_auth(token)
        .json(&permission)
        .send()
        .await;
    let perm_ok = matches!(perm_result, Ok(r) if r.status().is_success());
    if !perm_ok {
        // Clean up the orphaned private file
        let _ = delete_file(client, token, &file_id).await;
        return Err("Permission failed (placeholder cleaned up)".to_string());
    }

    let url = format!("https://drive.google.com/uc?id={}&export=view", file_id);
    Ok((file_id, url))
}

// ── Claim & replace ──────────────────────────────────────────────

/// Claim a placeholder for today. Returns (file_id, url) or None.
pub fn claim_placeholder(pool: &PoolInner) -> Option<(String, String)> {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut state = pool.state.lock();
    let idx = state.find_claimable(&today)?;
    let ph = &mut state.placeholders[idx];
    ph.claimed = true;
    let result = (ph.file_id.clone(), ph.url.clone());
    // Clone + release lock, then save (avoids holding mutex during disk I/O)
    let snapshot = state.clone();
    drop(state);
    save_pool_state(&snapshot);
    Some(result)
}

/// Replace placeholder content with real image data via single multipart PATCH.
/// Updates both the filename and the image content atomically in one request.
/// The file ID and URL remain the same.
pub async fn replace_placeholder_content(
    file_id: &str,
    image_data: Vec<u8>,
    filename: &str,
) -> Result<(), String> {
    let access_token = upload_gdrive::get_valid_token().await?;
    let client = upload_gdrive::http_client();

    // Single multipart PATCH: update metadata (name) + content atomically
    let metadata = serde_json::json!({ "name": filename });
    let boundary = upload_gdrive::multipart_boundary();
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    body.extend_from_slice(b"Content-Type: application/json; charset=UTF-8\r\n\r\n");
    body.extend_from_slice(metadata.to_string().as_bytes());
    body.extend_from_slice(format!("\r\n--{}\r\n", boundary).as_bytes());
    body.extend_from_slice(b"Content-Type: image/jpeg\r\n\r\n");
    body.extend_from_slice(&image_data);
    body.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());

    let resp = client
        .patch(format!(
            "https://www.googleapis.com/upload/drive/v3/files/{}?uploadType=multipart",
            file_id
        ))
        .bearer_auth(&access_token)
        .header("Content-Type", format!("multipart/related; boundary={}", boundary))
        .timeout(std::time::Duration::from_secs(120)) // big screenshots on slow links (3.19)
        .body(body)
        .send()
        .await
        .map_err(|e| format!("Content PATCH failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        return Err(format!("PATCH failed ({}): {}", status, body_text));
    }

    Ok(())
}

/// Trigger pool replenishment in background after a placeholder is consumed.
pub fn trigger_replenish(pool: Arc<PoolInner>) {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = maintain_pool(&pool).await {
            crate::log(&format!("[gdrive_pool] replenish error: {}", e));
        }
    });
}

// ── Cleanup ──────────────────────────────────────────────────────

async fn cleanup_old_placeholders(
    pool: &PoolInner,
    client: &reqwest::Client,
    token: &str,
    today: &chrono::NaiveDate,
) {
    let today_str = today.format("%Y-%m-%d").to_string();

    // Collect unclaimed placeholders from past dates
    let to_check: Vec<PlaceholderFile> = {
        let state = pool.state.lock();
        state
            .placeholders
            .iter()
            .filter(|p| p.date < today_str && !p.claimed)
            .cloned()
            .collect()
    };

    let mut deleted_ids: Vec<String> = Vec::new();
    let mut mark_claimed_ids: Vec<String> = Vec::new();

    for ph in &to_check {
        // Verify file size AND name before deleting
        match get_file_info(client, token, &ph.file_id).await {
            Ok((actual_size, name)) => match decide_cleanup(ph.size_bytes, actual_size, &name) {
                CleanupAction::Delete => {
                    // Still a placeholder — safe to delete
                    if delete_file(client, token, &ph.file_id).await.is_ok() {
                        deleted_ids.push(ph.file_id.clone());
                        crate::log(&format!(
                            "[gdrive_pool] deleted old placeholder {}",
                            ph.file_id
                        ));
                    }
                }
                CleanupAction::Keep => {
                    // Size or name changed — content was replaced, don't delete
                    crate::log(&format!(
                        "[gdrive_pool] skip delete {}: size={} name={}, marking claimed",
                        ph.file_id, actual_size, name
                    ));
                    mark_claimed_ids.push(ph.file_id.clone());
                }
            },
            Err(_) => {
                // Can't verify — skip, try next hour
            }
        }
    }

    // Apply changes
    if !deleted_ids.is_empty() || !mark_claimed_ids.is_empty() {
        let mut state = pool.state.lock();
        state.placeholders.retain(|p| !deleted_ids.contains(&p.file_id));
        for p in state.placeholders.iter_mut() {
            if mark_claimed_ids.contains(&p.file_id) {
                p.claimed = true;
            }
        }
        // Also remove claimed entries for old dates (no need to track used old files)
        state
            .placeholders
            .retain(|p| p.date >= today_str || !p.claimed);
        // Clean old folder cache
        state.folder_cache.retain(|path, _| {
            if let Some(date_part) = extract_date_from_path(path) {
                date_part >= today_str
            } else {
                true
            }
        });
    }
}

/// Get file size and name from GDrive API (for safe cleanup verification).
async fn get_file_info(
    client: &reqwest::Client,
    token: &str,
    file_id: &str,
) -> Result<(u64, String), String> {
    let resp: serde_json::Value = client
        .get(format!(
            "https://www.googleapis.com/drive/v3/files/{}?fields=size,name",
            file_id
        ))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("Get file info: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Parse file info: {}", e))?;

    let size = resp["size"]
        .as_str()
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(|| "No size field".to_string())?;
    let name = resp["name"]
        .as_str()
        .unwrap_or("")
        .to_string();

    Ok((size, name))
}

async fn delete_file(
    client: &reqwest::Client,
    token: &str,
    file_id: &str,
) -> Result<(), String> {
    let resp = client
        .delete(format!(
            "https://www.googleapis.com/drive/v3/files/{}",
            file_id
        ))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("Delete: {}", e))?;
    let status = resp.status();
    if status.is_success() || status == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    let body = resp.text().await.unwrap_or_default();
    Err(format!("Delete failed ({}): {}", status, body))
}

// ── Pool clearing (for gdrive_disconnect) ────────────────────────

pub fn clear_pool(pool: &PoolInner) {
    let mut state = pool.state.lock();
    state.placeholders.clear();
    state.folder_cache.clear();
    state.folder_name.clear();
    let snapshot = state.clone();
    drop(state);
    save_pool_state(&snapshot);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_date_valid() {
        assert_eq!(
            extract_date_from_path("public-images/2026/02/20"),
            Some("2026-02-20".to_string())
        );
    }

    #[test]
    fn extract_date_invalid() {
        assert_eq!(extract_date_from_path("foo/bar"), None);
        assert_eq!(extract_date_from_path("a/b/c/notadate"), None);
    }

    fn ph(date: &str, claimed: bool) -> PlaceholderFile {
        PlaceholderFile {
            file_id: format!("id-{}-{}", date, claimed),
            url: "u".into(),
            date: date.into(),
            size_bytes: 630,
            claimed,
        }
    }

    #[test]
    fn available_for_counts_only_unclaimed_of_date() {
        let s = PoolState {
            placeholders: vec![
                ph("2026-07-05", false),
                ph("2026-07-05", true),  // claimed — excluded
                ph("2026-07-06", false), // other day — excluded
            ],
            ..Default::default()
        };
        assert_eq!(s.available_for("2026-07-05"), 1);
        assert_eq!(s.available_for("2026-07-06"), 1);
        assert_eq!(s.available_for("2026-07-07"), 0);
    }

    #[test]
    fn find_claimable_returns_first_unclaimed_today() {
        let mut s = PoolState {
            placeholders: vec![ph("2026-07-05", true), ph("2026-07-05", false)],
            ..Default::default()
        };
        assert_eq!(s.find_claimable("2026-07-05"), Some(1));
        // none left after both claimed
        s.placeholders[1].claimed = true;
        assert_eq!(s.find_claimable("2026-07-05"), None);
    }

    #[test]
    fn needed_replenishes_up_to_pool_size() {
        assert_eq!(needed_placeholders(0), POOL_SIZE_PER_DAY);
        assert_eq!(needed_placeholders(POOL_SIZE_PER_DAY), 0);
        assert_eq!(needed_placeholders(POOL_SIZE_PER_DAY + 5), 0); // saturating
    }

    #[test]
    fn cleanup_deletes_only_untouched_placeholder() {
        // Same size + placeholder name → still a placeholder → delete.
        assert_eq!(decide_cleanup(630, 630, "_placeholder_abc.jpg"), CleanupAction::Delete);
        // Size changed → content swapped in → keep.
        assert_eq!(decide_cleanup(630, 51234, "_placeholder_abc.jpg"), CleanupAction::Keep);
        // Renamed (claimed) even if size coincidentally matches → keep.
        assert_eq!(decide_cleanup(630, 630, "cta_2026_07_05.jpg"), CleanupAction::Keep);
    }

    #[test]
    fn prune_expired_drops_old_unclaimed_and_old_cache() {
        let mut s = PoolState {
            placeholders: vec![
                ph("2026-07-04", false), // old + unclaimed → drop
                ph("2026-07-04", true),  // old + claimed → keep
                ph("2026-07-05", false), // today → keep
            ],
            ..Default::default()
        };
        s.folder_cache.insert("imgs/2026/07/04".into(), "old".into());
        s.folder_cache.insert("imgs/2026/07/05".into(), "new".into());
        s.folder_cache.insert("weird-path".into(), "keep".into()); // unparseable → keep

        s.prune_expired("2026-07-05");

        assert_eq!(s.placeholders.len(), 2);
        assert!(s.placeholders.iter().all(|p| p.date != "2026-07-04" || p.claimed));
        assert!(!s.folder_cache.contains_key("imgs/2026/07/04"));
        assert!(s.folder_cache.contains_key("imgs/2026/07/05"));
        assert!(s.folder_cache.contains_key("weird-path"));
    }
}
