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
>
> Branch: **`macos-port`** (not merged to `main`).
> Last updated: 2026-08-16.

---

## 1. Where the port is

Working and verified by hand on a real Mac (M1, macOS 26.5, Retina 2×):

| Area | State |
|---|---|
| Compiles + runs on macOS (arm64) | ✅ |
| Screen capture (`xcap` / CoreGraphics) | ✅ primary monitor only |
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

## 4. Next priority: ScreenCaptureKit (and the macOS 14 floor)

**The problem.** `xcap` captures via `CGWindowListCreateImage`, deprecated
since macOS 14. It costs **~270ms** for one 2560×1600 screen — the single
largest remaining chunk of the delay between pressing the hotkey and seeing
the dimmed overlay. ScreenCaptureKit is GPU-backed and typically lands in
the 30–80ms range.

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
a channel), pull `CVPixelBuffer` bytes. Keep the pixel-order contract in
§5 intact. Expected result: `capture_to_memory` drops from ~270ms to well
under 100ms.

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

---

## 7. Known gaps

| Gap | Where | Notes |
|---|---|---|
| **Multi-monitor capture** | `capture.rs::capture_to_memory` | Only the primary monitor is captured; Windows grabs the whole virtual screen. A selection dragged to a second display will misbehave. Flagged in-code. Decide the approach when doing ScreenCaptureKit — the two interact. |
| **Plugins directory inside the bundle** | `plugins.rs::plugins_dir` | Resolves to `<exe_dir>/plugins`, which inside a signed `.app` is read-only and signature-sealed. Pre-existing (same on Windows), not introduced by the port. Fix alongside the `~/Library/Application Support` work in Phase 5b. |
| **Plugins never run end-to-end on macOS** | Phase 4 | Discovery uses the executable bit instead of `.exe`; `encryption-plugin` was verified standalone, but no plugin has been driven through the app itself. Also still to do: Unix process-group kill (`setsid`/`killpg`) replacing Windows Job Objects. |
| **Signing / notarization / bundle** | Phase 5 | Certificates exist; nothing wired into the build. TCC screen-recording grant is tied to the code signature, so re-verify capture after signing changes. |
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
  clicks are unavailable; the `screencapture` CLI is likewise blocked by TCC
  (though `xcap` from the app's own process works fine). Practical
  consequence: **interactive verification is the human's job.** Give precise
  step-by-step instructions and read the log afterwards.
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
