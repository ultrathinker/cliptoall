use serde::{Deserialize, Serialize};
use std::net::TcpListener;
use std::io::{BufRead, BufReader, Write};
use tauri_plugin_opener::OpenerExt;
use parking_lot::Mutex; // non-poisoning; lock() returns the guard directly
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use std::sync::OnceLock;
use base64::Engine;
use sha2::{Digest, Sha256};

/// Extract a query param value from an HTTP request line ("GET /?a=1&b=2 HTTP/1.1").
fn extract_query_param(request_line: &str, key: &str) -> Option<String> {
    let path = request_line.split_whitespace().nth(1)?;
    let query = path.split('?').nth(1)?;
    for pair in query.split('&') {
        let mut it = pair.splitn(2, '=');
        if it.next() == Some(key) {
            return it.next().map(|v| v.to_string());
        }
    }
    None
}

/// Poll the loopback listener until the OAuth redirect carrying `code=` arrives,
/// verify its `state`, write the success page and return the raw code. Non-`code`
/// requests (favicon, browser pre-flight) get a 404 and the loop continues, so
/// they can't steal the single accepted connection. Bounded by `timeout`.
fn wait_for_oauth_code(
    listener: TcpListener,
    expected_state: &str,
    timeout: std::time::Duration,
) -> Result<String, String> {
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("Failed to set non-blocking: {}", e))?;
    let deadline = std::time::Instant::now() + timeout;

    loop {
        if std::time::Instant::now() >= deadline {
            return Err("Google authorization timed out (no response within 2 minutes).".to_string());
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                // The accepted socket may inherit non-blocking mode; force blocking
                // with a bounded read timeout so read_line waits for the request
                // line (data arrives in ms) but can never hang the thread forever.
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
                let mut request_line = String::new();
                {
                    let mut reader = BufReader::new(&stream);
                    if reader.read_line(&mut request_line).is_err() {
                        continue;
                    }
                }

                // Ignore anything that isn't the authorization redirect.
                if !request_line.contains("code=") {
                    let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\n\r\n");
                    continue;
                }

                // Verify the CSRF state BEFORE using the code (RFC 8252 §8.9).
                let returned_state = extract_query_param(&request_line, "state");
                if returned_state.as_deref() != Some(expected_state) {
                    let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\nState mismatch");
                    return Err("Authorization rejected: state mismatch (possible CSRF).".to_string());
                }

                // Parse the query properly: pick the exact `code` param (so a
                // substring like `promo_code=` can't be mistaken for it) and
                // URL-decode its value (BUGS#8).
                let code = extract_query_param(&request_line, "code").map(|raw| {
                    urlencoding::decode(&raw)
                        .map(|c| c.into_owned())
                        .unwrap_or(raw)
                });

                match code {
                    Some(c) => {
                        let response = "HTTP/1.1 200 OK\r\n\r\n<html><body><h1>Authorization successful!</h1><p>You can close this window.</p></body></html>";
                        let _ = stream.write_all(response.as_bytes());
                        return Ok(c);
                    }
                    None => {
                        let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\nNo code");
                        continue;
                    }
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => return Err(format!("Failed to accept connection: {}", e)),
        }
    }
}

// Google OAuth "Desktop app" client, injected at BUILD time from the
// GDRIVE_CLIENT_ID / GDRIVE_CLIENT_SECRET environment variables. Official
// release builds set these so the shipped binary "just works" — end users only
// log into their own Google account. The public source ships no credential.
// If a build has none, Google Drive is simply disabled (see gdrive_authorize).
// Per RFC 8252, a desktop client secret is not confidential; PKCE protects the flow.
const CLIENT_ID: &str = match option_env!("GDRIVE_CLIENT_ID") {
    Some(v) => v,
    None => "",
};
const CLIENT_SECRET: &str = match option_env!("GDRIVE_CLIENT_SECRET") {
    Some(v) => v,
    None => "",
};

/// True when this build was compiled with Google OAuth credentials.
fn gdrive_configured() -> bool {
    !CLIENT_ID.is_empty() && !CLIENT_SECRET.is_empty()
}

static GDRIVE_TOKEN: Mutex<Option<SavedToken>> = Mutex::new(None);
static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Shared HTTP client with sane timeouts. Every network op must complete in
/// bounded time — otherwise the Results window spins forever (BUGS#4a).
/// reqwest::Client is internally Arc-based, so cloning is cheap.
pub fn http_client() -> reqwest::Client {
    HTTP_CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .connect_timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new())
        })
        .clone()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct SavedToken {
    access_token: String,
    refresh_token: String,
    /// Unix timestamp (seconds) when access_token expires
    expires_at: u64,
}

