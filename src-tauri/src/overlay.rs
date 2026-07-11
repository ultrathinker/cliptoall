use std::collections::HashMap;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// HWND of the active overlay window (0 = none). Used by main.rs hotkey handler
/// to force a repaint when the user double-presses to toggle mode.
static OVERLAY_HWND: AtomicIsize = AtomicIsize::new(0);

/// Force the overlay window to repaint (called from hotkey handler on mode toggle).
/// Safe to call from any thread — InvalidateRect posts WM_PAINT to the window's queue.
pub fn invalidate_overlay() {
    let raw = OVERLAY_HWND.load(Ordering::SeqCst);
    if raw != 0 {
        unsafe {
            let hwnd = HWND(raw as *mut _);
            let _ = InvalidateRect(hwnd, None, false);
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SelectionRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug)]
pub enum OverlayResult {
    Selection(SelectionRect),
    PluginCall { path: String, function_id: String },
}

// SAFETY: OverlayState is only accessed from the overlay thread.
// The copy_image_flag pointer points to a 'static AtomicBool which is Sync.
unsafe impl Send for OverlayState {}

struct OverlayState {
    hdc_original: HDC,
    hdc_dimmed_pink: HDC,
    hdc_dimmed_green: HDC,
    hdc_back: HDC,
    hbm_original: HBITMAP,
    hbm_dimmed_pink: HBITMAP,
    hbm_dimmed_green: HBITMAP,
    hbm_back: HBITMAP,
    screen_width: i32,
    screen_height: i32,
    is_drawing: bool,
    start_x: i32,
    start_y: i32,
    current_x: i32,
    current_y: i32,
    result: Option<OverlayResult>,
    copy_image_flag: *const AtomicBool,
    /// Plugin key map: VK code → (plugin_path, function_id)
    key_map: HashMap<u16, (String, String)>,
}

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | ((g as u32) << 8) | ((b as u32) << 16))
}

fn loword(l: isize) -> i32 {
    (l & 0xFFFF) as i16 as i32
}

fn hiword(l: isize) -> i32 {
    ((l >> 16) & 0xFFFF) as i16 as i32
}

/// Virtual-key code for Shift (either shift key reports this).
const VK_SHIFT_CODE: u16 = 0x10;

/// Constrain the drag end point to a perfect SQUARE anchored at (start_x,start_y),
/// for Shift-drag. The side = max(|dx|,|dy|) so the square always contains the drag
/// vector, the drag DIRECTION is preserved via the sign of each delta (a zero delta
/// counts as positive), and the side is clamped to the pixels available from the
/// anchor toward the drag direction — clamping the SIDE (not the point) so the
/// selection can never stop being square, nor exceed the [0,max) client area (the
/// crop reads the captured buffer by this rect, so an out-of-bounds rect must be
/// impossible). Pure (no winapi) so it is unit-tested below.
fn constrain_square(start_x: i32, start_y: i32, cur_x: i32, cur_y: i32, max_w: i32, max_h: i32) -> (i32, i32) {
    let dx = cur_x - start_x;
    let dy = cur_y - start_y;
    let sx = if dx < 0 { -1 } else { 1 };
    let sy = if dy < 0 { -1 } else { 1 };
    // Available pixels from the anchor in each direction (client area is 0..max-1).
    let room_x = if sx < 0 { start_x } else { (max_w - 1 - start_x).max(0) };
    let room_y = if sy < 0 { start_y } else { (max_h - 1 - start_y).max(0) };
    let side = dx.abs().max(dy.abs()).min(room_x).min(room_y).max(0);
    (start_x + sx * side, start_y + sy * side)
}

