# Security Policy

## Reporting a vulnerability

Please **do not open a public issue** for security vulnerabilities.

Email **universeissilent42@gmail.com** with details and, if possible, a reproduction. You will get a response as soon as reasonably possible. Please allow time for a fix before any public disclosure.

## Supported versions

ClipToAll is developed as a rolling release; only the latest published version receives security fixes.

## Security model

**Secrets at rest.** Sensitive settings (Amazon S3 access/secret keys, the Google Drive OAuth token, and the clipboard-encryption password) are stored on Windows using **DPAPI** (`CryptProtectData`, CurrentUser scope) in `%APPDATA%\ClipToAll\settings.json`. Encrypted fields are tagged with a `dpapi:` prefix. Encryption never silently falls back to plaintext — if DPAPI fails, the save fails.

**Upload credentials.** S3 credentials are used only in the Rust backend; they are not passed across the IPC boundary to the WebView. Google Drive uses OAuth2 with PKCE; the token is stored separately and encrypted with DPAPI.

**WebView hardening.** The app ships a restrictive Content-Security-Policy and scopes Tauri capabilities per window. The `read_image_base64` command is limited to the app's own temporary screenshot directory, and plugin execution paths are constrained to the plugins directory.

**Clipboard text encryption.** The optional clipboard encrypt/decrypt feature uses AES-256-CBC. It is a lightweight convenience for sharing short encrypted snippets between machines that share the same password — not a high-assurance secrets vault. Do not rely on it to protect high-value secrets.

## Plugins

The plugin system runs external executables and scripts (native exe, Python, C#, PowerShell) that **you** place in the `plugins/` directory. Plugins run with your user privileges. Only install plugins you trust; treat plugin files like any other executable you download.

## Threat model notes

ClipToAll is a single-user desktop application. It does not run a network service or accept inbound connections. Its main trust boundaries are: the local settings/secret storage (DPAPI), the outbound upload endpoints (S3/Google Drive over TLS), and locally-installed plugins (user-trusted).
