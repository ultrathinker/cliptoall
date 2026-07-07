// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod aumid;
mod commands;
mod overlay;
mod plugins;
mod utils;

use std::collections::HashMap;
use parking_lot::Mutex; // non-poisoning; lock() returns the guard directly
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// Prevents multiple overlays from stacking when hotkey is pressed rapidly.
static CAPTURE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
/// When true, copy the IMAGE (not link) to clipboard after capture.
/// Set by pressing hotkey again while overlay is active.
static COPY_IMAGE_MODE: AtomicBool = AtomicBool::new(false);
/// Controls whether log() writes to file. Loaded from settings on startup,
/// updated when settings are saved.
pub static LOGGING_ON: AtomicBool = AtomicBool::new(false);
/// Cached default mode: true = "image" (green), false = "link" (pink).
/// Loaded from settings on startup, updated when settings are saved.
pub static DEFAULT_MODE_IS_IMAGE: AtomicBool = AtomicBool::new(true);
/// Tracks the currently registered global shortcut for unregister/reregister.
static CURRENT_SHORTCUT: Mutex<Option<Shortcut>> = Mutex::new(None);
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{TrayIconBuilder, TrayIconEvent, MouseButton, MouseButtonState},
    WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Code, Modifiers, Shortcut, ShortcutState};

/// Stores image paths and flags for newly created results windows.
/// Window fetches its data on mount via get_pending_image command.
struct PendingImage {
    path: String,
    copy_image_mode: bool,
    /// Capture-monitor DPI scale (1.0 = none). The image is stored full-res; this
    /// is applied only at OUTPUT (upload/clipboard) when the "resize shared images"
    /// setting is on. The editor always shows the image 1:1.
    output_scale: f32,
}
struct PendingResults(Mutex<HashMap<String, PendingImage>>);

/// Resolve a log file path under %APPDATA%\ClipToAll\logs (writable even when
/// the app is installed in Program Files, unlike a path next to the exe — BUGS#11).
fn log_file_path(name: &str) -> std::path::PathBuf {
    let mut dir = dirs::config_dir().unwrap_or_else(std::env::temp_dir);
    dir.push("ClipToAll");
    dir.push("logs");
    let _ = std::fs::create_dir_all(&dir);
    dir.push(name);
    dir
}

/// Write a timestamped line to the log file.
/// Only writes if LOGGING_ON is true (controlled by the "Write to Log File" setting).
pub fn log(msg: &str) {
    if !LOGGING_ON.load(Ordering::Relaxed) {
        return;
    }
    use std::io::Write;
    let log_path = log_file_path("cliptoall.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log_path) {
        let now = chrono::Local::now();
        let _ = writeln!(f, "[{:02}:{:02}:{:02}.{:03}] {}",
            now.format("%H"), now.format("%M"), now.format("%S"), now.format("%3f"), msg);
    }
}

