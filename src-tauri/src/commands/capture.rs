use chrono::{Datelike, Timelike};
use base64::Engine;
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::*;

/// Raw screenshot data kept in memory (no file I/O)
pub struct CaptureData {
    pub buffer: Vec<u8>,  // Raw BGRA pixels
    pub width: i32,
    pub height: i32,
    pub left: i32,
    pub top: i32,
}

#[tauri::command]
pub fn read_image_base64(path: String) -> Result<String, String> {
    ensure_temp_screenshot_path(&path)?;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
}

pub(crate) fn ensure_temp_screenshot_path(path: &str) -> Result<(), String> {
    let path = std::path::Path::new(path);
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("Invalid image path: {}", e))?;
    let temp_dir = std::env::temp_dir()
        .canonicalize()
        .map_err(|e| format!("Cannot resolve temp directory: {}", e))?;
    if canonical.parent() != Some(temp_dir.as_path()) {
        return Err("Image path must be in the temp screenshot directory".to_string());
    }

    let filename = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "Invalid image filename".to_string())?;
    let name_lower = filename.to_lowercase();
    let configured_prefix = crate::commands::settings::load_settings_sync().image_prefix;
    let configured_prefix = if configured_prefix.trim().is_empty() {
        "cta_".to_string()
    } else {
        configured_prefix
    };
    let prefix_lower = configured_prefix.to_lowercase();
    let has_expected_prefix =
        name_lower.starts_with("cta_") || name_lower.starts_with(&prefix_lower);
    let has_expected_ext = matches!(
        canonical
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase()),
        Some(ext) if ext == "png" || ext == "jpg" || ext == "jpeg"
    );
    if !has_expected_prefix || !has_expected_ext {
        return Err("Image path is not a ClipToAll temp screenshot".to_string());
    }
    Ok(())
}

/// Encode an RGB image to `path` as JPEG at the given quality (1-100).
/// This is the ONLY place JPEG is produced (upload / save-as), so the capture
/// and editor stay lossless — no JPEG artifacts while viewing/editing.
fn save_jpeg(rgb: &image::RgbImage, path: &std::path::Path, quality: u8) -> Result<(), String> {
    use image::ImageEncoder;
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(file), quality.clamp(1, 100))
        .write_image(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8)
        .map_err(|e| e.to_string())
}

/// User-configured JPEG quality for output (default 85).
fn output_jpeg_quality() -> u8 {
    crate::commands::settings::load_settings_sync().jpeg_quality
}

/// Timestamped output filename with a caller-chosen extension, honoring the
/// user's configured prefix.
fn output_filename_ext(ext: &str) -> String {
    let prefix = crate::commands::settings::load_settings_sync().image_prefix;
    let prefix = if prefix.trim().is_empty() { "cta_".to_string() } else { prefix };
    let t = chrono::Local::now();
    // 12 hex chars (48 bits) of randomness — S3/GDrive keys must not be guessable
    // from the timestamp alone.
    let uuid = uuid::Uuid::new_v4().simple().to_string();
    // Zero-padded so names sort lexicographically and match the frontend's
    // formatDate() (utils.ts): cta_2026_07_05_09_03_01_<12 hex chars>.jpg.
    format!(
        "{}{:04}_{:02}_{:02}_{:02}_{:02}_{:02}_{}.{}",
        prefix, t.year(), t.month(), t.day(), t.hour(), t.minute(), t.second(), &uuid[..12], ext
    )
}

/// The configured output mode: "off" | "resize" | "exif".
fn output_mode() -> String {
    crate::commands::settings::load_settings_sync().output_mode
}

/// Effective OUTPUT downscale factor: the capture monitor's scale (`output_scale`)
/// only in "resize" mode (and when the scale is meaningful, >1.05); otherwise 1.0.
/// "off" and "exif" keep full resolution — "exif" instead records the density in
/// the JPEG's EXIF so browsers display it at logical size without losing pixels.
pub fn effective_output_scale(output_scale: f32) -> f32 {
    if output_scale > 1.05 && output_mode() == "resize" {
        output_scale
    } else {
        1.0
    }
}

