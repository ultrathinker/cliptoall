#!/usr/bin/env bash
#
# Build, sign, notarize and staple the PUBLIC macOS (Apple silicon) DMG for
# direct distribution (Developer ID, hardened runtime, NOT sandboxed). This is
# the default build: no Cargo features, so no plugin system (checked below).
# macOS only. The Windows release path (release.sh / release.yml) is unrelated.
#
# Prerequisites (none of these are stored in the repo):
#   * .env.local with GDRIVE_CLIENT_ID / GDRIVE_CLIENT_SECRET (baked in at build time)
#   * a "Developer ID Application" identity in the login keychain
#       export APPLE_SIGNING_IDENTITY=<SHA-1 from `security find-identity -v -p codesigning`>
#   * a notarytool keychain profile, created once by you with:
#       xcrun notarytool store-credentials <profile-name>
#     and exported as NOTARY_PROFILE=<profile-name>
#
# Usage (repo root):
#   ./scripts/macos-release.sh                        # version from package.json
#   MACOS_VERSION=5.1.26 ./scripts/macos-release.sh   # macOS-only version bump
# MACOS_VERSION is applied through a Tauri config override (Info.plist, About
# window, DMG name), so a macOS release can move on without touching the
# version files the Windows build shares.
#
# Output: dist-macos/ClipToAll-<version>-macos-arm64.dmg (+ .sha256). The DMG
# is made with hdiutil from a staging folder holding only ClipToAll.app and an
# Applications link; Tauri's own DMG step is not used (it drives Finder, and
# once picked up a stray file). The intermediate .app copies are deleted at
# the end so Spotlight does not list them.
set -euo pipefail
cd "$(dirname "$0")/.."

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

# Keep local paths out of the binary. rustc records the source path of every
# crate it compiles (panic locations, file!()), including the cargo registry
# under the builder's home folder; remap them to neutral prefixes. The
# encoded form (0x1f-separated) survives paths with spaces. It replaces any
# RUSTFLAGS from the calling shell.
REPO_ROOT="$(pwd -P)"
SEP=$'\x1f'
export CARGO_ENCODED_RUSTFLAGS="--remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo${SEP}--remap-path-prefix=${RUSTUP_HOME:-$HOME/.rustup}=/rustup${SEP}--remap-path-prefix=$REPO_ROOT=/cliptoall"

# (Output is captured first: with pipefail, `cmd | grep -q` can fail when grep
# exits early and cmd dies of SIGPIPE.)
IDENTITIES="$(security find-identity -v -p codesigning)"
IDENTITY_LINE="$(grep -F "$APPLE_SIGNING_IDENTITY" <<<"$IDENTITIES" || true)"
grep -q "Developer ID Application" <<<"$IDENTITY_LINE" \
  || { echo "APPLE_SIGNING_IDENTITY is not a 'Developer ID Application' identity in the keychain" >&2; exit 1; }
if [ -z "${GDRIVE_CLIENT_ID:-}" ] || [ -z "${GDRIVE_CLIENT_SECRET:-}" ]; then
  echo "GDRIVE_CLIENT_ID/GDRIVE_CLIENT_SECRET are not set; a public release must have Google Drive" >&2
  exit 1
fi

