//! ScreenCaptureKit-based screen capture for macOS 14+.
//!
//! Replaces `xcap` / `CGWindowListCreateImage` (deprecated since macOS 14)
//! with `SCScreenshotManager.captureImage`, which is GPU-backed. The
//! xcap baseline was inherited from round-1 reporting at ~270 ms per capture
//! — that figure was measured here, but round 1 was capturing at a
//! quarter of the resolution it thought (HANDOFF bug #21); what gets
//! compared against this baseline is the round-2 number of 79–100 ms
//! end-to-end at the full 2560×1600 backing-store resolution. See
//! HANDOFF.md §4, `mx-sck-report.md`, and the SCK migration brief.
//!
//! ## Architecture
//!
//! `SCShareableContent.getShareableContentWithCompletionHandler:` is an IPC
//! round-trip to the window server plus a TCC check — measured at several
//! ms. Doing that on every capture would eat most of the speedup the
//! migration exists for, so we resolve it ONCE at startup and cache the
//! resulting primary `SCDisplay`. The hot path then just builds a fresh
//! `SCContentFilter` (cheap) and calls `captureImage`.
//!
//! Invalidation: the cache is dropped eagerly by
//! `invalidate_cached_content`, wired in `main.rs` to
//! `NSApplicationDidChangeScreenParametersNotification` (display
//! connected/disconnected, resolution or arrangement changed, lid closed).
//! If one slips through anyway, `captureImage` returns an error, we surface
//! it, and the next capture re-resolves. Silent capture of the wrong display
//! is not acceptable; degraded capture with an error is.
//!
//! Display *dimensions* are not cached at all — `DisplayGeometry` is
//! resolved per capture from `CGDisplayCopyDisplayMode`, which costs
//! microseconds and cannot go stale.
//!
//! ## Pixel format
//!
//! `SCScreenshotManager` hands back a `CGImage` whose pixel order is the
//! framework's choice (BGRA premultiplied on macOS). The overlay needs RGBA
//! (`ImageData` is RGBA by definition; see HANDOFF §4.1 / bug #14 — a
//! Rust-side channel-swap over a 16MP capture costs ~180ms and there used
//! to be TWO of them that cancelled each other out). We avoid producing
//! BGRA at all: the CGImage is drawn into a bitmap context we allocate
//! ourselves with `premultipliedLast | byteOrder32Big`, which places bytes
//! in memory as R, G, B, A. Core Graphics does the conversion in C — no
//! per-pixel loop in Rust, and `crop_and_save_from_buffer`'s `(0, 1, 2)`
//! channel indices (the non-Windows branch) keep working unchanged. The
//! change really is scoped to this one module.
//!
//! **Do not "simplify" `byteOrder32Big` to `byteOrder32Little`.** See
//! HANDOFF bug #22: `premultipliedLast | byteOrder32Little` lays the same
//! 32-bit word down LSB-first, i.e. A, B, G, R in memory. That shipped in
//! round 1 of this migration and passed a visual inspection, because a
//! screen capture's alpha is always 255 and the resulting image is
//! uniformly drenched in bright red — which read as "the red wallpaper".
//!
//! ## Coordinate spaces (read this before touching any dimension)
//!
//! This port's most expensive recurring bug class is logical points vs
//! physical pixels (HANDOFF bugs #7 and #21). Every dimension in this
//! module therefore carries its space in its name (`_px` / `_pts`) and
//! `DisplayGeometry` is the single place both are derived. In short:
//!
//! - `SCDisplay::frame()`, `SCDisplay::width()`, `CGDisplayBounds()`,
//!   `CGDisplayPixelsWide()`, `CGDisplayModeGetWidth()` — all **points**.
//!   `CGDisplayPixelsWide` is named as if it returned pixels; on any HiDPI
//!   display it does not (bug #21).
//! - `CGDisplayModeGetPixelWidth()` / `...PixelHeight()` — the actual
//!   **backing-store pixels**. This is the only CG call that gives them.
//! - `SCStreamConfiguration.width/height`, the returned `CGImage`,
//!   `CaptureData.{buffer,width,height,left,top}` — **physical pixels**.
//! - Tauri window geometry (`overlay_web`) — **points**.