/// Capture screen, show native overlay, crop, then open a NEW results window.
fn start_capture(app: AppHandle) {
    // Prevent stacking overlays when Alt+X is pressed rapidly
    if CAPTURE_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        return;
    }

    let t0 = Instant::now();
    log("=== CAPTURE START ===");

    std::thread::spawn(move || {

        // 1. Capture screen to memory (no file I/O)
        log(&format!("  calling capture_to_memory... | +{}ms", t0.elapsed().as_millis()));
        let capture = match commands::capture::capture_to_memory() {
            Ok(c) => c,
            Err(e) => {
                log(&format!("  CAPTURE FAILED: {} | +{}ms", e, t0.elapsed().as_millis()));
                CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
                return;
            }
        };
        log(&format!("  capture_to_memory OK ({}x{}) | +{}ms", capture.width, capture.height, t0.elapsed().as_millis()));

        // 2. Build plugin key map for the overlay
        let key_map = {
            if let Some(state) = app.try_state::<plugins::PluginManagerState>() {
                let mgr = state.0.lock();
                overlay::build_vk_key_map(mgr.get_key_map())
            } else {
                std::collections::HashMap::new()
            }
        };

        // 3. Show native Win32 overlay — blocks until selection or cancel
        log(&format!("  showing native overlay... | +{}ms", t0.elapsed().as_millis()));
        let overlay_result = overlay::show_native_overlay(
            &capture.buffer,
            capture.width,
            capture.height,
            capture.left,
            capture.top,
            &COPY_IMAGE_MODE,
            key_map,
        );
        // Snapshot this capture's final mode BEFORE releasing the in-progress
        // guard — otherwise a rapid next Alt+X could flip COPY_IMAGE_MODE while
        // we're still processing this screenshot (3.17).
        let captured_copy_image = COPY_IMAGE_MODE.load(Ordering::SeqCst);
        // Overlay closed — allow new captures immediately
        CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
        log(&format!("  native overlay returned | +{}ms", t0.elapsed().as_millis()));

        match overlay_result {
            Some(overlay::OverlayResult::PluginCall { path, function_id }) => {
                log(&format!("  plugin call: {} → {} | +{}ms", path, function_id, t0.elapsed().as_millis()));

                // Load plugin settings from config (owned, so we can run the
                // oneshot call WITHOUT holding the manager mutex).
                let plugin_configs = commands::plugins::load_plugin_configs_sync();
                let plugin_settings: Option<String> = plugin_configs.iter()
                    .find(|c| c.path == path)
                    .and_then(|c| if c.settings.is_empty() { None } else { Some(c.settings.clone()) });

                if let Some(state) = app.try_state::<plugins::PluginManagerState>() {
                    // Decide dispatch under a SHORT lock (just a map lookup)...
                    let target = state.0.lock().resolve_call(&path);
                    // ...then execute. Oneshot runs lock-free (bounded by its own
                    // 30s timeout) so a hung script can't wedge the mutex that the
                    // hotkey / Plugins tab / Exit all need. Daemon runs under the
                    // lock but is bounded by its 10s watchdog.
                    let result = match target {
                        Some(plugins::CallTarget::Oneshot { plugin_type }) => Some(
                            plugins::PluginManager::run_oneshot(&path, plugin_type, &function_id, plugin_settings.as_deref())
                        ),
                        Some(plugins::CallTarget::Daemon) => Some(
                            state.0.lock().call_function_daemon(&path, &function_id, plugin_settings.as_deref())
                        ),
                        None => {
                            log(&format!("  plugin not running: {}", path));
                            None
                        }
                    };
                    match result {
                        Some(Ok(result)) => {
                            log(&format!("  plugin result: {:?} | +{}ms", result.status, t0.elapsed().as_millis()));
                            if result.status == "error" {
                                if let Some(msg) = &result.message {
                                    log(&format!("  plugin error: {}", msg));
                                }
                                if result.action.as_deref() == Some("admin_required")
                                    && crate::aumid::show_admin_dialog() {
                                    crate::aumid::restart_as_admin();
                                }
                            }
                        }
                        Some(Err(e)) => {
                            log(&format!("  plugin call failed: {} | +{}ms", e, t0.elapsed().as_millis()));
                        }
                        None => {}
                    }
                }
            }
            Some(overlay::OverlayResult::Selection(sel)) => {
                log(&format!("  selection: {}x{} at ({},{}) | +{}ms", sel.width, sel.height, sel.x, sel.y, t0.elapsed().as_millis()));

                // 3. Crop from memory buffer and save a LOSSLESS full-res PNG.
                //    output_scale = capture-monitor scale, applied only at output.
                match commands::capture::crop_and_save_from_buffer(&capture, &sel) {
                    Ok((image_path, output_scale)) => {
                        log(&format!("  crop+save OK: {} | +{}ms", image_path, t0.elapsed().as_millis()));

                        // Use the mode snapshotted when the overlay returned (3.17).
                        let copy_image = captured_copy_image;
                        if copy_image {
                            log("  copy_image_mode: copying image to clipboard");
                            if let Err(e) = commands::clipboard::copy_image_to_clipboard(image_path.clone(), output_scale) {
                                log(&format!("  copy_image_to_clipboard failed: {}", e));
                            }
                        } else {
                            // Normal mode: clear clipboard so stale image from previous Alt+X+X doesn't linger
                            commands::clipboard::clear_clipboard();
                        }

                        // 4. Store image path + flag and create a NEW results window
                        let window_id = &uuid::Uuid::new_v4().to_string()[..8];
                        let label = format!("results-{}", window_id);

                        app.state::<PendingResults>().0.lock()
                            .insert(label.clone(), PendingImage { path: image_path, copy_image_mode: copy_image, output_scale });

                        // Load saved window size from settings
                        let saved = commands::settings::load_settings_sync();
                        let w = saved.results_width.max(620.0);
                        let h = saved.results_height.max(190.0);

                        match WebviewWindowBuilder::new(
                            &app, &label, WebviewUrl::App("/".into())
                        )
                        .title("ClipToAll")
                        .inner_size(w, h)
                        .min_inner_size(620.0, 190.0)
                        .center()
                        .focused(true)
                        .build()
                        {
                            Ok(win) => {
                                let _ = win.show();
                                let _ = win.set_focus();
                                log(&format!("  new window '{}' created | +{}ms", label, t0.elapsed().as_millis()));
                            }
                            Err(e) => {
                                log(&format!("  WINDOW CREATE FAILED: {} | +{}ms", e, t0.elapsed().as_millis()));
                            }
                        }

                        log(&format!("=== CAPTURE TOTAL: {}ms ===", t0.elapsed().as_millis()));
                    }
                    Err(e) => {
                        log(&format!("  CROP FAILED: {} | +{}ms", e, t0.elapsed().as_millis()));
                    }
                }
            }
            None => {
                log(&format!("  selection cancelled | +{}ms", t0.elapsed().as_millis()));
            }
        }

    });
}

