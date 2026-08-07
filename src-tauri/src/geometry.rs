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
