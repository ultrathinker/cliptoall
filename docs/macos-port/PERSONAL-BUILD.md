# Personal macOS build ("ClipToAll Personal")

The published macOS app (Developer ID release and the App Store edition) is a
pure screenshot tool: it has **no plugin system compiled in**. The personal
build is the same app built with the Cargo feature `personal`, which turns the
plugin system on, so the owner can run plugins such as Clipboard Encryption
(`plugins/encryption-plugin`) on their own Mac. It is never published: no
GitHub release, no CI artifact.

## What differs

| | Public build | Personal build |
|---|---|---|
| Cargo features | none (default) | `--features personal` |
| Plugin system | not compiled in | compiled in (`cfg(plugins)`) |
| App name | ClipToAll | ClipToAll Personal |
| Bundle identifier | `net.appshub.cliptoall` | `com.ultrathinker.cliptoall.personal` |
| Settings / data dir | `~/Library/Application Support/ClipToAll` | `~/Library/Application Support/ClipToAll Personal` |
| Keychain service | `ClipToAll` | `ClipToAll Personal` |
| Signing | Developer ID (or App Store, sandboxed) | Developer ID, hardened runtime, `Entitlements-direct.plist`, **not sandboxed** |

Windows is unaffected: it has always had the plugin system and keeps it in
every build.

How it is wired:

- `src-tauri/build.rs` sets the cfg alias `plugins` on Windows always and on
  macOS only when the `personal` feature is on. All plugin code and call sites
  are gated on `cfg(plugins)`.
- `src-tauri/src/utils/app_identity.rs` holds the one name the app chooses by
  itself (`APP_NAME`): the data directory, the Keychain service, the tray
  tooltip. Without the feature it is `ClipToAll`, exactly as before.
- `src-tauri/tauri.personal.conf.json` is a Tauri config overlay
  (`tauri build --config ...`) that sets the product name, the bundle
  identifier, the `app` bundle target only, and the non-sandboxed
  entitlements. Everything derived from the bundle identifier is therefore
  separate too: the Screen Recording (TCC) grant, the Open at Login item
  (SMAppService), the WebView data store and the single-instance lock.
- The frontend asks the binary what it has (`app_capabilities` command, see
  `src/lib/features.ts`), so Settings > Plugins appears only in a build that
  actually has the plugin commands.

## Building

Prerequisites, none of them in the repo: `.env.local` with the Google Drive
client values, a "Developer ID Application" identity in the login keychain,
and a `notarytool` keychain profile.

```bash
export APPLE_SIGNING_IDENTITY=<SHA-1 of the Developer ID Application identity>
export NOTARY_PROFILE=<notarytool keychain profile name>
./scripts/macos-personal.sh --install-app
```

The script builds the app with `--features personal` and the overlay, builds
the encryption plugin (arm64, release), signs both (hardened runtime, secure
timestamp), notarizes them in one submission, staples the app, writes
`dist-macos/personal/ClipToAll-Personal-<version>-macos-arm64.zip` (from a
clean staging folder; no Tauri DMG step), installs the plugin into the plugins
directory and, with `--install-app`, copies the app to `/Applications`. Quit a
running ClipToAll Personal first. It builds into its own Cargo target dir
(`src-tauri/target-personal`), so the personal `.app` never lands next to the
public one in `src-tauri/target/release/bundle/`, and it deletes its build
and staging copies of the `.app` at the end, so Spotlight only finds the
installed app.

For development, always pass the overlay together with the feature:

```bash
npm run tauri dev -- --features personal --config src-tauri/tauri.personal.conf.json
```

A `--features personal` binary refuses to start under any other bundle
identifier, because without the overlay it would share the public app's
single-instance lock, Screen Recording grant and WebView data.

## Plugins directory

```
~/Library/Application Support/ClipToAll Personal/plugins
```

It is outside the `.app` on purpose: adding a file inside a signed bundle
breaks its signature. Plugins run with the app's privileges and receive the
clipboard, so the app keeps this directory owner-only (`0700`), refuses to use
it if it is a symlink or cannot be made owner-only, and launches only regular
files with the executable bit that are not writable by group or others.
Symlinks are never launched and a plugin must sit directly in the directory
(not in a subfolder). These checks run right before every launch, not only
when plugins are listed.

## Adding a plugin

1. Build it as a standalone executable that speaks the protocol in
   `plugins/PLUGIN-PROTOCOL.md` (it is started with `--daemon`).
2. Sign it. Apple silicon refuses to run unsigned code; use the same Developer
   ID identity as the app:
   `codesign --force --sign "$APPLE_SIGNING_IDENTITY" --options runtime --timestamp <file>`
3. Copy it into the plugins directory with mode `0755`.
4. Open Settings > Plugins in ClipToAll Personal, enable it, set its settings
   (Clipboard Encryption needs `{"password": "..."}`; add `"scheme": "legacy"`
   only for interop with the old CBC format), and save.

Script plugins (`.py`, `.cs`) work too if the interpreter is on the app's
PATH (macOS runs `python3`); a GUI app's PATH holds only the system
directories.

If the app crashes, daemon plugins see end-of-file on stdin and exit
(protocol rule 6); there is no Job Object equivalent on macOS.

## Side by side with the public app

Both apps can be installed and running at the same time. They have separate
settings, logs, Google Drive tokens, Keychain items, login items, Screen
Recording permissions and single-instance locks; the personal build also
skips the public app's legacy LaunchAgent cleanup. Each needs its own Screen
Recording grant, and the two menu bar icons look the same (hover for the
tooltip: "ClipToAll" or "ClipToAll Personal"). Only one of them can own a
given capture hotkey, so give them different hotkeys.

## CI check

`scripts/check-plugin-gate.sh` builds the default and the `personal`
configuration and greps the binaries: the marker string
`CLIPTOALL_PLUGIN_SYSTEM_COMPILED_IN_V1` (a `#[used]` static in
`src-tauri/src/plugins.rs`) and a string from the plugin launcher itself must
be absent from the default binary and present in the personal one. CI
(`.github/workflows/build.yml`) runs it on macOS, together with clippy and
`cargo test` for both configurations. CI only compiles the personal build; it
never bundles, signs or uploads it.
