#!/bin/sh
# Vendors iroh-docs 0.101.0 (the version src-tauri pins) and applies the one
# upstream change the browser vault needs: a public Store constructor taking a
# redb::StorageBackend, so the replica can live on OPFS instead of a file.
#
# Vendored from crates.io rather than forked on GitHub, so the spike is
# reproducible from this repo alone. Output (.spike-vendor/) is gitignored.
set -eu
VERSION=0.101.0
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../.." && pwd)
OUT="$ROOT/.spike-vendor"
DEST="$OUT/iroh-docs-$VERSION"

if [ -f "$DEST/.patched" ]; then
  echo "already vendored: $DEST"
  exit 0
fi

mkdir -p "$OUT"
echo "fetching iroh-docs ${VERSION}"
curl -sL "https://static.crates.io/crates/iroh-docs/iroh-docs-${VERSION}.crate" | tar xz -C "$OUT"
patch -s -p1 -d "$DEST" < "$HERE/store-with-backend.patch"
# Marker so re-runs are cheap and idempotent.
touch "$DEST/.patched"
echo "vendored + patched: $DEST"