/// Google's token endpoint response
#[derive(Deserialize)]
struct GoogleTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    expires_in: u64,
    #[allow(dead_code)]
    token_type: String,
}

fn get_token_path() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("ClipToAll");
    fs::create_dir_all(&path).ok();
    path.push("gdrive_token.json");
    path
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// Generate a random multipart boundary. A fixed string could, in principle,
/// occur inside JPEG bytes and corrupt the request body.
pub fn multipart_boundary() -> String {
    format!("cta_{}", uuid::Uuid::new_v4().simple())
}

/// Escape a value for use inside a Google Drive query string literal
/// (`name='...'`). Backslash and single-quote must be escaped, otherwise a
/// folder name like `John's` breaks the query.
pub fn escape_drive_query(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

fn save_token_to_disk(token: &SavedToken) -> Result<(), String> {
    let path = get_token_path();
    let json = serde_json::to_string_pretty(token)
        .map_err(|e| format!("Failed to serialize gdrive token: {}", e))?;
    let encrypted = crate::utils::dpapi::dpapi_encrypt(&json)
        .map_err(|e| format!("Failed to encrypt gdrive token: {}", e))?;
    fs::write(&path, encrypted)
        .map_err(|e| format!("Failed to write gdrive token: {}", e))?;
    Ok(())
}

fn load_token_from_disk() -> Option<SavedToken> {
    let path = get_token_path();
    let content = fs::read_to_string(&path).ok()?;

    // Try DPAPI decryption first (new encrypted format)
    if let Ok(json_str) = crate::utils::dpapi::dpapi_decrypt(&content) {
        return serde_json::from_str(&json_str).ok();
    }

    // Fallback: plaintext JSON (migration from old unencrypted format)
    serde_json::from_str(&content).ok()
}

fn delete_token_from_disk() {
    let path = get_token_path();
    let _ = fs::remove_file(&path);
}

/// Ensure token is loaded from disk into memory (called on first use)
fn ensure_loaded() {
    let mut guard = GDRIVE_TOKEN.lock();
    if guard.is_none() {
        if let Some(saved) = load_token_from_disk() {
            *guard = Some(saved);
        }
    }
}

/// Get a valid access token, refreshing if expired
pub async fn get_valid_token() -> Result<String, String> {
    ensure_loaded();

    let token = GDRIVE_TOKEN.lock().clone()
        .ok_or("Not authorized. Please connect to Google Drive.")?;

    // If token expires in less than 60 seconds, refresh it
    if now_secs() + 60 >= token.expires_at {
        let client = http_client();
        let params = [
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
            ("refresh_token", token.refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ];

        let resp = client
            .post("https://oauth2.googleapis.com/token")
            .form(&params)
            .send()
            .await
            .map_err(|e| format!("Token refresh failed: {}", e))?
            .json::<GoogleTokenResponse>()
            .await
            .map_err(|e| format!("Failed to parse refresh response: {}", e))?;

        let refreshed = SavedToken {
            access_token: resp.access_token.clone(),
            // Google doesn't return refresh_token on refresh, keep the old one
            refresh_token: resp.refresh_token.unwrap_or(token.refresh_token),
            expires_at: now_secs() + resp.expires_in,
        };

        save_token_to_disk(&refreshed)?;
        *GDRIVE_TOKEN.lock() = Some(refreshed);

        Ok(resp.access_token)
    } else {
        Ok(token.access_token)
    }
}

#[tauri::command]
pub async fn gdrive_authorize(
    app: tauri::AppHandle,
    pool: tauri::State<'_, super::gdrive_pool::PoolRuntime>,
) -> Result<String, String> {
    if !gdrive_configured() {
        return Err("Google Drive is not configured in this build. Use a release build, or build with GDRIVE_CLIENT_ID and GDRIVE_CLIENT_SECRET set (see README). You can also use Amazon S3 instead.".to_string());
    }
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("Failed to bind to port: {}", e))?;
    let port = listener.local_addr()
        .map_err(|e| format!("Failed to get port: {}", e))?
        .port();

    let redirect_uri = format!("http://localhost:{}", port);

    // PKCE + CSRF state (RFC 8252 §8.9). code_verifier = 32 random bytes
    // (2×UUID) base64url; code_challenge = base64url(SHA256(verifier)). `state`
    // is verified on the redirect so a local process can't inject its own code.
    let mut verifier_bytes = Vec::with_capacity(32);
    verifier_bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    verifier_bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    let code_verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&verifier_bytes);
    let code_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(code_verifier.as_bytes()));
    let expected_state = uuid::Uuid::new_v4().to_string();

    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope=https://www.googleapis.com/auth/drive.file&access_type=offline&prompt=consent&code_challenge={}&code_challenge_method=S256&state={}",
        CLIENT_ID, urlencoding::encode(&redirect_uri), code_challenge, expected_state
    );

    app.opener().open_url(auth_url, None::<&str>).map_err(|e| e.to_string())?;

    // Wait for the browser redirect, but never forever: if the user closes the
    // browser or abandons the flow, time out instead of hanging the command
    // (and its tokio worker) indefinitely (BUGS#10). The listener is polled in a
    // loop so a browser pre-flight / favicon request that arrives BEFORE the real
    // redirect gets a 404 and doesn't consume the one accepted connection.
    let expected_state_clone = expected_state.clone();
    let code = tokio::task::spawn_blocking(move || {
        wait_for_oauth_code(listener, &expected_state_clone, std::time::Duration::from_secs(120))
    })
    .await
    .map_err(|e| format!("Authorization task failed: {}", e))??;

    let client = http_client();
    // client_secret is retained (Google "Desktop app" clients still expect it);
    // PKCE code_verifier is what actually protects the exchange.
    let params = [
        ("code", code.as_str()),
        ("client_id", CLIENT_ID),
        ("client_secret", CLIENT_SECRET),
        ("redirect_uri", &redirect_uri),
        ("grant_type", "authorization_code"),
        ("code_verifier", code_verifier.as_str()),
    ];

    let token_response = client
        .post("https://oauth2.googleapis.com/token")
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Token request failed: {}", e))?
        .json::<GoogleTokenResponse>()
        .await
        .map_err(|e| format!("Failed to parse token: {}", e))?;

    let saved = SavedToken {
        access_token: token_response.access_token.clone(),
        refresh_token: token_response.refresh_token.unwrap_or_default(),
        expires_at: now_secs() + token_response.expires_in,
    };

    save_token_to_disk(&saved)?;
    *GDRIVE_TOKEN.lock() = Some(saved);

    let user_info: serde_json::Value = client
        .get("https://www.googleapis.com/drive/v3/about?fields=user")
        .bearer_auth(&token_response.access_token)
        .send()
        .await
        .map_err(|e| format!("Failed to get user info: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Failed to parse user info: {}", e))?;

    let email = user_info["user"]["emailAddress"]
        .as_str()
        .unwrap_or("Connected")
        .to_string();

    // Start pre-allocation daemon after successful auth
    let pool_inner = pool.inner.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        super::gdrive_pool::start_daemon(pool_inner);
    });

    Ok(email)
}