use crate::commands::capture::CaptureData;
use objc2::rc::Retained;
use objc2::AnyThread;
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGDirectDisplayID, CGDisplayBounds,
    CGDisplayCopyDisplayMode, CGDisplayMode, CGImage, CGImageAlphaInfo, CGImageByteOrderInfo,
    CGMainDisplayID, CGPreflightScreenCaptureAccess, CGRequestScreenCaptureAccess,
};
use objc2_foundation::{NSArray, NSError};
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCShareableContent, SCScreenshotManager, SCStreamConfiguration, SCWindow,
};
use block2::RcBlock;
use std::ops::Deref;
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Instant;

// ---------------------------------------------------------------------------
// Display geometry — THE single place logical points and physical pixels are
// derived. Nothing else in this codebase should call a CG display-size API
// directly; add a field here instead.
// ---------------------------------------------------------------------------

/// Everything the capture path and the overlay need to know about the one
/// display we capture, with each number's coordinate space in its name.
///
/// Derived from the display's current *mode* rather than from
/// `CGDisplayPixelsWide/High`, because those two return **points**, not
/// backing-store pixels, on any HiDPI display (HANDOFF bug #21 — that is
/// exactly how round 1 of this migration shipped a quarter-resolution
/// capture and a scale factor of 1.0 on a 2× Retina panel).
#[derive(Debug, Clone, Copy)]
pub struct DisplayGeometry {
    /// CoreGraphics display id of the display we capture.
    pub id: CGDirectDisplayID,
    /// Backing-store width in PHYSICAL PIXELS. 2560 on this machine's
    /// 1280pt-wide 2× Retina panel. `CGDisplayModeGetPixelWidth` is the
    /// only CG accessor that reports this.
    pub width_px: usize,
    /// Backing-store height in PHYSICAL PIXELS. 1600 on this machine.
    pub height_px: usize,
    /// Current mode's width in LOGICAL POINTS. 1280 on this machine.
    pub width_pts: f64,
    /// Current mode's height in LOGICAL POINTS. 800 on this machine.
    pub height_pts: f64,
    /// Display origin in the global LOGICAL POINT space (`CGDisplayBounds`).
    /// (0, 0) for the main display by definition.
    pub origin_x_pts: f64,
    /// Display origin in the global LOGICAL POINT space.
    pub origin_y_pts: f64,
    /// `width_px / width_pts`. Exactly 2.0 on this machine's Retina panel,
    /// 1.0 on a non-HiDPI display. This is the ONLY correct way to get it:
    /// both operands must come from the same display mode, and they must be
    /// the pair that actually differ.
    pub scale: f32,
}

/// The display we capture. One definition, used by the capture path
/// (`pick_primary_display` matches on it) and by the overlay
/// (`overlay_web::primary_monitor_logical_bounds`), so the overlay can never
/// end up sized to a different screen than the one in the buffer.
///
/// `CGMainDisplayID()` is the display with the menu bar — the same one
/// `CGDisplayBounds` reports at origin (0, 0), and the one the user means by
/// "my screen". Note that `SCShareableContent.displays().firstObject()` is
/// *not* guaranteed to be it on a multi-monitor setup, which is why the
/// round-1 code had two different answers to "which display".
pub fn capture_display_id() -> CGDirectDisplayID {
    CGMainDisplayID()
}

/// Resolve `DisplayGeometry` for an arbitrary display id.
pub fn display_geometry(id: CGDirectDisplayID) -> Result<DisplayGeometry, String> {
    let mode = CGDisplayCopyDisplayMode(id)
        .ok_or_else(|| format!("CGDisplayCopyDisplayMode({}) returned None", id))?;
    // PHYSICAL PIXELS — backing store.
    let width_px = CGDisplayMode::pixel_width(Some(&mode));
    let height_px = CGDisplayMode::pixel_height(Some(&mode));
    // LOGICAL POINTS — same mode, so the ratio between the two is the
    // display's real backing scale factor.
    let width_pts = CGDisplayMode::width(Some(&mode)) as f64;
    let height_pts = CGDisplayMode::height(Some(&mode)) as f64;
    // LOGICAL POINTS — position in the global display arrangement.
    let bounds: CGRect = CGDisplayBounds(id);

    if width_px == 0 || height_px == 0 {
        return Err(format!(
            "display {} reported a zero-sized backing store ({}x{} px)",
            id, width_px, height_px
        ));
    }
    if width_pts <= 0.0 || height_pts <= 0.0 {
        return Err(format!(
            "display {} reported a zero-sized point size ({}x{} pts)",
            id, width_pts, height_pts
        ));
    }

    let scale = (width_px as f64 / width_pts) as f32;
    Ok(DisplayGeometry {
        id,
        width_px,
        height_px,
        width_pts,
        height_pts,
        origin_x_pts: bounds.origin.x,
        origin_y_pts: bounds.origin.y,
        scale,
    })
}