/// Stamp EXIF density into a JPEG so browsers (Chrome/Safari/Firefox 90+, per the
/// 2021 WHATWG density-correction) render a full-res HiDPI screenshot at its
/// logical on-screen size while using all pixels — matching the editor's crispness.
/// Best-effort: a failure here leaves a valid untagged JPEG.
fn write_exif_density(path: &std::path::Path, px_w: u32, px_h: u32, scale: f32) {
    use little_exif::exif_tag::ExifTag;
    use little_exif::metadata::Metadata;
    use little_exif::rational::uR64;
    // The WHATWG density correction applies ONLY when ResolutionUnit==2 AND
    //   physicalWidth * 72 / XResolution == PixelXDimension   (exact integer),
    // and it sets the corrected intrinsic size to PixelXDimension×PixelYDimension.
    // So PixelXDimension must be the LOGICAL size (physical/scale), and we pick
    // XResolution = physicalWidth*72 / logicalWidth as a rational so the equality
    // is exact for ANY width (avoids the round(72*scale) divisibility trap).
    let dim_x = ((px_w as f32) / scale).round().max(1.0) as u32;
    let dim_y = ((px_h as f32) / scale).round().max(1.0) as u32;
    let mut md = Metadata::new();
    md.set_tag(ExifTag::XResolution(vec![uR64 { nominator: px_w * 72, denominator: dim_x }]));
    md.set_tag(ExifTag::YResolution(vec![uR64 { nominator: px_h * 72, denominator: dim_y }]));
    md.set_tag(ExifTag::ResolutionUnit(vec![2])); // 2 = inches (required for density correction)
    md.set_tag(ExifTag::ExifImageWidth(vec![dim_x]));   // 0xA002 PixelXDimension (logical)
    md.set_tag(ExifTag::ExifImageHeight(vec![dim_y]));  // 0xA003 PixelYDimension (logical)
    if let Err(e) = md.write_to_file(path) {
        crate::log(&format!("    [output] EXIF density write failed: {}", e));
    }
}

/// Apply the effective output downscale to an RGB image (Lanczos3), or return it
/// unchanged if no downscale is warranted. Used by upload + clipboard paths.
pub fn apply_output_downscale(rgb: image::RgbImage, output_scale: f32) -> image::RgbImage {
    let scale = effective_output_scale(output_scale);
    if scale > 1.05 {
        let nw = (rgb.width() as f32 / scale).round().max(1.0) as u32;
        let nh = (rgb.height() as f32 / scale).round().max(1.0) as u32;
        image::DynamicImage::ImageRgb8(rgb)
            .resize_exact(nw, nh, image::imageops::FilterType::Lanczos3)
            .to_rgb8()
    } else {
        rgb
    }
}

/// Produce a JPEG path suitable for upload from a (lossless PNG) working copy:
/// applies the DPI downscale for output if enabled, then encodes JPEG once at the
/// configured quality. This is the single point where JPEG + output-downscale are
/// applied (no cumulative recompression, and the editor stays full-res/crisp).
pub fn ensure_jpeg_for_upload(path: &str, output_scale: f32) -> Result<String, String> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mode = output_mode();
    let wants_exif = mode == "exif" && output_scale > 1.05;
    // Fast path: already JPEG, "off" mode (no downscale, no EXIF) → use as-is.
    if (ext == "jpg" || ext == "jpeg") && effective_output_scale(output_scale) <= 1.05 && !wants_exif {
        return Ok(path.to_string());
    }
    let img = image::open(path).map_err(|e| format!("Failed to open image for upload: {}", e))?;
    // Downscale only in "resize" mode (no-op otherwise).
    let rgb = apply_output_downscale(img.to_rgb8(), output_scale);
    let out = std::env::temp_dir().join(output_filename_ext("jpg"));
    save_jpeg(&rgb, &out, output_jpeg_quality())?;
    if wants_exif {
        write_exif_density(&out, rgb.width(), rgb.height(), output_scale);
    }
    Ok(out.to_string_lossy().to_string())
}

