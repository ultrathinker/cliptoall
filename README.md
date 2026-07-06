<div align="center">

# ClipToAll

**Screenshot to link in 2 seconds.**

Press a hotkey, select a region, and the link (or the image itself) is on your clipboard — annotated and uploaded, without leaving the keyboard.

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)
![Platform: Windows](https://img.shields.io/badge/platform-Windows-0078D6)
![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-24C8DB)
![Svelte 5](https://img.shields.io/badge/Svelte-5-FF3E00)
![Rust](https://img.shields.io/badge/Rust-backend-000000)

</div>

<!-- Replace with a real demo GIF/screenshot -->
<!-- ![Demo](docs/screenshots/demo.gif) -->

## What it is

ClipToAll is a lightweight desktop screenshot tool. Press **Alt+X**, drag a rectangle over any part of the screen, optionally annotate it, and it is uploaded to your own **Amazon S3** or **Google Drive** with the public link copied to your clipboard. It lives in the tray and stays out of your way.

The app is built on **Tauri 2** (Rust backend + WebView2) with a **Svelte 5** frontend — a ~8 MB executable with no heavy runtime to install.

## Platform support

| Platform | Status |
|----------|--------|
| Windows 10 / 11 | ✅ Supported |
| macOS | 🚧 Planned (see [Roadmap](#roadmap)) |
| Linux (X11 / Wayland) | 🚧 Planned (see [Roadmap](#roadmap)) |

The backend currently uses Win32 APIs for screen capture, clipboard, the region-select overlay, and secret storage, so today it runs on Windows only. macOS/Linux support is a planned effort — contributions welcome.

## Features

- **Global hotkey capture** — Alt+X (configurable); drag to select any screen region.
- **Two modes** — copy the **image** straight to the clipboard (default), or copy the uploaded **link**; double-tap the hotkey to toggle, or set your preferred default in Settings.
- **Built-in annotation editor** — pencil, rectangles, arrows, and text in 7 colors / 3 sizes, with undo/redo and Ctrl+wheel zoom.
- **Upload targets** — Amazon S3 or Google Drive (OAuth2), link auto-copied.
- **HiDPI-correct output** — choose full-resolution, resized-to-logical, or full-res + EXIF density so shared images look right in browsers.
- **Adjustable JPEG quality** — capture and editing stay lossless; compression is applied only to the shared/uploaded file.
- **Themes** — 5 built-in themes with instant live preview.
- **Clipboard encryption** — optional AES-256 encrypt/decrypt of clipboard text.
- **Plugin system** — extend capture with external tools or scripts (Python / C# / PowerShell / native exe). See [plugins/PLUGIN-PROTOCOL.md](plugins/PLUGIN-PROTOCOL.md).
- **Tray-resident** — autorun, single-instance, minimal footprint.

## Quick start (build from source)

### Prerequisites

- **[Rust](https://rustup.rs/)** (stable toolchain)
- **[Node.js](https://nodejs.org/)** 18+ and npm
- **Windows:** [WebView2 runtime](https://developer.microsoft.com/microsoft-edge/webview2/) (preinstalled on Windows 11) and the **MSVC build tools** (Visual Studio Build Tools with the "Desktop development with C++" workload)

See the [Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/) for details.

### Run in development

```bash
npm install
npm run tauri dev
```

### Build a release binary

```bash
npm run tauri build
```

The executable is produced under `src-tauri/target/release/`, and the installer under `src-tauri/target/release/bundle/`.

## Usage

1. Press **Alt+X** and drag to select a screen region.
2. Release — the region is captured and the Results window opens.
3. In the default **Copy Image** mode the screenshot is placed on your clipboard directly. In **Copy Link** mode it is uploaded and the public link is copied instead — double-tap the hotkey to toggle, or set the default in Settings.
4. Click **Edit** to annotate before sharing.

Open **Settings** (from the tray) to configure the storage backend, hotkey, theme, output/DPI mode, JPEG quality, and plugins.

## Configuration

- **Amazon S3** — enter Access Key ID, Secret Access Key, bucket, region, and folder in Settings → Storage. Public access is expected to be provided by your bucket policy (the app uploads without ACLs).
- **Google Drive** — click *Connect to Google Drive* in Settings → Storage and authorize via OAuth2. Uploaded images are shared with a public link. Official release binaries ship with an embedded Google OAuth client, so you only need to log into your own Google account — no setup required.

Settings are stored as JSON in `%APPDATA%\ClipToAll`, with sensitive fields encrypted at rest via Windows DPAPI. See [SECURITY.md](SECURITY.md) for the security model.

> **Building Google Drive support from source:** the Google OAuth client is *not* committed to this repository. Official release builds inject it at compile time. If you build from source and want Google Drive to work, create your own [Google OAuth 2.0 "Desktop app" client](https://developers.google.com/identity/protocols/oauth2/native-app) and build with the credentials in the environment:
>
> ```bash
> GDRIVE_CLIENT_ID=your-id.apps.googleusercontent.com GDRIVE_CLIENT_SECRET=your-secret npm run tauri build
> ```
>
> Amazon S3 works with no build-time configuration. If neither is built in, uploads to that backend are simply disabled.

## Roadmap

- macOS support (ScreenCaptureKit capture, Keychain secrets, webview-based overlay, signing/notarization).
- Linux support (X11 first, then Wayland via xdg-desktop-portal).
- Cross-platform overlay unification.

Cross-platform support is a substantial effort concentrated in the platform-specific Rust modules; see the issues tracker if you'd like to help.

## Contributing

Contributions are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md). Please keep code comments in English and never commit secrets.

## Security

For the security model and how to report a vulnerability, see [SECURITY.md](SECURITY.md).

## License

Licensed under the [Apache License 2.0](LICENSE).

## Author

Created by ultrathinker — <https://cliptoall.appshub.net>