#[tauri::command]
pub async fn gdrive_upload(
    image_path: String,
    folder_name: String,
    output_scale: f32,
) -> Result<String, String> {
    // Reject any path outside the app's temp screenshot dir (BUGS#3).
    super::capture::ensure_temp_screenshot_path(&image_path)?;
    // Guarantee JPEG bytes (+ output downscale if enabled) even if invoked
    // directly with a full-res PNG working copy.
    let image_path = super::capture::ensure_jpeg_for_upload(&image_path, output_scale)?;
    let access_token = get_valid_token().await?;
    let client = http_client();

    // Create/resolve folder_name/YYYY/MM/DD (uncached direct path).
    let day_id = find_or_create_date_folder(&client, &access_token, &folder_name).await?;
    upload_image_to_folder(&client, &access_token, &image_path, &day_id).await
}

/// Direct upload that reuses the pool's folder cache to resolve today's folder
/// (avoids the 4 uncached folder lookups per upload — folder-cache unification).
/// Used by the pooled path's fallback so it doesn't re-walk the folder tree.
pub async fn gdrive_upload_cached(
    pool: &super::gdrive_pool::PoolInner,
    image_path: String,
    folder_name: String,
) -> Result<String, String> {
    let access_token = get_valid_token().await?;
    let client = http_client();
    let day_id = super::gdrive_pool::ensure_today_folder_cached(
        pool, &client, &access_token, &folder_name,
    ).await?;
    upload_image_to_folder(&client, &access_token, &image_path, &day_id).await
}

