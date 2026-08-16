//! Keeps one hidden, already-booted Results window standing by so a capture
//! doesn't pay WKWebView's cold-start cost (~1-2s: process spin-up, JS
//! bundle parse, Svelte mount) at the exact moment the user finishes a
//! selection and expects the window to appear.
//!
//! macOS only — Windows' results-window path is unchanged (it builds one per
//! capture, as it always has).
//!
//! Lifecycle: one spare is warmed at startup; `take()` hands it to a capture
//! and immediately after, `main.rs` warms a replacement. Multiple results
//! windows can be open at once (each capture gets its own), so this is a
//! one-deep pool, not a singleton window like the overlay's.

use parking_lot::Mutex;
use tauri::Manager;

static SPARE_LABEL: Mutex<Option<String>> = Mutex::new(None);

/// Create a hidden results window and hold it ready. No-op if one is
/// already standing by. Blocking (window creation), so call off-thread.
pub fn prewarm(app: &tauri::AppHandle) {
    {
        let held = SPARE_LABEL.lock();
        if let Some(label) = held.as_ref() {
            if app.get_webview_window(label).is_some() {
                return;
            }
        }
    }

    let label = format!("results-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let saved = crate::commands::settings::load_settings_sync();
    let w = saved.results_width.max(620.0);
    let h = saved.results_height.max(crate::RESULTS_MIN_HEIGHT);

    // `?spare=1` tells App.svelte this window has no capture yet, so it
    // should wait for the "results-show" event instead of closing itself
    // (its normal reaction to finding no pending image — see App.svelte).
    let built = tauri::WebviewWindowBuilder::new(app, &label, tauri::WebviewUrl::App("/?spare=1".into()))
        .title("ClipToAll")
        .inner_size(w, h)
        .min_inner_size(620.0, crate::RESULTS_MIN_HEIGHT)
        .center()
        .visible(false)
        .build();

    match built {
        Ok(_) => {
            *SPARE_LABEL.lock() = Some(label.clone());
            crate::log(&format!("results_spare: warmed '{}'", label));
        }
        Err(e) => crate::log(&format!("results_spare: prewarm failed: {}", e)),
    }
}

/// Hand over the standing-by window's label, if it's still alive, and
/// re-apply the currently saved size (settings may have changed since it
/// was warmed). Returns None when there's no usable spare — the caller then
/// builds a window the slow way.
pub fn take(app: &tauri::AppHandle) -> Option<String> {
    let label = SPARE_LABEL.lock().take()?;
    let window = app.get_webview_window(&label)?;

    let saved = crate::commands::settings::load_settings_sync();
    let w = saved.results_width.max(620.0);
    let h = saved.results_height.max(crate::RESULTS_MIN_HEIGHT);
    let _ = window.set_size(tauri::LogicalSize::new(w, h));
    let _ = window.center();

    Some(label)
}
