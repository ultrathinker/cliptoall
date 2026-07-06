use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, RwLock};
use tauri::{AppHandle, Emitter};

/// In-memory cache of the decrypted settings — avoids re-reading + DPAPI-decrypting
/// the file on every hotkey/tray/crop, and gives a single source of truth.
static SETTINGS_CACHE: RwLock<Option<AppSettings>> = RwLock::new(None);
/// Serializes writers so concurrent saves (Settings window + Results resize debounce)
/// can't interleave read-modify-write and clobber each other (3.12).
static SETTINGS_WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub image_prefix: String,
    pub autorun: bool,
    pub autoclose: bool,
    pub amazon_access_key_id: String,
    pub amazon_secret_access_key: String,
    pub amazon_bucket: String,
    pub amazon_s3folder: String,
    pub amazon_region: String,
    pub logging_on: bool,
    pub storage_type: String,
    pub google_drive_folder: String,
    /// Legacy boolean (kept for back-compat + old-file migration). The canonical
    /// setting is now `output_mode`; this is kept in sync = (output_mode=="resize").
    #[serde(default = "default_true")]
    pub downscale_for_dpi: bool,
    /// How the SHARED image (upload/clipboard/save-as) is produced on HiDPI:
    ///   "off"    — full physical resolution (looks 1.5× bigger in a browser)
    ///   "resize" — downscaled to logical size (smaller; slightly soft in a viewer)
    ///   "exif"   — full resolution + EXIF density tag (browsers show it crisp at
    ///              logical size; non-browser viewers show it full-size)
    /// Empty string = not set → migrated from `downscale_for_dpi` on load.
    #[serde(default = "default_output_mode")]
    pub output_mode: String,
    pub theme: String,
    #[serde(default = "default_results_width")]
    pub results_width: f64,
    #[serde(default = "default_results_height")]
    pub results_height: f64,
    #[serde(default = "default_true")]
    pub skip_upload_in_copy_mode: bool,
    #[serde(default = "default_capture_hotkey")]
    pub capture_hotkey: String,
    #[serde(default = "default_true")]
    pub escape_hides_results: bool,
    #[serde(default = "default_mode_image")]
    pub default_mode: String,
    /// JPEG quality (1-100) applied only at the final output step (upload / save
    /// as .jpg). Capture + editing stay lossless PNG, so this never affects what
    /// you see in the editor — only the shared/uploaded file.
    #[serde(default = "default_jpeg_quality")]
    pub jpeg_quality: u8,
}

fn default_results_width() -> f64 { 850.0 }
fn default_results_height() -> f64 { 190.0 }
fn default_true() -> bool { true }
fn default_capture_hotkey() -> String { "Alt+X".to_string() }
fn default_mode_image() -> String { "image".to_string() }
fn default_jpeg_quality() -> u8 { 85 }
/// Empty sentinel: an absent output_mode is migrated from downscale_for_dpi on load.
fn default_output_mode() -> String { String::new() }

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            image_prefix: "cta_".to_string(),
            autorun: true,
            autoclose: true,
            amazon_access_key_id: String::new(),
            amazon_secret_access_key: String::new(),
            amazon_bucket: "cliptoall".to_string(),
            amazon_s3folder: String::new(),
            amazon_region: "us-west-2".to_string(),
            logging_on: false,
            storage_type: "gdrive".to_string(),
            google_drive_folder: "public-images".to_string(),
            downscale_for_dpi: true,
            output_mode: "resize".to_string(),
            theme: "crimson".to_string(),
            results_width: 850.0,
            results_height: 190.0,
            skip_upload_in_copy_mode: true,
            capture_hotkey: "Alt+X".to_string(),
            escape_hides_results: true,
            default_mode: "image".to_string(),
            jpeg_quality: 85,
        }
    }
}

fn get_settings_path() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("ClipToAll");
    fs::create_dir_all(&path).ok();
    path.push("settings.json");
    path
}

#[tauri::command]
pub fn load_settings() -> Result<AppSettings, String> {
    let path = get_settings_path();
    if !path.exists() {
        let default = AppSettings::default();
        save_settings_to_disk(default.clone())?;
        return Ok(default);
    }
    Ok(load_settings_sync())
}

/// Read + decrypt settings straight from disk (no cache). Returns defaults if
/// the file is missing or unparseable.
fn read_settings_from_disk() -> AppSettings {
    let path = get_settings_path();
    if path.exists() {
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(mut s) = serde_json::from_str::<AppSettings>(&content) {
                decrypt_sensitive_fields(&mut s);
                // Migrate an old settings file (no output_mode) from the legacy
                // downscale_for_dpi boolean: false → "off", true → "resize".
                if s.output_mode.trim().is_empty() {
                    s.output_mode = if s.downscale_for_dpi { "resize" } else { "off" }.to_string();
                }
                return s;
            }
        }
    }
    AppSettings::default()
}

