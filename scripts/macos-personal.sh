#!/usr/bin/env bash
#
# Build, sign, notarize and staple the PERSONAL macOS build ("ClipToAll
# Personal": plugin system on, Clipboard Encryption plugin alongside) for
# Apple silicon, and install the plugin into its plugins dir.
# See docs/macos-port/PERSONAL-BUILD.md. This build is for the owner's own
# Mac: never upload its output anywhere public.
#
# Signed Developer ID with the hardened runtime and Entitlements-direct.plist
# (NOT sandboxed: the App Sandbox forbids spawning plugin executables). The
# overlay config src-tauri/tauri.personal.conf.json gives it its own name and
# bundle identifier so it installs side by side with the public app.
#
# Prerequisites (none of these are stored in the repo):
#   * .env.local with GDRIVE_CLIENT_ID / GDRIVE_CLIENT_SECRET (baked in at build time)
#   * a "Developer ID Application" identity in the login keychain
#       export APPLE_SIGNING_IDENTITY=<SHA-1 from `security find-identity -v -p codesigning`>
#   * a notarytool keychain profile, created once by you with
#       xcrun notarytool store-credentials <profile-name>
#     and exported as NOTARY_PROFILE=<profile-name>
#
# Usage (repo root):
#   ./scripts/macos-personal.sh                 # build, sign, notarize, zip, install plugin
#   ./scripts/macos-personal.sh --install-app   # ... and also copy the app to /Applications
#
# Output: dist-macos/personal/ClipToAll-Personal-<version>-macos-arm64.zip
# (the stapled app plus plugins/cliptoall-encryption-plugin).
set -euo pipefail
cd "$(dirname "$0")/.."

INSTALL_APP=0
for arg in "$@"; do
  case "$arg" in
    --install-app) INSTALL_APP=1 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

[ "$(uname -s)" = "Darwin" ] || { echo "macOS only" >&2; exit 2; }
[ "$(uname -m)" = "arm64" ] || { echo "this script builds for Apple silicon (arm64)" >&2; exit 2; }

# .env.local supplies the build-time Drive values; signing settings exported
# in the calling shell win over anything it might also define.
SHELL_SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:-}"
SHELL_NOTARY_PROFILE="${NOTARY_PROFILE:-}"
set -a; [ -f .env.local ] && source .env.local; set +a
[ -n "$SHELL_SIGNING_IDENTITY" ] && APPLE_SIGNING_IDENTITY="$SHELL_SIGNING_IDENTITY"
[ -n "$SHELL_NOTARY_PROFILE" ] && NOTARY_PROFILE="$SHELL_NOTARY_PROFILE"
: "${APPLE_SIGNING_IDENTITY:?set APPLE_SIGNING_IDENTITY to the Developer ID Application identity}"
: "${NOTARY_PROFILE:?set NOTARY_PROFILE to your notarytool keychain profile name}"
export APPLE_SIGNING_IDENTITY

# The Developer ID identity must be the one used: an Apple Development
# identity would sign fine but fail notarization at the very end.
# (Command output is captured first: with pipefail, `cmd | grep -q` can fail
# spuriously when grep exits early and cmd dies of SIGPIPE.)
IDENTITIES="$(security find-identity -v -p codesigning)"
IDENTITY_LINE="$(grep -F "$APPLE_SIGNING_IDENTITY" <<<"$IDENTITIES" || true)"
if ! grep -q "Developer ID Application" <<<"$IDENTITY_LINE"; then
  echo "APPLE_SIGNING_IDENTITY is not a valid 'Developer ID Application' identity in the keychain" >&2
  exit 1
fi
if [ -z "${GDRIVE_CLIENT_ID:-}" ] || [ -z "${GDRIVE_CLIENT_SECRET:-}" ]; then
  echo "WARNING: GDRIVE_CLIENT_ID/GDRIVE_CLIENT_SECRET not set; Google Drive upload will be disabled in this build" >&2
fi