/// Resolve `DisplayGeometry` for the display we capture.
pub fn capture_display_geometry() -> Result<DisplayGeometry, String> {
    display_geometry(capture_display_id())
}

/// LOGICAL POINT bounds `(origin_x, origin_y, width, height)` of the display
/// we capture — what `overlay_web` needs, because Tauri window geometry is in
/// points. Returns `None` only if the display mode cannot be read at all.
///
/// Deliberately does NOT go through the `SCShareableContent` cache: it must
/// answer the same question as `capture_display_id()` even before the prewarm
/// has run, and `displays().firstObject()` is not that question (see
/// `capture_display_id`).
pub fn capture_display_logical_bounds() -> Option<(f64, f64, f64, f64)> {
    match capture_display_geometry() {
        Ok(g) => Some((g.origin_x_pts, g.origin_y_pts, g.width_pts, g.height_pts)),
        Err(e) => {
            crate::log(&format!(
                "    [capture] capture_display_logical_bounds failed: {}",
                e
            ));
            None
        }
    }
}

/// Owning wrapper around `Retained<SCShareableContent>` that adds Send +
/// Sync. Apple's ScreenCaptureKit documentation states that
/// `SCShareableContent` instances are thread-safe for the read operations
/// we use (`.displays()`, `.windows()`, `.applications()`, and properties on
/// each display). We never mutate the content — only borrow it — and every
/// access goes through the parking_lot Mutex, so sharing across threads is
/// sound.
///
/// `Retained<SCShareableContent>` is normally `!Send + !Sync` because
/// `SCShareableContent` has no `unsafe impl Send/Sync` in
/// objc2-screen-capture-kit 0.3 (it only implements `NSObjectProtocol`).
/// We bridge the gap here so the cache can live in a `static`.
struct ShareableContent(Retained<SCShareableContent>);

impl Deref for ShareableContent {
    type Target = SCShareableContent;
    fn deref(&self) -> &SCShareableContent {
        &self.0
    }
}

// SAFETY: see struct doc comment.
unsafe impl Send for ShareableContent {}
unsafe impl Sync for ShareableContent {}

/// What the brief asks us to pre-warm. Holds a `+1`-retained
/// `SCShareableContent` across captures so the hot path doesn't pay the
/// window-server IPC + TCC check on every `Cmd+X`.
static CACHED_CONTENT: Mutex<Option<ShareableContent>> = Mutex::new(None);

/// Get an owned `Retained<SCShareableContent>` for the duration of one
/// capture. Resolves if the cache is empty (prewarm never ran or failed),
/// then borrows the cached ptr and re-retains it for our local use. Drops
/// the cache lock before returning — holding it across the ~30-200 ms
/// capture call interacts badly with SCK's dispatch queue (completion
/// handler never fires on subsequent runs in the same session).
fn take_owned_cached_content(t0: Instant) -> Result<Retained<SCShareableContent>, String> {
    // Populate cache if empty.
    {
        let mut guard = CACHED_CONTENT.lock().unwrap();
        if guard.is_none() {
            *guard = Some(resolve_shareable_content(t0)?);
        }
    }
    // Borrow the cached ptr briefly, then drop the lock and re-retain it.
    // The cache's +1 lives in its ShareableContent (never replaced after
    // the first install), so there's no use-after-free race in practice —
    // our extra Retained::retain is purely additive.
    let ptr: *mut SCShareableContent = {
        let guard = CACHED_CONTENT.lock().unwrap();
        match guard.as_ref() {
            Some(sc) => sc.0.as_ref() as *const SCShareableContent as *mut SCShareableContent,
            None => return Err("cached content disappeared after install".to_string()),
        }
    };
    unsafe { Retained::retain(ptr) }
        .ok_or_else(|| "Retained::retain returned None for cached content".to_string())
}

