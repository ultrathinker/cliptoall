#!/bin/bash
# Cargo `runner` for macOS dev builds (see ../.cargo/config.toml).
#
# WHY THIS EXISTS: Xcode's toolchain ad-hoc-signs every `cargo build` output
# with a signature derived from the binary's own bytes, so it changes on
# every rebuild. macOS's Keychain access-control prompt ("<app> wants to use
# your confidential information") is keyed off that signature — with an
# ever-changing one, every single rebuild looks like a brand-new app, so the
# password prompt never stops reappearing during normal dev iteration.
#
# Re-signing here with a STABLE identity (an actual Apple Development/
# Developer ID cert, not ad-hoc) fixes that: Keychain ties its "Always
# Allow" memory to the Team ID + identifier, which stays constant across
# rebuilds once a real cert is used consistently.
#
# No secret lives in this file — the identity (a certificate SHA-1 hash, not
# a private key) comes from $APPLE_DEV_SIGNING_IDENTITY, expected to be
# exported before `cargo run`/`npm run tauri:dev` (e.g. via .env.local,
# which is gitignored). Falls through to running unsigned if unset, so
# nothing breaks for a fresh checkout that hasn't configured this yet — the
# dialog just keeps reappearing until someone does.
set -e

BINARY="$1"
shift

if [ -n "$APPLE_DEV_SIGNING_IDENTITY" ]; then
    codesign --force --sign "$APPLE_DEV_SIGNING_IDENTITY" --options runtime "$BINARY" 2>&1 | grep -v "replacing existing signature" || true
else
    echo "macos-dev-runner: \$APPLE_DEV_SIGNING_IDENTITY not set — running unsigned (Keychain will re-prompt every rebuild)" >&2
fi

exec "$BINARY" "$@"