/// Upload a local image into an existing Drive folder and make it public.
/// Returns the shareable link. Shared by the cached and uncached upload paths.
async fn upload_image_to_folder(
    client: &reqwest::Client,
    access_token: &str,
    image_path: &str,
    day_id: &str,
) -> Result<String, String> {
    let file_data = fs::read(image_path)
        .map_err(|e| format!("Failed to read file: {}", e))?;

    let filename = std::path::Path::new(image_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("image.jpg");

    let metadata = serde_json::json!({
        "name": filename,
        "mimeType": "image/jpeg",
        "parents": [day_id]
    });

    // Google Drive API requires multipart/related (not multipart/form-data)
    let boundary = multipart_boundary();
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    body.extend_from_slice(b"Content-Type: application/json; charset=UTF-8\r\n\r\n");
    body.extend_from_slice(metadata.to_string().as_bytes());
    body.extend_from_slice(format!("\r\n--{}\r\n", boundary).as_bytes());
    body.extend_from_slice(b"Content-Type: image/jpeg\r\n\r\n");
    body.extend_from_slice(&file_data);
    body.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());

    let upload_response: serde_json::Value = client
        .post("https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart&fields=id")
        .bearer_auth(access_token)
        .header("Content-Type", format!("multipart/related; boundary={}", boundary))
        .timeout(std::time::Duration::from_secs(120)) // per-request: big screenshots on slow links (3.19)
        .body(body)
        .send()
        .await
        .map_err(|e| format!("Upload failed: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Failed to parse upload response: {}", e))?;

    let file_id = upload_response["id"]
        .as_str()
        .ok_or(format!("Upload error: {}", serde_json::to_string_pretty(&upload_response).unwrap_or_default()))?;

    // Make public — and confirm it actually succeeded, else the "link" is private
    // and the recipient gets Access Denied (3.11).
    let permission = serde_json::json!({
        "role": "reader",
        "type": "anyone"
    });

    let perm_resp = client
        .post(format!("https://www.googleapis.com/drive/v3/files/{}/permissions", file_id))
        .bearer_auth(access_token)
        .json(&permission)
        .send()
        .await
        .map_err(|e| format!("Failed to set permissions: {}", e))?;
    if !perm_resp.status().is_success() {
        let status = perm_resp.status();
        let body_text = perm_resp.text().await.unwrap_or_default();
        return Err(format!("Failed to make file public ({}): {}", status, body_text));
    }

    let link = format!("https://drive.google.com/uc?id={}&export=view", file_id);
    Ok(link)
}

/// Resolve (creating as needed) the folder_name/YYYY/MM/DD hierarchy for today,
/// returning the day folder id. Shared by the direct upload path.
pub async fn find_or_create_date_folder(
    client: &reqwest::Client,
    token: &str,
    folder_name: &str,
) -> Result<String, String> {
    let now = chrono::Local::now();
    let year = now.format("%Y").to_string();
    let month = now.format("%m").to_string();
    let day = now.format("%d").to_string();
    let root_id = find_or_create_folder(client, token, folder_name, None).await?;
    let year_id = find_or_create_folder(client, token, &year, Some(&root_id)).await?;
    let month_id = find_or_create_folder(client, token, &month, Some(&year_id)).await?;
    find_or_create_folder(client, token, &day, Some(&month_id)).await
}

pub async fn find_or_create_folder(
    client: &reqwest::Client,
    token: &str,
    name: &str,
    parent_id: Option<&str>,
) -> Result<String, String> {
    let parent_query = if let Some(pid) = parent_id {
        format!("'{}' in parents", pid)
    } else {
        "'root' in parents".to_string()
    };

    let query = format!(
        "mimeType='application/vnd.google-apps.folder' and name='{}' and {} and trashed=false",
        escape_drive_query(name), parent_query
    );

    let search_response: serde_json::Value = client
        .get("https://www.googleapis.com/drive/v3/files")
        .bearer_auth(token)
        .query(&[("q", &query), ("fields", &"files(id)".to_string())])
        .send()
        .await
        .map_err(|e| format!("Search failed: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Failed to parse search: {}", e))?;

    if let Some(files) = search_response["files"].as_array() {
        if !files.is_empty() {
            return files[0]["id"].as_str()
                .map(|s| s.to_string())
                .ok_or_else(|| format!("Folder search returned no id: {}", search_response));
        }
    }

    let mut metadata = serde_json::json!({
        "name": name,
        "mimeType": "application/vnd.google-apps.folder"
    });

    if let Some(pid) = parent_id {
        metadata["parents"] = serde_json::json!([pid]);
    }

    let create_response: serde_json::Value = client
        .post("https://www.googleapis.com/drive/v3/files?fields=id")
        .bearer_auth(token)
        .json(&metadata)
        .send()
        .await
        .map_err(|e| format!("Create folder failed: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Failed to parse create: {}", e))?;

    create_response["id"].as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| format!("Create folder returned no id: {}", create_response))
}