/// The effective selection rect (x, y, w, h) for the current drag — the single
/// source of truth shared by WM_PAINT (preview/label) and WM_LBUTTONUP (result),
/// so the highlighted area, the "W × H" label and the captured region can never
/// disagree. Applies the Shift = square constraint when Shift is held.
///
/// Uses GetKeyState (synchronized with the message currently being processed),
/// NOT GetAsyncKeyState (live physical state), so paint and button-up see the same
/// Shift state as the message stream that drove them.
unsafe fn effective_selection(state: &OverlayState) -> (i32, i32, i32, i32) {
    let (mut ex, mut ey) = (state.current_x, state.current_y);
    let shift_down = (GetKeyState(VK_SHIFT_CODE as i32) as u16 & 0x8000) != 0;
    if shift_down {
        let (sx, sy) = constrain_square(
            state.start_x, state.start_y, ex, ey, state.screen_width, state.screen_height,
        );
        ex = sx;
        ey = sy;
    }
    let x = state.start_x.min(ex);
    let y = state.start_y.min(ey);
    (x, y, (ex - state.start_x).abs(), (ey - state.start_y).abs())
}

/// Show a native Win32 fullscreen overlay for region selection.
/// Blocks until the user selects a region or cancels.
/// Returns Some(SelectionRect) on success, None on cancel.
/// Convert a key string (e.g. "A", "E", "1") to a Windows virtual key code.
fn key_string_to_vk(key: &str) -> Option<u16> {
    if key.len() == 1 {
        let ch = key.chars().next().unwrap().to_ascii_uppercase();
        if ch.is_ascii_alphanumeric() {
            return Some(ch as u16);
        }
    }
    None
}

/// Build VK code → (path, function_id) map from string key map.
pub fn build_vk_key_map(string_map: HashMap<String, (String, String)>) -> HashMap<u16, (String, String)> {
    let mut vk_map = HashMap::new();
    for (key_str, value) in string_map {
        if let Some(vk) = key_string_to_vk(&key_str) {
            vk_map.insert(vk, value);
        }
    }
    vk_map
}

