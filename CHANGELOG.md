# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project uses
date-based patch versions.

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

[5.1.14]: https://github.com/ultrathinker/ClipToAll/releases/tag/v5.1.14