/// Drop the cached `SCShareableContent` so the next capture re-resolves it.
///
/// Called from the `NSApplicationDidChangeScreenParametersNotification`
/// observer installed in `main.rs` — that notification fires when a display
/// is connected or disconnected, when the resolution or arrangement changes,
/// and when the lid closes. Previously the cache was only refreshed *after*
/// a capture had already failed, which meant the first capture following a
/// display change was the one that paid for it. Cheap to be wrong in this
/// direction: a spurious invalidation costs one `getShareableContent`
/// (~50-70 ms) on the next capture; a missed one risks capturing a display
/// that no longer exists.
///
/// Note this only invalidates the *content* cache. `DisplayGeometry` is
/// resolved fresh on every capture from `CGDisplayCopyDisplayMode`, so a
/// resolution change is already picked up there with no cache to clear.
pub fn invalidate_cached_content(reason: &str) {
    let had = CACHED_CONTENT.lock().unwrap().take().is_some();
    crate::log(&format!(
        "    [capture] SCShareableContent cache invalidated ({}), had_cached={}",
        reason, had
    ));
}

/// Resolve (and cache) the shareable content. Called from
/// `prewarm_capture_backend` at startup AND on demand when a capture fails
/// (the cached display may have gone stale — see file-level comment).
fn resolve_shareable_content(t0: Instant) -> Result<ShareableContent, String> {
    let (tx, rx) = mpsc::channel::<Result<ShareableContent, String>>();
    let block = RcBlock::new(move |content: *mut SCShareableContent, err: *mut NSError| {
        if !err.is_null() {
            let _ = tx.send(Err(format!(
                "SCShareableContent error: NSError ptr={:p}",
                err
            )));
            return;
        }
        if content.is_null() {
            let _ = tx.send(Err("SCShareableContent returned null".to_string()));
            return;
        }
        // SAFETY: Apple's docs for `getShareableContentWithCompletionHandler:`
        // say the SCShareableContent passed to the block is **not retained by
        // the framework** — "you must retain this object if you want to use it
        // beyond the lifetime of the block." Previous versions of this code
        // did `Retained::from_raw(content)` (which claims a +1 that wasn't
        // there), then dropped it, taking the refcount negative and causing
        // the next `content.displays()` call to SIGTRAP inside `object_getClass`
        // (PAC failure on a freed object). `Retained::retain` calls
        // `[content retain]` and wraps the result in `Retained<T>` — adding
        // the +1 we need and giving us explicit ownership.
        let reclaimed = unsafe { Retained::retain(content) };
        match reclaimed {
            Some(r) => {
                let _ = tx.send(Ok(ShareableContent(r)));
            }
            None => {
                let _ = tx.send(Err("Retained::retain failed".to_string()));
            }
        }
    });
    unsafe {
        SCShareableContent::getShareableContentWithCompletionHandler(&block);
    }
    let content = rx
        .recv()
        .map_err(|e| format!("SCShareableContent channel: {}", e))??;
    crate::log(&format!(
        "    [capture] shareableContent resolved | +{}ms",
        t0.elapsed().as_millis()
    ));
    Ok(content)
}

/// Find, among `content`'s displays, the `SCDisplay` for the display we
/// capture (`capture_display_id()`).
///
/// Round 1 took `displays().firstObject()` here and `firstObject()` again in
/// `overlay_web`, with no guarantee that either is the main display — on a
/// multi-monitor setup that silently captures one screen and sizes the
/// overlay to another. Matching on `displayID()` gives both callers one
/// definition. The `firstObject()` fallback stays only for the pathological
/// case where SCK enumerates no display with the main id at all; it logs
/// loudly, because a capture of the wrong screen must not be silent.
fn pick_primary_display(content: &SCShareableContent) -> Result<Retained<SCDisplay>, String> {
    let displays = unsafe { content.displays() };
    if displays.is_empty() {
        return Err("SCShareableContent has no displays".to_string());
    }
    let wanted = capture_display_id();
    for display in displays.iter() {
        if unsafe { display.displayID() } == wanted {
            return Ok(display);
        }
    }
    let fallback = displays
        .firstObject()
        .ok_or_else(|| "SCShareableContent.displays() returned empty".to_string())?;
    crate::log(&format!(
        "    [capture] WARN: no SCDisplay matches CGMainDisplayID {} ({} display(s) offered); \
         falling back to displays()[0] id={} — capture and overlay may disagree",
        wanted,
        displays.len(),
        unsafe { fallback.displayID() }
    ));
    Ok(fallback)
}

