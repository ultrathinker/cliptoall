//! Web overlay for macOS (Phase 3) — a Svelte canvas
//! (src/windows/OverlayWeb.svelte) running inside an opaque WebviewWindow, in
//! place of the native Win32 overlay. The window's content is the dimmed
//! screenshot drawn onto a canvas covering the whole screen (`100vw` ×
//! `100vh`, `position: fixed`, `inset: 0` in OverlayWeb.svelte), so it is
//! already opaque edge to edge — no NSWindow transparency is needed and
//! `macos-private-api` is intentionally NOT enabled (see
//! docs/macos-port/APP-STORE-PLAN.md §1; that flag would block the App Store
//! build). See docs/macos-port/OVERLAY-SPEC.md for the visual and behavioral
//! contract this is built against.
//!
//! Single-monitor only for now (matches `capture::capture_to_memory`'s
//! current scope — see its doc comment).
//!
//! The window is created ONCE (pre-warmed hidden at startup) and reused —
//! `show`/`hide`, not `build`/`close` — per capture. A fresh WKWebView is
//! expensive to spin up (measured ~2.3s cold-start on this machine: window
//! `build()` returning to the Svelte component's `onMount` actually firing),
//! which dwarfed every other stage combined and was the dominant cause of
//! the reported "overlay takes 1-2s to appear" — the native Win32 overlay
//! has no equivalent cost since it's a bare GDI window, not a WebView.

use crate::geometry::{OverlayResult, SelectionRect};
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::Mutex;
use tauri::Emitter;

/// The screenshot + plugin key map the overlay window fetches (via
/// `overlay_get_meta`/`overlay_get_pixels`) each time it's shown. Only one
/// overlay is ever open at a time (`CAPTURE_IN_PROGRESS` in main.rs gates
/// that), so a single slot is enough.
struct PendingOverlay {
    rgba: Vec<u8>,
    width: i32,
    height: i32,
    key_map: HashMap<String, (String, String)>,
}

static PENDING: Mutex<Option<PendingOverlay>> = Mutex::new(None);
static RESULT_TX: Mutex<Option<mpsc::Sender<Option<OverlayResult>>>> = Mutex::new(None);

/// When the hotkey fired, so every later stage can report its offset from the
/// user's keypress rather than from its own local start. The number that
/// actually matters to a user is "keypress → dimmed screen on screen", and
/// that spans two processes' worth of work (Rust capture, then JS repaint),
/// so no single local `Instant` can measure it.
static HOTKEY_T0: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// Called from `start_capture` the moment the hotkey handler runs.
pub fn mark_capture_start() {
    *HOTKEY_T0.lock().unwrap() = Some(std::time::Instant::now());
}

