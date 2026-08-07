# ClipToAll — macOS Support Plan (DRAFT)

> **Status:** draft / intention. Created 2026-08-07 on the Windows machine, grounded in **v5.1.25**.
> To be expanded and executed on the Mac.
> **Scope:** macOS only (not Linux). Region-select overlay on macOS = **web overlay** (Svelte canvas).

---

## 0. Guiding principles (agreed)

1. **Windows is frozen.** The shipped Windows version — look, controls, native Win32 region
   overlay, DPAPI, taskbar/caption icons, everything — must behave and look **exactly as today**.
   We do **not** unify or "modernize" Windows code paths for the sake of the port.
2. **Add, don't rewrite.** Every Windows-specific module stays compiled unchanged behind
   `#[cfg(windows)]`. macOS gets **parallel** implementations behind `#[cfg(target_os = "macos")]`.
   The Windows binary never compiles the macOS code and vice-versa, so shared logic that both use
   (uploads, settings, the Svelte UI windows) is the only thing they have in common — and none of
   it needs to change for macOS.
3. **No risky migrations.** Windows stays on DPAPI; macOS uses Keychain from scratch. We do **not**
   port existing Windows secrets to a new store (avoids the DPAPI→keyring migration risk).
4. **Overlay = web on macOS.** Native Win32 overlay stays on Windows untouched. macOS gets a
   transparent full-screen WebviewWindow with a Svelte canvas that **replicates the Windows
   overlay's look and behavior** (dim/tint, selection rect, size label, Shift-square, plugin keys,
   double-press mode toggle, Esc/right-click cancel).

The app's Settings/Results/Editor windows are **already** web (Svelte in WebView) and identical on
every OS — there are no native Windows "controls" to replace. The only native piece anywhere in the
app is the region overlay.

---

## 1. Prerequisites (have / to confirm on Mac)

- [x] A Mac to build & test on (Tauri can **not** cross-compile — native WebView + frameworks).
- [x] Apple Developer license (for signing + notarization).
- [ ] Xcode + command-line tools installed.
- [ ] Rust `aarch64-apple-darwin` (and `x86_64-apple-darwin` if universal) targets.
- [ ] Decide minimum macOS version (Tauri 2 floor is macOS 11 Big Sur; consider 12/13).
- [ ] Signing identity / Team ID noted for the CI + local build env.

---

## 2. Architecture — cfg-based platform split

Keep the current flat module layout; gate per-OS. No heavy HAL/trait framework needed — the code
already passes plain data (raw BGRA `Vec<u8>` + geometry), not OS handles.

Current Windows-specific surface (verified in v5.1.25):

| File | Lines | Win32 today | macOS plan |
|---|---|---|---|
| `src-tauri/src/overlay.rs` | 589 | GDI wndproc region overlay | **not ported** — macOS uses the web overlay |
| `src-tauri/src/commands/capture.rs` | 600 | GDI `BitBlt`/`GetDIBits` | `xcap` (CoreGraphics) under `cfg(macos)` |
| `src-tauri/src/commands/clipboard.rs` | 136 | `CF_DIB` clipboard | `arboard` (NSPasteboard) under `cfg(macos)` |
| `src-tauri/src/utils/dpapi.rs` | 112 | `CryptProtectData` | `keyring` (Keychain) under `cfg(macos)` |
| `src-tauri/src/utils/autorun.rs` | 23 | `winreg` HKCU\Run | LaunchAgent plist / `tauri-plugin-autostart` |
| `src-tauri/src/aumid.rs` | 42 | AUMID + run-as-admin | Windows-only concept → no-op on macOS |
| `src-tauri/src/main.rs` → `mod winicon` | (in main.rs) | already `#[cfg(windows)]` ✓ | n/a (macOS uses `.icns` bundle icon) |
| `src-tauri/src/plugins.rs` | 1017 | Job Objects / `CREATE_NO_WINDOW` (already `cfg(windows)`) | Unix process group (`setsid`/`killpg`) |

Good news already in the tree: the `windows` crate is **already** under
`[target.'cfg(windows)'.dependencies]` (`Cargo.toml:37`), and `winicon` + `apply_window_icons` are
already `#[cfg(windows)]`.