/// Per-stage wall-clock cost of one capture, in milliseconds. Returned by
/// `capture_via_sck_timed` so the self-test can report the breakdown
/// (`captureImage` vs the bitmap draw vs end-to-end) without re-deriving it
/// by subtracting log timestamps.
#[derive(Debug, Clone, Copy, Default)]
pub struct CaptureTimings {
    /// Getting an owned `SCShareableContent` (≈0 when the cache is warm).
    pub content_ms: u128,
    /// Building the `SCContentFilter` + `SCStreamConfiguration`.
    pub setup_ms: u128,
    /// `SCScreenshotManager.captureImage` — request to `CGImage` in hand.
    pub capture_image_ms: u128,
    /// Drawing the `CGImage` into our own RGBA bitmap context.
    pub draw_ms: u128,
    /// End-to-end `capture_via_sck`.
    pub total_ms: u128,
}

/// Background (capture-thread) side of the capture flow. Calls SCK and
/// copies the result into an `RGBA8` buffer owned by us, ready for
/// `crop_and_save_from_buffer` and the overlay.
pub fn capture_via_sck(t0: Instant) -> Result<CaptureData, String> {
    capture_via_sck_timed(t0).map(|(data, _)| data)
}

/// As `capture_via_sck`, but also returns the per-stage breakdown.
pub fn capture_via_sck_timed(t0: Instant) -> Result<(CaptureData, CaptureTimings), String> {
    let mut timings = CaptureTimings::default();

    // Ensure the cache is populated (resolving on first capture if prewarm
    // never ran), then take an owned +1 retain out of it for the duration
    // of THIS capture. This drops the cache lock before calling SCK —
    // holding it across the ~30-200 ms capture call appears to interact
    // badly with SCK's dispatch queue (the completion handler never fires
    // on subsequent runs in the same session).
    let t_content = Instant::now();
    let content = take_owned_cached_content(t0)?;
    timings.content_ms = t_content.elapsed().as_millis();
    // SAFETY: `content` is +1 retained; it lives until the end of this
    // function, when its `Retained` drops and releases the +1.
    let content_ref: &SCShareableContent = &content;

    let t_setup = Instant::now();
    let display = pick_primary_display(content_ref)?;

    // Everything dimensional comes from here. See `DisplayGeometry` — the
    // `_px` / `_pts` suffixes are load-bearing.
    let geom = capture_display_geometry()?;
    crate::log(&format!(
        "    [capture] display id={} backing {}x{} px, mode {}x{} pts, origin ({:.0},{:.0}) pts, scale {:.2} | +{}ms",
        geom.id, geom.width_px, geom.height_px, geom.width_pts, geom.height_pts,
        geom.origin_x_pts, geom.origin_y_pts, geom.scale,
        t0.elapsed().as_millis()
    ));

    // Build filter: capture the whole display, exclude no windows.
    // The Apple docs are explicit: initWithDisplay:includingWindows:
    // with an empty array captures NOTHING (only-windows is
    // intersection-with). excludingWindows: with empty is "capture the
    // display except these (none)" — i.e. capture everything. See brief
    // §4.3 — the wrong-init mistake looks identical to a permissions
    // failure and costs hours to diagnose.
    let empty_windows = NSArray::<SCWindow>::from_slice(&[]);
    let filter = unsafe {
        SCContentFilter::initWithDisplay_excludingWindows(
            SCContentFilter::alloc(),
            &display,
            &empty_windows,
        )
    };

    // Build SCStreamConfiguration. Per brief §4.4: showsCursor defaults to
    // true; CGWindowListCreateImage never drew the cursor, so leaving it
    // on is a silent regression.
    //
    // width/height are PHYSICAL PIXELS. SCK's content rect is the display's
    // point-space rect; the configuration size is what the framework
    // rasterises it to. Feeding it points here is what produced round 1's
    // quarter-resolution capture (HANDOFF bug #21).
    let config = unsafe { SCStreamConfiguration::new() };
    unsafe {
        config.setShowsCursor(false);
        config.setWidth(geom.width_px);
        config.setHeight(geom.height_px);
    }
    timings.setup_ms = t_setup.elapsed().as_millis();
    crate::log(&format!(
        "    [capture] filter + SCStreamConfiguration ({}x{} px, no cursor) | +{}ms",
        geom.width_px, geom.height_px,
        t0.elapsed().as_millis()
    ));

    // Capture. SCK's API is async (completion handler); the call site is
    // already off the main thread (brief §4.6), so a blocking recv is
    // safe here. We bridge the completion to sync via mpsc.
    let t_capture = Instant::now();
    let (tx, rx) = mpsc::channel::<Result<Retained<CGImage>, String>>();
    let block = RcBlock::new(move |img: *mut CGImage, err: *mut NSError| {
        if !err.is_null() {
            let _ = tx.send(Err(format!(
                "SCScreenshotManager.captureImage error: NSError ptr={:p}",
                err
            )));
            return;
        }
        if img.is_null() {
            let _ = tx.send(Err("SCScreenshotManager returned null CGImage".to_string()));
            return;
        }
        // SAFETY: Same +0 contract as SCShareableContent above — Apple's
        // docs for `captureImageWithFilter:configuration:completionHandler:`
        // hand us a CGImage we must retain if we want to use it past the
        // block. See the full reasoning in `resolve_shareable_content`.
        let reclaimed = unsafe { Retained::retain(img) };
        match reclaimed {
            Some(r) => {
                let _ = tx.send(Ok(r));
            }
            None => {
                let _ = tx.send(Err("Retained::retain on CGImage failed".to_string()));
            }
        }
    });
    unsafe {
        SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
            &filter,
            &config,
            Some(&block),
        );
    }
    let cg_image = rx
        .recv()
        .map_err(|e| format!("capture channel: {}", e))??;
    timings.capture_image_ms = t_capture.elapsed().as_millis();
    crate::log(&format!(
        "    [capture] CGImage returned (captureImage {}ms) | +{}ms",
        timings.capture_image_ms,
        t0.elapsed().as_millis()
    ));

    // CGImage dimensions are in PHYSICAL PIXELS. We asked for
    // geom.width_px × geom.height_px, so a mismatch means either the
    // display changed mode between the two calls or SCK clamped us —
    // either way `CaptureData.left/top` (physical) would no longer be in
    // the same space as the buffer, so this is an error, not a warning.
    let cg_w = CGImage::width(Some(&*cg_image));
    let cg_h = CGImage::height(Some(&*cg_image));
    if cg_w != geom.width_px || cg_h != geom.height_px {
        return Err(format!(
            "CGImage is {}x{} px but the configuration asked for {}x{} px \
             (display mode changed mid-capture?) — refusing to hand back a \
             buffer whose left/top are in a different space",
            cg_w, cg_h, geom.width_px, geom.height_px
        ));
    }

    // Allocate RGBA8 premultiplied-last + 32-bit BIG-endian buffer and draw
    // the CGImage into it. CoreGraphics does the BGRA→RGBA channel
    // conversion in C — no per-pixel loop in Rust — and the output is
    // densely packed because WE choose the stride (width * 4), so there is
    // no row padding to shear the image (brief §4.2).
    let t_draw = Instant::now();
    let rgba = draw_cgimage_into_rgba(&cg_image, cg_w, cg_h)?;
    timings.draw_ms = t_draw.elapsed().as_millis();
    crate::log(&format!(
        "    [capture] RGBA buffer {}x{} px ({} bytes, draw {}ms) | +{}ms",
        cg_w, cg_h, rgba.len(), timings.draw_ms,
        t0.elapsed().as_millis()
    ));

    // First pixel channel values, one log line per capture. Lets a reviewer
    // confirm byte order from the log alone rather than eyeballing a PNG —
    // though note that eyeballing a PNG is exactly what failed to catch the
    // round-1 byte-order bug, so the authoritative check is the oracle
    // comparison in `sck_selftest`, not this line. Alpha MUST be 255: a
    // screen capture is opaque.
    let mut first = [0u8; 4];
    if rgba.len() >= 4 {
        first.copy_from_slice(&rgba[..4]);
    }
    crate::log(&format!(
        "    [capture] first pixel RGBA = [{}, {}, {}, {}] (alpha must be 255)",
        first[0], first[1], first[2], first[3]
    ));

    // `CaptureData.left/top` are PHYSICAL PIXELS — the whole codebase's
    // selection math and `get_monitor_scale` live in that space, and the
    // buffer above certainly does. The display origin we have is in LOGICAL
    // POINTS, so convert with the geometry's scale. On the main display this
    // is (0,0) either way; it stops being a no-op the moment multi-monitor
    // capture lands.
    let left = (geom.origin_x_pts * geom.scale as f64).round() as i32;
    let top = (geom.origin_y_pts * geom.scale as f64).round() as i32;
    timings.total_ms = t0.elapsed().as_millis();
    crate::log(&format!(
        "    [capture] capture_via_sck done ({}x{} px at ({},{}) px) | +{}ms",
        cg_w, cg_h, left, top, timings.total_ms
    ));

    Ok((
        CaptureData {
            buffer: rgba,
            width: cg_w as i32,
            height: cg_h as i32,
            left,
            top,
        },
        timings,
    ))
}