/// Get the pending image data for a newly created results window.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PendingImageResult {
    path: String,
    copy_image_mode: bool,
    output_scale: f32,
}

#[tauri::command]
fn get_pending_image(window: tauri::Window, state: tauri::State<'_, PendingResults>) -> Option<PendingImageResult> {
    // Non-destructive read: the entry is removed when the window is destroyed
    // (see on_window_event), so a WebView reload re-initializes instead of
    // showing a blank window (BUGS#11).
    state.0.lock().get(window.label()).map(|p| PendingImageResult {
        path: p.path.clone(),
        copy_image_mode: p.copy_image_mode,
        output_scale: p.output_scale,
    })
}

#[tauri::command]
fn setup_editor_window(window: tauri::Window) {
    let _ = window.set_decorations(true);
    let _ = window.set_always_on_top(false);
    let _ = window.show();
    let _ = window.set_focus();
    let _ = window.maximize();
}

#[tauri::command]
fn restore_results_window(window: tauri::Window) {
    let saved = commands::settings::load_settings_sync();
    let w = saved.results_width.max(620.0);
    let h = saved.results_height.max(190.0);
    let _ = window.unmaximize();
    let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize { width: w, height: h }));
    let _ = window.center();
}

/// Parse a human-readable hotkey string like "Alt+X", "Ctrl+Shift+F5" into a Shortcut.
fn parse_hotkey(s: &str) -> Result<Shortcut, String> {
    let parts: Vec<&str> = s.split('+').map(|p| p.trim()).collect();
    if parts.is_empty() {
        return Err("Empty hotkey string".to_string());
    }

    let mut modifiers = Modifiers::empty();
    let mut key_part: Option<&str> = None;

    for part in &parts {
        match part.to_lowercase().as_str() {
            "alt" => modifiers |= Modifiers::ALT,
            "ctrl" | "control" => modifiers |= Modifiers::CONTROL,
            "shift" => modifiers |= Modifiers::SHIFT,
            "super" | "win" | "meta" | "cmd" => modifiers |= Modifiers::SUPER,
            _ => {
                if key_part.is_some() {
                    return Err(format!("Multiple non-modifier keys in '{}'", s));
                }
                key_part = Some(part);
            }
        }
    }

    let key_str = key_part.ok_or_else(|| "No key specified (only modifiers)".to_string())?;

    let code = match key_str.to_uppercase().as_str() {
        "A" => Code::KeyA, "B" => Code::KeyB, "C" => Code::KeyC, "D" => Code::KeyD,
        "E" => Code::KeyE, "F" => Code::KeyF, "G" => Code::KeyG, "H" => Code::KeyH,
        "I" => Code::KeyI, "J" => Code::KeyJ, "K" => Code::KeyK, "L" => Code::KeyL,
        "M" => Code::KeyM, "N" => Code::KeyN, "O" => Code::KeyO, "P" => Code::KeyP,
        "Q" => Code::KeyQ, "R" => Code::KeyR, "S" => Code::KeyS, "T" => Code::KeyT,
        "U" => Code::KeyU, "V" => Code::KeyV, "W" => Code::KeyW, "X" => Code::KeyX,
        "Y" => Code::KeyY, "Z" => Code::KeyZ,
        "0" => Code::Digit0, "1" => Code::Digit1, "2" => Code::Digit2, "3" => Code::Digit3,
        "4" => Code::Digit4, "5" => Code::Digit5, "6" => Code::Digit6, "7" => Code::Digit7,
        "8" => Code::Digit8, "9" => Code::Digit9,
        "F1" => Code::F1, "F2" => Code::F2, "F3" => Code::F3, "F4" => Code::F4,
        "F5" => Code::F5, "F6" => Code::F6, "F7" => Code::F7, "F8" => Code::F8,
        "F9" => Code::F9, "F10" => Code::F10, "F11" => Code::F11, "F12" => Code::F12,
        "SPACE" => Code::Space, "ENTER" | "RETURN" => Code::Enter, "TAB" => Code::Tab,
        "PRINTSCREEN" | "PRTSC" => Code::PrintScreen,
        "INSERT" | "INS" => Code::Insert, "DELETE" | "DEL" => Code::Delete,
        "HOME" => Code::Home, "END" => Code::End,
        "PAGEUP" | "PGUP" => Code::PageUp, "PAGEDOWN" | "PGDN" => Code::PageDown,
        _ => return Err(format!("Unknown key: '{}'", key_str)),
    };

    // Require at least one modifier unless it's a function key or PrintScreen
    let standalone_ok = matches!(code,
        Code::F1 | Code::F2 | Code::F3 | Code::F4 | Code::F5 | Code::F6 |
        Code::F7 | Code::F8 | Code::F9 | Code::F10 | Code::F11 | Code::F12 |
        Code::PrintScreen
    );
    if modifiers.is_empty() && !standalone_ok {
        return Err(format!("Key '{}' requires at least one modifier (Alt, Ctrl, Shift)", key_str));
    }

    let mods = if modifiers.is_empty() { None } else { Some(modifiers) };
    Ok(Shortcut::new(mods, code))
}