#[tauri::command]
pub fn gdrive_has_token() -> bool {
    ensure_loaded();
    GDRIVE_TOKEN.lock().is_some()
}

#[tauri::command]
pub fn gdrive_disconnect(
    pool: tauri::State<'_, super::gdrive_pool::PoolRuntime>,
) -> Result<(), String> {
    *GDRIVE_TOKEN.lock() = None;
    delete_token_from_disk();
    // Stop the pre-allocation daemon before clearing state, so it doesn't keep
    // running (and re-populating) against the now-disconnected account (BUGS#9).
    super::gdrive_pool::stop_daemon(&pool.inner);
    super::gdrive_pool::clear_pool(&pool.inner);
    Ok(())
}

// ── Pooled upload ────────────────────────────────────────────────

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GdriveUploadResult {
    pub url: String,
    pub instant: bool,
}

#[tauri::command]
pub async fn gdrive_upload_pooled(
    window: tauri::Window,
    image_path: String,
    folder_name: String,
    call_id: u64,
    output_scale: f32,
    pool: tauri::State<'_, super::gdrive_pool::PoolRuntime>,
) -> Result<GdriveUploadResult, String> {
    // Reject any path outside the app's temp screenshot dir (BUGS#3).
    super::capture::ensure_temp_screenshot_path(&image_path)?;
    // Working copies are full-res lossless PNG — transcode to JPEG (+ output
    // downscale if enabled) ONCE here so the placeholder PATCH / fallback upload
    // carry the final bytes and downstream paths never re-process them.
    let image_path = super::capture::ensure_jpeg_for_upload(&image_path, output_scale)?;

    let file_data = std::fs::read(&image_path)
        .map_err(|e| format!("Failed to read file: {}", e))?;

    let filename = std::path::Path::new(&image_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("image.jpg")
        .to_string();

    // Try claiming a pre-allocated placeholder
    if let Some((file_id, url)) = super::gdrive_pool::claim_placeholder(&pool.inner) {
        // Spawn background PATCH to replace placeholder content with real image.
        // Retry a few times; if it can never be filled, fall back to a direct
        // upload and tell the window to use the fresh link so the user never
        // ends up sharing a blank placeholder (BUGS#4b).
        let filename_clone = filename.clone();
        let image_path_clone = image_path.clone();
        let folder_clone = folder_name.clone();
        let window_clone = window.clone();
        let win_label = window.label().to_string();
        let pool_fallback = pool.inner.clone();
        tauri::async_runtime::spawn(async move {
            let mut patched = false;
            for attempt in 1..=3u32 {
                match super::gdrive_pool::replace_placeholder_content(
                    &file_id,
                    file_data.clone(),
                    &filename_clone,
                )
                .await
                {
                    Ok(()) => { patched = true; break; }
                    Err(e) => {
                        crate::log(&format!(
                            "[gdrive_pool] PATCH attempt {} failed for {}: {}",
                            attempt, file_id, e
                        ));
                        tokio::time::sleep(std::time::Duration::from_secs(2 * attempt as u64)).await;
                    }
                }
            }
            if !patched {
                match gdrive_upload_cached(&pool_fallback, image_path_clone, folder_clone).await {
                    Ok(new_url) => {
                        crate::log(&format!("[gdrive_pool] placeholder unfillable, fell back to direct upload: {}", new_url));
                        use tauri::Emitter;
                        // Target ONLY the window that made this request, and tag with
                        // call_id so the frontend can ignore it if a newer upload
                        // (e.g. an edited image) has since replaced the link (3.6).
                        let _ = window_clone.emit_to(
                            win_label.as_str(),
                            "gdrive-url-updated",
                            serde_json::json!({ "callId": call_id, "url": new_url }),
                        );
                    }
                    Err(e) => crate::log(&format!("[gdrive_pool] fallback direct upload failed: {}", e)),
                }
            }
        });

        // Trigger pool replenishment in background
        let pool_arc = pool.inner.clone();
        super::gdrive_pool::trigger_replenish(pool_arc);

        return Ok(GdriveUploadResult {
            url,
            instant: true,
        });
    }

    // No placeholder available — direct upload. Use the cached-folder path and
    // the ALREADY-prepared jpeg (image_path) so we don't transcode/downscale twice.
    let url = gdrive_upload_cached(&pool.inner, image_path, folder_name).await?;
    Ok(GdriveUploadResult {
        url,
        instant: false,
    })
}
