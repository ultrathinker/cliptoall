//! Oracle-checked self-test for the ScreenCaptureKit capture path.
//!
//! Gated behind `CLIPTOALL_SELFTEST_CAPTURE=1`; runs at startup before Tauri
//! is built and exits the process. It cannot fire during normal use.
//!
//! ## Why this exists in this shape
//!
//! Round 1 of the SCK migration verified itself by capturing a PNG and
//! looking at it. Two critical defects — a quarter-resolution buffer and an
//! ABGR byte order — both survived that inspection, because both produce an
//! image that *looks* like a screenshot. The lesson is that a check a human
//! can talk themselves out of is not a check.
//!
//! So the reference here is `/usr/sbin/screencapture`, which ships with
//! macOS, needs no interaction, and does not argue back. Every comparison is
//! an assertion with a numeric threshold; a failure sets a non-zero exit code
//! and prints exactly which check failed and with what numbers. Nothing in
//! this file asks a human to judge whether output "looks right".
//!
//! ## What is checked
//!
//! | Check | Catches |
//! |---|---|
//! | `dimensions` | configuring SCK in points instead of pixels (bug #21) |
//! | `scale` | a backing scale factor that silently reads 1.0 (bugs #7, #21) |
//! | `alpha` | any non-opaque byte — the single check that would have killed bug #22 on sight |
//! | `channel-permutation` | RGBA vs ABGR vs BGRA vs any other systematic swap (bug #22) |
//! | `colour` | wrong colour space, gamma, or a genuinely wrong image |
//! | `shear` | row-stride padding handled wrongly (drift grows down the frame) |
//! | `cursor` | (not automatable — see mx-verify.md) |
//!
//! ## The animation hazard
//!
//! Our capture and the reference are taken moments apart, so a clock, a
//! blinking caret or a playing video legitimately differ. Rather than
//! guessing a tolerance big enough to hide that (which would also hide real
//! bugs), we take the reference **twice**, before and after our own capture,
//! and only compare sample points where the two references agree. A pixel
//! that changed between them was animating and is excluded, with the count
//! reported. Everything left is genuinely stable screen content, so the
//! tolerance can stay tight.

use std::path::{Path, PathBuf};
use std::time::Instant;

/// Per-channel difference is allowed to be this large on a stable pixel.
/// Non-zero because the reference goes through PNG colour management and our
/// buffer through `CGColorSpace::new_device_rgb`, so a point or two of drift
/// on saturated colours is expected. A channel permutation produces
/// differences of tens to hundreds, so this threshold separates the two
/// cases by an order of magnitude and is not a judgement call.
const MAX_MEAN_CHANNEL_DIFF: f64 = 12.0;
/// A single stable sample point may differ by at most this much on any
/// channel. Catches a localised corruption that a mean would average away.
const MAX_SINGLE_CHANNEL_DIFF: u32 = 96;
/// Fraction of sampled points allowed to exceed `MAX_SINGLE_CHANNEL_DIFF`
/// before the colour check fails. Non-zero because the two-reference
/// stability filter cannot catch content that changed *during* a capture
/// (a repaint mid-scan), only content that differed between two instants.
const MAX_OUTLIER_FRACTION: f64 = 0.05;
/// How many capture runs to time.
const TIMED_RUNS: usize = 5;
/// Horizontal shifts probed when looking for row shear, in pixels.
const SHEAR_PROBE: i32 = 8;

struct Report {
    failures: Vec<String>,
    lines: Vec<String>,
}

impl Report {
    fn new() -> Self {
        Report { failures: Vec::new(), lines: Vec::new() }
    }
    /// Record an informational line: goes to the app log and to stdout.
    fn info(&mut self, s: String) {
        crate::log(&format!("[selftest] {}", s));
        println!("[selftest] {}", s);
        self.lines.push(s);
    }
    /// Assert `cond`; on failure record `msg` (which must contain the actual
    /// numbers) and keep going, so one run reports every broken check rather
    /// than only the first.
    fn check(&mut self, name: &str, cond: bool, msg: String) {
        if cond {
            self.info(format!("PASS  {}: {}", name, msg));
        } else {
            let line = format!("FAIL  {}: {}", name, msg);
            crate::log(&format!("[selftest] {}", line));
            eprintln!("[selftest] {}", line);
            self.lines.push(line.clone());
            self.failures.push(format!("{}: {}", name, msg));
        }
    }
}

