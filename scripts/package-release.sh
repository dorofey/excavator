#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
EXCAVATOR_BUILD_PROFILE=release ./scripts/build-macos-app.sh
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
archive="dist/updates/Excavator-$version-macos-arm64.zip"
mkdir -p dist/updates
if [ -f appcast.xml ]; then cp appcast.xml dist/updates/appcast.xml; fi
ditto -c -k --sequesterRsrc --keepParent dist/Excavator.app "$archive"
sparkle=$(bash scripts/fetch-sparkle.sh)
"$sparkle/bin/generate_appcast" --account excavator --download-url-prefix "https://github.com/dorofey/excavator/releases/download/v$version/" dist/updates
cp dist/updates/appcast.xml appcast.xml
shasum -a 256 "$archive" > "$archive.sha256"
printf 'Release archive: %s\n' "$archive"
