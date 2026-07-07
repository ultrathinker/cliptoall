#!/usr/bin/env bash
#
# Cut a new ClipToAll release.
#
# Bumps the version in every version file, commits, pushes `main`, then creates
# and pushes the vX.Y.Z tag. Pushing the tag triggers the "release" GitHub
# Actions workflow (.github/workflows/release.yml), which builds the Windows
# bundle and AUTO-PUBLISHES a GitHub Release with the installer.
#
# Usage (from the repo root, in Git Bash):
#   ./release.sh 5.1.16
#
set -euo pipefail

NEW="${1:-}"
if [[ ! "$NEW" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Usage: ./release.sh <version>   e.g. ./release.sh 5.1.16" >&2
  exit 1
fi

cd "$(dirname "$0")"

OLD="$(grep -m1 '"version"' package.json | sed -E 's/.*"version"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')"
if [[ "$OLD" == "$NEW" ]]; then
  echo "Version is already $NEW — nothing to bump." >&2
  exit 1
fi
echo "Bumping $OLD -> $NEW"

# 1) Version files — must all match or the Tauri build errors.
sed -i "s/\"version\": \"$OLD\"/\"version\": \"$NEW\"/" package.json src-tauri/tauri.conf.json
sed -i "s/^version = \"$OLD\"/version = \"$NEW\"/" src-tauri/Cargo.toml
# The app crate's version line inside the committed Cargo.lock (line right after its name).
sed -i "/name = \"cliptoall-tauri2\"/{n;s/^version = \".*\"/version = \"$NEW\"/}" src-tauri/Cargo.lock

# 2) Keep package-lock.json in sync — `npm ci` in CI fails on a version mismatch.
npm install --package-lock-only --no-audit --no-fund >/dev/null

# 3) Commit, push, tag, push tag -> triggers the auto-release workflow.
git add package.json package-lock.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "Release v$NEW"
git push github main
git tag "v$NEW"
git push github "v$NEW"

echo
echo "Done. GitHub Actions is building and publishing v$NEW:"
echo "  Actions:  https://github.com/ultrathinker/cliptoall/actions"
echo "  Releases: https://github.com/ultrathinker/cliptoall/releases"