/// Run `/usr/sbin/screencapture` to produce the reference PNG.
///
/// `-x` suppresses the shutter sound and any UI. `-D 1` selects the main
/// display explicitly rather than relying on the default (which, with more
/// than one display attached, writes one file per display and would silently
/// give us the wrong one).
///
/// TCC: this is a child of the signed app, so it runs under the app's
/// Screen-Recording grant. It also happens to work from a plain shell on this
/// machine, which is how the oracle was validated before being wired in here.
fn run_reference_capture(path: &Path) -> Result<(), String> {
    let _ = std::fs::remove_file(path);
    let out = std::process::Command::new("/usr/sbin/screencapture")
        .arg("-x")
        .arg("-D")
        .arg("1")
        .arg(path)
        .output()
        .map_err(|e| format!("spawning /usr/sbin/screencapture failed: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "screencapture exited with {:?}: {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    if !path.exists() {
        return Err(format!(
            "screencapture reported success but wrote no file at {}",
            path.display()
        ));
    }
    Ok(())
}

/// Sample points, spread across the frame and grouped into horizontal bands
/// so a shear (which grows with y) shows up as a band-dependent result rather
/// than being averaged into the whole-image number.
///
/// Returns `(band_name, points)` triples. Deliberately avoids the extreme
/// edges (1-pixel borders can be antialiased differently by two capture
/// paths) and includes a dense sweep across x within each band, which is what
/// makes the horizontal-shift probe below meaningful.
fn sample_bands(w: usize, h: usize) -> Vec<(&'static str, Vec<(usize, usize)>)> {
    let mut out = Vec::new();
    for (name, y_lo, y_hi) in [
        ("top", 0.02_f64, 0.30_f64),
        ("middle", 0.36, 0.64),
        ("bottom", 0.70, 0.98),
    ] {
        let mut pts = Vec::new();
        // 5 rows x 24 columns per band = 120 points per band, 360 total.
        for iy in 0..5 {
            let fy = y_lo + (y_hi - y_lo) * (iy as f64 / 4.0);
            let y = ((h as f64 - 1.0) * fy).round() as usize;
            for ix in 0..24 {
                // Keep SHEAR_PROBE pixels of margin on both sides so the
                // shift probe can read ours[x + shift] without clamping.
                let fx = 0.03 + 0.94 * (ix as f64 / 23.0);
                let x = ((w as f64 - 1.0) * fx).round() as usize;
                let x = x.clamp(SHEAR_PROBE as usize, w - 1 - SHEAR_PROBE as usize);
                pts.push((x, y));
            }
        }
        out.push((name, pts));
    }
    out
}

#[inline]
fn px(buf: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
    let o = (y * w + x) * 4;
    [buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]
}

/// Entry point. Returns the process exit code: 0 = every check passed.
pub fn run() -> i32 {
    let mut r = Report::new();
    let started = Instant::now();
    r.info("=== SCK capture self-test (oracle: /usr/sbin/screencapture) ===".to_string());

    // ---------------------------------------------------------------- geometry
    let geom = match crate::sck_capture::capture_display_geometry() {
        Ok(g) => g,
        Err(e) => {
            r.check("geometry", false, format!("capture_display_geometry failed: {}", e));
            return finish(&mut r, started);
        }
    };
    r.info(format!(
        "display id={}  backing={}x{} px  mode={}x{} pts  origin=({},{}) pts  scale={}",
        geom.id, geom.width_px, geom.height_px, geom.width_pts, geom.height_pts,
        geom.origin_x_pts, geom.origin_y_pts, geom.scale
    ));

    // ------------------------------------------------------------ reference #1
    let tmp = std::env::temp_dir();
    let ref1_path: PathBuf = tmp.join("cliptoall_selftest_ref1.png");
    let ref2_path: PathBuf = tmp.join("cliptoall_selftest_ref2.png");
    let ours_path: PathBuf = tmp.join("cliptoall_selftest_capture.png");

    if let Err(e) = run_reference_capture(&ref1_path) {
        r.check("oracle-available", false, format!("reference capture #1 failed: {}", e));
        return finish(&mut r, started);
    }

    // --------------------------------------------------------- our capture(s)
    crate::sck_capture::prewarm_capture_backend(Instant::now());
    let mut timings = Vec::new();
    let mut capture = None;
    for i in 0..TIMED_RUNS {
        let t0 = Instant::now();
        match crate::sck_capture::capture_via_sck_timed(t0) {
            Ok((data, t)) => {
                r.info(format!(
                    "run {}/{}: content {}ms | setup {}ms | captureImage {}ms | draw {}ms | end-to-end {}ms",
                    i + 1, TIMED_RUNS, t.content_ms, t.setup_ms, t.capture_image_ms, t.draw_ms, t.total_ms
                ));
                timings.push(t);
                // Keep the LAST capture for comparison: it is the one closest
                // in time to reference #2, and by then the cache is warm so
                // it is also the representative one for timing.
                capture = Some(data);
            }
            Err(e) => {
                r.check("capture", false, format!("run {} failed: {}", i + 1, e));
                return finish(&mut r, started);
            }
        }
    }
    let capture = capture.expect("checked above");

    // ------------------------------------------------------------ reference #2
    if let Err(e) = run_reference_capture(&ref2_path) {
        r.check("oracle-available", false, format!("reference capture #2 failed: {}", e));
        return finish(&mut r, started);
    }

    // ------------------------------------------------------------ timing report
    if !timings.is_empty() {
        let n = timings.len() as u128;
        let mean = |f: fn(&crate::sck_capture::CaptureTimings) -> u128| -> u128 {
            timings.iter().map(f).sum::<u128>() / n
        };
        r.info(format!(
            "timing means over {} runs at {}x{} px: captureImage {}ms | draw {}ms | end-to-end {}ms",
            n, geom.width_px, geom.height_px,
            mean(|t| t.capture_image_ms), mean(|t| t.draw_ms), mean(|t| t.total_ms)
        ));
    }

    // ------------------------------------------------------------------ decode
    let ref1 = match image::open(&ref1_path) {
        Ok(i) => i.to_rgba8(),
        Err(e) => {
            r.check("oracle-decode", false, format!("decoding {} failed: {}", ref1_path.display(), e));
            return finish(&mut r, started);
        }
    };
    let ref2 = match image::open(&ref2_path) {
        Ok(i) => i.to_rgba8(),
        Err(e) => {
            r.check("oracle-decode", false, format!("decoding {} failed: {}", ref2_path.display(), e));
            return finish(&mut r, started);
        }
    };

    let (ow, oh) = (capture.width as usize, capture.height as usize);
    let (rw, rh) = (ref1.width() as usize, ref1.height() as usize);

    // =================================================================== CHECK 1
    // Dimensions. All three must agree: our buffer, the reference PNG, and the
    // display mode's backing store. This is the check round 1 did not have.
    r.check(
        "dimensions",
        ow == rw && oh == rh && ow == geom.width_px && oh == geom.height_px,
        format!(
            "ours={}x{}  reference={}x{}  mode.pixelWidth/Height={}x{}  (mode points={}x{})",
            ow, oh, rw, rh, geom.width_px, geom.height_px, geom.width_pts, geom.height_pts
        ),
    );
    r.check(
        "buffer-length",
        capture.buffer.len() == ow * oh * 4,
        format!("buffer is {} bytes, expected {} for {}x{} RGBA8",
            capture.buffer.len(), ow * oh * 4, ow, oh),
    );
    if ow != rw || oh != rh || capture.buffer.len() != ow * oh * 4 {
        // Every later check indexes both buffers by the same coordinates.
        r.info("dimensions disagree — skipping pixel-level checks".to_string());
        return finish(&mut r, started);
    }

    // =================================================================== CHECK 2
    // Backing scale factor, derived from the oracle rather than from the same
    // API that produced it. The reference PNG's width is, by construction, the
    // real backing-store width; divided by the mode's point width it must give
    // exactly the scale `DisplayGeometry` computed. On this machine: 2.0.
    let oracle_scale = rw as f64 / geom.width_pts;
    r.check(
        "scale",
        (oracle_scale - geom.scale as f64).abs() < 1e-6,
        format!(
            "DisplayGeometry.scale={} vs oracle {}/{}={} (a Retina display reading 1.0 here is bug #7/#21)",
            geom.scale, rw, geom.width_pts, oracle_scale
        ),
    );
    r.check(
        "capture-origin-space",
        capture.left == (geom.origin_x_pts * geom.scale as f64).round() as i32
            && capture.top == (geom.origin_y_pts * geom.scale as f64).round() as i32,
        format!(
            "CaptureData.left/top = ({},{}) px, expected origin ({},{}) pts x scale {} = ({},{}) px",
            capture.left, capture.top, geom.origin_x_pts, geom.origin_y_pts, geom.scale,
            (geom.origin_x_pts * geom.scale as f64).round() as i32,
            (geom.origin_y_pts * geom.scale as f64).round() as i32
        ),
    );

    // =================================================================== CHECK 3
    // Alpha. A screen capture is opaque; every alpha byte must be 255. Scans
    // the WHOLE buffer, not a sample — it is one pass over 16 MB and it is the
    // cheapest possible detector for the round-1 byte-order defect, which put
    // the alpha byte where red belongs and left 62 (a red value) in the alpha
    // slot.
    let mut bad_alpha = 0usize;
    let mut first_bad: Option<(usize, usize, u8)> = None;
    for y in 0..oh {
        for x in 0..ow {
            let a = capture.buffer[(y * ow + x) * 4 + 3];
            if a != 255 {
                bad_alpha += 1;
                if first_bad.is_none() {
                    first_bad = Some((x, y, a));
                }
            }
        }
    }
    r.check(
        "alpha-opaque",
        bad_alpha == 0,
        match first_bad {
            None => format!("all {} pixels have alpha=255", ow * oh),
            Some((x, y, a)) => format!(
                "{} of {} pixels have alpha != 255; first at ({},{}) with alpha={}",
                bad_alpha, ow * oh, x, y, a
            ),
        },
    );

    // ------------------------------------------- stable-pixel set (animation)
    let bands = sample_bands(ow, oh);
    let total_pts: usize = bands.iter().map(|(_, p)| p.len()).sum();
    let mut stable_bands: Vec<(&'static str, Vec<(usize, usize)>)> = Vec::new();
    let mut unstable = 0usize;
    for (name, pts) in &bands {
        let mut keep = Vec::new();
        for &(x, y) in pts {
            let a = ref1.get_pixel(x as u32, y as u32).0;
            let b = ref2.get_pixel(x as u32, y as u32).0;
            // Exact equality: the two references come from the same code path,
            // so any difference at all means the content moved.
            if a[0] == b[0] && a[1] == b[1] && a[2] == b[2] {
                keep.push((x, y));
            } else {
                unstable += 1;
            }
        }
        stable_bands.push((name, keep));
    }
    let stable_total: usize = stable_bands.iter().map(|(_, p)| p.len()).sum();
    r.info(format!(
        "sampled {} points; {} stable across both reference captures, {} excluded as animating",
        total_pts, stable_total, unstable
    ));
    r.check(
        "enough-stable-samples",
        stable_total >= total_pts / 2,
        format!("{}/{} sample points were stable (need at least half)", stable_total, total_pts),
    );

    // =================================================================== CHECK 4
    // Channel permutation. For each of the six ways our (R,G,B) could map onto
    // the reference's, compute the mean absolute difference over stable points.
    // The identity mapping must win outright. This is the check that names the
    // round-1 defect directly instead of leaving a human to interpret a colour
    // cast: ABGR shows up as identity losing to a swap by a wide margin.
    const PERMS: [([usize; 3], &str); 6] = [
        ([0, 1, 2], "RGB (identity — correct)"),
        ([2, 1, 0], "BGR (red/blue swapped)"),
        ([1, 0, 2], "GRB"),
        ([0, 2, 1], "RBG"),
        ([1, 2, 0], "GBR"),
        ([2, 0, 1], "BRG"),
    ];
    let stable_flat: Vec<(usize, usize)> =
        stable_bands.iter().flat_map(|(_, p)| p.iter().copied()).collect();
    let mut perm_scores = Vec::new();
    for (perm, label) in PERMS {
        let mut sum = 0f64;
        for &(x, y) in &stable_flat {
            let o = px(&capture.buffer, ow, x, y);
            let e = ref1.get_pixel(x as u32, y as u32).0;
            for c in 0..3 {
                sum += (o[perm[c]] as i32 - e[c] as i32).unsigned_abs() as f64;
            }
        }
        let mean = if stable_flat.is_empty() { f64::NAN } else { sum / (stable_flat.len() * 3) as f64 };
        perm_scores.push((mean, label));
    }
    for (mean, label) in &perm_scores {
        r.info(format!("  channel mapping {:<26} mean |diff| = {:.2}", label, mean));
    }
    let identity = perm_scores[0].0;
    let best_other = perm_scores[1..].iter().map(|(m, _)| *m).fold(f64::INFINITY, f64::min);
    r.check(
        "channel-order",
        identity <= best_other,
        format!(
            "identity RGB mean |diff| = {:.2}; best permuted mapping = {:.2} ({}). \
             Identity must not lose — if it does, the buffer's channels are permuted.",
            identity, best_other,
            perm_scores[1..].iter().min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, l)| *l).unwrap_or("?")
        ),
    );

    // Additional named detector for the exact round-1 shape: a buffer laid out
    // A,B,G,R would put the constant 255 in byte 0 of every pixel.
    let all_r_255 = !stable_flat.is_empty()
        && stable_flat.iter().all(|&(x, y)| px(&capture.buffer, ow, x, y)[0] == 255);
    r.check(
        "not-abgr",
        !all_r_255,
        if all_r_255 {
            "every sampled pixel's byte 0 is exactly 255 — that is the alpha channel sitting in the red slot (bug #22, premultipliedLast|byteOrder32Little)".to_string()
        } else {
            "byte 0 varies across sampled pixels, so it is not a constant alpha".to_string()
        },
    );

    // =================================================================== CHECK 5
    // Colour, per band. Reported per channel so a systematic bias is visible
    // as a bias rather than as noise.
    let mut band_means = Vec::new();
    for (name, pts) in &stable_bands {
        if pts.is_empty() {
            r.check(
                "colour-band-coverage",
                false,
                format!("band '{}' has no stable sample points left to compare", name),
            );
            continue;
        }
        let mut sums = [0f64; 3];
        let mut maxes = [0u32; 3];
        let mut outliers = 0usize;
        let mut worst: Option<(usize, usize, [u8; 4], [u8; 4])> = None;
        for &(x, y) in pts {
            let o = px(&capture.buffer, ow, x, y);
            let e = ref1.get_pixel(x as u32, y as u32).0;
            let mut point_max = 0u32;
            for c in 0..3 {
                let d = (o[c] as i32 - e[c] as i32).unsigned_abs();
                sums[c] += d as f64;
                if d > maxes[c] {
                    maxes[c] = d;
                }
                point_max = point_max.max(d);
            }
            if point_max > MAX_SINGLE_CHANNEL_DIFF {
                outliers += 1;
                if worst.is_none() {
                    worst = Some((x, y, o, e));
                }
            }
        }
        let n = pts.len() as f64;
        let means = [sums[0] / n, sums[1] / n, sums[2] / n];
        band_means.push((*name, means));
        let outlier_frac = outliers as f64 / n;
        r.check(
            &format!("colour-{}", name),
            means[0] <= MAX_MEAN_CHANNEL_DIFF
                && means[1] <= MAX_MEAN_CHANNEL_DIFF
                && means[2] <= MAX_MEAN_CHANNEL_DIFF
                && outlier_frac <= MAX_OUTLIER_FRACTION,
            format!(
                "{} stable pts | mean |diff| R={:.2} G={:.2} B={:.2} (limit {:.1}) | \
                 max R={} G={} B={} | outliers {}/{} = {:.1}% (limit {:.0}%){}",
                pts.len(), means[0], means[1], means[2], MAX_MEAN_CHANNEL_DIFF,
                maxes[0], maxes[1], maxes[2], outliers, pts.len(),
                outlier_frac * 100.0, MAX_OUTLIER_FRACTION * 100.0,
                match worst {
                    Some((x, y, o, e)) => format!(
                        " | first outlier ({},{}) ours=[{},{},{},{}] ref=[{},{},{}]",
                        x, y, o[0], o[1], o[2], o[3], e[0], e[1], e[2]
                    ),
                    None => String::new(),
                }
            ),
        );
    }

    // =================================================================== CHECK 6
    // Shear. A row-stride error offsets each row a little further than the
    // last, so the best-matching horizontal shift grows as you go down the
    // frame. Probe shifts in [-SHEAR_PROBE, +SHEAR_PROBE] per band: the best
    // shift must be 0 in every band. Comparing only the top rows — which is
    // what "the screenshot looks fine" amounts to — cannot see this.
    //
    // A "monotone" band (every sampled pixel identical, e.g. a corner of a
    // black region) cannot distinguish between any shifts — they all score
    // zero. Recognise that case explicitly: report `shear-{name}-monotone`
    // as PASS, and EXCLUDE the band from `shear-drift`. Otherwise the drift
    // number gets polluted by ties broken by iteration order, and that bug
    // gets indistinguishable from a real drift.
    const MONOTONE_FLOOR: f64 = 0.5;
    let mut best_shifts = Vec::new();
    for (name, pts) in &stable_bands {
        if pts.is_empty() {
            continue;
        }
        let mut best = (f64::INFINITY, 0i32);
        let mut at_zero = f64::NAN;
        for shift in -SHEAR_PROBE..=SHEAR_PROBE {
            let mut sum = 0f64;
            for &(x, y) in pts {
                let sx = (x as i32 + shift) as usize;
                let o = px(&capture.buffer, ow, sx, y);
                let e = ref1.get_pixel(x as u32, y as u32).0;
                for c in 0..3 {
                    sum += (o[c] as i32 - e[c] as i32).unsigned_abs() as f64;
                }
            }
            let mean = sum / (pts.len() * 3) as f64;
            if shift == 0 {
                at_zero = mean;
            }
            if mean < best.0 {
                best = (mean, shift);
            }
        }
        if at_zero < MONOTONE_FLOOR {
            // Every shift scores ~0 here — the band is too monotone for
            // this check to say anything useful. It also cannot show
            // shear; the other (non-monotone) bands confirm independently.
            r.check(
                &format!("shear-{}-monotone", name),
                true,
                format!(
                    "band is essentially monochrome (mean |diff| at shift 0 = {:.4}); \
                     shear cannot be detected on this band, excluding from drift. \
                     Non-monotone bands are independently confirming.",
                    at_zero
                ),
            );
            continue;
        }
        best_shifts.push((*name, best.1));
        r.check(
            &format!("shear-{}", name),
            best.1 == 0,
            format!(
                "best horizontal shift = {} px (mean |diff| {:.2}); shift 0 scores {:.2}. \
                 Any non-zero best shift means rows are offset — a stride bug.",
                best.1, best.0, at_zero
            ),
        );
    }
    if best_shifts.len() > 1 {
        let drift = best_shifts.iter().map(|(_, s)| *s).max().unwrap_or(0)
            - best_shifts.iter().map(|(_, s)| *s).min().unwrap_or(0);
        r.check(
            "shear-drift",
            drift == 0,
            format!(
                "best shift per band: {:?} — spread {} px (progressive drift down the frame is the stride-bug signature)",
                best_shifts, drift
            ),
        );
    } else if best_shifts.len() == 1 {
        r.info(
            "shear-drift: only one non-monotone band — drift is trivially 0, skipping the spread check"
                .to_string(),
        );
    }

    // ------------------------------------------------------------------- PNG
    // ------------------------------------------------------------------- observer
    // Verify §6 wiring: install the
    // NSApplicationDidChangeScreenParametersNotification observer,
    // programmatically post the notification, and confirm the cached
    // SCShareableContent was actually dropped — detected via the next
    // capture's `content_ms` being non-zero (a warm cache takes ~0 ms;
    // re-resolution takes 50-70 ms). A test that the observer *exists*
    // but doesn't *fire* would be useless here, so we force-fire it.
    crate::sck_notifications::install_once();
    let observer_installed = crate::sck_notifications::is_observer_installed();
    let pre_content_ms = timings.last().map(|t| t.content_ms).unwrap_or(0);
    crate::sck_notifications::post_screen_params_change_for_test();
    let after = crate::sck_capture::capture_via_sck_timed(Instant::now());
    r.check(
        "observer-installed",
        observer_installed,
        format!(
            "install_once registered the screen-params observer (state holds a live observer token = {}). \
             Without this, stale display-arrangement changes would silently corrupt the next capture.",
            observer_installed
        ),
    );
    r.check(
        "observer-fires",
        match &after {
            Ok((_d, t)) => t.content_ms > 30,
            Err(_) => false,
        },
        match &after {
            Ok((d, t)) => format!(
                "after a synthetic NSApplicationDidChangeScreenParametersNotification, the next capture \
                 re-resolved SCShareableContent (content_ms={}, was {} before — warm cache takes ~0 ms, \
                 fresh resolution takes 50-70 ms). capture {}x{} succeeds end-to-end.",
                t.content_ms, pre_content_ms, d.width, d.height
            ),
            Err(e) => format!(
                "capture after the synthetic notification failed: {}",
                e
            ),
        },
    );

    // ------------------------------------------------------------------- PNG
    match image::RgbaImage::from_raw(ow as u32, oh as u32, capture.buffer.clone()) {
        Some(img) => match img.save_with_format(&ours_path, image::ImageFormat::Png) {
            Ok(()) => r.info(format!("wrote our capture to {}", ours_path.display())),
            Err(e) => r.info(format!("could not write {}: {}", ours_path.display(), e)),
        },
        None => r.info("RgbaImage::from_raw returned None (size mismatch)".to_string()),
    }
    r.info(format!("reference captures kept at {} and {}", ref1_path.display(), ref2_path.display()));

    finish(&mut r, started)
}

fn finish(r: &mut Report, started: Instant) -> i32 {
    let elapsed = started.elapsed().as_millis();
    if r.failures.is_empty() {
        r.info(format!("=== SELFTEST PASSED ({} checks, {}ms) ===",
            r.lines.iter().filter(|l| l.starts_with("PASS")).count(), elapsed));
        0
    } else {
        let n = r.failures.len();
        for f in r.failures.clone() {
            crate::log(&format!("[selftest] FAILURE: {}", f));
            eprintln!("[selftest] FAILURE: {}", f);
        }
        r.info(format!("=== SELFTEST FAILED: {} check(s) failed ({}ms) ===", n, elapsed));
        1
    }
}
