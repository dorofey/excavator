#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION=2.10.0
SHA256=c2bf58aa8387266ac179357b1415d6f2635f044da8be41042af32425dae6da0c
CACHE="$ROOT/target/sparkle/$VERSION"
ARCHIVE="$CACHE/Sparkle-$VERSION.tar.xz"
mkdir -p "$CACHE"
if [[ ! -f "$ARCHIVE" ]]; then
  curl --fail --location --proto '=https' --proto-redir '=https' --tlsv1.2 \
    "https://github.com/sparkle-project/Sparkle/releases/download/$VERSION/Sparkle-$VERSION.tar.xz" \
    --output "$ARCHIVE.download"
  mv "$ARCHIVE.download" "$ARCHIVE"
fi
echo "$SHA256  $ARCHIVE" | shasum -a 256 --check >/dev/null
if [[ ! -d "$CACHE/Sparkle.framework" ]]; then
  tar -xJf "$ARCHIVE" -C "$CACHE"
fi
printf '%s\n' "$CACHE"