/// Minimum length a *custom* prefix must have before we trust it for deletion.
/// A 1-2 char prefix (e.g. "a") is far too generic and could match unrelated
/// files in %TEMP%, so such prefixes are ignored and only the built-in `cta_`
/// pattern is cleaned up (BUGS#6).
const MIN_CUSTOM_PREFIX_LEN: usize = 3;

/// Strip a *known* ClipToAll prefix from a (lowercased) filename, returning the
/// remainder. Always honors the built-in `cta_`; honors the user's configured
/// prefix only if it is specific enough (see `MIN_CUSTOM_PREFIX_LEN`). Never
/// matches on an empty/too-short prefix.
fn strip_known_prefix<'a>(name: &'a str, prefix: &str) -> Option<&'a str> {
    if let Some(rest) = name.strip_prefix("cta_") {
        return Some(rest);
    }
    if prefix.len() >= MIN_CUSTOM_PREFIX_LEN {
        if let Some(rest) = name.strip_prefix(prefix) {
            return Some(rest);
        }
    }
    None
}

/// True if the remainder after the prefix matches the app's generated timestamp
/// stem `YYYY_MM_DD_HH_MM_SS_xxx` (see `output_filename_ext`). Requiring this
/// shape means even a valid prefix can only ever match our own files, never an
/// unrelated `<prefix>foo.png` a user happened to drop in %TEMP% (BUGS#6).
fn matches_timestamp_stem(stem: &str) -> bool {
    let parts: Vec<&str> = stem.split('_').collect();
    // 6 date/time groups + at least one random-suffix group.
    if parts.len() < 7 {
        return false;
    }
    const LENS: [usize; 6] = [4, 2, 2, 2, 2, 2];
    for (part, &len) in parts.iter().zip(LENS.iter()) {
        if part.len() != len || !part.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
    }
    !parts[6].is_empty()
}

/// True if `name` (already lowercased) is a file ClipToAll itself generated and
/// is therefore safe to auto-delete: `<prefix>YYYY_MM_DD_HH_MM_SS_xxx.(png|jpg|
/// jpeg)`, or the legacy fixed-name fullscreen bitmap. `prefix` is the (already
/// lowercased) configured image prefix.
fn is_app_screenshot_name(name: &str, prefix: &str) -> bool {
    // Legacy fullscreen bitmap written by older builds (exact name).
    if name == "cliptoall_fullscreen.bmp" {
        return true;
    }
    let Some(rest) = strip_known_prefix(name, prefix) else {
        return false;
    };
    let Some((stem, ext)) = rest.rsplit_once('.') else {
        return false;
    };
    if !matches!(ext, "png" | "jpg" | "jpeg") {
        return false;
    }
    matches_timestamp_stem(stem)
}

