#!/usr/bin/env bash
#
# Proves that the DEFAULT macOS build contains no plugin system and that the
# personal build (`--features personal`) does. Run on macOS; CI runs it on
# every push/PR (.github/workflows/build.yml).
#
# How: src-tauri/src/plugins.rs carries a `#[used]` marker string that exists
# only when the plugin module is compiled in (`cfg(plugins)`, see build.rs).
# As a second, independent signal we also look for a string the plugin
# launcher itself uses (the hello-handshake timeout message), so a renamed or
# stripped marker cannot make a binary that CAN spawn plugins look clean.
#
# Usage (repo root):  ./scripts/check-plugin-gate.sh            # debug builds
#                     PROFILE=release ./scripts/check-plugin-gate.sh
set -euo pipefail
cd "$(dirname "$0")/../src-tauri"

[ "$(uname -s)" = "Darwin" ] || { echo "check-plugin-gate: macOS only" >&2; exit 2; }

PROFILE="${PROFILE:-debug}"
PROFILE_FLAG=()
[ "$PROFILE" = "release" ] && PROFILE_FLAG=(--release)
BIN="${CARGO_TARGET_DIR:-target}/$PROFILE/cliptoall-tauri2"

MARKER='CLIPTOALL_PLUGIN_SYSTEM_COMPILED_IN_V1'
SPAWN_SIGNAL='timeout: no hello within 20s'

has() {
  [ -f "$BIN" ] || { echo "FAIL: built binary not found at $BIN" >&2; exit 1; }
  LC_ALL=C grep -aqF -- "$1" "$BIN"
}

fail=0

echo "== default build (no features)"
cargo build ${PROFILE_FLAG[@]+"${PROFILE_FLAG[@]}"}
if has "$MARKER"; then echo "FAIL: plugin marker found in the default build"; fail=1; else echo "ok: no plugin marker"; fi
if has "$SPAWN_SIGNAL"; then echo "FAIL: plugin launcher code found in the default build"; fail=1; else echo "ok: no plugin launcher code"; fi

echo "== personal build (--features personal)"
cargo build ${PROFILE_FLAG[@]+"${PROFILE_FLAG[@]}"} --features personal
if has "$MARKER"; then echo "ok: plugin marker present"; else echo "FAIL: plugin marker missing from the personal build"; fail=1; fi
if has "$SPAWN_SIGNAL"; then echo "ok: plugin launcher code present"; else echo "FAIL: plugin launcher code missing from the personal build"; fail=1; fi

[ "$fail" -eq 0 ] && echo "plugin gate: OK" || { echo "plugin gate: FAILED"; exit 1; }
