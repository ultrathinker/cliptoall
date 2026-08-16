#[cfg(windows)]
use winreg::enums::*;
#[cfg(windows)]
use winreg::RegKey;

#[cfg(windows)]
pub fn set_autorun(enable: bool) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let path = r"Software\Microsoft\Windows\CurrentVersion\Run";
    let key = hkcu.open_subkey_with_flags(path, KEY_WRITE)
        .map_err(|e| format!("Failed to open registry key: {}", e))?;

    if enable {
        let exe_path = std::env::current_exe()
            .map_err(|e| format!("Failed to get exe path: {}", e))?;

        // Quote the path so spaces (e.g. C:\Program Files\...) parse unambiguously.
        let value = format!("\"{}\"", exe_path.to_string_lossy());
        key.set_value("ClipToAll", &value)
            .map_err(|e| format!("Failed to set registry value: {}", e))?;
    } else {
        key.delete_value("ClipToAll").ok();
    }

    Ok(())
}

/// macOS autostart via `SMAppService.mainApp`.
///
/// Why not the previous LaunchAgent plist: a sandboxed app cannot write
/// `~/Library/LaunchAgents/<label>.plist` and cannot invoke `launchctl`, so the
/// old implementation would have silently failed in the App Store build.
/// `SMAppService` (ServiceManagement framework, macOS 13+; our floor is 14) is
/// the sandbox-safe replacement: the framework writes the plist into
/// `~/Library/Containers/<bundle-id>/Data/Library/LaunchAgents/` on the user's
/// behalf, and `launchd` reads it from there.
///
/// ### Why a raw `msg_send!` and not a crate
///
/// `objc2-service-management` does not exist on crates.io. ServiceManagement's
/// `SMAppService` API surface is tiny — four methods, all public, all stable
/// since macOS 13 — so a hand-rolled binding against the framework is simpler
/// than adopting a new crate family for one class. The `objc2 0.6` stack
/// already in `Cargo.toml` provides `msg_send!`, `class!`, `Retained`, and the
/// NSError/NSObject bridges; the framework link is added in `build.rs`.
///
/// ### Registration is tied to the user's explicit toggle
///
/// `set_autorun(true)` only runs when the user flips the setting in
/// Settings.svelte (the call site is `save_settings_to_disk_locked` in
/// `commands/settings.rs`). The previous "default on" behaviour was changed
/// in Phase 3 (HANDOFF bug #26's neighbour).
///
/// ### Surface real errors
///
/// `registerAndReturnError:` returns `false` if the user denied the
/// authorization in System Settings → General → Login Items (the OS only
/// prompts the first time, but the user can revoke later). The previous
/// implementation reported success unconditionally — that would have meant the
/// settings checkbox and the system disagreeing silently. We propagate the
/// error string verbatim so the Settings log shows the real reason.
#[cfg(target_os = "macos")]
mod smapp {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSError, NSString};

    /// `SMAppService.mainApp` — the application-as-a-login-item service.
    /// Returned autoreleased by the framework; we retain it locally.
    pub(super) fn main_app() -> Option<Retained<AnyObject>> {
        let cls: *const objc2::runtime::AnyClass = objc2::class!(SMAppService);
        if cls.is_null() {
            return None;
        }
        // The selector is `mainAppService`. `mainApp` is only the SWIFT name
        // (`NS_SWIFT_NAME(mainApp)` on the `mainAppService` class property in
        // SMAppService.h) — sending `mainApp` raises
        // NSInvalidArgumentException, which unwinds through Rust and aborts
        // the process on launch, because settings are loaded before any window
        // exists. Do not "simplify" this back to the name in the Swift docs.
        //
        // Ask before sending: an unrecognised selector is an Objective-C
        // exception, and there is no way to catch it here, so a wrong name is
        // a crash rather than an error. This check turns any future mismatch
        // into a logged failure and a disabled checkbox.
        let responds: bool =
            unsafe { objc2::msg_send![cls, respondsToSelector: objc2::sel!(mainAppService)] };
        if !responds {
            crate::log("[autorun] SMAppService does not respond to mainAppService — autostart unavailable");
            return None;
        }
        let ptr: *mut AnyObject = unsafe { objc2::msg_send![cls, mainAppService] };
        if ptr.is_null() {
            return None;
        }
        // SAFETY: SMAppService.mainApp returns +0 (autoreleased). `retain`
        // gives us a +1 we own for the duration of the local binding; the
        // returned Retained drops it when it goes out of scope.
        unsafe { Retained::retain(ptr) }
    }

    /// `SMAppService.status` — an `SMAppServiceStatus` (NSInteger) enum:
    ///   0 = notRegistered, 1 = enabled, 2 = requiresApproval, 3 = notFound.
    /// We only use it to log the post-register state — the actual
    /// success/failure signal is `registerAndReturnError:`'s BOOL.
    ///
    /// The Objective-C signature returns `NSInteger` directly (not an
    /// `NSNumber*`); on the Rust side we get the raw `long` value. `-1` is
    /// returned on any FFI failure as a sentinel — the only caller logs it
    /// rather than acting on it.
    pub(super) fn status(_service: &AnyObject) -> i64 {
        // SAFETY: `status` is a `- (NSInteger)status` method on SMAppService.
        // NSInteger is `long` on LP64 platforms (all our targets), so the
        // return type matches `i64` after a sign-extending cast.
        let raw: i64 = unsafe { objc2::msg_send![_service, status] };
        raw
    }

    /// `registerAndReturnError:` — returns `true` on success, populates
    /// `*error` with an NSError on failure. The OS shows its own
    /// authorization prompt on the first call (and only the first call);
    /// a user who later revokes in System Settings sees an error on the
    /// next call instead.
    pub(super) fn register(service: &AnyObject) -> Result<(), String> {
        let mut err_ptr: *mut NSError = std::ptr::null_mut();
        let ok: bool = unsafe {
            objc2::msg_send![service, registerAndReturnError: &mut err_ptr]
        };
        if ok {
            Ok(())
        } else {
            Err(format_smapp_error("register", err_ptr))
        }
    }

    /// `unregisterAndReturnError:` — symmetric to `register`.
    pub(super) fn unregister(service: &AnyObject) -> Result<(), String> {
        let mut err_ptr: *mut NSError = std::ptr::null_mut();
        let ok: bool = unsafe {
            objc2::msg_send![service, unregisterAndReturnError: &mut err_ptr]
        };
        if ok {
            Ok(())
        } else {
            Err(format_smapp_error("unregister", err_ptr))
        }
    }

    /// Render an NSError returned from `*AndReturnError:` into something
    /// useful for the Settings log. NSError's own `localizedDescription`
    /// method returns the userInfo's `NSLocalizedDescription` (Apple's docs:
    /// "the human-readable error message") — using it avoids having to dig
    /// into the userInfo dictionary directly with `objectForKey:`, which
    /// the objc2 0.6 `msg_send!` macro parser does not handle cleanly
    /// (selectors with colons after a multi-token selector segment fail to
    /// parse).
    fn format_smapp_error(op: &str, err: *mut NSError) -> String {
        if err.is_null() {
            return format!("SMAppService.{} failed with no NSError", op);
        }
        // SAFETY: err is a valid +0 NSError (the framework guarantees it for
        // the lifetime of the call, and we read it synchronously here).
        let err_ref: &NSError = unsafe { &*err };
        let code = err_ref.code();
        // `localizedDescription` returns +1 (Retained<NSString>) per objc2
        // 0.6's NSError declaration; use the objc2-generated method to keep
        // the retain/release bookkeeping honest.
        let desc_nsstring: Retained<NSString> = err_ref.localizedDescription();
        let desc = desc_nsstring.to_string();
        if desc.is_empty() {
            format!("SMAppService.{} failed (code {})", op, code)
        } else {
            format!("SMAppService.{} failed (code {}): {}", op, code, desc)
        }
    }
}