/// Load settings, using the in-memory cache when populated. Used everywhere in
/// Rust (main.rs hotkey/tray/crop) — avoids repeated disk reads + DPAPI decrypts.
pub fn load_settings_sync() -> AppSettings {
    {
        let cache = SETTINGS_CACHE.read().unwrap();
        if let Some(s) = cache.as_ref() {
            return s.clone();
        }
    }
    let loaded = read_settings_from_disk();
    *SETTINGS_CACHE.write().unwrap() = Some(loaded.clone());
    loaded
}

#[tauri::command]
pub fn save_results_window_size(width: f64, height: f64) -> Result<(), String> {
    let mut settings = load_settings_sync();
    settings.results_width = width;
    settings.results_height = height;
    save_settings_to_disk(settings)
}

/// Decrypt DPAPI-protected fields after loading from disk.
/// Fields without the "dpapi:" prefix are returned as-is (plaintext migration).
fn decrypt_sensitive_fields(settings: &mut AppSettings) {
    settings.amazon_access_key_id = crate::utils::dpapi::decrypt_field(&settings.amazon_access_key_id);
    settings.amazon_secret_access_key = crate::utils::dpapi::decrypt_field(&settings.amazon_secret_access_key);
}

/// Tauri command: persist settings AND notify all open windows so they can
/// live-update theme/behaviour without a restart (BUGS#11).
#[tauri::command]
pub fn save_settings(app: AppHandle, settings: AppSettings) -> Result<(), String> {
    save_settings_to_disk(settings.clone())?;
    // Broadcast to every window for live theme/behaviour updates — but strip
    // secrets: they must not travel the event bus to all WebViews (3.8). Each
    // window keeps the secret values it already loaded via load_settings.
    let mut broadcast = settings;
    broadcast.amazon_access_key_id = String::new();
    broadcast.amazon_secret_access_key = String::new();
    let _ = app.emit("settings-changed", &broadcast);
    Ok(())
}

/// Persist settings to disk and refresh cached atomics/autorun.
/// Used both by the command above and internally (no event emission).
pub fn save_settings_to_disk(settings: AppSettings) -> Result<(), String> {
    // Serialize writers so a Results-resize save and a Settings save can't
    // interleave and clobber each other (3.12).
    let _guard = SETTINGS_WRITE_LOCK.lock().unwrap();

    // Normalize + keep the legacy downscale_for_dpi boolean in sync with the
    // canonical output_mode (back-compat if an older build ever reads the file).
    let mut settings = settings;
    if settings.output_mode.trim().is_empty() {
        settings.output_mode = if settings.downscale_for_dpi { "resize" } else { "off" }.to_string();
    }
    settings.downscale_for_dpi = settings.output_mode == "resize";

    // Update cached atomics immediately
    crate::LOGGING_ON.store(settings.logging_on, Ordering::Relaxed);
    crate::DEFAULT_MODE_IS_IMAGE.store(settings.default_mode == "image", Ordering::Relaxed);

    // Update Windows autorun registry entry
    if let Err(e) = crate::utils::autorun::set_autorun(settings.autorun) {
        crate::log(&format!("Autorun registry update failed: {}", e));
    }

    // Encrypt sensitive fields before writing to disk (keep `settings` plaintext
    // for the cache).
    let mut to_save = settings.clone();
    to_save.amazon_access_key_id = crate::utils::dpapi::encrypt_field(&to_save.amazon_access_key_id)
        .map_err(|e| format!("Failed to encrypt Amazon access key: {}", e))?;
    to_save.amazon_secret_access_key = crate::utils::dpapi::encrypt_field(&to_save.amazon_secret_access_key)
        .map_err(|e| format!("Failed to encrypt Amazon secret key: {}", e))?;

    let path = get_settings_path();
    let content = serde_json::to_string_pretty(&to_save)
        .map_err(|e| format!("Failed to serialize settings: {}", e))?;

    // Atomic write: write to a temp file then rename over the target, so a crash
    // mid-write can't leave a truncated settings.json (→ silent reset to defaults).
    let mut tmp = path.clone();
    tmp.set_file_name("settings.json.tmp");
    fs::write(&tmp, &content)
        .map_err(|e| format!("Failed to write settings: {}", e))?;
    fs::rename(&tmp, &path)
        .map_err(|e| format!("Failed to commit settings: {}", e))?;

    // Refresh the cache with the plaintext version.
    *SETTINGS_CACHE.write().unwrap() = Some(settings);
    Ok(())
}