VERSION="$(grep -m1 '"version"' package.json | sed -E 's/.*"([0-9.]+)".*/\1/')"
PRODUCT="ClipToAll Personal"
BUNDLE_ID="$(sed -nE 's/.*"identifier": *"([^"]+)".*/\1/p' src-tauri/tauri.personal.conf.json)"
PLUGIN_NAME="cliptoall-encryption-plugin"
PLUGINS_DIR="$HOME/Library/Application Support/$PRODUCT/plugins"
OUT="dist-macos/personal"
# Own target dir: the personal .app must never sit in the public build's
# target/release/bundle/, where a later public packaging step (or a person)
# could pick it up by mistake.
export CARGO_TARGET_DIR="$PWD/src-tauri/target-personal"
APP_SRC="$CARGO_TARGET_DIR/release/bundle/macos/$PRODUCT.app"
PLUGIN_SRC="plugins/encryption-plugin/target/aarch64-apple-darwin/release/$PLUGIN_NAME"
ZIP="$OUT/ClipToAll-Personal-${VERSION}-macos-arm64.zip"
DEST="/Applications/$PRODUCT.app"

# Fail before the long build + notarization, not after it.
if [ "$INSTALL_APP" -eq 1 ] && pgrep -f "$DEST/Contents/MacOS/" >/dev/null; then
  echo "$PRODUCT is running; quit it (menu bar icon > Exit) and re-run with --install-app" >&2
  exit 1
fi

# ── 1. App ─────────────────────────────────────────────────────────
# build.rs does not rerun when env changes; force the option_env! values in.
touch src-tauri/src/commands/upload_gdrive.rs

# CARGO_PROFILE_RELEASE_STRIP=none: with the current Apple toolchain the
# default release strip step corrupts proc-macro dylibs (dyld: "mis-aligned
# LINKEDIT string pool") and the build fails.
# The overlay config sets the name, identifier, the "app" target only (no
# Tauri DMG step, which drives Finder and once picked up a stray file) and
# Entitlements-direct.plist + hardened runtime. With APPLE_SIGNING_IDENTITY
# exported, Tauri signs the bundle with it (and a secure timestamp).
CARGO_PROFILE_RELEASE_STRIP=none npm run tauri build -- \
  --features personal \
  --config src-tauri/tauri.personal.conf.json

[ -d "$APP_SRC" ] || { echo "expected bundle not found: $APP_SRC" >&2; exit 1; }
APP_BIN="$APP_SRC/Contents/MacOS/$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$APP_SRC/Contents/Info.plist")"

# Sanity checks on what was built, before spending a notarization round-trip.
[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$APP_SRC/Contents/Info.plist")" = "$BUNDLE_ID" ] \
  || { echo "bundle identifier is not $BUNDLE_ID" >&2; exit 1; }
LC_ALL=C grep -aqF 'CLIPTOALL_PLUGIN_SYSTEM_COMPILED_IN_V1' "$APP_BIN" \
  || { echo "plugin system marker missing: was --features personal applied?" >&2; exit 1; }
for var in GDRIVE_CLIENT_ID GDRIVE_CLIENT_SECRET; do
  if [ -n "${!var:-}" ]; then
    LC_ALL=C grep -aqF -- "${!var}" "$APP_BIN" \
      || { echo "$var is not baked into the binary" >&2; exit 1; }
    echo "ok: $var is baked in (value not shown)"
  fi
done
ENTITLEMENTS="$(codesign -d --entitlements - "$APP_SRC" 2>/dev/null)"
if grep -q "com.apple.security.app-sandbox" <<<"$ENTITLEMENTS"; then
  echo "the personal build must not carry the App Store sandbox entitlements" >&2; exit 1
fi
SIGNATURE="$(codesign -dvv "$APP_SRC" 2>&1)"
grep -Eq '^CodeDirectory .*flags=.*runtime' <<<"$SIGNATURE" \
  || { echo "hardened runtime is not enabled on the app" >&2; exit 1; }

# ── 2. Encryption plugin ───────────────────────────────────────────
rustup target list --installed | grep -qx aarch64-apple-darwin || rustup target add aarch64-apple-darwin
env -u CARGO_TARGET_DIR CARGO_PROFILE_RELEASE_STRIP=none cargo build --release \
  --manifest-path plugins/encryption-plugin/Cargo.toml --target aarch64-apple-darwin
# Spawned executables must be signed on Apple silicon; sign it like the app.
codesign --force --sign "$APPLE_SIGNING_IDENTITY" --options runtime --timestamp \
  --identifier "$BUNDLE_ID.encryption-plugin" "$PLUGIN_SRC"

# ── 3. Notarize (app + plugin in one submission), staple the app ──
rm -rf "$OUT"; mkdir -p "$OUT/stage/plugins"
ditto "$APP_SRC" "$OUT/stage/$PRODUCT.app"
cp "$PLUGIN_SRC" "$OUT/stage/plugins/$PLUGIN_NAME"
codesign --verify --deep --strict "$OUT/stage/$PRODUCT.app"
codesign --verify --strict "$OUT/stage/plugins/$PLUGIN_NAME"

SUBMIT_ZIP="$OUT/notarize-submit.zip"
ditto -c -k --sequesterRsrc "$OUT/stage" "$SUBMIT_ZIP"
xcrun notarytool submit "$SUBMIT_ZIP" --keychain-profile "$NOTARY_PROFILE" --wait
rm -f "$SUBMIT_ZIP"
# A bare Mach-O cannot carry a stapled ticket; the plugin's ticket stays
# online with Apple, the app gets its ticket stapled.
xcrun stapler staple "$OUT/stage/$PRODUCT.app"
xcrun stapler validate "$OUT/stage/$PRODUCT.app"
spctl -a -t exec -vv "$OUT/stage/$PRODUCT.app"

# Distributable zip, made from the clean staging folder only.
ditto -c -k --sequesterRsrc "$OUT/stage" "$ZIP"
(cd "$OUT" && shasum -a 256 "$(basename "$ZIP")" > "$(basename "$ZIP").sha256")

# ── 4. Install ─────────────────────────────────────────────────────
# Plugins dir: outside the bundle, owner-only (the app enforces 0700 too).
mkdir -p "$PLUGINS_DIR"
chmod 700 "$PLUGINS_DIR"
install -m 0755 "$OUT/stage/plugins/$PLUGIN_NAME" "$PLUGINS_DIR/$PLUGIN_NAME"
codesign --verify --strict "$PLUGINS_DIR/$PLUGIN_NAME"
echo "Installed plugin: $PLUGINS_DIR/$PLUGIN_NAME"

if [ "$INSTALL_APP" -eq 1 ]; then
  if pgrep -f "$DEST/Contents/MacOS/" >/dev/null; then
    echo "$PRODUCT is running; quit it (menu bar icon > Exit) and re-run with --install-app" >&2
    exit 1
  fi
  rm -rf "$DEST"
  ditto "$OUT/stage/$PRODUCT.app" "$DEST"
  codesign --verify --deep --strict "$DEST"
  echo "Installed app: $DEST"
fi

echo "Done: $ZIP"
