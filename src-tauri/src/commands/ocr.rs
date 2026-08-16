//! On-device text recognition ("Copy text") for the macOS build.
//!
//! Why this module exists: see `docs/macos-port/APP-STORE-PLAN.md` §6 and
//! the brief at `mx-ocr.md`. The capture is already a temp PNG on disk;
//! Apple's Vision framework (`VNRecognizeTextRequest`) reads it, runs
//! recognition entirely on-device with no entitlement and no network, and
//! returns the recognised text. It is the strongest single answer to
//! App Review guideline 4.3 ("why admit another screenshotter").
//!
//! ## Design notes
//!
//! * **Why an existing crate, not hand-rolled bindings.** `objc2-vision`
//!   0.3 exists and is compatible with the project's `objc2 0.6` stack
//!   (`objc2-vision` requires `objc2 >=0.6.2, <0.8.0`; the lock resolves to
//!   0.6.4). The other macOS bindings in this repo are hand-rolled because
//!   the relevant crates do not exist on crates.io (`objc2-service-management`)
//!   or because the binding is one selector long and not worth a new dep
//!   (`SMAppService` in `utils/autorun.rs`). Vision's surface is large
//!   enough — `VNRecognizeTextRequest` alone has 8 typed properties plus
//!   the `supportedRecognitionLanguagesAndReturnError:` and the array of
//!   observations — that typed bindings catch more mistakes than they cost.
//!   The features we opt into are listed in `Cargo.toml` (default features
//!   are off; everything else drags in `objc2-core-image`, `objc2-image-io`
//!   and dozens of Vision request types we will never touch).
//!
//! * **`recognize_text` is `async` + `spawn_blocking`.** Vision's "accurate"
//!   recognition takes a few hundred milliseconds. Tauri dispatches sync
//!   commands on its own threadpool, which would block other IPC traffic
//!   for that window; wrapping in `spawn_blocking` makes it explicit and
//!   lets Tokio schedule concurrent captures if needed.
//!
//! * **Empty result is success.** Vision returning no recognised text is a
//!   normal outcome (e.g. a screenshot of a photo, a UI without copyable
//!   labels), not a failure. The brief says to return an empty string in
//!   that case, and that is what the frontend distinguishes "no text found"
//!   from "OCR failed".
//!
//! * **Language handling.** Per the brief: query
//!   `supportedRecognitionLanguagesAndReturnError:` (macOS 12+, our floor
//!   is 14) on a fresh `VNRecognizeTextRequest` to learn what the running
//!   Vision supports; intersect with `NSLocale.preferredLanguages`; pass
//!   the intersection as `recognitionLanguages` (preserving the user's
//!   preferred order, which is the order Vision consults its language
//!   models). Also enable `automaticallyDetectsLanguage` so Vision can
//!   switch off our list when the visible text is in another script
//!   (e.g. Russian user pasting a Chinese screenshot).
//!
//! * **Reading order.** Vision returns observations in top-to-bottom order
//!   of the text blocks it detected; we join top-candidate strings with
//!   `\n` and do not attempt layout reconstruction. The brief is explicit
//!   about this: "Do not attempt layout reconstruction".

#![cfg(target_os = "macos")]

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send, sel, AnyThread};
use objc2_foundation::{NSArray, NSDictionary, NSString, NSURL};
use objc2_vision::{
    VNImageRequestHandler, VNRecognizeTextRequest, VNRecognizedTextObservation,
    VNRequest, VNRequestTextRecognitionLevel,
};
use std::path::Path;

use crate::commands::capture::ensure_temp_screenshot_path;

/// On-device OCR entry point exposed to the frontend. Takes the path of an
/// existing ClipToAll temp screenshot, returns the recognised text (empty
/// string when no text is found, error string when recognition failed).
///
/// Async + `spawn_blocking` so the recognition CPU time does not block the
/// Tauri command worker or any other in-flight IPC.
#[tauri::command]
pub async fn recognize_text(image_path: String) -> Result<String, String> {
    // Reuse the existing path validator — the brief is explicit that this
    // is "not optional" and the same rationale (don't OCR an attacker-chosen
    // file) applies here as for every other path-taking command.
    ensure_temp_screenshot_path(&image_path)?;
    let path_buf = std::path::PathBuf::from(&image_path);
    let span = std::time::Instant::now();
    crate::log(&format!("    [ocr] recognize_text begin: {}", image_path));
    let result = tokio::task::spawn_blocking(move || recognize_text_blocking(&path_buf))
        .await
        .map_err(|e| format!("OCR task join failed: {}", e))?;
    crate::log(&format!(
        "    [ocr] recognize_text done ({} ms, {} bytes)",
        span.elapsed().as_millis(),
        result.as_ref().map(|s| s.len()).unwrap_or(0)
    ));
    result
}