/// Register a new capture hotkey, then release the old one. Registering FIRST
/// means that if the new combo is already taken by another app, the failure
/// leaves the existing hotkey working instead of dropping it (3.4).
fn register_hotkey(app: &AppHandle, shortcut: Shortcut) -> Result<(), String> {
    let mut current = CURRENT_SHORTCUT.lock();
    if current.as_ref() == Some(&shortcut) {
        return Ok(()); // already registered — nothing to do
    }

    app.global_shortcut().on_shortcut(shortcut, |app, _shortcut, event| {
        if event.state == ShortcutState::Pressed {
            if CAPTURE_IN_PROGRESS.load(Ordering::SeqCst) {
                // Double-press: toggle to the OTHER mode
                let current = COPY_IMAGE_MODE.load(Ordering::SeqCst);
                COPY_IMAGE_MODE.store(!current, Ordering::SeqCst);
                // Force overlay to repaint immediately so tint changes visually
                overlay::invalidate_overlay();
                log(&format!("  Hotkey double-press → toggled to {}", if !current { "copy image" } else { "copy link" }));
                return;
            }
            // Single press: use cached default mode (no disk I/O)
            let default_is_image = DEFAULT_MODE_IS_IMAGE.load(Ordering::Relaxed);
            COPY_IMAGE_MODE.store(default_is_image, Ordering::SeqCst);
            log(&format!("  Hotkey press → default mode: {}", if default_is_image { "image" } else { "link" }));
            start_capture(app.clone());
        }
    }).map_err(|e| format!("Failed to register shortcut: {}", e))?;

    // New one is live — now drop the previous binding.
    if let Some(old) = current.take() {
        let _ = app.global_shortcut().unregister(old);
    }
    *current = Some(shortcut);
    Ok(())
}

#[tauri::command]
fn update_hotkey(app: AppHandle, hotkey: String) -> Result<(), String> {
    let shortcut = parse_hotkey(&hotkey)?;
    register_hotkey(&app, shortcut)?;
    log(&format!("Hotkey updated to: {}", hotkey));
    Ok(())
}