/// Milliseconds since the hotkey, or `-1` if the clock was never started.
fn since_hotkey_ms() -> i64 {
    HOTKEY_T0
        .lock()
        .unwrap()
        .map(|t| t.elapsed().as_millis() as i64)
        .unwrap_or(-1)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayMetaDto {
    width: i32,
    height: i32,
    mode_is_image: bool,
    key_map: HashMap<String, (String, String)>,
}

/// Only the overlay window itself should ever call these — it's the only
/// window Tauri creates with this label, and Rust drives its visibility
/// (show/hide), but check anyway (matches `require_main_window`'s pattern
/// elsewhere: Tauri doesn't scope app commands per-window on its own, see
/// commands/mod.rs).
fn require_overlay_window(window: &tauri::Window) -> Result<(), String> {
    if window.label() != "overlay" {
        return Err("This operation is only allowed from the overlay window".into());
    }
    Ok(())
}

#[tauri::command]
pub fn overlay_get_meta(window: tauri::Window) -> Result<OverlayMetaDto, String> {
    require_overlay_window(&window)?;
    let pending = PENDING.lock().unwrap();
    let p = pending.as_ref().ok_or_else(|| "No pending overlay screenshot".to_string())?;
    Ok(OverlayMetaDto {
        width: p.width,
        height: p.height,
        mode_is_image: crate::COPY_IMAGE_MODE.load(std::sync::atomic::Ordering::SeqCst),
        key_map: p.key_map.clone(),
    })
}

/// Raw RGBA pixel bytes via Tauri's binary IPC response (JS gets an
/// ArrayBuffer directly) — no PNG encode/decode, no base64/JSON inflation.
/// That round trip alone used to cost ~1.3s combined (PNG encode ~586ms +
/// base64 encode ~114ms + the giant base64-JSON transfer + JS-side
/// `Image.decode()` ~143ms) for a single 2560x1600 capture.
#[tauri::command]
pub fn overlay_get_pixels(window: tauri::Window) -> Result<tauri::ipc::Response, String> {
    require_overlay_window(&window)?;
    let pending = PENDING.lock().unwrap();
    let p = pending.as_ref().ok_or_else(|| "No pending overlay screenshot".to_string())?;
    crate::log(&format!(
        "  overlay_web: JS pulled {} bytes of pixels | HOTKEY+{}ms",
        p.rgba.len(),
        since_hotkey_ms()
    ));
    Ok(tauri::ipc::Response::new(p.rgba.clone()))
}

#[tauri::command]
pub fn overlay_finish(window: tauri::Window, x: i32, y: i32, width: i32, height: i32) -> Result<(), String> {
    require_overlay_window(&window)?;
    send_result(Some(OverlayResult::Selection(SelectionRect { x, y, width, height })))
}

#[tauri::command]
#[cfg(windows)]
pub fn overlay_plugin_call(window: tauri::Window, path: String, function_id: String) -> Result<(), String> {
    require_overlay_window(&window)?;
    send_result(Some(OverlayResult::PluginCall { path, function_id }))
}

#[tauri::command]
pub fn overlay_cancel(window: tauri::Window) -> Result<(), String> {
    require_overlay_window(&window)?;
    send_result(None)
}

/// JS reports the new frame is painted → NOW reveal the window. Showing it
/// any earlier flashes the previous capture's leftover canvas content (the
/// window is reused, not rebuilt, so its last frame is still there).
#[tauri::command]
pub fn overlay_ready(window: tauri::Window) -> Result<(), String> {
    require_overlay_window(&window)?;
    let _ = window.show();
    let _ = window.set_focus();
    crate::log(&format!(
        "  overlay_web: OVERLAY VISIBLE (via JS ready) | HOTKEY+{}ms",
        since_hotkey_ms()
    ));
    Ok(())
}

fn send_result(result: Option<OverlayResult>) -> Result<(), String> {
    let tx = RESULT_TX.lock().unwrap().take();
    match tx {
        Some(tx) => {
            let _ = tx.send(result);
            Ok(())
        }
        None => Err("No overlay waiting for a result".to_string()),
    }
}

/// Bounds of the display we capture, in LOGICAL POINTS — not the physical
/// pixels `CaptureData` uses. Tauri window position/size
/// (`WebviewWindowBuilder`/`Window::set_position`/`set_size` with
/// `LogicalPosition`/`LogicalSize`) are in points, so points is what this
/// must return; converting to pixels here would make the overlay twice the
/// size of the screen on a Retina display.
///
/// Delegates to `sck_capture::capture_display_logical_bounds`, which resolves
/// `CGMainDisplayID()` — deliberately the SAME display the capture path
/// picks. Round 1 read `SCShareableContent.displays().firstObject()` here
/// while the capture path resolved its own display separately; on a
/// multi-monitor setup those can be different screens, which would put the
/// overlay on the wrong one. One helper, one definition of "the display we
/// capture" (HANDOFF §4).
///
/// Falls back to the supplied defaults only if the display mode cannot be
/// read at all.
fn primary_monitor_logical_bounds(fallback_w: i32, fallback_h: i32) -> (f64, f64, f64, f64) {
    // `sck_capture` is `#[cfg(target_os = "macos")]`; this module is
    // `#[cfg(not(windows))]`, so it also parses on Linux, where there is no
    // capture backend to ask (EXECUTION-PLAN §2 — Linux is out of scope but
    // must not break the build).
    #[cfg(target_os = "macos")]
    if let Some(bounds) = crate::sck_capture::capture_display_logical_bounds() {
        return bounds;
    }
    (0.0, 0.0, fallback_w as f64, fallback_h as f64)
}

/// Create the overlay window HIDDEN so its WKWebView is warm (JS bundle
/// loaded, Svelte mounted once) well before the user's first capture.
/// Call once from `main.rs`'s `.setup()`. Safe to call again — a no-op if
/// the window already exists (e.g. if some future caller races this).
pub fn prewarm(app: &tauri::AppHandle) {
    if app.get_webview_window("overlay").is_some() {
        return;
    }
    let t0 = std::time::Instant::now();
    let (left, top, w, h) = primary_monitor_logical_bounds(1920, 1080);
    use tauri::Manager;
    let result = tauri::WebviewWindowBuilder::new(app, "overlay", tauri::WebviewUrl::App("/#overlay".into()))
        .title("")
        .position(left, top)
        .inner_size(w, h)
        .decorations(false)
        // Intentionally NOT transparent — the overlay's visible content is the
        // dimmed screenshot drawn onto a full-window canvas (OverlayWeb.svelte),
        // already opaque edge to edge. NSWindow transparency was only ever here
        // to let the live desktop show through during the wipe-and-reshow gap
        // between captures; with an opaque window the App.svelte <main> /
        // app.css `body.overlay-window` background is what would show during
        // that gap instead. The 400 ms fallback timer in `show_web_overlay`
        // still relies on the window being initially hidden, so removing
        // transparency does not change that path. Removing this call is what
        // lets us drop Tauri's `macos-private-api` Cargo feature, which is
        // the App Store blocker.
        .always_on_top(true)
        .resizable(false)
        .skip_taskbar(true)
        .visible(false)
        .build();
    match result {
        Ok(_) => crate::log(&format!("  overlay_web: prewarm build() done | +{}ms", t0.elapsed().as_millis())),
        Err(e) => crate::log(&format!("  overlay_web: prewarm failed: {}", e)),
    }
}

/// Show the web overlay: reuse the pre-warmed (or lazily created, if
/// `prewarm` hasn't run yet for some reason) WebviewWindow, hand it the
/// screenshot, and block the calling thread until it reports a result.
/// Mirrors `overlay::show_native_overlay`'s blocking contract so
/// `main.rs::start_capture` treats both platforms identically.
pub fn show_web_overlay(
    app: &tauri::AppHandle,
    pixels_rgba: &[u8],
    width: i32,
    height: i32,
    key_map: HashMap<String, (String, String)>,
) -> Option<OverlayResult> {
    use tauri::Manager;
    let t0 = std::time::Instant::now();

    // `capture_to_memory` (macOS) produces RGBA — ScreenCaptureKit hands
    // back a CGImage in BGRA premultiplied, but `sck_capture` draws it
    // through an RGBA bitmap context (see HANDOFF §4.1 / bug #14 on why
    // we never produce BGRA), so the bytes here are already RGBA and no
    // conversion is needed. This is one copy: `capture` in main.rs's
    // start_capture is used again afterward (crop_and_save_from_buffer),
    // so this can't take ownership of it — but a plain memcpy of a few MB
    // is unmeasurable next to what the removed per-pixel channel swap cost.
    let rgba = pixels_rgba.to_vec();
    crate::log(&format!("  overlay_web: pixel buffer copied | +{}ms", t0.elapsed().as_millis()));

    *PENDING.lock().unwrap() = Some(PendingOverlay { rgba, width, height, key_map });

    let (tx, rx) = mpsc::channel::<Option<OverlayResult>>();
    *RESULT_TX.lock().unwrap() = Some(tx);

    let (left, top, logical_w, logical_h) = primary_monitor_logical_bounds(width, height);

    let existing = app.get_webview_window("overlay");
    let window = match existing {
        Some(w) => {
            let _ = w.set_position(tauri::LogicalPosition::new(left, top));
            let _ = w.set_size(tauri::LogicalSize::new(logical_w, logical_h));
            // Deliberately NOT shown here — the window still holds the
            // PREVIOUS capture's frame on its canvas (it's reused, not
            // rebuilt), so showing it now flashes the last selection
            // rectangle for the ~100ms until JS repaints. Instead JS calls
            // `overlay_ready` once the new frame is on the canvas, and THAT
            // shows the window. Safety net below covers a JS that never does.
            let _ = app.emit_to("overlay", "overlay-show", ());
            let fallback = w.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(400));
                if !fallback.is_visible().unwrap_or(true) {
                    crate::log(&format!(
                        "  overlay_web: *** FALLBACK FIRED *** JS never signalled ready within 400ms — \
                         showing the window with WHATEVER is on its canvas, i.e. the PREVIOUS \
                         capture's frame (bug #16 symptom) | HOTKEY+{}ms",
                        since_hotkey_ms()
                    ));
                    let _ = fallback.show();
                    let _ = fallback.set_focus();
                }
            });
            crate::log(&format!("  overlay_web: reused existing window | +{}ms", t0.elapsed().as_millis()));
            w
        }
        None => {
            // Shouldn't normally happen (prewarm runs at startup), but
            // don't leave the user without an overlay if it does.
            crate::log("  overlay_web: no pre-warmed window found, building one now (will be slow)");
            let built = tauri::WebviewWindowBuilder::new(app, "overlay", tauri::WebviewUrl::App("/#overlay".into()))
                .title("")
                .position(left, top)
                .inner_size(logical_w, logical_h)
                .decorations(false)
                // See prewarm() above for why this is not `.transparent(true)`.
                .always_on_top(true)
                .resizable(false)
                .skip_taskbar(true)
                .visible(true)
                .focused(true)
                .build();
            match built {
                Ok(w) => w,
                Err(e) => {
                    crate::log(&format!("  overlay_web: window create failed: {}", e));
                    *PENDING.lock().unwrap() = None;
                    *RESULT_TX.lock().unwrap() = None;
                    return None;
                }
            }
        }
    };

    // Block this (non-async, spawned) thread until a Tauri command sends a
    // result. Safety-net timeout so a stuck/never-responding overlay can't
    // wedge CAPTURE_IN_PROGRESS forever — the native Win32 overlay has no
    // such timeout (its message loop always ends via DestroyWindow), but the
    // web overlay's JS could in principle hang before ever calling back.
    let result = rx.recv_timeout(std::time::Duration::from_secs(600)).ok().flatten();

    *PENDING.lock().unwrap() = None;
    *RESULT_TX.lock().unwrap() = None;
    // Hide, don't close — keeps the WKWebView warm for the next capture.
    let _ = window.hide();

    result
}
