# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project uses
date-based patch versions.

## [5.1.25]

### Added

- Hold **Shift** while dragging the region selection to constrain it to a perfect
  square. It works in every drag direction, updates live when you press or release
  Shift mid-drag, and stays square when clamped near a screen edge.

## [5.1.24]

### Fixed

- Crisp window icons on Windows. Tauri 2 builds the runtime window icon from only
  the first entry of the `.ico` and stretches that one bitmap to every size
  (tauri#14596), which made the taskbar and title-bar icons blurry/pixelated. We
  now set the icons the standard Win32 way: for each window we pick the frame that
  matches the size Windows actually wants — `SM_CXSMICON` for the caption,
  `SM_CXICON` for the taskbar/Alt-Tab (both DPI-dependent) — out of the multi-size
  icon and assign them separately via `WM_SETICON`. Applied to the main and
  Results/Editor windows.
- Regenerated the app icons cleanly (high-quality resampling) and ordered
  `ClipToAll-all.ico` largest-frame-first.

## [5.1.23]

### Fixed

- No more white flash when a screenshot's Results window opens. The window is now
  created hidden and only shown once its themed UI has painted, so you never
  briefly see a blank white window before the content appears. A backend fallback
  reveals the window anyway if the frontend fails to signal ready.

## [5.1.22]

### Diagnostics

- Cap the log file: when `cliptoall.log` reaches 25 MB it rotates to
  `cliptoall.log.old` (one generation kept), so on-disk log use is bounded at
  ~50 MB even if "Write to Log File" is left on indefinitely — instead of growing
  without limit.

## [5.1.21]

### Fixed

- Startup no longer blocks on plugin initialization. Temp cleanup and plugin
  startup (whose per-plugin hello handshake can take up to 20s) now run on a
  background thread, so the tray icon and global hotkey are responsive the
  instant the app launches. Previously a slow or hung plugin during startup could
  leave the tray menu drawn but unresponsive — the classic "menu opens but Exit
  does nothing" freeze, most likely to hit right after login when the app
  auto-starts.
- Tray "Exit" is now guaranteed to quit: it no longer blocks trying to take the
  plugin lock (it skips the graceful stop if the lock is held). Plugin child
  processes are reaped anyway by their Job Object on process exit.
- Hard timeout on the Google Drive token refresh so a stuck refresh can't pin the
  shared single-flight refresh gate.

### Diagnostics

- Added startup/lifecycle log breadcrumbs (setup phases, plugin startup, Exit).
  These honor the existing "Write to Log File" setting — nothing is logged unless
  it is enabled.

## [5.1.20]

### Security

- Actually close the Results/Editor event-broadcast vector: the previous release
  dropped the explicit `core:event:allow-emit` but left `core:default`, which
  transitively re-granted emit. The capability now lists `core:default`'s members
  explicitly, minus `core:event:default`, so those overlays can only listen — a
  compromised one can no longer emit a spoofed `settings-changed`.

### Robustness

- Atomic writes (temp file + rename) for `settings.json`, `plugins.json`, and
  `gdrive_token.json`, so a crash/power loss mid-write can't corrupt them.
- Single-flight Google token refresh: concurrent uploads that all find the token
  expired no longer each hit the token endpoint and race to overwrite it.
- OAuth: a cancelled/denied authorization (`?error=`) now fails fast instead of
  hanging until the 2-minute timeout.
- Editing the image while an upload is in flight now correctly marks the link
  stale (previously only handled once the upload had finished).

### Changed

- Image prefix is restricted to `[A-Za-z0-9_]` (else it falls back to `cta_`).
- Screenshot path validation recognizes files by their timestamp signature
  rather than the current prefix, so changing `image_prefix` no longer breaks a
  path an already-open window is holding.

## [5.1.19]

### Packaging

- Publish a standalone portable `ClipToAll-<version>-portable.exe` alongside the
  installer, so the app can be run without installing (same binary, frontend
  embedded, autostart self-registered at runtime).
- Ship only the NSIS installer; drop the redundant MSI.

No functional code changes from 5.1.18.

## [5.1.18]

### Security

- Finish the per-window IPC gating started in 5.1.17: `gdrive_authorize`,
  `gdrive_disconnect`, `update_hotkey`, `discover_plugins`, and `check_runtime`
  now reject calls from non-main windows (Tauri does not scope app-defined
  commands per-window, so these account/hotkey/plugin operations were callable
  from any WebView).
- Resolve the Google Drive destination folder backend-side from settings (as S3
  and the pre-allocation daemon already do) instead of taking it as an IPC
  parameter, so a caller can't redirect an upload to an arbitrary folder.
- Sanitize the configured image prefix (drop filename-reserved / control
  characters, clamp length, enforce the minimum length used by cleanup) and
  normalize the S3/Drive bucket, region, and folder values on every save, so a
  malformed value can't steer the screenshot write outside `%TEMP%` or break
  saving.
- Tighten `ensure_temp_screenshot_path` to the same predicate as temp cleanup
  (prefix + generated timestamp stem + image extension); the previous check
  accepted a one-character prefix.
- Bound the `save_image_base64` payload before decoding (memory/disk DoS guard).
- Drop `core:event:allow-emit`/`core:event:default` from the Results/Editor
  window capability so a compromised overlay can't broadcast spoofed events;
  those windows only need to listen.

### Housekeeping

- OAuth loopback: on a `state` mismatch, reject that request and keep listening
  until the deadline instead of aborting the whole flow.
- Remove the unused `assetProtocol` scope from the Tauri config.
- Backfill this changelog for 5.1.15–5.1.18.

## [5.1.17]

### Security

- Gate mutating/executing IPC commands (`save_settings` and the plugin
  commands) to the main window and validate `save_results_window_size` under a
  single write-lock acquisition.
- Validate and clamp settings on every persist (window size, JPEG quality,
  theme/storage/output/mode whitelists).
- `save_image_to_file` now validates its source path like every other
  path-taking command.

### Changed

- Encryption plugin: drop the format marker entirely — ciphertext is now plain
  base64 with nothing identifying the tool or scheme. Decryption relies on
  AES-GCM authentication (strong scheme tried first, legacy fallback).
- Reject plugin key bindings the overlay can't map; align the default theme
  (crimson) between backend and frontend.

## [5.1.16]

### Changed

- Make the strong scheme (PBKDF2-HMAC-SHA256 + AES-256-GCM) the default in the
  encryption plugin; legacy AES-256-CBC is now explicit opt-in for interop.

### Security

- Gate `load_plugin_configs` so decrypted plugin secrets reach only the main
  Settings window.
- Widen output-filename randomness from 3 to 12 hex chars so upload URLs aren't
  guessable from the timestamp.
- Handle a revoked/expired Google refresh token gracefully; use `127.0.0.1`
  (not `localhost`) for the OAuth loopback; resolve PowerShell by absolute path.
- Use path-style S3 URLs for buckets whose name contains a dot.

### Removed

- Dead in-app encrypt/decrypt commands and unreachable Tauri commands; unused
  crypto, tracing, and build dependencies.

## [5.1.15]

### Security

- Do not ship S3 secrets to non-settings windows (blanked outside main).
- Validate that upload/clipboard commands only read files from the temp
  screenshot directory.
- Restrict temp-file cleanup to the app filename pattern (safe with short
  prefixes).
- Robust OAuth redirect parsing (exact `code` param + URL-decode).

### Changed

- Add an opt-in strong clipboard-encryption scheme alongside the legacy scheme
  (auto-detected on decrypt); replace poison-prone locks with `parking_lot`;
  stop the Drive pool daemon on disconnect; fix a GDI handle leak on the capture
  error path.

## [5.1.14] — Initial public release

First open-source release of the Tauri 2 rewrite of ClipToAll.

### Features

- Global-hotkey region capture (Alt+X, configurable) with a native selection overlay.
- Copy-link and copy-image modes (double-tap the hotkey for image mode).
- Built-in annotation editor: pencil, rectangles, arrows, text; undo/redo; Ctrl+wheel zoom; lossless PNG working copy.
- Upload to Amazon S3 or Google Drive (OAuth2) with the link copied to the clipboard.
- HiDPI-aware output modes (full resolution / resize to logical / full-res + EXIF density) and an adjustable JPEG quality applied only to the shared file.
- Five themes with instant live preview.
- Optional AES-256 clipboard text encryption.
- Plugin system for external tools and scripts (native exe / Python / C# / PowerShell).
- Tray-resident operation, autorun, single-instance.

### Security

- Secrets encrypted at rest via Windows DPAPI (no plaintext fallback).
- S3 credentials kept in the backend, not passed across the IPC boundary.
- Restrictive Content-Security-Policy and per-window capability scoping.
- `read_image_base64` restricted to the temp screenshot directory; plugin execution constrained to the plugins directory.

[5.1.25]: https://github.com/ultrathinker/cliptoall/releases/tag/v5.1.25
[5.1.24]: https://github.com/ultrathinker/cliptoall/releases/tag/v5.1.24
[5.1.23]: https://github.com/ultrathinker/cliptoall/releases/tag/v5.1.23
[5.1.22]: https://github.com/ultrathinker/cliptoall/releases/tag/v5.1.22
[5.1.21]: https://github.com/ultrathinker/cliptoall/releases/tag/v5.1.21
[5.1.20]: https://github.com/ultrathinker/cliptoall/releases/tag/v5.1.20
[5.1.19]: https://github.com/ultrathinker/cliptoall/releases/tag/v5.1.19
[5.1.18]: https://github.com/ultrathinker/cliptoall/releases/tag/v5.1.18
[5.1.17]: https://github.com/ultrathinker/cliptoall/releases/tag/v5.1.17
[5.1.16]: https://github.com/ultrathinker/cliptoall/releases/tag/v5.1.16
[5.1.15]: https://github.com/ultrathinker/cliptoall/releases/tag/v5.1.15
[5.1.14]: https://github.com/ultrathinker/cliptoall/releases/tag/v5.1.14
