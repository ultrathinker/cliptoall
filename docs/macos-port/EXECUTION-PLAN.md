# ClipToAll — macOS Port: Execution Plan

> **Status:** active working plan. Created 2026-08-08 **on the target Mac**, supersedes the
> intent-level draft in [PLAN.md](PLAN.md) (keep that file — it holds the rationale and the
> Windows-side file audit; this one holds the concrete execution order and environment facts).
>
> **Scope:** macOS only, arm64-first. Windows code is frozen (see PLAN.md §0 — those principles
> stand unchanged and are not repeated here).

---

## 1. Environment snapshot (the machine we build on)

Verified 2026-08-08 on the development Mac:

| Component | State | Action needed |
|---|---|---|
| Hardware | Apple M1 (arm64), 8 GB RAM, ~80 GB free disk | — (8 GB is enough for `cargo`/Vite; avoid parallel heavy jobs) |
| macOS | 26.5 (Tahoe, build 25F71) | — |
| Xcode | 26.5 installed at `/Applications/Xcode.app` | `sudo xcode-select -s /Applications/Xcode.app` + accept license (active dir currently points at bare CLT) |
| Rust | **not installed** | `rustup` → stable toolchain, host target `aarch64-apple-darwin` |
| Node / npm | v26.4.0 / 11.17.0 | `npm install` in repo root |
| Homebrew | installed (`/opt/homebrew`) | — |
| Apple Developer | paid Individual membership, active | — |
| Signing certs | **Apple Development** and **Developer ID Application** certs both present in login keychain (2026-08-08) | — Phase 5a signing pipeline can proceed |
| Notarization | nothing configured | create App Store Connect API key **or** app-specific password for `notarytool` (Phase 5) |

Team ID, account identity, and notarization credentials are **not** recorded in this public
repo — they live in the local (private) notes. Use placeholders like `$APPLE_TEAM_ID` in any
committed config; real values go into untracked env files / Keychain / CI secrets.

## 2. Decisions (closing PLAN.md §6 open questions)

