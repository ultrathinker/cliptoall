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
/// Plugin commands exist only where the plugin system is compiled in:
/// `cfg(plugins)` = Windows always, macOS only with `--features personal`
/// (set by build.rs; see the comment above `[features]` in Cargo.toml). The
/// `commands::plugins::*` symbols referenced from main.rs's `generate_handler!`
/// and `setup()` are likewise cfg-gated there.
#[cfg(plugins)]
pub mod plugins;

/// What this binary was built with, for the frontend: the Settings window
/// shows the Plugins tab only when `plugins` is true, so the UI can never
/// offer a tab whose commands are not registered.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppCapabilities {
    /// The plugin system is compiled in (`cfg(plugins)`).
    plugins: bool,
    /// Where plugins are discovered (None when there is no plugin system).
    plugins_dir: Option<String>,
    /// Directory name of this build's settings/data dir (`app_identity::APP_NAME`).
    app_dir_name: &'static str,
}

#[tauri::command]
pub fn app_capabilities() -> AppCapabilities {
    #[cfg(plugins)]
    let plugins_dir = crate::plugins::PluginManager::plugins_dir()
        .map(|d| d.to_string_lossy().into_owned());
    #[cfg(not(plugins))]
    let plugins_dir = None;
    AppCapabilities {
        plugins: cfg!(plugins),
        plugins_dir,
        app_dir_name: crate::utils::app_identity::APP_NAME,
    }
}

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
