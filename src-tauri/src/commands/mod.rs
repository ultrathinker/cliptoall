pub mod settings;
pub mod capture;
pub mod clipboard;
/// On-device OCR — "Copy text" feature (APP-STORE-PLAN.md §6). macOS-only;
/// the module is gated so the build does not link Vision on other targets
/// and `tauri::generate_handler!` does not try to register a non-existent
/// command on Windows.
#[cfg(target_os = "macos")]
pub mod ocr;
pub mod upload_s3;
pub mod upload_gdrive;
pub mod gdrive_pool;
/// Plugin commands are gated by the `plugins` Cargo feature (TASK B / Phase 4a).
/// The Mac App Store build compiles this module out entirely with
/// `--no-default-features` — see Cargo.toml for the rationale. The
/// `commands::plugins::*` symbols referenced from main.rs's `generate_handler!`
/// and `setup()` are likewise cfg-gated there.
#[cfg(windows)]
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
