// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod aumid;
mod commands;
mod geometry;
#[cfg(windows)]
mod overlay;
#[cfg(not(windows))]
mod overlay_web;
/// The plugin manager is gated by the `plugins` Cargo feature (TASK B /
/// Phase 4a). The Mac App Store edition compiles this out with
/// `--no-default-features`; every reference below is similarly cfg-gated.
#[cfg(windows)]
mod plugins;
#[cfg(not(windows))]
mod results_spare;
#[cfg(target_os = "macos")]
mod sck_capture;
#[cfg(target_os = "macos")]
mod sck_notifications;
#[cfg(target_os = "macos")]
mod sck_selftest;
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
/// Floor for the Results window's height — below this, the fixed-size
/// content (120px preview box + 30px url input + paddings/gaps on the left,
/// or four 30px action buttons on the right, whichever is taller) no longer
/// fits and the bottom-most control gets visually clipped by
/// `.results-container`'s `overflow: hidden` (all pixel values, not
/// font-dependent, so this floor holds on every platform). The previous
/// 190.0 was already below the ~200px this content actually needs; 210
/// leaves a small margin. Single source of truth for both the window's
/// `min_inner_size` (main.rs) and the saved-settings default (settings.rs).
pub const RESULTS_MIN_HEIGHT: f64 = 210.0;
use tauri::{WebviewUrl, WebviewWindowBuilder};
use tauri::{AppHandle, Emitter, Manager,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{TrayIconBuilder, TrayIconEvent, MouseButton, MouseButtonState},
    WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Code, Modifiers, Shortcut, ShortcutState};

/// Per-window icon handling on Windows.
///
/// WHY THIS EXISTS: Tauri 2 has a bug (tauri#14596) where the runtime window
/// icon is built from ONLY the first entry of the .ico and that single bitmap is
/// then stretched to every size — so the caption (16px) and the taskbar (32/48px)
/// share one poorly-scaled image. We instead set the icons the standard Win32
/// way: pick the frame that matches the size Windows actually wants for each
/// context (SM_CXSMICON for the caption, SM_CXICON for the taskbar/Alt-Tab, both
/// DPI-dependent) out of our multi-size .ico, and assign them separately via
/// WM_SETICON. One embedded multi-size .ico is enough — no need for many files.
#[cfg(windows)]
mod winicon {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateIconFromResourceEx, GetSystemMetrics, SendMessageW, HICON,
        LR_DEFAULTCOLOR, SM_CXICON, SM_CXSMICON, WM_SETICON,
    };

    // The same multi-size icon used for the exe/bundle, embedded so we can build
    // exact-size HICONs at runtime. Frames are PNG-encoded (CreateIconFromResourceEx
    // accepts PNG icon images on Vista+).
    const ICO: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/icons/ClipToAll-all.ico"));
    const ICON_SMALL: usize = 0;
    const ICON_BIG: usize = 1;

    struct Frame { size: u32, data: &'static [u8] }

    fn frames() -> Vec<Frame> {
        let mut out = Vec::new();
        if ICO.len() < 6 { return out; }
        let count = u16::from_le_bytes([ICO[4], ICO[5]]) as usize;
        for i in 0..count {
            let e = 6 + i * 16;
            if e + 16 > ICO.len() { break; }
            let mut w = ICO[e] as u32;
            if w == 0 { w = 256; }
            let len = u32::from_le_bytes([ICO[e + 8], ICO[e + 9], ICO[e + 10], ICO[e + 11]]) as usize;
            let off = u32::from_le_bytes([ICO[e + 12], ICO[e + 13], ICO[e + 14], ICO[e + 15]]) as usize;
            if off + len <= ICO.len() {
                out.push(Frame { size: w, data: &ICO[off..off + len] });
            }
        }
        out
    }

    /// Build an HICON at exactly `target` px, sourced from the frame closest to
    /// (and at least) that size so the scale-down is minimal and crisp.
    fn make_icon(target: i32) -> Option<HICON> {
        let t = target.max(1) as u32;
        let fs = frames();
        if fs.is_empty() { return None; }
        let best = fs.iter().filter(|f| f.size >= t).min_by_key(|f| f.size)
            .or_else(|| fs.iter().max_by_key(|f| f.size))?;
        unsafe {
            CreateIconFromResourceEx(best.data, BOOL(1), 0x0003_0000, target, target, LR_DEFAULTCOLOR).ok()
        }
    }

    pub fn apply(hwnd: HWND) {
        unsafe {
            let small = GetSystemMetrics(SM_CXSMICON);
            let big = GetSystemMetrics(SM_CXICON);
            if let Some(h) = make_icon(small) {
                let _ = SendMessageW(hwnd, WM_SETICON, WPARAM(ICON_SMALL), LPARAM(h.0 as isize));
            }
            if let Some(h) = make_icon(big) {
                let _ = SendMessageW(hwnd, WM_SETICON, WPARAM(ICON_BIG), LPARAM(h.0 as isize));
            }
        }
    }
}