VERSION="${MACOS_VERSION:-$(grep -m1 '"version"' package.json | sed -E 's/.*"([0-9.]+)".*/\1/')}"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "bad version: $VERSION" >&2; exit 1; }
PRODUCT="ClipToAll"
BUNDLE_ID="$(sed -nE 's/.*"identifier": *"([^"]+)".*/\1/p' src-tauri/tauri.conf.json)"
APP_SRC="src-tauri/target/release/bundle/macos/$PRODUCT.app"
OUT="dist-macos"
STAGE="$OUT/release-stage"
DMG="$OUT/$PRODUCT-${VERSION}-macos-arm64.dmg"

# ── 1. App ─────────────────────────────────────────────────────────
rm -rf "$APP_SRC"
# build.rs does not rerun when env changes; force the option_env! values in.
touch src-tauri/src/commands/upload_gdrive.rs

# CARGO_PROFILE_RELEASE_STRIP=none: with the current Apple toolchain, the
# default release strip step corrupts proc-macro dylibs (dyld: "mis-aligned
# LINKEDIT string pool") and the build fails. The final binary is larger but
# otherwise identical in behaviour.
# No `--features`: the public build never has the plugin system.
CONFIG="{\"version\":\"$VERSION\",\"bundle\":{\"targets\":[\"app\"],\"macOS\":{\"entitlements\":\"Entitlements-direct.plist\",\"hardenedRuntime\":true}}}"
CARGO_PROFILE_RELEASE_STRIP=none npm run tauri build -- --config "$CONFIG"

[ -d "$APP_SRC" ] || { echo "expected bundle not found: $APP_SRC" >&2; exit 1; }
PLIST="$APP_SRC/Contents/Info.plist"
APP_BIN="$APP_SRC/Contents/MacOS/$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$PLIST")"

# Sanity checks before spending notarization round-trips.
[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$PLIST")" = "$BUNDLE_ID" ] \
  || { echo "bundle identifier is not $BUNDLE_ID" >&2; exit 1; }
[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$PLIST")" = "$VERSION" ] \
  || { echo "bundle version is not $VERSION" >&2; exit 1; }
./scripts/check-plugin-gate.sh --public "$APP_BIN"
for needle in "$HOME" "$(id -un)" "$REPO_ROOT"; do
  if LC_ALL=C grep -r -l -aF -- "$needle" "$APP_SRC" >/dev/null; then
    echo "the app bundle contains a local path or the user name; check the remap flags above" >&2
    LC_ALL=C grep -r -l -aF -- "$needle" "$APP_SRC" >&2
    exit 1
  fi
done
echo "ok: no local paths or user name in the app bundle"
for var in GDRIVE_CLIENT_ID GDRIVE_CLIENT_SECRET; do
  LC_ALL=C grep -aqF -- "${!var}" "$APP_BIN" \
    || { echo "$var is not baked into the binary" >&2; exit 1; }
  echo "ok: $var is baked in (value not shown)"
done
ENTITLEMENTS="$(codesign -d --entitlements - "$APP_SRC" 2>/dev/null)"
if grep -q "com.apple.security.app-sandbox" <<<"$ENTITLEMENTS"; then
  echo "the direct-distribution build must not carry the App Store sandbox entitlements" >&2; exit 1
fi
SIGNATURE="$(codesign -dvv "$APP_SRC" 2>&1)"
grep -Eq '^CodeDirectory .*flags=.*runtime' <<<"$SIGNATURE" \
  || { echo "hardened runtime is not enabled on the app" >&2; exit 1; }

# ── 2. Notarize + staple the app ───────────────────────────────────
rm -rf "$STAGE"; mkdir -p "$STAGE"
ditto "$APP_SRC" "$STAGE/$PRODUCT.app"
codesign --verify --deep --strict "$STAGE/$PRODUCT.app"
APP_ZIP="$OUT/notarize-app.zip"
ditto -c -k --sequesterRsrc --keepParent "$STAGE/$PRODUCT.app" "$APP_ZIP"
xcrun notarytool submit "$APP_ZIP" --keychain-profile "$NOTARY_PROFILE" --wait
rm -f "$APP_ZIP"
xcrun stapler staple "$STAGE/$PRODUCT.app"
xcrun stapler validate "$STAGE/$PRODUCT.app"
spctl -a -t exec -vv "$STAGE/$PRODUCT.app"

# ── 3. DMG from the clean staging folder ───────────────────────────
ln -s /Applications "$STAGE/Applications"
STAGED="$(ls -A "$STAGE" | sort | tr '\n' ' ')"
[ "$STAGED" = "Applications $PRODUCT.app " ] || { echo "unexpected staging contents: $STAGED" >&2; exit 1; }
rm -f "$DMG" "$DMG.sha256"
hdiutil create -volname "$PRODUCT $VERSION" -srcfolder "$STAGE" -fs HFS+ -format UDZO -ov "$DMG"

# Look inside before shipping it.
MNT="$(mktemp -d)"
hdiutil attach -nobrowse -readonly -mountpoint "$MNT" "$DMG" >/dev/null
echo "DMG contents:"; ls -la "$MNT"
MOUNTED="$(ls -A "$MNT" | sort | tr '\n' ' ')"
codesign --verify --deep --strict "$MNT/$PRODUCT.app" && xcrun stapler validate "$MNT/$PRODUCT.app" >/dev/null
hdiutil detach "$MNT" >/dev/null; rmdir "$MNT"
# A volume may also carry hidden filesystem entries (.fseventsd etc.); allow those only.
for entry in $MOUNTED; do
  case "$entry" in
    "$PRODUCT.app"|Applications|.fseventsd|.Trashes|.HFS+*) ;;
    *) echo "unexpected entry in the DMG: $entry" >&2; exit 1 ;;
  esac
done

codesign --force --sign "$APPLE_SIGNING_IDENTITY" --timestamp "$DMG"
xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
xcrun stapler staple "$DMG"
xcrun stapler validate "$DMG"
spctl -a -t open --context context:primary-signature -vv "$DMG"
(cd "$OUT" && shasum -a 256 "$(basename "$DMG")" > "$(basename "$DMG").sha256")

# ── 4. Clean up ────────────────────────────────────────────────────
rm -rf "$APP_SRC" "$STAGE"
echo "Done: $DMG"