The crate **does not compile off Windows today** because `winreg` is in the unconditional
`[dependencies]` (`Cargo.toml:29`) and `autorun.rs` imports it unconditionally, plus `mod aumid` /
`mod overlay` are declared unconditionally. Phase 0 fixes exactly this.

---

## 3. Phases

### Phase 0 — Compile on macOS (Windows behavior unchanged)

Goal: `cargo check` / `tauri build` succeed on macOS. App launches; capture/overlay/clipboard are
stubbed. **Windows logic identical to today.**

| # | Task | Location |
|---|---|---|
| 0.1 | Move `winreg = "0.52"` from `[dependencies]` into `[target.'cfg(windows)'.dependencies]` | `Cargo.toml:29`, `:37` |
| 0.2 | Gate `mod aumid;` and `mod overlay;` with `#[cfg(windows)]`; add macOS module(s) for the web overlay | `main.rs:4`, `:6` |
| 0.3 | Gate the Win32 call sites in `main.rs`: `overlay::show_native_overlay` (`:209`), `overlay::invalidate_overlay` (`:480`), `aumid::show_admin_dialog`/`restart_as_admin` (`:264`,`:265`), `apply_window_icons` (`:327`,`:633`) — macOS takes the web-overlay / no-op branch | `main.rs` |
| 0.4 | Split `autorun.rs`: keep `winreg` impl under `cfg(windows)`, add macOS impl (Phase 1.3) | `utils/autorun.rs` |
| 0.5 | Gate the Win32 bodies of `capture.rs` / `clipboard.rs` / `dpapi.rs` under `cfg(windows)`; add `cfg(macos)` stubs (`unimplemented!()` for now, filled in Phases 1–2) | those 3 files |
| 0.6 | Confirm `plugins.rs` Job Objects / `CREATE_NO_WINDOW` (already `cfg(windows)`) don't break the non-Windows compile | `plugins.rs` |
| 0.7 | Add macOS bundle config: `.icns` icon set, `.dmg`/`.app` targets (consider a `tauri.macos.conf.json` overlay so Windows `tauri.conf.json` stays as-is) | `tauri.conf.json`, `icons/` |

### Phase 1 — Portable macOS backends

| # | Task | macOS approach | Windows |
|---|---|---|---|
| 1.1 | Clipboard image | `arboard` `set_image` (NSPasteboard PNG/TIFF) | keeps `CF_DIB` (unchanged) |
| 1.2 | Secret storage | re-add `keyring` crate → Keychain; same `encrypt_field`/`decrypt_field` interface | keeps DPAPI (unchanged, **no migration**) |
| 1.3 | Autostart | LaunchAgent plist in `~/Library/LaunchAgents/` (or `tauri-plugin-autostart`) | keeps `winreg` (unchanged) |
| 1.4 | Plugin discovery | executable bit instead of `.exe` | keeps `.exe` filter |
| 1.5 | `encryption-plugin` | swap `clipboard-win` → `arboard` (cfg-gated) | keeps `clipboard-win` |

`keyring` and `oauth2` were removed as dead deps in 5.1.15–16 — `keyring` must be **re-added** for 1.2.

### Phase 2 — Screen capture (macOS)

| # | Task | Notes |
|---|---|---|
| 2.1 | Implement `capture_full_screen` / `capture_to_memory` equivalents via `xcap` (wraps CoreGraphics) under `cfg(macos)` | evaluate multi-monitor, speed, HiDPI first |
| 2.2 | DPI/scale: `backingScaleFactor` (Retina 2×) or Tauri `Monitor::scale_factor()`, mapped to `capture.rs`'s existing scale logic | keep output modes (full-res / logical / EXIF density) identical |
| 2.3 | Note deprecation: `CGDisplayCreateImage` deprecated in macOS 14 → `xcap` shields us now; plan a **ScreenCaptureKit** migration later | tracked risk |

### Phase 3 — Web overlay (macOS) — the big chunk (~1–1.5 wk)

| # | Task |
|---|---|
| 3.1 | New `OverlayWeb.svelte`: frozen screenshot on canvas + dim/tint (pink=link, green=image, matching Windows), live selection rect, size label |
| 3.2 | Transparent, borderless, always-on-top fullscreen WebviewWindow **per monitor**; pass the screenshot as an ArrayBuffer (not base64) for speed |
| 3.3 | Drag-select → region coords → crop command (reuse existing crop path) |
| 3.4 | **Behavior parity** with the native overlay: Shift-square constraint, plugin hotkeys, double-press → mode toggle, Esc / right-click cancel, physical↔logical coordinate mapping |
| 3.5 | Route in `main.rs`: `cfg(windows)` → `overlay::show_native_overlay` (unchanged); `cfg(macos)` → spawn the web overlay |
| 3.6 | UX check: capture→overlay latency (~50–100 ms slower than native is acceptable), drag smoothness |

