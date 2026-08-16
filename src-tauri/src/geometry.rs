//! Platform-agnostic geometry types shared between the capture pipeline and
//! whichever overlay produced the selection (native Win32 overlay on Windows,
//! the web overlay on macOS — see docs/macos-port/PLAN.md Phase 3).

#[derive(Clone, Copy, Debug)]
pub struct SelectionRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// What an overlay (native or web) resolved to: a region selection, a plugin
/// hotkey pressed while the overlay was up, or (via `None` at the call site)
/// a cancel. Shared so `main.rs`'s post-selection handling (crop/save/
/// clipboard/results-window/plugin dispatch) is identical on every platform.
#[derive(Debug)]
pub enum OverlayResult {
    Selection(SelectionRect),
    PluginCall { path: String, function_id: String },
}