/// Set crisp per-context taskbar/caption icons on a window (Windows only).
#[cfg(windows)]
fn apply_window_icons(window: &tauri::WebviewWindow) {
    // tauri's hwnd() returns an HWND from its own (different) windows-crate
    // version, so rebuild ours from the raw pointer (same underlying *mut c_void).
    if let Ok(hwnd) = window.hwnd() {
        winicon::apply(windows::Win32::Foundation::HWND(hwnd.0));
    }
}

/// No-op on macOS: the .icns bundle icon covers Dock/window chrome, no
/// per-context caption/taskbar icon fixup needed. The trait is kept
/// platform-symmetric so the call sites in `handle_overlay_result` and
/// `setup` (both `#[cfg(windows)]`-gated) don't have to diverge, but no
/// macOS code path actually invokes it — silence the dead-code lint rather
/// than delete the function, since the whole point of having a parallel
/// stub is to keep the call sites identical across `cfg(windows)`.
#[cfg(not(windows))]
#[allow(dead_code)]
fn apply_window_icons(_window: &tauri::WebviewWindow) {}

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

/// Max size of the active log before it is rotated (bytes). Rotation keeps one
/// previous generation (`cliptoall.log.old`), so total on-disk log use is capped
/// at ~2x this (~50 MB) — even if a user leaves "Write to Log File" on forever.
const LOG_MAX_BYTES: u64 = 25 * 1024 * 1024;
/// Serializes log writes so the size check + rotation can't race between threads.
static LOG_LOCK: Mutex<()> = Mutex::new(());