/// Draw `cg_image` into a freshly-allocated RGBA8 bitmap buffer and return
/// it. The CGImage's native pixel order is whatever CoreGraphics gave us
/// (BGRA premultiplied); Core Graphics converts as it composites.
///
/// Byte order: `premultipliedLast | byteOrder32Big` puts the 32-bit word
/// `(R<<24)|(G<<16)|(B<<8)|A` down MSB-first, i.e. **R, G, B, A in memory**
/// — which is what `ImageData` in the overlay and `crop_and_save_from_buffer`'s
/// `(ri, gi, bi) = (0, 1, 2)` both require. The little-endian variant of the
/// same word is A, B, G, R; that is HANDOFF bug #22, and it is invisible to
/// the naked eye on a screenshot (alpha is always 255, so every pixel comes
/// out saturated in one channel and the image looks like a colour cast).
fn draw_cgimage_into_rgba(
    cg_image: &CGImage,
    width: usize,
    height: usize,
) -> Result<Vec<u8>, String> {
    let bytes_per_row = width * 4;
    let mut buffer: Vec<u8> = vec![0u8; bytes_per_row * height];
    let space = CGColorSpace::new_device_rgb().ok_or_else(|| "CGColorSpace::new_device_rgb returned None".to_string())?;

    // Named constants, not magic numbers: PremultipliedLast = 1,
    // Order32Big = 4 << 12 = 0x4000. (Round 1 wrote `1 | (2 << 12)` with a
    // comment claiming it produced RGBA; `2 << 12` is Order32Little, which
    // produces ABGR. Naming the constants makes the claim checkable.)
    let bitmap_info: u32 =
        CGImageAlphaInfo::PremultipliedLast.0 | CGImageByteOrderInfo::Order32Big.0;

    let ctx = unsafe {
        CGBitmapContextCreate(
            buffer.as_mut_ptr() as *mut std::ffi::c_void,
            width,
            height,
            8,
            bytes_per_row,
            Some(&*space),
            bitmap_info,
        )
    }
    .ok_or_else(|| "CGBitmapContextCreate returned None".to_string())?;

    let ctx_ref: &CGContext = &ctx;
    let rect = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size: CGSize {
            width: width as f64,
            height: height as f64,
        },
    };
    // `CGContext::draw_image` is the non-deprecated form of the free
    // `CGContextDrawImage` function in objc2-core-graphics 0.3.
    CGContext::draw_image(Some(ctx_ref), rect, Some(cg_image));
    Ok(buffer)
}