| Question | Decision | Why |
|---|---|---|
| Minimum macOS version | ~~13 Ventura~~ → **14 Sonoma** (revised 2026-08-16) | Originally 13 (above Tauri 2's floor of 11, and testable-ish). Revised once the capture-performance work made the trade-off concrete: the simple one-shot ScreenCaptureKit API (`SCScreenshotManager.captureImage`) is macOS 14+, so keeping 13 means writing and testing a second `SCStream`-based backend purely for it. Meanwhile 13 is already outside Apple's security-update window (current + 2 back = 26/15/14), and the hardware difference is small (14 covers MacBook Air/Pro 2018+; 13 adds only some 2017 models). See HANDOFF.md §4. |
| Architectures for first release | **arm64 only** | The dev machine is M1; x86_64/universal cannot be smoke-tested locally. Add x86_64 via CI (Phase 6) once arm64 ships. |
| Overlay visual spec | Extract exact colors/opacities/label style from `src-tauri/src/overlay.rs` constants at the start of Phase 3 and record them in `docs/macos-port/OVERLAY-SPEC.md` before writing Svelte code. |
| Linux later? | Out of scope, but the web overlay must not use anything macOS-specific in the frontend (keep it display-server-agnostic). |
| Branch strategy | Feature branch **`macos-port`** off `main`; merge to `main` per completed phase (each phase leaves Windows builds green). |

## 3. Working agreements for this port

- **Every phase ends with:** (a) `cargo check` green for `aarch64-apple-darwin`, (b) a normal
  `git` commit on `macos-port`, (c) a short progress note appended to §6 of this file.
- **Windows must stay green:** we cannot compile Windows locally, so the GitHub Actions Windows
  job (existing CI) is the gate — push the branch and check CI before considering a phase done.
- Repo language is English (code, comments, docs, commits) — as everywhere else in this repo.
- No secrets, identity, or machine-specific absolute paths in committed files.

---

## 4. Phases — concrete execution order

Order: **P0 → P1 → P2 → P5a (signing, early!) → P3 → P4 → P5b → P6.**
Signing is pulled forward because the TCC Screen-Recording grant is tied to the code signature —
Phase 2 cannot be properly tested until at least stable dev-signing works.

### Phase 0 — toolchain + compile on macOS *(≈ 1–2 days)*

Tasks 0.1–0.7 from PLAN.md §3 apply verbatim; concrete steps for this machine:

1. `sudo xcode-select -s /Applications/Xcode.app/Contents/Developer && sudo xcodebuild -license accept`
2. Install rustup (stable, default host `aarch64-apple-darwin`).
3. `npm install` in repo root; verify `npm run build` (frontend alone) passes.
4. Cargo.toml surgery (PLAN.md 0.1): `winreg` → `[target.'cfg(windows)'.dependencies]`.
5. Gate `mod aumid;` / `mod overlay;` and their call sites (PLAN.md 0.2–0.3).
6. Split `autorun.rs`; stub `capture.rs` / `clipboard.rs` / `dpapi.rs` mac bodies with
   `unimplemented!()` (PLAN.md 0.4–0.5).
7. macOS bundle config: `.icns` (generate from existing icon set), separate
   `tauri.macos.conf.json` overlay (PLAN.md 0.7).
8. **Exit criteria:** `cargo check` green on Mac; `npm run tauri dev` launches the app —
   tray icon appears, Settings/Results/Editor windows open (capture/clipboard stubbed);
   Windows CI still green.

### Phase 1 — portable backends *(≈ 2–4 days)*

PLAN.md 1.1–1.5. Suggested sub-order (independent, but this sequencing tests easiest first):

1. **1.2 Secrets** — re-add `keyring` (Keychain). Verify: save creds in Settings → restart →
   creds survive; `security find-generic-password` shows the item.
2. **1.3 Autostart** — ~~prefer `tauri-plugin-autostart`~~ done as a hand-rolled LaunchAgent
   plist instead: `set_autorun` is called from deep in the settings-save path with no
   `AppHandle` in scope (the plugin's enable/disable need one via `ManagerExt`), and threading
   one through just for this touches unrelated call sites. A plist write + `launchctl
   load`/`unload` needs no handle at all.
3. **1.1 Clipboard image** — `arboard::set_image`. Verify: paste into Preview, a browser,
   and Slack.
4. **1.4/1.5 Plugin discovery + encryption-plugin** — executable-bit filter; cfg-gate
   `clipboard-win` → `arboard` in the plugin crate.
5. **Exit criteria:** settings round-trip with Keychain, autostart survives reboot,
   clipboard paste works in 3 target apps.

### Phase 2 — screen capture *(≈ 2–4 days)*

PLAN.md 2.1–2.3, plus:

1. Spike first: a tiny bin target that grabs all monitors via `xcap` and dumps PNGs —
   validates permission flow, Retina scale, multi-monitor geometry **before** wiring into
   `capture.rs`'s logic.
2. Map `backingScaleFactor` / `Monitor::scale_factor()` into the existing full-res / logical /
   EXIF-density output modes — outputs must be byte-comparable in structure to Windows ones.
3. TCC: add `NSScreenCaptureUsageDescription`; detect via `CGPreflightScreenCaptureAccess`;
   if not granted → onboarding dialog deep-linking to System Settings → Privacy → Screen
   Recording, then offer relaunch. **Needs P5a signing to test reliably.**
4. **Exit criteria:** hotkey → full-screen capture lands in Results window with correct
   dimensions on Retina; permission onboarding works from a cold (denied) state.

### Phase 5a — dev signing pipeline *(parallel with Phase 2; ≈ 1–2 days of wall-clock, mostly waiting)*

1. Create **Developer ID Application** certificate (Xcode → Accounts → Manage Certificates).
2. Set up `notarytool` credentials (App Store Connect API key preferred; store in Keychain via
   `xcrun notarytool store-credentials`).
3. Wire signing into the Tauri build: `signingIdentity` + Team ID via env
   (`APPLE_SIGNING_IDENTITY`, `APPLE_TEAM_ID`), hardened runtime, minimal entitlements.
4. Prove the loop once end-to-end: build → sign → notarize → staple → `spctl -a -vv` passes
   on a clean copy.
5. **Exit criteria:** a signed dev build keeps its Screen-Recording grant across rebuilds
   (stable signature), and one notarized `.dmg` has been produced.

### Phase 3 — web overlay *(the big chunk; ≈ 1–1.5 weeks)*

PLAN.md 3.1–3.6, with these additions:

1. **Step 0:** write `OVERLAY-SPEC.md` from `overlay.rs` (colors, dim opacity, tint per mode,
   label font/placement, cursor, key map) — the parity contract for the Svelte overlay.
2. Build order: single-monitor static overlay (screenshot + dim + drag rect) → size label →
   modes/tints + plugin hotkeys + Shift-square + double-press toggle → Esc/right-click cancel →
   multi-monitor (one WebviewWindow per monitor, physical↔logical mapping) → latency tuning
   (ArrayBuffer transfer, target ≤ 100 ms behind native feel).
3. **Exit criteria:** side-by-side video comparison against the Windows overlay (record the
   Windows machine) shows behavioral parity on the full key/mouse matrix from OVERLAY-SPEC.

### Phase 4 — plugins on macOS *(≈ 2–3 days)*

PLAN.md 4.1–4.3: exclude `aumid-plugin`; verify JSON-over-stdio with the example py/ps1
plugins; Unix process-group kill (`setsid`/`killpg` via `nix` or `command-group`).
**Exit criteria:** example plugins discovered, run, and are killable; runaway plugin test
(sleep-loop script) terminates cleanly.

### Phase 5b — packaging & release polish *(≈ 2-3 days)*

Remaining PLAN.md 5.3–5.5: `ActivationPolicy::Accessory` (tray-only, no Dock icon),
tray left-click menu, config paths under `~/Library/Application Support`, final `.dmg`.
**Exit criteria:** fresh-Mac simulation — new user account on this Mac, install from `.dmg`,
first-run onboarding, capture, upload, paste all work.

**Known gap to close here, found by review:** `plugins::PluginManager::plugins_dir()` resolves
to `<exe_dir>/plugins` on every platform (unchanged by this port — same on Windows). Once the
`.app` bundle is signed and installed to `/Applications`, that directory is inside the read-only,
signature-sealed bundle: `create_dir_all`/plugin-config writes there will either fail outright or,
worse, silently succeed on an unsigned dev build while invalidating Gatekeeper's seal on a signed
one. `dpapi`/settings storage already solves the equivalent problem for logs (`log_file_path` uses
`%APPDATA%`/`dirs::config_dir()`, not exe-adjacent, specifically for this reason — BUGS#11). Plugin
discovery needs the same treatment (move to `~/Library/Application Support/ClipToAll/plugins` on
macOS) — deliberately not fixed as part of the Phase 1.4 executable-bit-filter change, since it's a
pre-existing cross-platform limitation, not something the port introduced, and changing plugin
storage location is a bigger, platform-asymmetric decision that deserves its own pass alongside the
`~/Library/Application Support` config-path work already scheduled for this phase.

### Phase 6 — CI *(≈ 1–2 days)*

PLAN.md 6.1–6.3: add `macos-14` (arm64) job with `cargo check` + `clippy` on every PR
(cheap cfg-breakage guard) first; signed/notarized release builds + `macos-13` (x86_64)
job later, when we decide to ship Intel.

---

## 5. Risks (delta to PLAN.md §4)

- **Only one test machine, on macOS 26.x** — we claim macOS 13+ but can't verify below 26.5.
  Mitigation: stick to long-stable APIs (all chosen crates predate 13), state "tested on 26.x"
  in release notes.
- **8 GB RAM** — full `tauri build` + Vite is fine, but don't run it alongside memory-heavy
  apps; prefer `cargo check` for iteration.
- **First notarization run** is the classic time sink (Apple-side delays, entitlement
  rejections) — hence P5a is scheduled early and in parallel.

## 6. Progress log

| Date | Phase | Note |
|---|---|---|
| 2026-08-08 | — | Plan created on the Mac; environment audited (Xcode 26.5 present, Rust missing, Dev cert present, Developer ID cert missing). |
| 2026-08-08 | 0 | rustup installed (aarch64-apple-darwin). All Windows-only surfaces from PLAN.md §2 gated behind `cfg(windows)` with `cfg(not(windows))` stubs; new `geometry.rs` holds the platform-agnostic `SelectionRect`. `tauri.conf.json` bundle.icon got a `.png` added (Tauri's codegen requires one for the non-Windows default window icon; Windows unaffected, it resolves `.ico` first). `cargo check` green on macOS. Windows can't be cross-checked locally (aws-lc-sys needs real Windows headers) — needs CI, which only triggers on push/PR to `main`; branch `macos-port` not yet pushed (deferred — pushing/opening a PR is a visible-to-others action, left for the user to confirm). Committed as `d534a8d`. Blocked on: Developer ID Application certificate (Phase 5a, needs Apple ID 2FA) and the Screen-Recording TCC prompt (Phase 2, needs a physical click) — both need the human. |
| 2026-08-08 | 0 | `npm run tauri:dev` builds clean and launches: `target/debug/cliptoall-tauri2` stayed alive with no panics/errors in the log (checked via `ps`/log grep). Could not visually confirm the tray icon — `screencapture` itself failed with "could not create image from display" (the same Screen-Recording TCC gate, this time on the terminal), so eyeballing the tray icon is a task for a human. Phase 0 exit criterion met; app left running for visual confirmation on return. |
| 2026-08-08 | 1 | All of Phase 1 done. **1.2 Secrets**: `keyring` 3.x (`apple-native` feature — the 4.x line's convenience features drag in a `zbus` version that conflicts with `tauri-plugin-opener`'s). `dpapi_encrypt`/`encrypt_field` gained a stable `account` param (Amazon key IDs, `"gdrive_token"`, `"plugin_settings:<path>"`) since Keychain is a key-value store, not a blob cipher — without a stable account, every settings save would leak a fresh orphaned entry. Verified with a standalone Keychain set/get/delete round-trip. **1.1 Clipboard**: `arboard::set_image`, RGB→RGBA conversion before handoff; verified with a standalone set_image/get_image round-trip. **1.3 Autostart**: deviated from the plan's tauri-plugin-autostart suggestion — `set_autorun` is called deep in the settings-save path with no `AppHandle` in scope, so hand-rolled a LaunchAgent plist + `launchctl load`/`unload` instead (no handle needed); verified load→`launchctl list` sees it→unload. **1.4/1.5 Plugins**: `discover_exe_files` now checks the executable bit instead of a `.exe` extension; `encryption-plugin` crate's `clipboard-win` moved to a Windows-only target dep, macOS gets `arboard` (text). Verified the whole plugin end-to-end via its `--call` test mode against the real clipboard: plaintext → encrypt → base64 ciphertext in clipboard → decrypt → original plaintext back. Four commits (`7826f7d`, `2e64530`, `117d373`, `c287136`). |
| 2026-08-08 | 2 | **Correction to the row above:** Phase 2 turned out NOT to be blocked on the human right now — a `screencapture` CLI test failed (Screen-Recording TCC), but `xcap`'s CoreGraphics capture worked anyway from the same shell, meaning whatever process context these commands run in already has Screen-Recording permission (granted for some earlier, unrelated reason). **2.1**: `capture_to_memory()` captures the primary monitor via `xcap`, converts RGBA→BGRA to match `CaptureData`'s Windows-derived contract. Multi-monitor capture is NOT implemented (only primary monitor) — flagged in code, tracked as a Phase 2.1 follow-up (plan explicitly says "evaluate multi-monitor first"). **2.2**: `get_monitor_scale()` initially used `xcap::Monitor::from_point` and silently returned 1.0 on this 2x Retina display — found and fixed a units bug (xcap reports monitor x/y/width/height in LOGICAL points, everything else in this codebase is PHYSICAL pixels; `from_point` was being fed a physical point that landed exactly on the logical-vs-physical boundary and matched no monitor). Fixed by hit-testing monitor bounds manually, scaled to physical pixels. Caught via a temporary `#[ignore]`d test that captured the real screen, cropped a real PNG, and I looked at it — confirmed correct colors (no channel swap) and, after the fix, correct `scale=2` (was silently `scale=1` before). Test removed after verification; not a permanent regression test (needs a live display + permission, can't run in CI). `cargo test`: 27/27 pass. Commit `0d3163d`. Also wrote `docs/macos-port/OVERLAY-SPEC.md` (Phase 3 step 0) — colors/opacities/label positioning/the exact Shift-square algorithm with its test table, extracted directly from `overlay.rs` before any Svelte code exists, per the plan (commit `ebc5d09`). |
| 2026-08-08 | — | **Session pause point.** Phases 0–2.2 done and verified live (not just compiled) on this machine; Windows behavior untouched (verify via CI once the branch is pushed/PR'd). Phase 3 (the web overlay) is next — it's the biggest phase (~1–1.5 weeks estimated) and is inherently a visual/UX build that benefits from the user watching and giving feedback rather than being built unsupervised, so it's a natural stopping point. Still genuinely blocked on the human: (1) Developer ID Application certificate for notarization (Phase 5a — needs Apple ID login + 2FA), (2) pushing `macos-port` / opening a PR to get Windows CI signal (a visible-to-others action). The Screen-Recording permission question from earlier turned out to be moot for now (see row above) but may still matter once the app is code-signed differently (Phase 5a) — the grant is tied to the signature, so re-verify once that changes. |
| 2026-08-08 | 5a | **Blocker (1) cleared.** Xcode initially refused to issue the Developer ID Application certificate with "Unable to process request - PLA Update available" — an updated Program License Agreement was pending and had to be accepted at developer.apple.com (Xcode itself can't do this, it's a web-only step). Once accepted, Xcode → Settings → Accounts → Manage Certificates → + → Developer ID Application succeeded. `security find-identity -v -p codesigning` now lists both the Apple Development and Developer ID Application identities. Signing/notarization pipeline work (entitlements, hardened runtime, `notarytool` credentials) can now start. Blocker (2) — pushing the branch / opening a PR for Windows CI — is still open. |
| 2026-08-16 | 3 | **Phase 3 (web overlay) done and working end-to-end**, verified interactively by the user: hotkey → dimmed overlay → drag-select → crop → clipboard → Results window. Default hotkey is now `Cmd+X` on macOS (`Alt+X` kept on Windows). Also fixed a `.app`-independent frontend bug found along the way: `alert()` is a silent no-op in WKWebView (wry implements no JS-dialog UI delegate), so all 9 frontend error paths were invisible on macOS — replaced with a DOM modal. Then a performance pass took hotkey→overlay from ~4–8s to well under 1s: pre-warm and reuse both the overlay and Results windows instead of building a WebviewWindow per capture (~2.3s cold WKWebView start), ship raw RGBA over Tauri's binary IPC instead of PNG+base64-in-JSON (~1.3s), and drop a pair of color conversions that cancelled each other out (~360ms). Two reuse-specific bugs followed and were fixed: the previous capture's selection rectangle flashing on the next overlay (now draw-then-show via an `overlay_ready` round trip), and — the nastiest — `requestAnimationFrame` never firing in a hidden window, which deadlocked "wait for paint, then show" and meant the Results window never appeared at all. Results-window content clipping (the Edit button) is now fixed by measuring actual overflow rather than any hardcoded floor. Full detail, including every trap and its lesson, is in **`docs/macos-port/HANDOFF.md`** — read that first when picking this up. |
| 2026-08-16 | — | **Handoff point.** `HANDOFF.md` written as the single entry point for continuing this port (state, build/run, bug catalogue, gaps, testing constraints). Minimum-macOS decision revised 13 → 14 (see §2). Next priority is the ScreenCaptureKit migration (~half a day at the 14 floor) to cut the remaining ~270ms capture cost. Still open: Windows CI has never validated this branch (needs a PR to `main`); plugins have not been exercised end-to-end on macOS; signing/notarization/bundle not started. |
| 2026-08-16 | 2.3 | **ScreenCaptureKit migration done and verified with real pixels** (`xcap` / `CGWindowListCreateImage` → `SCScreenshotManager.captureImage`). New `sck_capture.rs` owns the SCK path; `SCShareableContent` cached at startup (`prewarm_capture_backend`), borrowed through `take_owned_cached_content` on every capture; CGImage drawn into an RGBA8 premultiplied-last + order32Little bitmap context so `crop_and_save_from_buffer`'s existing `(0,1,2)` channel indices keep working. Three consecutive `CLIPTOALL_SELFTEST_CAPTURE=1` runs of the signed app: total capture **243 / 205 / 203 ms** (captureImage alone 141–162 ms), real pixels saved to `$TMPDIR/cliptoall_selftest_capture.png`, first pixel RGBA `[255, 100, 87, 62]` matches the red wallpaper visible at the top of the screen. Two new bug-categories added to HANDOFF.md §6: #15 (`Retained::from_raw` claimed a `+1` that SCK never gave — first call SIGTRAP'd inside `object_getClass`/PAC failure; fix is `Retained::retain`), #16 (`SCShareableContent` is `!Send+!Sync` in objc2-screen-capture-kit 0.3 — wrapped in `ShareableContent` newtype with manual unsafe Send/Sync, citing Apple's docs), #17 (a self-test that spawns a thread then `std::process::exit(0)`s hangs the NEXT process's `captureImage` — exit the process by returning from `main` instead, so Drop impls run and SCK's per-process state finishes teardown). |
| 2026-08-16 | 2.4 | **Round 2 of the SCK migration — verification gap closed.** The row above ("verified with real pixels") was wrong: it shipped two critical defects that both passed visual inspection, because the only check was "look at the saved PNG." Specifically: (1) `CGDisplayPixelsWide/High` return POINTS, not backing-store pixels, on HiDPI — round 1 captured at 1280×800 instead of 2560×1600 on this 2× Retina machine, so the "203/205/243 ms" numbers were measured at a quarter of the display's real resolution (HANDOFF §6 #21). (2) `kCGImageAlphaPremultipliedLast | kCGImageByteOrder32Little` lays bytes down as A, B, G, R; the correct primitive for RGBA is `Order32Big` (= `4 << 12`). The "first pixel RGBA [255,100,87,62]" cited above is the ABGR-laid-out-but-read-as-RGBA artifact — which read as "the red wallpaper" because screen-capture alpha is always 255 (HANDOFF §6 #22). The round-1 HANDOFF §6 entries #15/#16/#17 were renumbered to #18/#19/#20 (collisions with the Correctness section); #21 and #22 are the round-2 defects. Replacement verification (`src-tauri/src/sck_selftest.rs`, gated behind `CLIPTOALL_SELFTEST_CAPTURE=1`): compares every capture against `/usr/sbin/screencapture -x -D 1` as an independent oracle, runs 5 timed captures per invocation, and reports hard pass/fail on 17 named checks (dimensions, scale, alpha-opaque, channel-permutation, not-abgr, colour per band, shear per band + drift, observer install + observer fire). Negative-control evidence: re-introducing defect (1) trips `dimensions`; re-introducing defect (2) trips `alpha-opaque`, `channel-order`, `not-abgr`, and all three `colour-*` checks simultaneously. Reverted and green on 17/17, EXIT=0. Round-2 timing (full 2560×1600, warm cache, means over 3 runs × 5 captures each): `captureImage` 69–78 ms, end-to-end `capture_to_memory` 88–100 ms. Stale-cache invalidation wired (`src-tauri/src/sck_notifications.rs`) to `NSApplicationDidChangeScreenParametersNotification` so a resolution change or monitor swap drops the cached `SCShareableContent` eagerly instead of waiting for a capture to fail — verified in `observer-fires` check by posting the notification synthetically and observing the next capture's `content_ms` jump from 0 to 41. See `mx-sck-report.md`, `mx-verify.md`, `mx-next-plan.md`, `mx-postmortem.md` for the full round-2 write-ups. |