/// Write a timestamped line to the log file.
/// Only writes if LOGGING_ON is true (controlled by the "Write to Log File" setting).
pub fn log(msg: &str) {
    if !LOGGING_ON.load(Ordering::Relaxed) {
        return;
    }
    use std::io::Write;
    let _guard = LOG_LOCK.lock();
    let log_path = log_file_path("cliptoall.log");
    // Rotate when the active log passes the cap: drop the old generation and move
    // the current file to `.old`, then start a fresh one. Keeping one generation
    // bounds disk use while never truncating history mid-run (the most recent
    // lines always survive in `.log`, the run before in `.old`).
    if let Ok(meta) = std::fs::metadata(&log_path) {
        if meta.len() >= LOG_MAX_BYTES {
            let old_path = log_file_path("cliptoall.log.old");
            let _ = std::fs::remove_file(&old_path);
            let _ = std::fs::rename(&log_path, &old_path);
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log_path) {
        let now = chrono::Local::now();
        let _ = writeln!(f, "[{:02}:{:02}:{:02}.{:03}] {}",
            now.format("%H"), now.format("%M"), now.format("%S"), now.format("%3f"), msg);
    }
}

/// Handle what the overlay (native Win32 on Windows, web on macOS) resolved
/// to: run a plugin call, or crop+save+clipboard+open a results window for a
/// selection, or just log a cancel. Shared between both platforms'
/// `start_capture` so this dispatch can't drift between them — the geometry
/// types (`geometry::OverlayResult`/`SelectionRect`) are already
/// platform-agnostic, `overlay::OverlayResult` on Windows is a `pub use`
/// re-export of the exact same type, not a distinct one.
fn handle_overlay_result(
    app: &AppHandle,
    capture: &commands::capture::CaptureData,
    overlay_result: Option<geometry::OverlayResult>,
    captured_copy_image: bool,
    t0: Instant,
) {
    match overlay_result {
        Some(geometry::OverlayResult::PluginCall { path, function_id }) => {
            // Keep the variant and the arm regardless so the match stays
            // exhaustive on every feature combo. The body is what flips:
            // with `--no-default-features` the App Store build still receives
            // an `OverlayResult::PluginCall` (the variant is in `geometry`,
            // shared), but has no `plugins` module to dispatch to — log and
            // drop. In practice the overlay never sends one (no key bindings
            // are registered when plugins are off), but if a future change
            // forgot to gate the JS side we want a clear log line, not a panic.
            #[cfg(windows)]
            {
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
                                #[cfg(windows)]
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
            #[cfg(not(windows))]
            {
                log(&format!("  plugin call {} → {} ignored: plugins feature is off", path, function_id));
                let _ = (path, function_id);
            }
        }
        Some(geometry::OverlayResult::Selection(sel)) => {
            log(&format!("  selection: {}x{} at ({},{}) | +{}ms", sel.width, sel.height, sel.x, sel.y, t0.elapsed().as_millis()));

            // Crop from memory buffer and save a LOSSLESS full-res PNG.
            // output_scale = capture-monitor scale, applied only at output.
            match commands::capture::crop_and_save_from_buffer(capture, &sel) {
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
                        // Normal mode: clear clipboard so stale image from previous hotkey-double-press doesn't linger
                        commands::clipboard::clear_clipboard();
                    }

                    // Reuse a pre-warmed (hidden, already-booted) results
                    // window if one is standing by — building a fresh
                    // WebviewWindow costs ~1-2s of cold WKWebView startup,
                    // which is exactly the "the results window takes a
                    // second or two to appear" lag. Falls through to
                    // building one when no spare exists (Windows never
                    // pre-warms — its code path is unchanged — and on
                    // macOS the very first capture can outrun the warmer).
                    #[cfg(not(windows))]
                    if let Some(label) = results_spare::take(app) {
                        app.state::<PendingResults>().0.lock()
                            .insert(label.clone(), PendingImage { path: image_path, copy_image_mode: copy_image, output_scale });
                        let _ = app.emit_to(label.as_str(), "results-show", ());
                        log(&format!("  reused pre-warmed window '{}' | +{}ms", label, t0.elapsed().as_millis()));
                        // Immediately start warming the replacement for the
                        // next capture, off-thread so it can't delay this one.
                        let warm_app = app.clone();
                        std::thread::spawn(move || results_spare::prewarm(&warm_app));
                        log(&format!("=== CAPTURE TOTAL: {}ms ===", t0.elapsed().as_millis()));
                        return;
                    }

                    // Store image path + flag and create a NEW results window
                    let window_id = &uuid::Uuid::new_v4().to_string()[..8];
                    let label = format!("results-{}", window_id);

                    app.state::<PendingResults>().0.lock()
                        .insert(label.clone(), PendingImage { path: image_path, copy_image_mode: copy_image, output_scale });

                    // Load saved window size from settings
                    let saved = commands::settings::load_settings_sync();
                    let w = saved.results_width.max(620.0);
                    let h = saved.results_height.max(RESULTS_MIN_HEIGHT);

                    match WebviewWindowBuilder::new(
                        app, &label, WebviewUrl::App("/".into())
                    )
                    .title("ClipToAll")
                    .inner_size(w, h)
                    .min_inner_size(620.0, RESULTS_MIN_HEIGHT)
                    .center()
                    .focused(true)
                    // Created HIDDEN so the user never sees the WebView's blank
                    // white page before Svelte paints. The frontend calls show()
                    // once the themed UI is rendered (App.svelte). A fallback
                    // below reveals it anyway if the frontend never signals.
                    .visible(false)
                    .build()
                    {
                        Ok(win) => {
                            // Crisp per-size caption/taskbar icons (see winicon).
                            #[cfg(windows)]
                            apply_window_icons(&win);
                            // Safety net: if the frontend fails to load / never
                            // signals ready, show the window anyway after a short
                            // delay so it can't stay invisible forever.
                            let win_fallback = win.clone();
                            tauri::async_runtime::spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
                                if !win_fallback.is_visible().unwrap_or(true) {
                                    let _ = win_fallback.show();
                                    let _ = win_fallback.set_focus();
                                }
                            });
                            log(&format!("  new window '{}' created (hidden; shown when ready) | +{}ms", label, t0.elapsed().as_millis()));
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
}

/// Capture screen, show native overlay, crop, then open a NEW results window.
#[cfg(windows)]
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

        // 2. Build plugin key map for the overlay. Gated by the `plugins`
        // feature (TASK B): when plugins are off, no hotkey bindings exist,
        // so the overlay receives an empty map and `overlay_plugin_call` is
        // never reached.
        let key_map = {
            #[cfg(windows)]
            {
                if let Some(state) = app.try_state::<plugins::PluginManagerState>() {
                    let mgr = state.0.lock();
                    overlay::build_vk_key_map(mgr.get_key_map())
                } else {
                    std::collections::HashMap::new()
                }
            }
            #[cfg(not(windows))]
            {
                std::collections::HashMap::<String, (String, String)>::new()
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

        handle_overlay_result(&app, &capture, overlay_result, captured_copy_image, t0);
    });
}

/// Capture screen, show the web overlay, crop, then open a NEW results window.
#[cfg(not(windows))]
fn start_capture(app: AppHandle) {
    if CAPTURE_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        return;
    }

    let t0 = Instant::now();
    overlay_web::mark_capture_start();
    log("=== CAPTURE START ===");

    // Screen-Recording TCC preflight (Phase 3 / TASK 1). Without the grant,
    // `SCScreenshotManager.captureImage` silently returns a black buffer and
    // the app looks broken — the single strongest "unfinished" signal an App
    // Store reviewer hits on first launch. We check here, on the hotkey
    // path, so a user who never tries to capture is never prompted. The
    // existing capture-failure path remains as the fallback for the rare
    // case where TCC has been revoked between preflight and capture.
    #[cfg(target_os = "macos")]
    {
        if !sck_capture::screen_capture_access_granted() {
            log("  preflight: Screen Recording not granted — requesting");
            // Ask the system FIRST, before pointing the user at System
            // Settings. This is not just politeness: an app is listed under
            // Privacy & Security → Screen Recording only once it has actually
            // requested the grant. Sending the user to that pane without
            // requesting can show them a list our app is not in — which reads
            // as broken far worse than the original silent failure did.
            //
            // The call also raises the system's own prompt on a first run, and
            // returns immediately (the user's later answer does not change the
            // return value), so it is safe on this thread.
            if sck_capture::request_screen_capture_access() {
                // Already granted between preflight and request — nothing to
                // explain, fall through and capture.
                log("  preflight: granted on request, continuing");
            } else {
                // Make sure the user can SEE the explanation: the main window
                // is hidden by default (this is a tray app), so without
                // showing it the emitted event would fire on a hidden WebView
                // and the dialog would never paint.
                if let Some(main) = app.get_webview_window("main") {
                    let _ = main.show();
                    let _ = main.set_focus();
                }
                let _ = app.emit(
                    "screen-recording-required",
                    serde_json::json!({
                        // Deep link to the Screen Recording privacy pane. If
                        // the schema changes in a future macOS release the
                        // fallback is the parent "Privacy & Security" pane at
                        // `x-apple.systempreferences:com.apple.preference.security`.
                        "settings_url": "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
                    }),
                );
                CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
                return;
            }
        }
    }

    std::thread::spawn(move || {
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

        let key_map = {
            #[cfg(windows)]
            {
                if let Some(state) = app.try_state::<plugins::PluginManagerState>() {
                    let mgr = state.0.lock();
                    mgr.get_key_map()
                } else {
                    std::collections::HashMap::new()
                }
            }
            #[cfg(not(windows))]
            {
                std::collections::HashMap::<String, (String, String)>::new()
            }
        };

        log(&format!("  showing web overlay... | +{}ms", t0.elapsed().as_millis()));
        let overlay_result = overlay_web::show_web_overlay(
            &app,
            &capture.buffer,
            capture.width,
            capture.height,
            key_map,
        );
        let captured_copy_image = COPY_IMAGE_MODE.load(Ordering::SeqCst);
        CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
        log(&format!("  web overlay returned | +{}ms", t0.elapsed().as_millis()));

        handle_overlay_result(&app, &capture, overlay_result, captured_copy_image, t0);
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

/// Open a System Settings privacy pane.
///
/// Deliberately NOT routed through tauri-plugin-opener from the frontend, for
/// two independent reasons:
///
///  * the `opener:allow-open-url` capability granted to the main window allows
///    only `https://**` and `http://**`, so an `x-apple.systempreferences:`
///    link is rejected before it reaches the OS — the button appeared to do
///    nothing at all;
///  * the plugin opens URLs by spawning `/usr/bin/open`, and the App Sandbox
///    forbids spawning external processes. Widening the capability would have
///    moved the failure rather than fixed it.
///
/// `NSWorkspace.openURL:` is the sandbox-safe route: the open is performed by
/// LaunchServices on our behalf, no process is spawned, and no entitlement is
/// required.
///
/// The scheme is checked here rather than trusted from the frontend — this
/// command must not become a general-purpose "open any URL" bypass of the
/// capability system it exists to work around.
#[cfg(target_os = "macos")]
#[tauri::command]
fn open_system_settings(url: String) -> Result<(), String> {
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSString, NSURL};

    if !url.starts_with("x-apple.systempreferences:") {
        return Err(format!("refusing to open a non-System-Settings URL: {}", url));
    }

    let ns_url = NSURL::URLWithString(&NSString::from_str(&url))
        .ok_or_else(|| format!("NSURL could not parse '{}'", url))?;

    // SAFETY: +sharedWorkspace is a documented class method returning a
    // long-lived singleton, and -openURL: takes an NSURL and returns BOOL.
    // Both are checked for existence first: an unrecognised selector is an
    // Objective-C exception, which cannot be caught here and would abort the
    // process (this is exactly how the SMAppService selector mistake crashed
    // the app on launch).
    let cls = objc2::class!(NSWorkspace);
    let responds: bool =
        unsafe { objc2::msg_send![cls, respondsToSelector: objc2::sel!(sharedWorkspace)] };
    if !responds {
        return Err("NSWorkspace does not respond to sharedWorkspace".to_string());
    }
    let workspace: *mut AnyObject = unsafe { objc2::msg_send![cls, sharedWorkspace] };
    if workspace.is_null() {
        return Err("NSWorkspace.sharedWorkspace returned null".to_string());
    }
    let responds: bool =
        unsafe { objc2::msg_send![workspace, respondsToSelector: objc2::sel!(openURL:)] };
    if !responds {
        return Err("NSWorkspace does not respond to openURL:".to_string());
    }
    let opened: bool = unsafe { objc2::msg_send![workspace, openURL: &*ns_url] };
    if opened {
        Ok(())
    } else {
        Err(format!("LaunchServices refused to open '{}'", url))
    }
}

/// Bring up the Settings window on a given tab.
///
/// Exists as a command rather than an `emit` from the frontend on purpose: the
/// Results/Editor windows are denied `core:event:default` precisely so a
/// compromised one cannot broadcast events at the main window (see the
/// description in capabilities/results.json). Routing through Rust keeps that
/// property — the payload is a tab name this function validates, not an
/// arbitrary event.
#[tauri::command]
fn open_settings_tab(app: AppHandle, tab: String) -> Result<(), String> {
    // Whitelist rather than pass through: the frontend switches on this value,
    // and an unknown tab would leave the Settings window on a blank pane.
    let tab = match tab.as_str() {
        "general" | "storage" => tab,
        #[cfg(windows)]
        "plugins" => tab,
        other => return Err(format!("unknown settings tab: {}", other)),
    };
    let main = app
        .get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?;
    let _ = main.unminimize();
    main.show().map_err(|e| e.to_string())?;
    main.set_focus().map_err(|e| e.to_string())?;
    main.emit("show-settings", serde_json::json!({ "tab": tab }))
        .map_err(|e| e.to_string())
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
    let h = saved.results_height.max(RESULTS_MIN_HEIGHT);
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
                #[cfg(windows)]
                overlay::invalidate_overlay();
                #[cfg(not(windows))]
                let _ = app.emit_to("overlay", "overlay-mode-changed", !current);
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
fn update_hotkey(window: tauri::Window, app: AppHandle, hotkey: String) -> Result<(), String> {
    // Re-registering the global capture hotkey is a settings operation; gate it to
    // the main window so a non-main WebView can't sabotage or hijack the hotkey.
    commands::require_main_window(&window)?;
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

    // Self-test path (gated by env var, see SCK migration brief §3.5).
    // This is the only way to verify the new capture path with REAL pixels
    // from the REAL signed app — `cargo test` cannot capture the screen
    // (Screen-Recording TCC grant is tied to the signed application, not
    // to a test binary) and the agent cannot drive the GUI (no
    // Accessibility grant).
    //
    // Run with:
    //   CLIPTOALL_SELFTEST_CAPTURE=1 ./target/<triple>/debug/cliptoall-tauri2
    //
    // Effect: capture several times at startup, compare the result against a
    // `/usr/sbin/screencapture` reference, assert on dimensions / scale /
    // alpha / channel order / colour / shear, write a PNG, then exit with a
    // non-zero status if any assertion failed. Cannot fire during normal use.
    // The logic lives in `sck_selftest` — see that module's header for why it
    // is built around an external oracle rather than around a human looking
    // at a screenshot.
    #[cfg(target_os = "macos")]
    if std::env::var("CLIPTOALL_SELFTEST_CAPTURE").ok().as_deref() == Some("1") {
        // Force the file logger ON so the orchestrator can read timings
        // from ~/Library/Application Support/ClipToAll/logs/cliptoall.log.
        LOGGING_ON.store(true, Ordering::Relaxed);
        // Run synchronously on the main thread — spawning then calling
        // std::process::exit was observed to lose the last log lines. The
        // capture itself blocks on an SCK completion handler, which is
        // delivered on SCK's own dispatch queue, not on this thread, so
        // blocking here does not deadlock (brief §4.6 covers the same point
        // for the real hot path, which runs on a spawned thread).
        let code = sck_selftest::run();
        // Sync the log file so every line above is durable before we exit;
        // std::process::exit() bypasses Drop.
        if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(log_file_path("cliptoall.log")) {
            let _ = f.sync_all();
        }
        std::process::exit(code);
    }

    // Plugin manager state is gated by the `plugins` Cargo feature (TASK B /
// Phase 4a). The cfg attribute cannot sit directly on a `.manage(...)` call
// in a builder chain (the parser attaches it to the prior statement), so the
// conditional manage is hoisted into its own `let` binding instead.
    let builder = tauri::Builder::default()
        .manage(PendingResults(Mutex::new(HashMap::new())))
        .manage(commands::gdrive_pool::init_pool());
    let builder = {
        #[cfg(windows)]
        { builder.manage(plugins::PluginManagerState(Mutex::new(plugins::PluginManager::new()))) }
        #[cfg(not(windows))]
        { builder }
    };
    builder
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.emit("show-settings", ());
            }
        }))
        .plugin(tauri_plugin_opener::init())
        // Save-As native panel on macOS — see commands/capture.rs's
        // `save_image_to_path` for the consumer. The frontend picks the
        // destination via the dialog plugin's `save()` and then invokes the
        // Rust command with that path. No-op on Windows, where the existing
        // Win32 OFN dialog inside `save_image_to_file` stays in charge.
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            // Menu-bar utility, not a windowed application: no Dock icon and no
            // Cmd+Tab entry. macOS defaults every app to `Regular`, which keeps
            // it in the Dock for as long as the process lives, regardless of
            // whether any window is open — unlike Windows, where taskbar
            // presence follows the windows. A tray app has to ask for
            // `Accessory` explicitly.
            //
            // Windows and Linux have no equivalent concept, hence the gate.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // Load settings FIRST so LOGGING_ON is set before anything else runs —
            // then the lifecycle breadcrumbs below actually write WHEN the user has
            // the "Write to Log File" option on (they all go through log(), which is
            // a no-op while the flag is off — nothing is ever logged without it).
            let saved_settings = commands::settings::load_settings_sync();
            LOGGING_ON.store(saved_settings.logging_on, Ordering::Relaxed);
            DEFAULT_MODE_IS_IMAGE.store(saved_settings.default_mode == "image", Ordering::Relaxed);
            log("setup: begin");

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
                            log("tray: Exit requested");
                            // Best-effort graceful plugin stop, but NEVER block Exit on
                            // the plugin lock: if a plugin op is stuck holding it, we must
                            // still exit. try_lock skips the stop rather than hanging; the
                            // plugin children die anyway via the Job Object's
                            // KILL_ON_JOB_CLOSE when this process exits.
                            #[cfg(windows)]
                            if let Some(state) = app.try_state::<plugins::PluginManagerState>() {
                                if let Some(mut mgr) = state.0.try_lock() {
                                    plugins::PluginManager::stop_all(&mut mgr);
                                }
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

            log("setup: tray ready");

            // Give the main window crisp per-size caption/taskbar icons (see winicon).
            #[cfg(windows)]
            if let Some(mw) = app.get_webview_window("main") {
                apply_window_icons(&mw);
            }

            // Register global shortcut from settings (default: Alt+X)
            let shortcut = parse_hotkey(&saved_settings.capture_hotkey)
                .unwrap_or_else(|e| {
                    log(&format!("Failed to parse hotkey '{}': {}, falling back to Alt+X",
                        saved_settings.capture_hotkey, e));
                    Shortcut::new(Some(Modifiers::ALT), Code::KeyX)
                });
            let app_handle = app.handle().clone();
            register_hotkey(&app_handle, shortcut)?;
            log("setup: hotkey registered");

            // Pre-warm the overlay's WebviewWindow (hidden) so its WKWebView
            // is already up and its Svelte component already mounted by the
            // time the user actually captures — spinning one up cold was
            // measured at ~2.3s, the dominant cost in "overlay takes a
            // couple seconds to appear" (see overlay_web.rs's doc comment).
            // Off the main setup thread so it can't delay tray/hotkey
            // readiness; window creation itself is safe from any thread (the
            // results-window path already does this — see start_capture).
            #[cfg(not(windows))]
            {
                let prewarm_app = app.handle().clone();
                std::thread::spawn(move || {
                    // SCShareableContent FIRST (so the overlay prewarm that
                    // reads primary_monitor_logical_bounds has it cached),
                    // then the overlay window, then the Results window.
                    let t0 = std::time::Instant::now();
                    #[cfg(target_os = "macos")]
                    sck_capture::prewarm_capture_backend(t0);
                    overlay_web::prewarm(&prewarm_app);
                    // Same reasoning for the Results window — a cold
                    // WKWebView is what made it appear a second or two
                    // after the selection finished.
                    results_spare::prewarm(&prewarm_app);
                });
            }

            // Install the NSApplicationDidChangeScreenParametersNotification
            // observer so the cached SCShareableContent is dropped *eagerly*
            // when a display is connected/disconnected, the resolution
            // changes, or the lid closes. Without this, the first capture
            // after any of those events is the one that discovers the
            // staleness — either via a captureImage error or, worse, via a
            // silent capture of the wrong screen. Eager invalidation costs at
            // most one extra getShareableContent round-trip on the next
            // capture (~50-70 ms) when the notification fires spuriously.
            // See sck_notifications.rs for the full reasoning.
            #[cfg(target_os = "macos")]
            sck_notifications::install_once();

            // Housekeeping + plugin startup run in a BACKGROUND thread. Both can be
            // slow — cleanup scans %TEMP%, and each plugin's hello handshake can take
            // up to 20s — and NONE of it must delay the Tauri event loop from starting.
            // If this ran on the setup thread (as before) a slow/hung plugin would
            // leave the tray drawn but unresponsive: the OS shows the menu, but no
            // event is processed, so "Exit does nothing". Doing it off-thread keeps
            // the tray/hotkey live from the first moment.
            //
            // The plugin startup block is cfg-gated by the `plugins` Cargo feature
            // (TASK B / Phase 4a). Temp cleanup is shared and stays unconditional;
            // when plugins are off the handle is unused but the closure still runs
            // the cleanup, so suppress the unused warning at the binding.
            #[allow(unused_variables)]
            let bg_app = app.handle().clone();
            std::thread::spawn(move || {
                log("startup(bg): begin");
                // Remove stale temp screenshots from previous runs (BUGS#7).
                commands::capture::cleanup_temp_files();
                log("startup(bg): temp cleanup done");

                #[cfg(windows)]
                {
                    // Start enabled plugins from saved config.
                    let plugin_configs = commands::plugins::load_plugin_configs_sync();
                    let plugin_state = bg_app.state::<plugins::PluginManagerState>();
                    let mut mgr = plugin_state.0.lock();
                    let enabled = plugin_configs.iter().filter(|c| c.enabled).count();
                    log(&format!("startup(bg): starting {} enabled plugin(s)", enabled));
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
                                log(&format!("startup(bg): starting exe plugin {}", cfg.path));
                                match mgr.start_plugin(&cfg.path, &cfg.key_bindings) {
                                    Ok(hello) => log(&format!("Plugin started: {} ({})", hello.name, cfg.path)),
                                    Err(e) => log(&format!("Plugin failed to start {}: {}", cfg.path, e)),
                                }
                            }
                            _ => {
                                // Script plugin — read metadata, then start
                                log(&format!("startup(bg): starting script plugin {}", cfg.path));
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
                    drop(mgr);
                    log("startup(bg): plugin startup complete");
                }
            });

            // Start GDrive pre-allocation daemon after 15s delay (if configured)
            if saved_settings.storage_type == "gdrive" && commands::upload_gdrive::gdrive_has_token() {
                let pool_state = app.state::<commands::gdrive_pool::PoolRuntime>();
                let pool_inner = pool_state.inner.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                    log("gdrive: starting pre-allocation daemon");
                    commands::gdrive_pool::start_daemon(pool_inner);
                });
            }

            log("setup: complete, event loop starting");
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
            #[cfg(target_os = "macos")]
            commands::capture::save_image_to_path,
            #[cfg(target_os = "macos")]
            commands::ocr::recognize_text,
            commands::upload_s3::upload_to_s3,
            commands::upload_gdrive::gdrive_authorize,
            commands::upload_gdrive::gdrive_upload_pooled,
            commands::upload_gdrive::gdrive_has_token,
            commands::upload_gdrive::gdrive_disconnect,
            commands::clipboard::copy_image_to_clipboard,
            commands::settings::save_results_window_size,
            get_pending_image,
            open_settings_tab,
            #[cfg(target_os = "macos")]
            open_system_settings,
            setup_editor_window,
            restore_results_window,
            update_hotkey,
            #[cfg(windows)]
            commands::plugins::discover_plugins,
            #[cfg(windows)]
            commands::plugins::apply_plugin_config,
            #[cfg(windows)]
            commands::plugins::load_plugin_configs,
            #[cfg(windows)]
            commands::plugins::run_script,
            #[cfg(windows)]
            commands::plugins::run_script_in_terminal,
            #[cfg(windows)]
            commands::plugins::save_script,
            #[cfg(windows)]
            commands::plugins::delete_script,
            #[cfg(windows)]
            commands::plugins::check_runtime,
            #[cfg(windows)]
            commands::plugins::read_script,
            #[cfg(windows)]
            commands::plugins::precompile_script,
            #[cfg(not(windows))]
            overlay_web::overlay_get_meta,
            #[cfg(not(windows))]
            overlay_web::overlay_get_pixels,
            #[cfg(not(windows))]
            overlay_web::overlay_finish,
            #[cfg(not(windows))]
            #[cfg(windows)]
            overlay_web::overlay_plugin_call,
            #[cfg(not(windows))]
            overlay_web::overlay_cancel,
            #[cfg(not(windows))]
            overlay_web::overlay_ready,
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