/// Called once at startup (from `main.rs`'s `.setup()`) so the first
/// capture doesn't pay the SCShareableContent IPC cost. Failure here is
/// not fatal — the first capture will retry.
pub fn prewarm_capture_backend(t0: Instant) {
    crate::log("    [capture] prewarming SCShareableContent");
    match resolve_shareable_content(t0) {
        Ok(content) => {
            *CACHED_CONTENT.lock().unwrap() = Some(content);
        }
        Err(e) => {
            crate::log(&format!("    [capture] SCShareableContent prewarm failed: {}", e));
        }
    }
}

/// Effective DPI scale (1.0 = 96 dpi/100%, 2.0 = Retina) of the display we
/// capture. The capture path is single-monitor (HANDOFF §7) so we don't need
/// to hit-test the given point — every selection is on the same display.
///
/// Computed as backing-store pixels ÷ logical points of the SAME display
/// mode. Round 1 computed it as `CGDisplayPixelsWide / SCDisplay.width()`,
/// which is points ÷ points = 1.0 on every Retina display — bug #7 all over
/// again through a different API (HANDOFF bug #21). Deliberately does not
/// touch the `SCShareableContent` cache: this must answer correctly even if
/// the prewarm failed.
#[allow(dead_code)]
pub fn primary_monitor_scale() -> f32 {
    match capture_display_geometry() {
        Ok(g) => g.scale.max(1.0),
        Err(e) => {
            crate::log(&format!(
                "    [capture] primary_monitor_scale: geometry unavailable ({}) — falling back to 1.0",
                e
            ));
            1.0
        }
    }
}

