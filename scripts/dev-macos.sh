#!/usr/bin/env bash
#
# Build and install a side-by-side "Vellum Dev" on macOS, so you can test
# changes without touching the production Vellum.app or its notes.
#
# Uses tauri.dev.conf.json to override the product name + bundle identifier
# (com.andymitch.vellum.dev), so it installs as a separate app with its own
# data directory.
#
# Usage:
#   ./scripts/dev-macos.sh
#
set -euo pipefail

APP_NAME="Vellum Dev"
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUNDLE_DIR="$PROJECT_ROOT/src-tauri/target/release/bundle/macos"
DEST="/Applications/$APP_NAME.app"

if [[ "$(uname)" != "Darwin" ]]; then
  echo "This script only runs on macOS." >&2
  exit 1
fi

cd "$PROJECT_ROOT"

echo "==> Installing JS dependencies"
bun install

echo "==> Building $APP_NAME (tauri build, dev config)"
bun tauri build --config src-tauri/tauri.dev.conf.json

SRC_APP="$BUNDLE_DIR/$APP_NAME.app"
if [[ ! -d "$SRC_APP" ]]; then
  echo "Build did not produce $SRC_APP" >&2
  echo "Check the bundle directory: $BUNDLE_DIR" >&2
  exit 1
fi

echo "==> Installing to $DEST"
rm -rf "$DEST"
cp -R "$SRC_APP" "$DEST"

echo "==> Clearing Gatekeeper quarantine"
xattr -dr com.apple.quarantine "$DEST" || true

echo "==> Done. Launch with: open -a \"$APP_NAME\""
echo "    (Separate app + data dir from production Vellum.)"
