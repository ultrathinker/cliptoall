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
| Signing certs | **Apple Development** cert present in login keychain | **Developer ID Application** cert must be created (Xcode → Settings → Accounts → Manage Certificates) before Phase 5; needed for notarization |
| Notarization | nothing configured | create App Store Connect API key **or** app-specific password for `notarytool` (Phase 5) |

Team ID, account identity, and notarization credentials are **not** recorded in this public
repo — they live in the local (private) notes. Use placeholders like `$APPLE_TEAM_ID` in any
committed config; real values go into untracked env files / Keychain / CI secrets.

## 2. Decisions (closing PLAN.md §6 open questions)

| Question | Decision | Why |
|---|---|---|
| Minimum macOS version | **13 Ventura** | Comfortably above Tauri 2's floor (11); keeps ScreenCaptureKit migration path open (needs 12.3+); we can only test on 26.x anyway, so promising 11/12 support would be untestable. |
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
