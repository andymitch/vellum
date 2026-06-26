#!/usr/bin/env sh
#
# Install the latest released Vellum into /Applications on macOS.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/andymitch/vellum/main/scripts/install.sh | sh
#
# Downloads the signed release build (Apple Silicon) and unpacks it; it does
# not build from source. For Android, see the README.
set -eu

REPO="andymitch/vellum"
APP_NAME="Vellum"
DEST="/Applications/$APP_NAME.app"

os="$(uname -s)"
arch="$(uname -m)"

if [ "$os" != "Darwin" ]; then
  echo "This installer supports macOS only (got: $os)." >&2
  echo "For Android, download vellum-release.apk from the Releases page." >&2
  exit 1
fi
if [ "$arch" != "arm64" ]; then
  echo "Only Apple Silicon (arm64) builds are published (got: $arch)." >&2
  echo "Build from source instead — see the README's Development section." >&2
  exit 1
fi

url="https://github.com/$REPO/releases/latest/download/${APP_NAME}_aarch64.app.tar.gz"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "==> Downloading the latest $APP_NAME release"
curl -fsSL "$url" -o "$tmp/vellum.app.tar.gz"

echo "==> Installing to $DEST"
if [ -d "$DEST" ]; then
  rm -rf "$DEST"
fi
tar -xzf "$tmp/vellum.app.tar.gz" -C /Applications

if [ ! -d "$DEST" ]; then
  echo "Archive did not contain $APP_NAME.app" >&2
  exit 1
fi

echo "==> Clearing Gatekeeper quarantine"
xattr -dr com.apple.quarantine "$DEST" 2>/dev/null || true

echo "==> Done. Launch with: open -a \"$APP_NAME\""
echo "    Vellum updates itself from here on — no need to re-run this."