/// Synchronous worker. Lives off the Tokio task so we can name it in
/// `spawn_blocking` without dragging `Send` requirements into the recognition
/// code itself. Returns the joined top-candidate strings, or an error.
fn recognize_text_blocking(path: &Path) -> Result<String, String> {
    // The temp screenshot is on disk; let ImageIO (used by Vision under the
    // hood) read it via `initWithURL:options:`. This is the cheapest path in
    // — no pixels across the IPC boundary, no re-encoding.
    let url = NSURL::from_file_path(path)
        .ok_or_else(|| format!("Could not build NSURL from '{}'", path.display()))?;

    // `initWithURL:options:` takes an NSDictionary<VNImageOption, AnyObject>.
    // For a still PNG the dictionary is empty — the auxiliary options
    // (camera intrinsics, CIContext) all apply to live capture / sample
    // buffer inputs. We construct an empty NSDictionary rather than
    // `nil`-ing it because the parameter is typed `&NSDictionary`, not
    // `Option<&NSDictionary>`.
    //
    // VNImageRequestHandler header (`VNRequestHandler.h`): no NSDictionary
    // feature gate is required, but we do need the feature for `NSDictionary`
    // in objc2-foundation (added in Cargo.toml alongside this module).
    let empty_options: Retained<NSDictionary<NSString, AnyObject>> =
        NSDictionary::new();
    let handler = unsafe {
        VNImageRequestHandler::initWithURL_options(
            VNImageRequestHandler::alloc(),
            &url,
            &empty_options,
        )
    };

    // Create the request. VNRecognizeTextRequest inherits from
    // VNImageBasedRequest -> VNRequest, whose designated init is
    // `initWithCompletionHandler:`; the no-completion init (`init`) is
    // inherited from VNRequest and is what we want — we read the result
    // synchronously from `results` after `performRequests:error:` returns.
    let request: Retained<VNRecognizeTextRequest> = unsafe {
        msg_send![VNRecognizeTextRequest::alloc(), init]
    };

    // ── Languages: query support, intersect with user preference ──────────
    //
    // `supportedRecognitionLanguagesAndReturnError:` is an instance method
    // (macOS 12+, our floor 14) — its result reflects the request's *current*
    // configuration. We haven't set anything yet, so it gives us the universe
    // of languages Vision supports in this configuration.
    let supported_ns: Retained<NSArray<NSString>> = supported_languages(&request)?;
    let preferred_ns: Retained<NSArray<NSString>> = preferred_languages()?;

    let supported: Vec<String> = (0..supported_ns.len())
        .map(|i| supported_ns.objectAtIndex(i).to_string())
        .collect();
    let preferred: Vec<String> = (0..preferred_ns.len())
        .map(|i| preferred_ns.objectAtIndex(i).to_string())
        .collect();

    // Log the two lists — the brief asks us to "report what the query actually
    // returned on this machine", and this is the only place that information
    // lives. The format is fixed so it is greppable in cliptoall.log.
    crate::log(&format!(
        "    [ocr] supported={:?} preferred={:?}",
        supported, preferred
    ));

    // Preserve the user's preferred order (Vision consults languages in the
    // order of the array — see VNRecognizeTextRequest.h:54 docstring on
    // `recognitionLanguages`). A user-preferred language not in Vision's
    // supported list is dropped silently; the rest pass through unchanged.
    let chosen: Vec<Retained<NSString>> = preferred
        .iter()
        .filter(|p| supported.iter().any(|s| s == *p))
        .map(|p| NSString::from_str(p))
        .collect();

    if chosen.is_empty() {
        // No overlap. Skip setRecognitionLanguages so Vision keeps its
        // default (English for the accurate model); `automaticallyDetectsLanguage`
        // is still set below, which is how the brief expects non-preferred
        // text to be recognised.
        crate::log("    [ocr] no overlap between supported and preferred languages — relying on auto-detect");
    } else {
        let lang_array = NSArray::from_slice(&chosen.iter().map(|s| &**s).collect::<Vec<_>>());
        request.setRecognitionLanguages(&lang_array);
    }

    // ── Configuration: accurate, with language correction, auto-detect ────
    //
    // 0 = VNRequestTextRecognitionLevelAccurate. Defined in
    // VNRecognizeTextRequest.h:19. The brief mandates "accurate".
    request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    request.setUsesLanguageCorrection(true);
    // `automaticallyDetectsLanguage` is gated behind VNRecognizeTextRequestRevision3
    // (VNRecognizeTextRequest.h:74, "API_AVAILABLE(macos(13.0))"). Our floor is 14,
    // so it is always available at runtime; the binding accepts the setter
    // unconditionally because the objc2-vision codegen doesn't know about
    // API_AVAILABLE.
    request.setAutomaticallyDetectsLanguage(true);

    // ── Run ────────────────────────────────────────────────────────────────
    //
    // `performRequests:error:` is a synchronous call that runs all the
    // requests in the array on the handler's image and returns once they
    // are done. Definition: VNRequestHandler.h:238.
    let requests_array = NSArray::from_slice(&[(&*request as &VNRequest) as &VNRequest]);
    let perform_result = handler.performRequests_error(&requests_array);
    if let Err(ns_err) = perform_result {
        return Err(format!(
            "VNImageRequestHandler.performRequests failed: {}",
            ns_err.localizedDescription()
        ));
    }

    // ── Collect recognised text ────────────────────────────────────────────
    //
    // `results` is the array of VNRecognizedTextObservation sorted by Vision's
    // own reading-order pass (top-to-bottom of detected blocks). For each
    // observation we ask for the single top candidate (`topCandidates:1` —
    // VNRecognizedTextObservation at VNObservation.h:399) and read `.string`
    // (VNRecognizedText at VNObservation.h:371). The brief is explicit: do
    // not attempt layout reconstruction — `\n` between observations is the
    // entire job.
    let Some(observations) = request.results() else {
        // `results` is `Option<Retained<...>>`; nil here means the request
        // failed before populating. We already returned Err on
        // performRequests_error above, so a None here is unusual — log and
        // return empty (the brief: "no text found" is a normal outcome).
        crate::log("    [ocr] request.results() returned None — treating as empty");
        return Ok(String::new());
    };

    let mut lines: Vec<String> = Vec::with_capacity(observations.len());
    for i in 0..observations.len() {
        let obs: &VNRecognizedTextObservation = &observations.objectAtIndex(i);
        let candidates = obs.topCandidates(1);
        let first = candidates
            .iter()
            .next()
            .map(|c| c.string().to_string());
        if let Some(s) = first {
            if !s.is_empty() {
                lines.push(s);
            }
        }
    }

    Ok(lines.join("\n"))
}