fn main() {
    // Write crash info to %APPDATA%\ClipToAll\logs\cliptoall.crash.log
    std::panic::set_hook(Box::new(|info| {
        use std::io::Write;
        let crash_path = log_file_path("cliptoall.crash.log");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&crash_path) {
            let now = chrono::Local::now();
            let _ = writeln!(f, "[{}] PANIC: {}", now.format("%Y-%m-%d %H:%M:%S"), info);
        }
    }));

    tauri::Builder::default()
        .manage(PendingResults(Mutex::new(HashMap::new())))
        .manage(commands::gdrive_pool::init_pool())
        .manage(plugins::PluginManagerState(Mutex::new(plugins::PluginManager::new())))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.emit("show-settings", ());
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            // Create tray menu (right-click only)
            let settings_item = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
            let about_item = MenuItem::with_id(app, "about", "About", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Exit", true, None::<&str>)?;

            let menu = Menu::with_items(app, &[
                &settings_item,
                &about_item,
                &PredefinedMenuItem::separator(app)?,
                &quit_item,
            ])?;

            // Create tray icon
            let _tray = TrayIconBuilder::new()
                .icon(tauri::include_image!("icons/ClipToAll-32x32.png"))
                .tooltip("ClipToAll")
                .menu(&menu)
                .show_menu_on_left_click(false) // Left click = capture, not menu
                .on_menu_event(|app, event| {
                    match event.id.as_ref() {
                        "settings" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.emit("show-settings", ());
                                let _ = window.set_decorations(true);
                                let _ = window.set_fullscreen(false);
                                let _ = window.set_always_on_top(false);
                                let _ = window.set_resizable(false);
                                let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize { width: 680.0, height: 540.0 }));
                                let _ = window.center();
                                let _ = window.unminimize();
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "about" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.emit("show-about", ());
                                let _ = window.set_decorations(true);
                                let _ = window.set_fullscreen(false);
                                let _ = window.set_always_on_top(false);
                                let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize { width: 540.0, height: 260.0 }));
                                let _ = window.center();
                                let _ = window.unminimize();
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "quit" => {
                            // Stop all plugins before exit
                            if let Some(state) = app.try_state::<plugins::PluginManagerState>() {
                                let mut mgr = state.0.lock();
                                plugins::PluginManager::stop_all(&mut mgr);
                            }
                            std::process::exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    // Left click on tray icon → use default mode from settings,
                    // unless no storage is configured → force copy image mode
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        let settings = commands::settings::load_settings_sync();
                        let storage_configured = if settings.storage_type == "s3" {
                            !settings.amazon_access_key_id.is_empty() && !settings.amazon_secret_access_key.is_empty()
                        } else {
                            commands::upload_gdrive::gdrive_has_token()
                        };
                        if storage_configured {
                            COPY_IMAGE_MODE.store(DEFAULT_MODE_IS_IMAGE.load(Ordering::Relaxed), Ordering::SeqCst);
                        } else {
                            COPY_IMAGE_MODE.store(true, Ordering::SeqCst);
                        }
                        start_capture(tray.app_handle().clone());
                    }
                })
                .build(app)?;

            // Remove stale temp screenshots from previous runs (BUGS#7).
            commands::capture::cleanup_temp_files();

            // Apply settings to cached atomics
            let saved_settings = commands::settings::load_settings_sync();
            LOGGING_ON.store(saved_settings.logging_on, Ordering::Relaxed);
            DEFAULT_MODE_IS_IMAGE.store(saved_settings.default_mode == "image", Ordering::Relaxed);

            // Register global shortcut from settings (default: Alt+X)
            let shortcut = parse_hotkey(&saved_settings.capture_hotkey)
                .unwrap_or_else(|e| {
                    log(&format!("Failed to parse hotkey '{}': {}, falling back to Alt+X",
                        saved_settings.capture_hotkey, e));
                    Shortcut::new(Some(Modifiers::ALT), Code::KeyX)
                });
            let app_handle = app.handle().clone();
            register_hotkey(&app_handle, shortcut)?;

            // Start enabled plugins from saved config
            {
                let plugin_configs = commands::plugins::load_plugin_configs_sync();
                let plugin_state = app.state::<plugins::PluginManagerState>();
                let mut mgr = plugin_state.0.lock();
                for cfg in &plugin_configs {
                    if !cfg.enabled { continue; }
                    if let Err(e) = commands::plugins::ensure_in_plugins_dir(
                        std::path::Path::new(&cfg.path)
                    ) {
                        log(&format!("Plugin skipped due to invalid path {}: {}", cfg.path, e));
                        continue;
                    }

                    let (ptype, mode) = plugins::detect_plugin_type(&cfg.path);
                    match ptype {
                        plugins::PluginType::Exe => {
                            match mgr.start_plugin(&cfg.path, &cfg.key_bindings) {
                                Ok(hello) => log(&format!("Plugin started: {} ({})", hello.name, cfg.path)),
                                Err(e) => log(&format!("Plugin failed to start {}: {}", cfg.path, e)),
                            }
                        }
                        _ => {
                            // Script plugin — read metadata, then start
                            if let Ok(content) = std::fs::read_to_string(&cfg.path) {
                                if let Some((hello, _)) = plugins::parse_script_metadata(&content, ptype) {
                                    match mgr.start_plugin_ext(&cfg.path, ptype, mode, &hello, &cfg.key_bindings) {
                                        Ok(_) => log(&format!("Script plugin started: {} ({})", hello.name, cfg.path)),
                                        Err(e) => log(&format!("Script plugin failed to start {}: {}", cfg.path, e)),
                                    }
                                } else {
                                    log(&format!("Script plugin has no valid metadata: {}", cfg.path));
                                }
                            } else {
                                log(&format!("Failed to read script plugin: {}", cfg.path));
                            }
                        }
                    }
                }
            }

            // Start GDrive pre-allocation daemon after 15s delay (if configured)
            if saved_settings.storage_type == "gdrive" && commands::upload_gdrive::gdrive_has_token() {
                let pool_state = app.state::<commands::gdrive_pool::PoolRuntime>();
                let pool_inner = pool_state.inner.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                    commands::gdrive_pool::start_daemon(pool_inner);
                });
            }

            Ok(())
        })
        // Only prevent close on main window; results windows close normally
        .on_window_event(|window, event| {
            match event {
                WindowEvent::CloseRequested { api, .. } => {
                    if window.label() == "main" {
                        api.prevent_close();
                        let _ = window.hide();
                        // State is managed by explicitly calling handleClose or tray events.
                        // Emitting window-hidden here causes unintended resets during capture cycles.
                    }
                    // Results/editor windows close and destroy normally
                }
                // Free the pending-image entry for a destroyed results window
                // so the map doesn't grow unbounded across captures (BUGS#5/#11).
                WindowEvent::Destroyed if window.label() != "main" => {
                    if let Some(state) = window.try_state::<PendingResults>() {
                        state.0.lock().remove(window.label());
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::settings::load_settings,
            commands::settings::save_settings,
            commands::capture::read_image_base64,
            commands::capture::save_image_base64,
            commands::capture::save_image_to_file,
            commands::upload_s3::upload_to_s3,
            commands::upload_gdrive::gdrive_authorize,
            commands::upload_gdrive::gdrive_upload_pooled,
            commands::upload_gdrive::gdrive_has_token,
            commands::upload_gdrive::gdrive_disconnect,
            commands::clipboard::copy_image_to_clipboard,
            commands::settings::save_results_window_size,
            get_pending_image,
            setup_editor_window,
            restore_results_window,
            update_hotkey,
            commands::plugins::discover_plugins,
            commands::plugins::apply_plugin_config,
            commands::plugins::load_plugin_configs,
            commands::plugins::run_script,
            commands::plugins::run_script_in_terminal,
            commands::plugins::save_script,
            commands::plugins::delete_script,
            commands::plugins::check_runtime,
            commands::plugins::read_script,
            commands::plugins::precompile_script,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkey_with_modifier_ok() {
        assert!(parse_hotkey("Alt+X").is_ok());
        assert!(parse_hotkey("Ctrl+Shift+F5").is_ok());
    }

    #[test]
    fn hotkey_function_key_standalone_ok() {
        assert!(parse_hotkey("F5").is_ok());
        assert!(parse_hotkey("PrintScreen").is_ok());
    }

    #[test]
    fn hotkey_letter_without_modifier_rejected() {
        assert!(parse_hotkey("X").is_err());
    }

    #[test]
    fn hotkey_unknown_key_rejected() {
        assert!(parse_hotkey("Alt+Foo").is_err());
        assert!(parse_hotkey("").is_err());
    }
}