pub fn show_native_overlay(
    pixels: &[u8],
    screen_width: i32,
    screen_height: i32,
    screen_left: i32,
    screen_top: i32,
    copy_image_flag: &'static AtomicBool,
    key_map: HashMap<u16, (String, String)>,
) -> Option<OverlayResult> {
    unsafe {
        let hdc_screen = GetDC(None);

        // Create original bitmap from raw pixels
        let hdc_original = CreateCompatibleDC(hdc_screen);
        let hbm_original = create_bitmap_from_pixels(
            hdc_screen, pixels, screen_width, screen_height,
        );
        SelectObject(hdc_original, hbm_original);

        // Create TWO dimmed bitmaps with very subtle tints so overlay
        // can switch instantly when user double-presses the hotkey.
        // Tint is barely noticeable — just ~3% color shift from neutral dim.

        // Pink-tinted dimmed (for "copy link" mode)
        let hdc_dimmed_pink = CreateCompatibleDC(hdc_screen);
        let mut pink_pixels = pixels.to_vec();
        for chunk in pink_pixels.chunks_exact_mut(4) {
            let b = chunk[0] as u16;
            let g = chunk[1] as u16;
            let r = chunk[2] as u16;
            chunk[0] = (b * 150 / 255) as u8;
            chunk[1] = (g * 148 / 255) as u8;
            chunk[2] = (r * 158 / 255) as u8;
        }
        let hbm_dimmed_pink = create_bitmap_from_pixels(
            hdc_screen, &pink_pixels, screen_width, screen_height,
        );
        SelectObject(hdc_dimmed_pink, hbm_dimmed_pink);
        drop(pink_pixels);

        // Green-tinted dimmed (for "copy image" mode)
        let hdc_dimmed_green = CreateCompatibleDC(hdc_screen);
        let mut green_pixels = pixels.to_vec();
        for chunk in green_pixels.chunks_exact_mut(4) {
            let b = chunk[0] as u16;
            let g = chunk[1] as u16;
            let r = chunk[2] as u16;
            chunk[0] = (b * 150 / 255) as u8;
            chunk[1] = (g * 158 / 255) as u8;
            chunk[2] = (r * 148 / 255) as u8;
        }
        let hbm_dimmed_green = create_bitmap_from_pixels(
            hdc_screen, &green_pixels, screen_width, screen_height,
        );
        SelectObject(hdc_dimmed_green, hbm_dimmed_green);
        drop(green_pixels);

        // Create back buffer for double-buffered rendering
        let hdc_back = CreateCompatibleDC(hdc_screen);
        let hbm_back = CreateCompatibleBitmap(hdc_screen, screen_width, screen_height);
        SelectObject(hdc_back, hbm_back);

        let _ = ReleaseDC(None, hdc_screen);

        // Build state
        let state = Box::new(OverlayState {
            hdc_original: HDC(hdc_original.0),
            hdc_dimmed_pink: HDC(hdc_dimmed_pink.0),
            hdc_dimmed_green: HDC(hdc_dimmed_green.0),
            hdc_back: HDC(hdc_back.0),
            hbm_original,
            hbm_dimmed_pink,
            hbm_dimmed_green,
            hbm_back,
            screen_width,
            screen_height,
            is_drawing: false,
            start_x: 0,
            start_y: 0,
            current_x: 0,
            current_y: 0,
            result: None,
            copy_image_flag: copy_image_flag as *const AtomicBool,
            key_map,
        });
        let state_ptr = Box::into_raw(state);

        // Register window class
        let class_name = w!("ClipToAll_Overlay");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(overlay_wndproc),
            hCursor: LoadCursorW(None, IDC_CROSS).unwrap_or_default(),
            lpszClassName: class_name,
            ..Default::default()
        };
        RegisterClassExW(&wc);

        // Create fullscreen popup window covering virtual screen
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST,
            class_name,
            w!(""),
            WS_POPUP | WS_VISIBLE,
            screen_left,
            screen_top,
            screen_width,
            screen_height,
            None,
            None,
            None,
            Some(state_ptr as *const std::ffi::c_void),
        )
        .unwrap_or_default();

        if hwnd.0.is_null() {
            // Failed to create window — clean up
            let _ = Box::from_raw(state_ptr);
            let _ = DeleteDC(hdc_original);
            let _ = DeleteDC(hdc_dimmed_pink);
            let _ = DeleteDC(hdc_dimmed_green);
            let _ = DeleteDC(hdc_back);
            let _ = DeleteObject(hbm_original);
            let _ = DeleteObject(hbm_dimmed_pink);
            let _ = DeleteObject(hbm_dimmed_green);
            let _ = DeleteObject(hbm_back);
            return None;
        }

        // Store HWND so hotkey handler can force repaint on mode toggle
        OVERLAY_HWND.store(hwnd.0 as isize, Ordering::SeqCst);

        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(hwnd);

        // Message loop — blocks until PostQuitMessage
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        // Extract result before freeing state
        let state = Box::from_raw(state_ptr);
        let result = state.result;

        // Cleanup GDI resources
        let _ = DeleteDC(state.hdc_original);
        let _ = DeleteDC(state.hdc_dimmed_pink);
        let _ = DeleteDC(state.hdc_dimmed_green);
        let _ = DeleteDC(state.hdc_back);
        let _ = DeleteObject(state.hbm_original);
        let _ = DeleteObject(state.hbm_dimmed_pink);
        let _ = DeleteObject(state.hbm_dimmed_green);
        let _ = DeleteObject(state.hbm_back);

        // Unregister window class
        let _ = UnregisterClassW(class_name, None);

        result
    }
}

unsafe fn create_bitmap_from_pixels(
    hdc: HDC, pixels: &[u8], width: i32, height: i32,
) -> HBITMAP {
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height, // Top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut bits_ptr: *mut std::ffi::c_void = ptr::null_mut();
    let hbitmap = CreateDIBSection(hdc, &bmi, DIB_RGB_COLORS, &mut bits_ptr, None, 0)
        .unwrap_or_default();

    if !bits_ptr.is_null() {
        ptr::copy_nonoverlapping(pixels.as_ptr(), bits_ptr as *mut u8, pixels.len());
    }

    hbitmap
}

