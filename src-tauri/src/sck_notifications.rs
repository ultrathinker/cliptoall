//! Stale-cache invalidation for the ScreenCaptureKit capture path.
//!
//! macOS posts `NSApplicationDidChangeScreenParametersNotification` when
//! anything about the display arrangement changes — a monitor is connected
//! or disconnected, the resolution of an existing display changes, the lid
//! is closed (which on a closed laptop with an external display changes
//! which one is the active display), or the dock moves to a different
//! monitor. When that happens, our cached `SCShareableContent` is stale:
//! the `SCDisplay` it carries may no longer exist, or may point at a
//! display whose backing size is different from what the next capture will
//! think. Without this hook, the first capture after any of those events
//! is the one that discovers the staleness — either via a `captureImage`
//! error (if the display is gone) or, worse, silently via a capture of the
//! wrong screen at the wrong resolution (if SCK still resolves one).
//!
//! We invalidate the cache *eagerly* in response to the notification, so
//! the next capture re-resolves before it does anything visible. The
//! cheap-but-sometimes-wrong cost is one extra `getShareableContent`
//! round-trip (~50-70 ms) when nothing actually changed; the alternative
//! is hidden-by-a-bug captures, which is not acceptable.
//!
//! Only the *content* cache needs invalidating. `DisplayGeometry` is
//! derived from `CGDisplayCopyDisplayMode` per capture, so a resolution
//! change is already picked up there without any cache to clear.

use block2::RcBlock;
use objc2::runtime::AnyObject;
use std::ffi::c_void;
use std::sync::Mutex;

/// Newtype wrapping the raw `*mut AnyObject` returned by
/// `addObserverForName:object:queue:usingBlock:`. We must retain this
/// pointer for the lifetime of the app — the framework stores its block
/// table by observer pointer, so dropping it unsubscribes. The field is
/// therefore deliberately never read; it exists only to keep the
/// observer alive.
struct ObserverPtr(#[allow(dead_code)] *mut AnyObject);
// SAFETY: We never mutate or free the observed object; we just hold the
// pointer alive. We pass it to `invalidate_cached_content` from off the
// main queue ONLY to identify it in logs (never called), and we never
// cross thread boundaries in practice — but Send+Sync are sound anyway
// because nothing owns the pointee uniquely.
unsafe impl Send for ObserverPtr {}
unsafe impl Sync for ObserverPtr {}

static OBSERVER: Mutex<Option<ObserverPtr>> = Mutex::new(None);

/// Install the observer exactly once. Safe to call more than once — a
/// second call without a matching removal would leak an observer, so we
/// no-op. `Tauri` calls `.setup()` once; this is a belt for that single
/// call's braces, not a replacement for process-scoped uniqueness.
pub fn install_once() {
    let mut guard = OBSERVER.lock().unwrap();
    if guard.is_some() {
        return;
    }

    // `NSNotificationName` is a type alias for `NSString` in
    // `objc2-foundation`, so the runtime treats this transparently.
    let name = objc2_foundation::NSString::from_str(
        "NSApplicationDidChangeScreenParametersNotification",
    );

    // The block runs on the main queue, so it does not contend with the
    // capture-hot-path thread. All it does is clear one `Mutex<Option<…>>`,
    // which is measured in nanoseconds.
    let block = RcBlock::new(move |_note: *mut c_void| {
        crate::sck_capture::invalidate_cached_content(
            "NSApplicationDidChangeScreenParametersNotification",
        );
    });

    // `queue: nil` → posts synchronously on the notifying thread (per
    // Apple docs). macOS posts this notification from a system thread,
    // not from any UI thread we care about — and `invalidate_cached_content`
    // does nothing but take a Mutex, which is safe from any thread. We
    // deliberately avoid the main queue: (a) it gives us no ordering
    // benefit since our handler isn't UI-adjacent, and (b) synchronous
    // posting is what the self-test relies on to verify the observer
    // actually fires (`post_screen_params_change_for_test` below returns
    // only after the cache has been cleared, so a cache-empty check that
    // follows it is unambiguous).
    //
    // `object: nil` → all senders; we don't care who posted.
    //
    // SAFETY: the lifetime of `name` and `block` covers this call (both
    // are held alive by local variables until after the msg_send
    // returns and we've retained the observer token). The retained
    // observer token outlives both via `OBSERVER`; if the process exits
    // before then, that's the process exiting, so any "leak" stops.
    let center: *mut AnyObject = unsafe {
        objc2::msg_send![objc2::class!(NSNotificationCenter), defaultCenter]
    };
    let observer: *mut AnyObject = unsafe {
        objc2::msg_send![
            center,
            addObserverForName: &*name,
            object: std::ptr::null::<AnyObject>(),
            queue: std::ptr::null::<AnyObject>(),
            usingBlock: &*block,
        ]
    };

    if observer.is_null() {
        crate::log("    [capture] WARN: failed to install NSApplicationDidChangeScreenParametersNotification observer");
        return;
    }

    // Acquire a +1 retain immediately. The +1 lives in OBSERVER until
    // process exit; same observer-ownership contract as
    // SCShareableContent/CGImage above.
    let retained_ptr: *mut AnyObject = unsafe { objc2::msg_send![observer, retain] };
    if retained_ptr.is_null() {
        crate::log("    [capture] WARN: NSNotificationCenter observer retain returned None");
        return;
    }
    crate::log("    [capture] installed NSApplicationDidChangeScreenParametersNotification observer (cache will be eagerly invalidated on display changes)");
    *guard = Some(ObserverPtr(retained_ptr));
}

/// True when `install_once` succeeded (a live observer is held in `OBSERVER`).
/// Used by the self-test to verify the install step without forcing a real
/// display change.
#[allow(dead_code)]
pub fn is_observer_installed() -> bool {
    OBSERVER.lock().unwrap().is_some()
}

/// Post `NSApplicationDidChangeScreenParametersNotification` programmatically
/// so the self-test can confirm the observer actually fires — without a real
/// display change. The notification observer will run on the main queue, so
/// this must be called from a thread that allows the main queue's run loop
/// to drain. `run_on_main` below provides that.
#[allow(dead_code)]
pub fn post_screen_params_change_for_test() {
    let name = objc2_foundation::NSString::from_str(
        "NSApplicationDidChangeScreenParametersNotification",
    );
    let center: *mut AnyObject = unsafe {
        objc2::msg_send![objc2::class!(NSNotificationCenter), defaultCenter]
    };
    unsafe {
        objc2::msg_send![
            center,
            postNotificationName: &*name,
            object: std::ptr::null::<AnyObject>(),
            userInfo: std::ptr::null::<AnyObject>(),
        ]
    }
}