/// Delete leftover temp screenshots older than 7 days from our temp subdir and
/// the flat temp dir (older builds wrote there). Called once at startup so the
/// temp folder doesn't grow without bound (BUGS#7).
pub fn cleanup_temp_files() {
    let now = std::time::SystemTime::now();
    let max_age = std::time::Duration::from_secs(7 * 24 * 3600);
    // Only delete files that match the app's own generated filename pattern.
    // A short/empty custom prefix is ignored so cleanup can never sweep away
    // unrelated files that merely share a generic prefix (BUGS#6).
    let prefix = crate::commands::settings::load_settings_sync().image_prefix.to_lowercase();
    let dir = std::env::temp_dir();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if !is_app_screenshot_name(&name, &prefix) { continue; }
            if let Ok(meta) = entry.metadata() {
                if let Ok(modified) = meta.modified() {
                    if now.duration_since(modified).map(|age| age > max_age).unwrap_or(false) {
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
        }
    }
}

/// Capture the screen to memory (no file I/O). Used by native overlay.
pub fn capture_to_memory() -> Result<CaptureData, String> {
    use std::time::Instant;
    let t0 = Instant::now();

    unsafe {
        let screen_width = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let screen_height = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        let screen_left = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let screen_top = GetSystemMetrics(SM_YVIRTUALSCREEN);
        crate::log(&format!("    [capture] screen={}x{} | +{}ms", screen_width, screen_height, t0.elapsed().as_millis()));

        let hdc_screen = GetDC(None);
        let hdc_mem = CreateCompatibleDC(hdc_screen);
        let hbitmap = CreateCompatibleBitmap(hdc_screen, screen_width, screen_height);
        // Keep the DC's original bitmap so we can restore it before deleting ours
        // (MSDN: an object must not be deleted while selected into a DC — 3.9).
        let old_bitmap = SelectObject(hdc_mem, hbitmap);

        // RAII guard: release ALL acquired GDI objects on EVERY exit path,
        // including the early `?`/return paths below. Previously an early return
        // (e.g. BitBlt or GetDIBits failure) leaked the HDCs/HBITMAP, so repeated
        // capture failures exhausted GDI handles (BUGS#5). Drop runs the exact
        // MSDN-ordered teardown once, replacing the old manual cleanup on the
        // success path: restore the DC's original bitmap, delete our bitmap,
        // delete the memory DC, then release the screen DC.
        struct GdiGuard {
            hdc_screen: HDC,
            hdc_mem: HDC,
            hbitmap: HBITMAP,
            old_bitmap: HGDIOBJ,
        }
        impl Drop for GdiGuard {
            fn drop(&mut self) {
                unsafe {
                    // Restore the DC's original bitmap before deleting ours.
                    SelectObject(self.hdc_mem, self.old_bitmap);
                    let _ = DeleteObject(self.hbitmap);
                    let _ = DeleteDC(self.hdc_mem);
                    let _ = ReleaseDC(None, self.hdc_screen);
                }
            }
        }
        let _gdi = GdiGuard { hdc_screen, hdc_mem, hbitmap, old_bitmap };

        BitBlt(hdc_mem, 0, 0, screen_width, screen_height, hdc_screen, screen_left, screen_top, SRCCOPY)
            .ok().ok_or("BitBlt failed")?;
        crate::log(&format!("    [capture] BitBlt done | +{}ms", t0.elapsed().as_millis()));

        let mut bmp_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: screen_width,
                biHeight: -screen_height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut buffer = vec![0u8; (screen_width * screen_height * 4) as usize];
        let scan_lines = GetDIBits(hdc_mem, hbitmap, 0, screen_height as u32,
            Some(buffer.as_mut_ptr() as *mut _), &mut bmp_info, DIB_RGB_COLORS);
        crate::log(&format!("    [capture] GetDIBits done ({} lines) | +{}ms", scan_lines, t0.elapsed().as_millis()));

        // A zero return means GetDIBits failed — the buffer would be black, so
        // report the error instead of silently handing back an empty screenshot.
        // GDI teardown is handled by `_gdi`'s Drop on this early return too.
        if scan_lines == 0 {
            return Err("GetDIBits failed to copy pixels".to_string());
        }

        crate::log(&format!("    [capture] capture_to_memory done | +{}ms", t0.elapsed().as_millis()));
        Ok(CaptureData { buffer, width: screen_width, height: screen_height, left: screen_left, top: screen_top })
    }
}

/// Crop a region from raw BGRA pixel buffer and save as JPEG.
///
/// Returns `(path, applied_scale)`. `applied_scale` is the factor the image was
/// shrunk by (1.0 = not downscaled). The editor multiplies its CSS size by this
/// so it does NOT divide by the DPI a second time — the double compensation was
/// the "image shows smaller than actual" bug (BUGS#3).
///
/// The downscale factor comes from the DPI of the monitor the selection is ON
/// (not the primary/system DPI), so captures on a secondary monitor with a
/// different scale are handled correctly.
pub fn crop_and_save_from_buffer(
    data: &CaptureData,
    sel: &crate::overlay::SelectionRect,
) -> Result<(String, f32), String> {
    let sw = data.width as usize;
    let sh = data.height as usize;

    // Clamp the selection to the buffer bounds. The overlay hands correct
    // coordinates today, but any future/exotic caller with out-of-range values
    // would otherwise index past the buffer and panic the capture thread (3.18).
    let x0 = sel.x.max(0) as usize;
    let y0 = sel.y.max(0) as usize;
    let sel_w = (sel.width.max(0) as usize).min(sw.saturating_sub(x0));
    let sel_h = (sel.height.max(0) as usize).min(sh.saturating_sub(y0));
    if sel_w == 0 || sel_h == 0 {
        return Err("Selection is outside the captured area".to_string());
    }

    // Extract crop region and convert BGRA → RGB
    let mut rgb_buf = Vec::with_capacity(sel_w * sel_h * 3);
    for y in 0..sel_h {
        let src_y = (y0 + y) * sw * 4;
        for x in 0..sel_w {
            let offset = src_y + (x0 + x) * 4;
            rgb_buf.push(data.buffer[offset + 2]); // R
            rgb_buf.push(data.buffer[offset + 1]); // G
            rgb_buf.push(data.buffer[offset]);     // B
        }
    }

    let img = image::RgbImage::from_raw(sel_w as u32, sel_h as u32, rgb_buf)
        .ok_or("Failed to create image from buffer")?;

    // DPI of the monitor the selection sits on (absolute virtual-screen point).
    // We do NOT downscale here — the working copy stays at FULL physical
    // resolution so the editor shows it pixel-for-pixel (crisp). The monitor
    // scale is returned as `output_scale` and the DPI downscale, like JPEG, is
    // applied only at OUTPUT (upload / clipboard) when the setting is on.
    let center_x = data.left + sel.x + sel.width / 2;
    let center_y = data.top + sel.y + sel.height / 2;
    let monitor_scale = get_monitor_scale(center_x, center_y);
    let output_scale = if monitor_scale > 1.05 { monitor_scale } else { 1.0 };

    // Save the capture LOSSLESS (PNG) at full resolution.
    let output_path = std::env::temp_dir().join(output_filename_ext("png"));
    img.save_with_format(&output_path, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;

    Ok((output_path.to_string_lossy().to_string(), output_scale))
}

/// Effective DPI scale (1.0 = 96 dpi/100%) of the monitor containing a point.
fn get_monitor_scale(x: i32, y: i32) -> f32 {
    unsafe {
        let hmon = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut dpi_x: u32 = 96;
        let mut dpi_y: u32 = 96;
        if GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y).is_ok() {
            (dpi_x as f32 / 96.0).max(1.0)
        } else {
            1.0
        }
    }
}

/// Save the editor canvas (PNG base64) as a LOSSLESS PNG working copy. Keeping
/// the working copy in PNG means repeated Edit→Save→Edit cycles no longer
/// recompress a JPEG each time (cumulative quality loss). JPEG encoding happens
/// only at upload/save-as time via `ensure_jpeg_for_upload`/`save_image_to_file`.
#[tauri::command]
pub fn save_image_base64(base64_data: String) -> Result<String, String> {
    let data = base64::engine::general_purpose::STANDARD
        .decode(&base64_data)
        .map_err(|e| format!("Invalid base64: {}", e))?;

    // Validate the bytes are a decodable image, then persist as PNG. The canvas
    // already produces PNG, so re-encoding is lossless.
    let img = image::load_from_memory(&data).map_err(|e| format!("Invalid image: {}", e))?;

    let output_path = std::env::temp_dir().join(output_filename_ext("png"));
    img.save_with_format(&output_path, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;

    Ok(output_path.to_string_lossy().to_string())
}

/// Show a native Win32 Save File dialog and copy the image to the chosen path.
/// Returns Some(path) if saved, None if user cancelled.
#[tauri::command]
pub fn save_image_to_file(source_path: String, output_scale: f32) -> Result<Option<String>, String> {
    use std::path::Path;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Controls::Dialogs::*;

    // Only ever re-encode a genuine ClipToAll temp screenshot. Every other
    // path-taking command validates its input this way; save_image_to_file was
    // the lone exception, letting a caller point it at an arbitrary file to be
    // read and rewritten to the user-chosen destination (Codex finding #1).
    ensure_temp_screenshot_path(&source_path)?;

    let src = Path::new(&source_path);
    let default_name = src.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "screenshot.jpg".to_string());

    // Roomy file-name buffer so long paths aren't truncated (MAX_PATH=260 was
    // the old limit). 4096 UTF-16 units comfortably covers extended paths.
    const FILE_BUF_LEN: usize = 4096;
    let mut file_buf: Vec<u16> = default_name.encode_utf16().collect();
    file_buf.resize(FILE_BUF_LEN, 0);

    // Double-null-terminated filter: pairs of (description\0pattern\0) + final \0
    let filter: Vec<u16> = "JPEG (*.jpg)\0*.jpg\0PNG (*.png)\0*.png\0All Files (*.*)\0*.*\0"
        .encode_utf16().chain(std::iter::once(0)).collect();

    let title: Vec<u16> = "Save image\0".encode_utf16().collect();

    // Own the dialog to the current foreground window so it's modal and never
    // hides behind the Results window.
    let owner = unsafe { GetForegroundWindow() };

    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: windows::core::PWSTR(file_buf.as_mut_ptr()),
        nMaxFile: FILE_BUF_LEN as u32,
        nFilterIndex: 1,
        lpstrTitle: PCWSTR(title.as_ptr()),
        Flags: OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST,
        ..Default::default()
    };

    let ok = unsafe { GetSaveFileNameW(&mut ofn) };
    if !ok.as_bool() {
        return Ok(None);
    }

    let chosen = String::from_utf16_lossy(
        &file_buf[..file_buf.iter().position(|&c| c == 0).unwrap_or(file_buf.len())]
    );

    // Auto-append extension based on selected filter if user didn't type one
    let dest = Path::new(&chosen);
    let final_path = if dest.extension().is_some() {
        chosen
    } else {
        match ofn.nFilterIndex {
            2 => format!("{}.png", chosen),
            _ => format!("{}.jpg", chosen),
        }
    };

    // If saving as PNG, re-encode; otherwise just copy the file
    let dest_ext = Path::new(&final_path).extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    // Transcode to the chosen format (source is a lossless PNG working copy, so
    // JPEG is encoded exactly once here rather than copied byte-for-byte).
    // Honor "resize shared images to logical size" for saved files too, so all
    // output paths (upload / clipboard / save-as) behave consistently. Users who
    // want full-resolution archival saves simply leave that setting off.
    let rgb = apply_output_downscale(image::open(&source_path).map_err(|e| e.to_string())?.to_rgb8(), output_scale);
    if dest_ext == "png" {
        // EXIF density is ignored by browsers for PNG, so PNG save is just full-res.
        rgb.save_with_format(&final_path, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    } else {
        save_jpeg(&rgb, Path::new(&final_path), output_jpeg_quality())?;
        if output_mode() == "exif" && output_scale > 1.05 {
            write_exif_density(Path::new(&final_path), rgb.width(), rgb.height(), output_scale);
        }
    }

    Ok(Some(final_path))
}