### Phase 4 — Plugins on macOS

| # | Task |
|---|---|
| 4.1 | `aumid-plugin` — Windows-only; exclude on macOS |
| 4.2 | Verify JSON-over-stdio protocol on macOS |
| 4.3 | Process kill: Windows keeps Job Objects; macOS uses Unix process group (`setsid`/`killpg`, e.g. `nix`/`command-group`) under `cfg(macos)` |

### Phase 5 — Packaging, permissions, signing (the macOS "pipeline" work)

| # | Task |
|---|---|
| 5.1 | **TCC Screen-Recording permission**: `NSScreenCaptureUsageDescription` in Info.plist; detect via `CGPreflightScreenCaptureAccess`; first-run onboarding (deep-link to System Settings → Privacy, offer relaunch). Grant is **tied to the code signature** and usually needs a relaunch — so signing must work even in dev. |
| 5.2 | **Signing + notarization** (a pipeline, not a checkbox): Developer ID, hardened runtime, entitlements, `notarytool`, `staple`. Unnotarized → Gatekeeper "damaged app". |
| 5.3 | `.icns` icon set; `ActivationPolicy::Accessory` (tray-only, no Dock icon); tray left-click opens menu (macOS convention) |
| 5.4 | Universal binary (arm64 + x86_64) or per-arch; `.dmg` / `.app` bundle |
| 5.5 | Config paths via `dirs` (`~/Library/Application Support/…`) — verify `settings.json` / `gdrive_token.json` land correctly |

### Phase 6 — CI & test

| # | Task |
|---|---|
| 6.1 | GitHub Actions: add `macos-14` (arm64) + `macos-13` (x86_64) to the matrix; signing/notarization secrets |
| 6.2 | `cargo check` + `clippy` for the macOS target on every PR (catches cfg breakage cheaply) |
| 6.3 | Manual smoke tests on the Mac: capture, permission flow, overlay parity, clipboard paste into browser/Slack/Office, autostart, plugins |

---

## 4. macOS gotchas to keep in mind

- TCC ↔ signature coupling: capture won't work in dev until signing is set up. Front-load Phase 5.1/5.2.
- `CGDisplayCreateImage` deprecation (macOS 14) → ScreenCaptureKit migration is a later, separate task.
- Web overlay is a **new** UI surface — budget for making it match the Windows look pixel-wise, not
  just "draw a rectangle".
- No cross-compilation — everything Mac-side builds/tests on the Mac (or macOS CI runners).

## 5. Rough estimate

macOS-only, Windows frozen, web overlay: **~4–7 weeks** focused work. Difficulty **medium** (the
medium-high driver from the old 3-OS plans was Wayland — out of scope here). The wildcard is the
first-time signing/notarization pipeline.

Suggested order: Phase 0 → 1 → 2 → 3 → 5 (start the Apple account/signing early, in parallel) → 4 → 6.

---

## 6. Open questions (expand on the Mac)

- Minimum macOS version target?
- Universal binary for the first release, or arm64-only?
- Exact overlay visual spec to match Windows (colors, dim opacity, label style, cursor)?
- Is Linux a later goal? If yes, keep the web overlay display-server-agnostic so it can be reused.
- Signing identity / Team ID / notarization credentials for CI.

## 7. References (source material)

- Private monorepo brainstorm: `…/ClipToAll.Tauri2/cross-platform-plans/` — `CROSS-PLATFORM-PLAN.md`
  (synthesis) + `PLAN-fable.md` / `PLAN-kilo.md` / `PLAN-codex.md`. Grounded in v5.1.14; covers all
  three OSes and the Wayland risk (Linux-only, ignore for this macOS-scoped plan).
- Bee memory bank: *"ПЛАН: Кроссплатформенное портирование (Windows → Linux + macOS)"* under
  `/Проекты Не-Работа/ClipToAll-GitHub/Архитектура` (has the v5.1.14→v5.1.25 correction notes).
