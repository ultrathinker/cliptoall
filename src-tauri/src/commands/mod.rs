pub mod settings;
pub mod capture;
pub mod clipboard;
pub mod upload_s3;
pub mod upload_gdrive;
pub mod gdrive_pool;
pub mod plugins;

/// Reject an IPC call that comes from any window other than the main Settings
/// window. Tauri does NOT gate app-defined commands per-window, so a command
/// registered in the global invoke handler is callable from every WebView
/// (Results/Editor overlays included). Commands that mutate persisted settings,
/// write/delete plugin scripts, or execute code must therefore refuse calls
/// from non-main windows: those windows only render captured images and never
/// legitimately drive settings/plugin management, so a compromised WebView
/// could otherwise redirect uploads or drop-and-run a script. This mirrors the
/// secret-blanking already done in `load_settings` / `load_plugin_configs`.
pub(crate) fn require_main_window(window: &tauri::Window) -> Result<(), String> {
    if window.label() != "main" {
        return Err("This operation is only allowed from the main window".into());
    }
    Ok(())
}