// ---------------------------------------------------------------------------
// Screen-Recording TCC preflight
// ---------------------------------------------------------------------------
//
// Without Screen Recording permission, `SCScreenshotManager.captureImage`
// silently returns a black buffer (or, depending on macOS version, an error
// that mentions TCC internally) — the app looks broken with no user-visible
// explanation. This is the single strongest "unfinished" signal an App
// Store reviewer would hit on first launch, because the prompt fires the
// very first time capture is attempted.
//
// `CGPreflightScreenCaptureAccess` is a cheap query that returns whether
// the calling *signed* application already has the TCC grant, without
// triggering any UI. `CGRequestScreenCaptureAccess` is the one that
// actually triggers the system prompt on the very first call — but on a
// signed developer build the user must also re-grant after every code
// change, so the prompt alone is not enough; the user has to know to
// restart the app for the new signature to take effect.
//
// We preflight on the hotkey path (not at startup) so the first launch
// is unobtrusive: if the user never captures, they never see a permission
// dialog. The hotkey handler in main.rs is the single caller.
//
/// True if the signed app already holds the Screen Recording TCC grant.
///
/// Cheap and side-effect-free; safe to call on every hotkey press. Returns
/// `false` for an unsigned ad-hoc build (the OS only recognizes stable
/// signatures for TCC purposes — HANDOFF bug #11 covers why this project's
/// dev runs use a stable codesigning identity rather than Xcode's default).
pub fn screen_capture_access_granted() -> bool {
    // CGPreflightScreenCaptureAccess is declared `extern "C-unwind"` by
    // objc2-core-graphics — wrap in catch_unwind so a panic on the FFI
    // boundary (Apple's docs explicitly allow it to crash on mis-signed
    // callers) doesn't tear down the capture thread.
    let granted = std::panic::catch_unwind(|| CGPreflightScreenCaptureAccess());
    match granted {
        Ok(g) => g,
        Err(_) => {
            crate::log("    [tcc] CGPreflightScreenCaptureAccess panicked — treating as denied");
            false
        }
    }
}

/// Trigger the system's Screen Recording permission prompt. Returns true
/// if the user granted access in the dialog. Idempotent — calling it
/// after access has already been granted is a no-op.
///
/// **Note**: macOS only honours a freshly-granted TCC entry after the
/// app is *restarted* (the new signature has to be the one running when
/// SCK/TCC re-checks). The dialog that the frontend shows in response to
/// a `false` preflight has to say this — see `start_capture` in main.rs.
pub fn request_screen_capture_access() -> bool {
    std::panic::catch_unwind(|| CGRequestScreenCaptureAccess()).unwrap_or(false)
}
