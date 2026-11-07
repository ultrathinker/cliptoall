#!/usr/bin/env bash
#
# Build, sign, notarize and staple the macOS (Apple silicon) DMG for direct
# distribution (Developer ID, hardened runtime, NOT sandboxed). macOS only.
# The Windows release path (release.sh / release.yml) is unrelated to this.
#
# Prerequisites (none of these are stored in the repo):
#   * .env.local with GDRIVE_CLIENT_ID / GDRIVE_CLIENT_SECRET (baked in at build time)
#   * a "Developer ID Application" identity in the login keychain
#       export APPLE_SIGNING_IDENTITY=<SHA-1 from `security find-identity -v -p codesigning`>
#   * a notarytool keychain profile, created once by you with:
#       xcrun notarytool store-credentials cliptoall-notary
#     and exported as NOTARY_PROFILE=cliptoall-notary
#
# Usage (repo root):  ./scripts/macos-release.sh
set -euo pipefail
cd "$(dirname "$0")/.."

: "${APPLE_SIGNING_IDENTITY:?set APPLE_SIGNING_IDENTITY to the Developer ID Application identity}"
: "${NOTARY_PROFILE:?set NOTARY_PROFILE to your notarytool keychain profile name}"

VERSION="$(grep -m1 '"version"' package.json | sed -E 's/.*"([0-9.]+)".*/\1/')"
set -a; [ -f .env.local ] && source .env.local; set +a

# build.rs does not rerun when env changes; force the option_env! values in.
touch src-tauri/src/commands/upload_gdrive.rs

# CARGO_PROFILE_RELEASE_STRIP=none: with the current Apple toolchain, the
# default release strip step corrupts proc-macro dylibs (dyld: "mis-aligned
# LINKEDIT string pool") and the build fails. The final binary is larger but
# otherwise identical in behaviour.
CARGO_PROFILE_RELEASE_STRIP=none npm run tauri build -- \
  --config '{"bundle":{"macOS":{"entitlements":"Entitlements-direct.plist","hardenedRuntime":true}}}'

DMG_SRC="src-tauri/target/release/bundle/dmg/ClipToAll_${VERSION}_aarch64.dmg"
OUT="dist-macos"; mkdir -p "$OUT"
DMG="$OUT/ClipToAll-${VERSION}-macos-arm64.dmg"
cp "$DMG_SRC" "$DMG"
codesign --force --sign "$APPLE_SIGNING_IDENTITY" --timestamp "$DMG"

xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
xcrun stapler staple "$DMG"

xcrun stapler validate "$DMG"
spctl -a -t open --context context:primary-signature -vv "$DMG"
(cd "$OUT" && shasum -a 256 "$(basename "$DMG")" > "$(basename "$DMG").sha256")
echo "Done: $DMG"