/// Languages this Vision build supports, in the order the framework reports
/// them. `supportedRecognitionLanguagesAndReturnError:` is macOS 12+ (our
/// floor is 14); declared at VNRecognizeTextRequest.h:47.
fn supported_languages(
    request: &VNRecognizeTextRequest,
) -> Result<Retained<NSArray<NSString>>, String> {
    // SAFETY: msg_send! into objc2-vision's typed binding would be cleaner,
    // but the typed binding is a `Result<Retained<...>, Retained<NSError>>`
    // and we want the same error path regardless of which side fails. Hand-
    // rolling here keeps both failure modes in one place.
    let mut err_ptr: *mut objc2_foundation::NSError = std::ptr::null_mut();
    let ptr: *mut NSArray<NSString> = unsafe {
        msg_send![request, supportedRecognitionLanguagesAndReturnError: &mut err_ptr]
    };
    if ptr.is_null() {
        let msg = if err_ptr.is_null() {
            "supportedRecognitionLanguagesAndReturnError returned null with no NSError".to_string()
        } else {
            // SAFETY: err_ptr is non-null; we read it synchronously and the
            // error object is valid for the call's duration.
            let err: &objc2_foundation::NSError = unsafe { &*err_ptr };
            format!(
                "supportedRecognitionLanguagesAndReturnError failed: {}",
                err.localizedDescription()
            )
        };
        return Err(msg);
    }
    // SAFETY: Vision returns a +0 autoreleased NSArray per the Objective-C
    // memory model. Retain for the duration of our Retained<...>.
    unsafe { Retained::retain(ptr) }.ok_or_else(|| "Retained::retain failed on supported languages".to_string())
}

/// User's preferred languages, in System Settings → Language & Region order.
/// `+[NSLocale preferredLanguages]` returns NSArray<NSString*>. The
/// `objc2-foundation` 0.3 crate's `NSLocale` feature is NOT in our feature
/// list (we only need NSArray / NSDictionary / NSError / NSString / NSURL),
/// so we use a raw `msg_send!` against the class — exactly the pattern
/// `utils/autorun.rs` uses for `SMAppService`, which has the same "small
/// framework, no crate" shape.
fn preferred_languages() -> Result<Retained<NSArray<NSString>>, String> {
    let cls: *const objc2::runtime::AnyClass = class!(NSLocale);
    if cls.is_null() {
        return Err("NSLocale class unavailable".to_string());
    }
    // Ask before sending: an unrecognised selector is an Objective-C
    // exception, which cannot be caught here and would abort the process.
    // (The whole reason HANDOFF's bug catalogue lists the SMAppService
    // `mainApp` mistake first.)
    let responds: bool = unsafe {
        msg_send![cls, respondsToSelector: sel!(preferredLanguages)]
    };
    if !responds {
        return Err("NSLocale does not respond to preferredLanguages".to_string());
    }
    let ptr: *mut NSArray<NSString> = unsafe {
        msg_send![cls, preferredLanguages]
    };
    if ptr.is_null() {
        return Err("NSLocale.preferredLanguages returned null".to_string());
    }
    // SAFETY: NSLocale class methods return +0 (autoreleased). Retain for
    // our use; the returned Retained releases the +1 when it drops.
    unsafe { Retained::retain(ptr) }
        .ok_or_else(|| "Retained::retain failed on preferred languages".to_string())
}
