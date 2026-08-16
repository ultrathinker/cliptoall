# macOS Port — Handoff

> **Read this first.** It is the single entry point for anyone (human or AI)
> continuing the macOS port. It states where the port actually is, how to
> build and run it, every non-obvious bug already hit and what the fix taught
> us, and what to do next.
>
> Companion documents, in reading order:
> - [`PLAN.md`](PLAN.md) — original intent + the audit of which Windows files
>   are platform-specific. Historical context; still accurate.
> - [`EXECUTION-PLAN.md`](EXECUTION-PLAN.md) — phase breakdown, decisions,
>   and a dated progress log. **Keep its progress log updated.**
> - [`OVERLAY-SPEC.md`](OVERLAY-SPEC.md) — the pixel/behaviour contract the
>   macOS web overlay must match, extracted from the Windows `overlay.rs`.
> - [`WORKLOG-2026-08-16.md`](WORKLOG-2026-08-16.md) — one day's narrative:
>   the ScreenCaptureKit migration, the five bugs fixed that day, measured
>   before/after numbers, and what is left open. Read it for the story behind
>   catalogue entries #21–#25; this file remains the reference.
>
> Branch: **`macos-port`** (not merged to `main`).
> Last updated: 2026-08-16.

---

## 1. Where the port is

Working and verified by hand on a real Mac (M1, macOS 26.5, Retina 2×):

