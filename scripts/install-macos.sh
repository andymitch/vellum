#!/usr/bin/env bash
#
# Build Vellum and install it into /Applications on macOS.
#
# Usage:
#   ./scripts/install-macos.sh
#
set -euo pipefail

APP_NAME="Vellum"
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

echo "==> Building $APP_NAME (tauri build)"
bun run tauri build

SRC_APP="$BUNDLE_DIR/$APP_NAME.app"
if [[ ! -d "$SRC_APP" ]]; then
  echo "Build did not produce $SRC_APP" >&2
  echo "Check the bundle directory: $BUNDLE_DIR" >&2
  exit 1
fi

echo "==> Installing to $DEST"
if [[ -d "$DEST" ]]; then
  echo "    Removing existing $DEST"
  rm -rf "$DEST"
fi
cp -R "$SRC_APP" "$DEST"

echo "==> Clearing Gatekeeper quarantine"
xattr -dr com.apple.quarantine "$DEST" || true

echo "==> Done. Launch with: open -a \"$APP_NAME\""
