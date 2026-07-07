use std::ptr;
use windows::Win32::Foundation::{GlobalFree, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

const CF_DIB: u32 = 8;

#[repr(C, packed)]
struct BitmapInfoHeader {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_pels_per_meter: i32,
    y_pels_per_meter: i32,
    clr_used: u32,
    clr_important: u32,
}

/// Clear the Windows clipboard.
pub fn clear_clipboard() {
    unsafe {
        if OpenClipboard(HWND(ptr::null_mut())).is_ok() {
            let _ = EmptyClipboard();
            let _ = CloseClipboard();
        }
    }
}

/// Copy an image file to the Windows clipboard as a bitmap.
/// `output_scale` applies the same DPI downscale used for uploads (when enabled),
/// so a pasted image matches the logical on-screen size instead of being oversized
/// on HiDPI captures. The source stays a full-res lossless working copy.
#[tauri::command]
pub fn copy_image_to_clipboard(path: String, output_scale: f32) -> Result<(), String> {
    // Only ever read the app's own temp screenshot — a compromised WebView must
    // not be able to copy an arbitrary readable file to the clipboard (BUGS#3).
    crate::commands::capture::ensure_temp_screenshot_path(&path)?;
    // Decode image, then apply the output downscale (no-op unless enabled + HiDPI).
    let img = image::open(&path).map_err(|e| format!("Failed to open image: {}", e))?;
    let rgb = crate::commands::capture::apply_output_downscale(img.to_rgb8(), output_scale);
    let width = rgb.width() as i32;
    let height = rgb.height() as i32;

    // CF_DIB format: BITMAPINFOHEADER + BGR pixel rows (bottom-up, padded to 4-byte boundary)
    let row_size = ((width * 3 + 3) & !3) as usize;
    let pixel_data_size = row_size * height as usize;
    let header_size = std::mem::size_of::<BitmapInfoHeader>();
    let total_size = header_size + pixel_data_size;

    let header = BitmapInfoHeader {
        size: header_size as u32,
        width,
        height, // positive = bottom-up
        planes: 1,
        bit_count: 24,
        compression: 0,
        size_image: pixel_data_size as u32,
        x_pels_per_meter: 0,
        y_pels_per_meter: 0,
        clr_used: 0,
        clr_important: 0,
    };

    unsafe {
        // Allocate global memory
        let hmem = GlobalAlloc(GMEM_MOVEABLE, total_size)
            .map_err(|e| format!("GlobalAlloc failed: {}", e))?;
        let ptr = GlobalLock(hmem);
        if ptr.is_null() {
            let _ = GlobalFree(hmem);
            return Err("GlobalLock failed".into());
        }

        // Write header
        ptr::copy_nonoverlapping(
            &header as *const BitmapInfoHeader as *const u8,
            ptr as *mut u8,
            header_size,
        );

        // Write pixels: bottom-up, RGB → BGR
        let pixel_base = (ptr as *mut u8).add(header_size);
        for y in 0..height as u32 {
            let src_row = height as u32 - 1 - y; // flip vertically
            let dst_offset = y as usize * row_size;
            for x in 0..width as u32 {
                let pixel = rgb.get_pixel(x, src_row);
                let dst = dst_offset + x as usize * 3;
                // RGB → BGR
                *pixel_base.add(dst) = pixel[2];
                *pixel_base.add(dst + 1) = pixel[1];
                *pixel_base.add(dst + 2) = pixel[0];
            }
        }

        let _ = GlobalUnlock(hmem);

        // Set clipboard. Windows frequently reports the clipboard as busy when a
        // clipboard-manager app holds it — retry a few times before giving up.
        let mut opened = false;
        let mut last_err = String::new();
        for _ in 0..5 {
            match OpenClipboard(HWND(ptr::null_mut())) {
                Ok(_) => { opened = true; break; }
                Err(e) => {
                    last_err = e.to_string();
                    std::thread::sleep(std::time::Duration::from_millis(60));
                }
            }
        }
        if !opened {
            let _ = GlobalFree(hmem);
            return Err(format!("OpenClipboard failed (clipboard busy): {}", last_err));
        }
        let _ = EmptyClipboard();

        let handle = windows::Win32::Foundation::HANDLE(hmem.0);
        if SetClipboardData(CF_DIB, handle).is_err() {
            let _ = CloseClipboard();
            // SetClipboardData failed → ownership NOT transferred, we must free.
            let _ = GlobalFree(hmem);
            return Err("SetClipboardData failed".into());
        }
        // On success the system owns hmem — do NOT free it.

        let _ = CloseClipboard();
    }

    Ok(())
}