| Area | State |
|---|---|
| Compiles + runs on macOS (arm64) | ✅ |
| Screen capture (`ScreenCaptureKit`) | ✅ DONE — `SCScreenshotManager.captureImage` (2560×1600, primary monitor only). Round 1 of the migration shipped two critical defects that both passed visual inspection; round 2 caught them with an oracle-checked self-test (see §4, bugs #21, #22). |
| Region-select overlay (Svelte canvas) | ✅ drag, size label, Esc / right-click cancel, Shift-square, plugin hotkeys, mode tint |
| Crop → save → clipboard → Results window | ✅ full round trip |
| Clipboard image (`arboard` / NSPasteboard) | ✅ |
| Secret storage (Keychain via `keyring`) | ✅ |
| Autostart (LaunchAgent plist) | ✅ |
| Google Drive OAuth | ✅ (needs build-time credentials, see §3) |
| Plugin discovery / encryption-plugin | ✅ compiles + runs; **not** exercised end-to-end with real plugins |
| Signing / notarization pipeline | ❌ not started (certs exist, see §3) |
| `.app` / `.dmg` bundle | ❌ not started |
| Windows build still green | ❓ **unverified** — see §8 |

**Not done, deliberately:** multi-monitor capture, ScreenCaptureKit
migration (§4), plugins-directory relocation (§7), Phases 4–6 of
`EXECUTION-PLAN.md`.

---

## 2. Ground rules for this port

1. **Windows is frozen.** Every Windows code path must stay byte-identical.
   New macOS code goes behind `#[cfg(target_os = "macos")]` (or
   `#[cfg(not(windows))]` — but see bug #8 for when that distinction bites).
   Never "unify" or "modernize" a Windows path while here.
2. **Repo language is English** — code, comments, commit messages, docs.
3. **The repo is public** (`ultrathinker/cliptoall`). No credentials, no
   Team ID, no personal names/emails in committed files. Commits are authored
   as `ultrathinker <universeissilent42@gmail.com>` (already configured
   per-repo).
4. **Verify by running, not by reasoning.** Nearly every bug in §6 compiled
   fine and looked correct on review. Several were found only by printing a
   measured number or looking at a real screenshot.

---

## 3. Build and run

```bash
cd <repo root>
set -a; source .env.local; set +a     # NOT in git — see below
npm run tauri:dev
```

`.env.local` (gitignored via the existing `.env.*` rule) must contain:

| Variable | What it is |
|---|---|
| `GDRIVE_CLIENT_ID` / `GDRIVE_CLIENT_SECRET` | Google OAuth "Desktop app" client, baked in at build time by `option_env!` in `commands/upload_gdrive.rs`. Without them Drive silently disables itself with a clear runtime error; S3 still works. Official releases inject the same values from GitHub Actions secrets. Ask the repo owner for the values. |
| `APPLE_DEV_SIGNING_IDENTITY` | SHA-1 of a **stable** codesigning identity (`security find-identity -v -p codesigning`). Consumed by `src-tauri/scripts/macos-dev-runner.sh` via `src-tauri/.cargo/config.toml`'s `runner`. See bug #11 for why this exists. |

**Certificates already issued on this machine:** `Apple Development` and
`Developer ID Application` (the latter needed an updated Program License
Agreement accepted at developer.apple.com first — Xcode reports that as the
unhelpful `Unable to process request - PLA Update available`).

**Logs:** `~/Library/Application Support/ClipToAll/logs/cliptoall.log`.
Only written when `loggingOn: true` in
`~/Library/Application Support/ClipToAll/settings.json`. Turn it on before
debugging anything timing-related — the capture path is already instrumented
with `+Nms` breadcrumbs at every stage.

---

## 4. ScreenCaptureKit migration (DONE — 2026-08-16)

`xcap` / `CGWindowListCreateImage` (deprecated since macOS 14) was replaced
with `SCScreenshotManager.captureImage` via `objc2-screen-capture-kit`.
**Round-2 timing on the dev machine (2560×1600 primary monitor, full
backing-store resolution — 4× the pixels of the round-1 measurements
below)**, three `CLIPTOALL_SELFTEST_CAPTURE=1` runs of the signed app,
each running 5 timed captures:

| metric | round 1 (1280×800) | round 2 (2560×1600, warm cache) |
|---|---|---|
| `captureImage` (SCK call only) | 141–162 ms | 60–62 ms |
| full end-to-end `capture_to_memory` | 147–169 ms | 79–100 ms |
| source of ground truth | `+Nms` breadcrumbs in the log (catch rounds logged) | `CaptureTimings` struct returned by `sck_capture::capture_via_sck_timed` |

The round-1 "203 / 205 / 243 ms" numbers are now void and are not safe to
quote: round 1 was capturing at **a quarter of the display's backing-store
resolution** (bug #21 below), so a "fast capture at 1280×800" was the
result of asking for fewer pixels rather than a real speedup over xcap.
The same applies to the cached "first pixel RGBA" comment — round 1's
output was also byte-swapped (bug #22), so the recorded value is not
trustworthy either. Round 2 captures the full 2560×1600 backing store
correctly and is genuinely faster than xcap was; the latest numbers are
in `mx-sck-report.md`.

**The verification approach changed as well.** Round 1 verified by
saving a PNG and eyeballing it. Two critical defects both passed that
inspection (see bugs #21 and #22 below). The current self-test
(`src-tauri/src/sck_selftest.rs`, gated behind `CLIPTOALL_SELFTEST_CAPTURE=1`)
compares against `/usr/sbin/screencapture` as an independent oracle,
emits hard pass/fail on each of 17 named checks (dimensions, scale,
alpha, channel order, colour per band, shear per band, plus observer
install and observer fire), and exits non-zero on any failure. See
`mx-verify.md` for how a human re-runs it and `mx-sck-report.md` for the
current numbers.

### Architecture

- **`src-tauri/src/sck_capture.rs`** (new) — owns the whole SCK path: cache
  for `SCShareableContent`, filter/content-filter/config building, the
  `CGImage` → RGBA8 draw via a `CGBitmapContext` with
  `PremultipliedLast | Order32Big` so memory is laid out as R, G, B, A.
  This is what the overlay's `ImageData` contract needs and what
  `crop_and_save_from_buffer`'s `(ri, gi, bi) = (0, 1, 2)` indices assume.
  **The big-endian choice is load-bearing** — see bug #22 for why the
  little-endian variant silently produces an "ABGR" buffer that still
  *looks* like a screenshot.
- **`SCShareableContent` cache** (`sck_capture::CACHED_CONTENT`) is
  populated once at startup by `prewarm_capture_backend` (called from
  `main.rs`'s `.setup()`) and reused on every capture. The window-server
  IPC + TCC check is measured ~50–73ms; without the cache every capture
  would pay that, eating most of the migration's speedup.
- **Display geometry** (`sck_capture::DisplayGeometry`) is the single
  source of truth for which display to capture and its dimensions in both
  coordinate spaces. It is resolved **per capture** from
  `CGDisplayCopyDisplayMode` — never from `CGDisplayPixelsWide`, which
  on a HiDPI display returns points despite its name (bug #21). Adding
  any new dimension must add a field here, not call a CG display API
  inline elsewhere — the `_pts` / `_px` suffixes are load-bearing.
- **Capture flow:** `take_owned_cached_content` borrows the cached ptr
  briefly under the lock, drops the lock, then `Retained::retain`s it for
  the duration of THIS capture (drops at end of function). Holding the
  cache lock across the `captureImage` call would prevent the SCK
  completion handler from firing on subsequent runs (lock + dispatch
  queue interaction).
- **`overlay_web::primary_monitor_logical_bounds`** delegates to
  `sck_capture::capture_display_logical_bounds` (round-2 unification),
  so the overlay is sized to the **same** display the capture path
  picks. Round 1 read `SCShareableContent.displays().firstObject()` here
  while the capture path resolved its own display separately — on a
  multi-monitor setup those can be different screens.
- **Stale-cache invalidation** (`sck_notifications::install_once`,
  called from `main.rs`'s `.setup()`) installs an observer for
  `NSApplicationDidChangeScreenParametersNotification`. When a display
  is connected or disconnected, the resolution changes, or the lid
  closes, the cached `SCShareableContent` is dropped eagerly so the
  next capture re-resolves before doing anything visible — round 1
  only invalidated after a capture had already failed (and only then
  via a stale-completion retry). Verified in
  `sck_selftest::observer-fires` by programmatically posting the
  notification and asserting the next capture's `content_ms` jumps
  from ~0 (warm cache) to >30 (re-resolved).
- **`cargo` deps:** `objc2-screen-capture-kit` 0.3 (features
  `SCScreenshotManager`, `SCShareableContent`), `objc2` 0.6,
  `objc2-foundation` 0.3 (`NSArray`, `NSError`), `objc2-core-graphics`
  0.3 (`CGImage`, `CGColorSpace`, `CGContext`, `CGDataProvider`,
  `CGDirectDisplay`), `objc2-core-foundation` 0.3 (`CFCGTypes`),
  `block2` 0.6. All declared explicitly under
  `[target.'cfg(target_os = "macos")'.dependencies]` per bug #8 — being in
  `Cargo.lock` transitively is not the same as being usable.

### Why the floor was raised to macOS 14

**The problem.** `xcap` captures via `CGWindowListCreateImage`, deprecated
since macOS 14. The xcap crate, the prior baseline, took roughly 270 ms
per capture at full resolution on this machine (the ~270 ms number
inherited from round-1 reporting — see §6 of `mx-sck-report.md` for
the round-2 honesty check). ScreenCaptureKit is GPU-backed and the
current path measures in the 60–100 ms range at full 2560×1600 on the
same machine.

**The minimum macOS has been raised 13 → 14** (`EXECUTION-PLAN.md` §2, decided
2026-08-16) specifically to make this migration cheap. The reasoning, so you
can re-litigate it if something changes:

- `SCScreenshotManager.captureImage` — the *simple* one-shot SCK API — is
  **macOS 14+**. On 13 you would need the far more involved `SCStream`
  single-frame dance, i.e. writing and testing two capture backends.
- Apple ships security updates for the current release and two prior. With
  macOS 26 current, that is 26 / 15 / 14 — **13 is already out of support.**
- Hardware cost of the bump is small: 14 supports MacBook Air/Pro 2018+,
  13 adds only some 2017 models.

With the floor at 14 this is roughly **half a day**; keeping 13 alive roughly
doubles it. For the owner's personal build the question is moot — that
machine runs macOS 26.

**Implementation sketch:** `objc2-screen-capture-kit`, get
`SCShareableContent`, build an `SCContentFilter` for the display, call
`SCScreenshotManager.captureImage` (async → bridge the completion handler to
a channel), pull the result. Keep the pixel-order contract in §5 intact.
Round-2 measured result: `capture_to_memory` lands at 79–100 ms on this
machine at the full 2560×1600 backing-store resolution.

---

## 5. macOS-specific architecture

Files added by this port:

| File | Role |
|---|---|
| `src-tauri/src/geometry.rs` | `SelectionRect` + `OverlayResult`, platform-agnostic. Extracted so `capture.rs` and both overlays share one type; `overlay.rs` re-exports them, so Windows sees no change. |
| `src-tauri/src/overlay_web.rs` | macOS overlay host: owns the pre-warmed window, hands the screenshot to JS, blocks until JS reports a result. Mirrors `show_native_overlay`'s blocking contract exactly, so `start_capture` looks the same on both platforms. |
| `src-tauri/src/results_spare.rs` | One-deep pool of pre-warmed hidden Results windows (bug #12). |
| `src/windows/OverlayWeb.svelte` | The overlay itself. Must obey `OVERLAY-SPEC.md`. |
| `src/lib/AlertModal.svelte` + `src/lib/stores/alert.svelte.ts` | DOM replacement for `alert()` (bug #9). |
| `src-tauri/capabilities/overlay.json` | Capability set for the `overlay` window label. |
| `src-tauri/.cargo/config.toml` + `scripts/macos-dev-runner.sh` | Stable dev signing (bug #11). |

Two structural points worth knowing:

**`handle_overlay_result` in `main.rs` is shared.** Both platforms' `start_capture`
capture → show overlay → then call it. All the post-selection work
(crop, save, clipboard, Results window, plugin dispatch) lives there once, so
it cannot drift between platforms.

**Pixel order is NOT normalized.** `CaptureData.buffer` holds whatever the
platform's backend natively produces — BGRA on Windows (GDI), RGBA on macOS
(xcap). `crop_and_save_from_buffer` is the one place that knows the
difference (a `(ri, gi, bi)` cfg triple). This is deliberate: normalizing
cost a full-screen channel-swap pass on every capture, and on macOS a second
one to undo it for the browser canvas — see bug #14. **Preserve this when
swapping in ScreenCaptureKit**: whatever SCK gives you, record its order and
adjust those indices rather than converting.

---

## 6. Bug catalogue

Every one of these compiled cleanly and passed review. Read this section
before debugging anything — the same traps recur.

### Toolchain / configuration

**#1 — Tauri codegen needs a PNG icon on non-Windows.**
`generate_context!` panicked with `failed to open icon icons/icon.png`.
Tauri resolves the default window icon from `bundle.icon` by extension:
`.ico` first on Windows, `.png` everywhere else — and the list had only an
`.ico`. Fixed by adding a `.png` to `bundle.icon`. Windows unaffected (it
still matches the `.ico` first).

**#2 — `keyring` 4.x breaks the dependency graph.**
Its convenience features pull a `zbus` (Linux D-Bus) version that conflicts
with the one `tauri-plugin-opener` locks. Cargo cannot express "only the
Apple backend" for 4.x (feature names with slashes are rejected in a
dependency's `features` list). **Use `keyring` 3.x with
`default-features = false, features = ["apple-native"]`.**

**#3 — Transparent windows need an opt-in on macOS.**
`WebviewWindowBuilder::transparent()` does not exist on macOS unless the
`tauri` crate has feature `macos-private-api` **and** `tauri.conf.json` has
`app.macOSPrivateApi: true`. Both are set now.

**#8 — `cfg(not(windows))` vs `cfg(target_os = "macos")`.**
Code was gated `not(windows)` while its dependency (`keyring`) was declared
only under `[target.'cfg(target_os = "macos")'.dependencies]`. Compiles on
macOS; would fail confusingly on any other non-Windows target. **Match the
code gate to the dependency gate.** Same applies to anything using
`launchctl` (macOS-only, not general Unix).

**#10 — `option_env!` does not force a rebuild when the variable changes.**
`build.rs` emits no `cargo:rerun-if-env-changed`, so changing
`GDRIVE_CLIENT_ID` may leave a stale binary. **Verify the value actually
landed:** `strings target/debug/cliptoall-tauri2 | grep -c "<value>"`. Touch
`upload_gdrive.rs` to force a rebuild if not.

**#11 — Keychain re-prompting for the password on every rebuild.**
Xcode's toolchain ad-hoc-signs each build with a signature derived from the
binary's bytes, so it changes on every rebuild — and macOS keys Keychain
access-control on the signature. Every rebuild therefore looked like a brand
new app. Fixed by re-signing with a *stable* identity via a cargo `runner`
(`src-tauri/.cargo/config.toml` → `scripts/macos-dev-runner.sh`). This also
removed multi-minute stalls where the app sat waiting on an unnoticed
password dialog.

### Correctness

**#4 / #5 / #6 — Autostart.**
`tauri-plugin-autostart` was the plan, but its API needs an `AppHandle`, and
`set_autorun` is called deep inside the settings-save path where none is in
scope; threading one through would touch unrelated call sites. Hand-rolled a
LaunchAgent plist + `launchctl load/unload` instead. Two follow-on fixes:
XML-escape the executable path (a path containing `&`/`<`/`>` produced a
plist `launchd` silently refuses to parse), and skip the whole write +
`launchctl` spawn when the plist already matches — `set_autorun` runs on
*every* settings save, not only when the flag changes.

**#7 — Retina scale silently resolved to 1.0.** ⚠️ *The most instructive one.*
`xcap` reports monitor `x/y/width/height` in **logical points**, but
`capture_image()` returns **physical pixels**, and everything else in this
codebase (`CaptureData`, `SelectionRect`) is physical. `Monitor::from_point`
was being fed a physical point, matched no monitor, and fell back to
`1.0` — no error, no warning, just silently wrong DPI handling on every
Retina capture. Found only by printing the resolved scale in a throwaway
test. **Whenever a coordinate crosses an API boundary on macOS, state which
space it is in.** (It is now simplified further: since only the primary
monitor is captured, the function just returns that monitor's scale.)

**#9 — `alert()` is a silent no-op.**
`wry`'s WKWebView UI delegate implements the file-upload and media-capture
panels but **not** `runJavaScriptAlertPanel` / `Confirm` / `TextInput`. So
`alert()` does nothing at all on macOS, while WebView2 on Windows shows it
natively. Every error path in the frontend (9 call sites) was invisible —
which is what "the Google Drive button does nothing" actually was: an error
*was* thrown and caught, and the message vanished. Replaced with a DOM modal
(`AlertModal.svelte`). **Never use `alert`/`confirm`/`prompt` in this app.**

**#15 — Results window clipping the bottom button.** ⚠️ *Instructive.*
Three attempts at a hardcoded minimum height (600×200 → 620×210) were all
still slightly too small, because the true content height depends on font
metrics, checkbox size, and whether the status line wraps. Also,
`Results.svelte` called `setMinSize` *after* window creation and thereby
overrode the Rust-side `min_inner_size`. Final fix stops guessing: the
window measures `scrollHeight - clientHeight` on the clipping container —
which *is*, by definition, the number of pixels being cut off — and grows by
exactly that, re-checking via a `ResizeObserver`. **Prefer a measurement
that is self-correcting over a constant you derived by hand.**

**#16 — Previous selection rectangle flashing on the next capture.**
The overlay window is reused, so its canvas still held the previous frame,
and Rust showed the window before JS repainted. Fixed by inverting the
order: JS draws, then calls `overlay_ready`, and *that* shows the window
(with a 400ms safety net in case JS never answers).

**#17 — `requestAnimationFrame` never fires in a hidden window.** ⚠️ *The nastiest.*
macOS suspends rendering for invisible windows, so rAF callbacks never run.
Both the overlay and the Results window waited for a frame *before* calling
`show()` — a deadlock: the frame cannot happen until the window is shown,
and the show is waiting on the frame. Symptom was total: **the Results
window never appeared at all**, and the overlay only appeared via its
fallback timer. It did not surface until windows started being *reused*,
because a freshly created hidden window still gets its initial frames while
a long-idle one does not. Fixed with `waitForPaint()` in `App.svelte`, which
races rAF against a short timer, and by not wrapping the overlay's ready
signal in rAF at all. **Layout (`scrollHeight` etc.) IS computed while
hidden; painting is not.** Measure freely, but never *await a frame* from a
hidden window.

### Performance (hotkey → overlay visible went from ~4–8s to well under 1s)

**#12 — Cold WKWebView start ≈ 2.3s.**
Creating a `WebviewWindow` per capture dominated everything else. Both the
overlay and the Results window are now **pre-warmed hidden at startup and
reused** (`overlay_web::prewarm`, `results_spare`). The Results pool warms a
replacement immediately after handing one out. Reuse changes the contract:
a reused window's Svelte component is already mounted, so it will not
re-fetch on its own — Rust emits an event (`overlay-show`, `results-show`)
and the component reloads on that. Pre-warmed Results windows carry
`?spare=1` so `App.svelte` knows to wait for that event instead of closing
itself when it finds no pending image.

**#13 — PNG + base64 to move a screenshot ≈ 1.3s.**
Encoding the capture to PNG, base64-ing it, shipping it through JSON IPC and
decoding it via `Image.decode()` cost roughly 1.3s per capture. Replaced
with raw RGBA bytes through `tauri::ipc::Response` (JS receives an
`ArrayBuffer`, wrapped straight into `ImageData`) — ~35ms. **For big
same-machine payloads, use Tauri's binary IPC, never base64-in-JSON.**

**#14 — A color conversion that undid itself.**
`capture_to_memory` converted RGBA→BGRA, then `show_web_overlay` converted
BGRA→RGBA back — two full passes over 16MB that cancelled out (~180ms each).
Both removed; see §5 on pixel order.

### ScreenCaptureKit retain semantics & cross-process hangs

(Numbers #18–#20 — the previous round-1 publication reused #15–#17
which collide with the Correctness entries above.)

**#18 — `SCShareableContent` and `CGImage` are passed to the block as `+0`,
not `+1`.** Apple's docs for `getShareableContentWithCompletionHandler:`
and `captureImageWithFilter:configuration:completionHandler:` are explicit:
the object handed to the completion block is *not* retained by the
framework — "you must retain this object if you want to use it beyond the
lifetime of the block." Earlier code did `Retained::from_raw(content)`
(claims a `+1` that wasn't there). The first `content.displays()` call after
the next capture would then SIGTRAP inside `object_getClass` — a PAC failure
on the freed object (`Library/Logs/DiagnosticReports/cliptoall-tauri2-
...ips` confirms `EXC_BREAKPOINT` at `object_getClass` →
`msg_send_check`). **Always use `Retained::retain(ptr)` here, never
`Retained::from_raw(ptr)`** — the former calls `[obj retain]` to add the
`+1` we need; the latter claims one we never got.

**#19 — `SCShareableContent` is `!Send + !Sync` in objc2-screen-capture-kit
0.3.** Its `extern_class!` declaration only implements `NSObjectProtocol`,
no `unsafe impl Send/Sync`. Wrapped in a `ShareableContent` newtype that
adds them manually (`sck_capture.rs`), citing Apple's SCK docs as
thread-safe for the read operations we use (`.displays()`, `.windows()`,
properties). Borrowing + the parking_lot `Mutex` make sharing sound; we
never mutate the content.

**#20 — Spawning the self-test thread then calling `std::process::exit(0)`
hung the SECOND run of the binary at the next `SCScreenshotManager.
captureImage` call.** First run captured fine (proved the retain fix
worked — real pixels, PNG dimensions correct). Second run hung at
SCStreamConfiguration with the completion handler never firing. Diagnosed
by eliminating variables one at a time: the retain contract was correct,
the lock wasn't held across the call (refactored to borrow ptr, drop lock,
re-retain), the channel wasn't closed (a fresh process owns a fresh channel
each run). The hang cleared only when the self-test was moved to the main
thread and exited via a normal return from `main` (no `std::process::exit`
from a spawned thread). Working theory: `exit(2)` bypasses the runloop
shutdown that SCK depends on to release its in-process state, leaving a
per-binary "session" resource that the next process's `captureImage`
waits on forever. **For SCK-touching code paths, exit the process by
returning from `main` so all Drop impls (including SCShareableContent) run
and the framework can finish its teardown.** The self-test in `main.rs`
now does exactly that, plus an explicit `f.sync_all()` on the log file
before exit so the verification lines don't get lost.

### Round-2 defects (2560×1600 path, validation gone silent)

(Both shipped in round 1 because the round-1 verification consisted of
saving a PNG and looking at it — *see the lesson below*. The oracle-checked
self-test introduced in round 2 was designed specifically to catch this
class of bug without human judgement.)

**#21 — `CGDisplayPixelsWide`/`High` return POINTS on HiDPI displays, not
backing-store pixels, despite their name.** Symptom: capture at 1280×800
instead of 2560×1600 on this 2× Retina machine, plus a `scale` of 1.0 from
`CGDisplayPixelsWide / SCDisplay.width()` (1280 / 1280 = 1). The captured
PNG looked like a screenshot; nothing flagged it. Root cause: those two
APIs report the *mode's point size* (`CGDisplayModeGetWidth`), not the
backing-store pixel count. The only CG accessor that gives the real pixel
size is `CGDisplayMode::pixel_width` / `...pixel_height`. This is bug #7
"scale reads 1.0 on Retina" re-entering through a different API:
the original code called `CGDisplayPixelsWide` too. Fix:
`sck_capture::DisplayGeometry` reads both `pixel_width`/`pixel_height` AND
`width`/`height` from the SAME display mode and exposes them with
`_px`/`_pts` suffixes; nothing else in the codebase is allowed to call a CG
display-size API. Self-test catches via `dimensions` (ours vs reference vs
mode.pixelWidth/Height), `scale` (DisplayGeometry vs oracle),
`capture-origin-space`. **Lesson:** the API name you wanted is not
necessarily the API name that exists. "Pixels" in the CG/HI-DPI world is
a coordinate space, not a unit.

**#22 — `kCGImageAlphaPremultipliedLast | kCGImageByteOrder32Little` lays
the 32-bit word down LSB-first — i.e. memory is A, B, G, R, not R, G, B,
A.** Symptom: the buffer's `(ri, gi, bi) = (0, 1, 2)` indices read what
was the alpha channel first; the "red channel" was actually the alpha
sitting at byte 0 (always 255 for an opaque screen capture), so the
whole image came out drenched in bright red. Visual inspection read it as
"the red wallpaper at the top of the screen" — a coincidence that
happened to fit a believable diagnosis. Root cause: little-endian
byte-order reverses the layout. The correct primitive is
`kCGImageByteOrder32Big` (= `4 << 12`), which lays the word MSB-first as
R, G, B, A. Fix: `bitmap_info: u32 = CGImageAlphaInfo::PremultipliedLast.0
| CGImageByteOrderInfo::Order32Big.0;` in `draw_cgimage_into_rgba`.
Also: replaced the deprecated free `CGContextDrawImage` with the
`CGContext::draw_image` associated function. Self-test catches via
`alpha-opaque` (whole-buffer scan: byte 3 must be 255 on every pixel —
a single miss trips it), `channel-order` (six permutations scored,
identity must win; ABGR loses by ~25 %), `not-abgr` (dedicated detector:
every byte 0 is constant 255), and the per-band `colour-*` checks. **Lesson:**
a screen capture's alpha is always 255, so byte-order errors that put
alpha in the red slot produce a uniformly-coloured red image that looks
like a screenshot of a red wall. Visual inspection will not catch it.
**A verification you can talk yourself out of is not a verification.**

**#23 — A reused window shows its last COMPOSITED frame, not its last drawn
one.** ⚠️ *Instructive — the companion to #17.*
Symptom: the previous capture's green selection rectangle intermittently
appeared on the next capture, under an overlay that had already been
redrawn. The earlier fix (draw the new frame, then have JS call
`overlay_ready` to reveal the window) was correct but insufficient, and the
intermittency made it look fixed. Root cause: `draw()` updates the canvas
backing store synchronously, but the layer only reaches the screen on the
next **compositor** frame — and macOS suspends rendering for hidden windows.
The overlay window is reused, not rebuilt, so `show()` reveals the last
*composited* texture (the previous capture, rectangle included) and swaps to
the new content only a frame or two later. Whether the user saw it depended
on whether the compositor happened to wake in between. Diagnosis mattered
more than the fix here: an unmissable log line proved the 400 ms safety
timer fired **0 times in 19 captures**, which killed the obvious hypothesis
(slow JS, fallback revealing an unpainted window) in seconds. Fix: you
cannot wait for a paint before showing — a hidden window never paints, which
is #17 — so instead leave nothing worth showing. `wipeFrame()` clears the
canvas **while the window is still visible**, awaits a real frame so the
blank is composited, and only then reports the result to Rust (which is what
hides the window). Applied to every exit path: selection finished,
cancelled, plugin hotkey. It also drops the three cached full-screen
canvases (~48 MB per capture at 2560×1600) so no code path can redraw the
old shot, and `loadAndReset` wipes again on the way in. **Lesson: on macOS,
nothing you draw into a window that is about to be hidden is guaranteed to
be on screen. If a window is reused, wipe it while it can still paint —
"I drew the new frame first" is not the same as "the old frame is gone".**

**#24 — `outputMode: "resize"` is a sensible Windows default and a bad macOS
one.**
Symptom: text in uploaded screenshots looked soft. Not a regression — the
setting was doing exactly what it says. `resize` downscales the capture by
the monitor scale before upload; at Windows' typical 125–150% that discards
little, but at Retina's flat 2× it throws away three quarters of the pixels.
The codebase already had the answer in `exif` mode: keep every pixel and tag
the JPEG's density so browsers still render it at logical size (2021 WHATWG
density correction), while zoom and HiDPI viewers get the full detail. Fix:
the default is now platform-gated — `exif` on macOS, `resize` untouched on
Windows; all three modes stay selectable on both. **Lesson: a default
inherited from the other platform can be a bug without being wrong. Also,
changing a default does not touch existing settings files — an already-
configured machine must be migrated or edited by hand.**

**#25 — The GDrive placeholder pool hands out a link before the bytes
exist.**
Symptom: a freshly-shared Drive link sometimes rendered a tiny blank image.
Cause is a deliberate trade-off, not a fault: a pool of pre-allocated
~630-byte placeholder files lets the app return a real public URL instantly
while a background PATCH writes the real bytes. `Show` was already gated on
the upload being "done" — but the upload completes the moment the
placeholder is claimed, which is only halfway. Fix: the backend emits
`gdrive-content-ready` when the PATCH lands; the session carries a
`contentPending` flag; and only the actions that **fetch** the URL wait on it
(`Show` plus the reverse-image searches). Copying the link is deliberately
NOT gated — instant sharing is the whole reason the pool exists. A 30 s
client-side timeout releases the buttons if neither the ready event nor the
fallback arrives, so a dropped event degrades to the old behaviour rather
than a permanently dead button. **Lesson: "our upload returned" and "the
resource is fetchable" are different events. Gate on the one that matches
what the button actually does — and note that reverse-image search fetches
once and caches, so firing it early poisons the result permanently rather
than merely showing a blank.**

---

## 7. Known gaps

| Gap | Where | Notes |
|---|---|---|
| **Multi-monitor capture** | `capture.rs::capture_to_memory` | Only the primary monitor is captured; Windows grabs the whole virtual screen. A selection dragged to a second display will misbehave. Flagged in-code. Now unblocked from the SCK migration — decide approach in a follow-up. |
| **Plugins directory inside the bundle** | `plugins.rs::plugins_dir` | Resolves to `<exe_dir>/plugins`, which inside a signed `.app` is read-only and signature-sealed. Pre-existing (same on Windows), not introduced by the port. Fix alongside the `~/Library/Application Support` work in Phase 5b. |
| **Plugins never run end-to-end on macOS** | Phase 4 | Discovery uses the executable bit instead of `.exe`; `encryption-plugin` was verified standalone, but no plugin has been driven through the app itself. Also still to do: Unix process-group kill (`setsid`/`killpg`) replacing Windows Job Objects. |
| **Signing / notarization / bundle** | Phase 5 | Certificates exist; nothing wired into the build. Screen-Recording TCC was re-verified after the SCK migration's stable dev signing (the self-test runs from the signed binary and succeeds — see §4). Notarization and bundle assembly are still open. |
| **CI for macOS** | Phase 6 | `.github/workflows/build.yml` is Windows-only and triggers only on `main`. |

---

## 8. Verification and testing on this machine

- **`cargo check` / `cargo test` are cheap — run them constantly.**
  27 Rust tests exist. `npm run check` (svelte-check) must stay at 0 errors.
- **Windows cannot be verified locally.** Cross-compiling fails in
  `aws-lc-sys`, which needs real Windows headers. The only Windows signal is
  the GitHub Actions job — and it triggers on push/PR to `main`, so the
  `macos-port` branch needs a PR to get it. **This has not happened yet.**
- **The agent cannot drive the GUI.** `osascript`/System Events needs an
  Accessibility grant that is not in place, so synthetic keystrokes and
  clicks are unavailable. The `screencapture` CLI runs fine from a shell
  on this machine (round 2 verified this by using it as the comparison
  oracle — `screencapture -x -D 1 …` returns rc=0 and a 2560×1600 PNG) AND
  runs as a child of the signed app. Practical consequence: **the self-test
  path can be verified without a human at the keyboard**, but interactive
  capture-cycle testing still is the human's job. Give precise step-by-step
  instructions and read the log afterwards.
- **A throwaway `#[ignore]`d test is the best way to exercise real code
  paths** (`cargo test <name> -- --ignored --nocapture`). That is how the
  Retina-scale bug (#7) was caught. Delete it once it has done its job — do
  not leave a test that needs a live display in the suite.
- **Restart recipe** after any change:
  ```bash
  pkill -f "target/debug/cliptoall-tauri2"; pkill -f "node.*vite"
  cd <repo root> && set -a && source .env.local && set +a
  nohup npm run tauri:dev > /tmp/cliptoall_dev.log 2>&1 & disown
  ```
