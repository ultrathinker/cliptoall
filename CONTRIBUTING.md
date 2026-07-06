# Contributing to ClipToAll

Thanks for your interest in improving ClipToAll! This document covers how to set up the project and the conventions we follow.

## Development setup

### Prerequisites

- [Rust](https://rustup.rs/) (stable toolchain)
- [Node.js](https://nodejs.org/) 18+ and npm
- **Windows:** WebView2 runtime (preinstalled on Windows 11) and the MSVC build tools ("Desktop development with C++")

See the [Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/) for full details.

### Build & run

```bash
npm install
npm run tauri dev      # run with hot reload
npm run tauri build    # produce a release binary + installer
npm run check          # svelte-check (frontend type/a11y checks)
```

Rust code lives in `src-tauri/`; the Svelte 5 frontend in `src/`.

## Project layout

```
src/                     Svelte 5 + TypeScript frontend (windows, stores, lib)
src-tauri/               Rust backend
  src/commands/          Tauri commands (capture, upload, clipboard, settings, plugins, ...)
  src/                   overlay, plugin process management, utils (dpapi, autorun)
  capabilities/          per-window capability scoping
plugins/                 plugin protocol + example plugins (see PLUGIN-PROTOCOL.md)
```

## Coding conventions

- **All code and comments in English.**
- **Never commit secrets** (API keys, tokens, passwords) or personal data (real hostnames, absolute local paths, emails). CI and review will reject them.
- Frontend: Svelte 5 runes (`$state`, `$derived`, `$props`, `$bindable`, `$effect`), TypeScript. Keep `npm run check` clean (0 errors).
- Rust: keep `cargo clippy` clean; run `cargo fmt`.
- Match the style of the surrounding code; see `.editorconfig` for whitespace rules.

## Pull requests

1. Fork and branch from `main`.
2. Make focused commits with clear messages.
3. Ensure `npm run tauri build` succeeds and `npm run check` / `cargo clippy` are clean.
4. **Test on a real Windows machine** — this is a desktop capture tool; automated tests cannot exercise screen capture, the overlay, clipboard, or global hotkeys. Describe what you exercised.
5. Open a PR using the template; link any related issue.

## Cross-platform work

macOS and Linux support is on the roadmap and concentrated in the platform-specific Rust modules (capture, overlay, clipboard, secret storage). If you'd like to help, open an issue first to coordinate the platform abstraction approach.

## License

By contributing, you agree that your contributions are licensed under the [Apache License 2.0](LICENSE).