unsafe extern "system" fn overlay_wndproc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            let cs = lparam.0 as *const CREATESTRUCTW;
            if !cs.is_null() {
                let state_ptr = (*cs).lpCreateParams as isize;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_ERASEBKGND => LRESULT(1), // Prevent flicker

        WM_PAINT => {
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut OverlayState;
            if state_ptr.is_null() {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }
            let state = &*state_ptr;

            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);

            let w = state.screen_width;
            let h = state.screen_height;

            // 1. Draw dimmed screenshot to back buffer (pick tint based on current mode)
            let is_copy_image = !state.copy_image_flag.is_null()
                && (*state.copy_image_flag).load(Ordering::Relaxed);
            let hdc_dimmed = if is_copy_image { state.hdc_dimmed_green } else { state.hdc_dimmed_pink };
            let _ = BitBlt(state.hdc_back, 0, 0, w, h, hdc_dimmed, 0, 0, SRCCOPY);

            // 2. If selection, draw bright original in selection area
            if state.is_drawing {
                let (sel_x, sel_y, sel_w, sel_h) = effective_selection(state);

                if sel_w > 1 && sel_h > 1 {
                    // BitBlt only the selection area from original
                    let _ = BitBlt(
                        state.hdc_back, sel_x, sel_y, sel_w, sel_h,
                        state.hdc_original, sel_x, sel_y, SRCCOPY,
                    );

                    // Color depends on mode: green for copy-image, crimson for normal
                    let is_copy_image = !state.copy_image_flag.is_null()
                        && (*state.copy_image_flag).load(Ordering::Relaxed);
                    let border_color = if is_copy_image {
                        rgb(50, 200, 90)   // Green = image copied
                    } else {
                        rgb(200, 50, 90)   // Crimson = normal (link)
                    };

                    // Selection border
                    let pen = CreatePen(PS_SOLID, 2, border_color);
                    let old_pen = SelectObject(state.hdc_back, pen);
                    let null_brush = GetStockObject(NULL_BRUSH);
                    let old_brush = SelectObject(state.hdc_back, null_brush);
                    let _ = Rectangle(state.hdc_back, sel_x, sel_y, sel_x + sel_w, sel_y + sel_h);
                    SelectObject(state.hdc_back, old_pen);
                    SelectObject(state.hdc_back, old_brush);
                    let _ = DeleteObject(pen);

                    // Size label
                    let label = format!("{} \u{00D7} {}", sel_w, sel_h);
                    let wide: Vec<u16> = label.encode_utf16().collect();

                    let _ = SetTextColor(state.hdc_back, border_color);
                    let _ = SetBkMode(state.hdc_back, TRANSPARENT);

                    let font = GetStockObject(DEFAULT_GUI_FONT);
                    let old_font = SelectObject(state.hdc_back, font);

                    let mut text_size = SIZE::default();
                    let _ = GetTextExtentPoint32W(state.hdc_back, &wide, &mut text_size);

                    let label_x = sel_x + sel_w / 2 - text_size.cx / 2;
                    let label_y = if sel_y > 25 { sel_y - 20 } else { sel_y + sel_h + 5 };
                    let _ = TextOutW(state.hdc_back, label_x, label_y, &wide);

                    SelectObject(state.hdc_back, old_font);
                }
            }

            // 3. Flip back buffer to screen
            let _ = BitBlt(hdc, 0, 0, w, h, state.hdc_back, 0, 0, SRCCOPY);

            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }

        WM_LBUTTONDOWN => {
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut OverlayState;
            if state_ptr.is_null() { return LRESULT(0); }
            let state = &mut *state_ptr;

            let x = loword(lparam.0);
            let y = hiword(lparam.0);
            state.is_drawing = true;
            state.start_x = x;
            state.start_y = y;
            state.current_x = x;
            state.current_y = y;
            let _ = SetCapture(hwnd);
            LRESULT(0)
        }

        WM_MOUSEMOVE => {
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut OverlayState;
            if state_ptr.is_null() { return LRESULT(0); }
            let state = &mut *state_ptr;

            if state.is_drawing {
                state.current_x = loword(lparam.0);
                state.current_y = hiword(lparam.0);
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }

        WM_LBUTTONUP => {
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut OverlayState;
            if state_ptr.is_null() { return LRESULT(0); }
            let state = &mut *state_ptr;

            if state.is_drawing {
                state.is_drawing = false;
                let _ = ReleaseCapture();

                // Same shared helper as WM_PAINT → the captured region matches the
                // square the user was shown (incl. the Shift constraint).
                let (x, y, w, h) = effective_selection(state);

                if w >= 5 && h >= 5 {
                    state.result = Some(OverlayResult::Selection(SelectionRect {
                        x, y, width: w, height: h,
                    }));
                    let _ = DestroyWindow(hwnd);
                } else {
                    // Too small — treat as cancel
                    let _ = DestroyWindow(hwnd);
                }
            }
            LRESULT(0)
        }

        WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
            // Cancel on right or middle click
            let _ = ReleaseCapture();
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }

        // WM_SYSKEYDOWN fires when Alt is held; WM_KEYDOWN fires otherwise.
        // Handle both so keys work with or without Alt.
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut OverlayState;
            let key = wparam.0 as u16;

            if key == 0x1B {
                // VK_ESCAPE — always cancel
                let _ = ReleaseCapture();
                let _ = DestroyWindow(hwnd);
            } else if key == VK_SHIFT_CODE {
                // Shift pressed → repaint so the square constraint appears live even
                // if the mouse is held still. Skip keyboard autorepeat (lparam bit
                // 30 = the key was already down) to avoid a stream of no-op repaints.
                if !state_ptr.is_null() && (*state_ptr).is_drawing && (lparam.0 & (1 << 30)) == 0 {
                    let _ = InvalidateRect(hwnd, None, false);
                }
            } else if !state_ptr.is_null() {
                let state = &mut *state_ptr;
                // Check plugin key map first
                if let Some((path, func_id)) = state.key_map.get(&key) {
                    let path = path.clone();
                    let func_id = func_id.clone();
                    state.result = Some(OverlayResult::PluginCall { path, function_id: func_id });
                    let _ = ReleaseCapture();
                    let _ = DestroyWindow(hwnd);
                }
            }
            LRESULT(0)
        }

        // Shift released → repaint so the square constraint drops immediately (no
        // mouse move is generated by a key release). WM_SYSKEYUP covers Shift
        // released while Alt is held.
        WM_KEYUP | WM_SYSKEYUP => {
            if (wparam.0 as u16) == VK_SHIFT_CODE {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut OverlayState;
                if !state_ptr.is_null() && (*state_ptr).is_drawing {
                    let _ = InvalidateRect(hwnd, None, false);
                }
            }
            LRESULT(0)
        }

        WM_SETCURSOR => {
            let _ = SetCursor(LoadCursorW(None, IDC_CROSS).unwrap_or_default());
            LRESULT(1)
        }

        WM_DESTROY => {
            OVERLAY_HWND.store(0, Ordering::SeqCst);
            PostQuitMessage(0);
            LRESULT(0)
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::constrain_square;

    #[test]
    fn square_extends_in_drag_direction() {
        // down-right, dx dominant → side 30, anchored at start
        assert_eq!(constrain_square(10, 10, 40, 25, 1000, 1000), (40, 40));
        // up-left → both signs negative, dy dominant → side 80
        assert_eq!(constrain_square(100, 100, 60, 20, 1000, 1000), (20, 20));
        // up-right
        assert_eq!(constrain_square(100, 100, 160, 40, 1000, 1000), (160, 40));
    }

    #[test]
    fn zero_delta_axis_counts_as_positive() {
        // Perfectly vertical drag: dx == 0 → square grows to the right.
        assert_eq!(constrain_square(50, 50, 50, 90, 1000, 1000), (90, 90));
        // Perfectly horizontal drag: dy == 0 → square grows downward.
        assert_eq!(constrain_square(50, 50, 90, 50, 1000, 1000), (90, 90));
    }

    #[test]
    fn side_is_clamped_to_screen_but_stays_square() {
        // Anchored near the right edge, long downward drag: room_x limits the side
        // to 9, and the result is still a square (equal offset on both axes).
        assert_eq!(constrain_square(990, 10, 995, 900, 1000, 1000), (999, 19));
        // Near the top edge dragging up: room_y clamps the side to the anchor's y.
        assert_eq!(constrain_square(500, 8, 400, 0, 1000, 1000), (492, 0));
    }

    #[test]
    fn degenerate_drag_is_zero_sized() {
        assert_eq!(constrain_square(42, 42, 42, 42, 1000, 1000), (42, 42));
    }
}