/// Path of the LaunchAgent plist the previous (pre-SMAppService) implementation
/// used to write. Kept as a constant so the migration in
/// `remove_legacy_launch_agent_if_present` is unambiguous about what it is
/// trying to clean up.
///
/// The plist label and filename are derived from `tauri.conf.json`'s
/// `identifier` (`net.appshub.cliptoall`); if that ever changes, the
/// migration in `remove_legacy_launch_agent_if_present` would silently miss
/// the old file. The HANDOFF.md §1 ground rule "Windows is frozen" applies to
/// Windows code paths; this constant is the single place the old label lives.
#[cfg(target_os = "macos")]
const LEGACY_LAUNCH_AGENT_LABEL: &str = "net.appshub.cliptoall";

#[cfg(target_os = "macos")]
fn legacy_launch_agent_path() -> Option<std::path::PathBuf> {
    let home = dirs::home_dir()?;
    Some(
        home.join("Library/LaunchAgents")
            .join(format!("{}.plist", LEGACY_LAUNCH_AGENT_LABEL)),
    )
}

/// One-shot migration: a user who already has the old LaunchAgent plist (from
/// an earlier unsandboxed build of this app) would otherwise end up with TWO
/// autostart mechanisms — the old `launchctl`-managed plist plus the new
/// SMAppService-registered job. Calling `launchctl unload` from inside a
/// sandboxed app is blocked, and removing the file in
/// `~/Library/LaunchAgents` is also blocked (that path is OUTSIDE the app's
/// container, so the sandbox forbids writes there). We attempt the removal
/// best-effort and log the result either way; the user-visible "Open at Login"
/// state stays correct because SMAppService's view of the world is the
/// authoritative one.
///
/// Why this is acceptable: on a future unsandboxed run (e.g. the Developer ID
/// GitHub edition that signs without `app-sandbox`), the SAME function will
/// successfully remove the legacy file. The two distributions differ only in
/// signing, not in this code path.
#[cfg(target_os = "macos")]
fn remove_legacy_launch_agent_if_present() {
    let Some(path) = legacy_launch_agent_path() else {
        return;
    };
    match std::fs::remove_file(&path) {
        Ok(()) => crate::log(&format!(
            "[autorun] removed legacy LaunchAgent plist at {}",
            path.display()
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // No legacy file: the expected case on a clean install.
        }
        Err(e) => {
            // Either sandbox-blocked (EACCES / EPERM) or some other I/O error.
            // Log it so a reviewer can see we tried; do NOT fail set_autorun
            // for this — the new SMAppService registration is independent and
            // correct on its own.
            crate::log(&format!(
                "[autorun] WARN: could not remove legacy LaunchAgent plist at {} \
                 (likely sandbox-blocked — the user may have a duplicate autostart entry \
                 until they manually run `launchctl unload -w {}`): {}",
                path.display(),
                path.display(),
                e,
            ));
        }
    }
}

#[cfg(target_os = "macos")]
pub fn set_autorun(enable: bool) -> Result<(), String> {
    // One-shot migration: clear any LaunchAgent plist left by the pre-SMAppService
    // build. Idempotent and safe on every call — it's a single `remove_file`
    // that succeeds once and then errors with NotFound forever after.
    remove_legacy_launch_agent_if_present();

    let service = smapp::main_app()
        .ok_or_else(|| "SMAppService.mainApp returned null (framework unavailable?)".to_string())?;
    let service_ref: &objc2::runtime::AnyObject = &service;

    // Do nothing if we are already in the requested state. `settings.rs` calls
    // this on EVERY settings save, not only when the flag changes — the
    // LaunchAgent implementation skipped its work the same way, for the same
    // reason. Without the check, every save reaches out to a system service,
    // and every save with autostart off calls `unregister` on a service that
    // was never registered, which logs a failure that is not one.
    //
    // `requiresApproval` (2) counts as registered: the job exists and re-calling
    // register cannot clear it — only the user can, in System Settings.
    let before = smapp::status(service_ref);
    let registered = before == 1 || before == 2;
    if registered == enable {
        if before == 2 {
            crate::log("[autorun] SMAppService status is requiresApproval — the user must enable ClipToAll under System Settings > General > Login Items");
        }
        return Ok(());
    }

    let result = if enable {
        smapp::register(service_ref)
    } else {
        smapp::unregister(service_ref)
    };

    // Post-state log: enabled/requiresApproval/etc. The status enum is
    // informational; failure has already been reported via the result above.
    let status = smapp::status(service_ref);
    crate::log(&format!(
        "[autorun] SMAppService.mainApp {} -> status {} (0=notRegistered, 1=enabled, 2=requiresApproval, 3=notFound)",
        if enable { "register" } else { "unregister" },
        status,
    ));

    result
}
