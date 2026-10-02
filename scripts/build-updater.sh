#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="${1:?Usage: build-updater.sh /path/to/Excavator.app}"
SPARKLE="$(bash "$ROOT/scripts/fetch-sparkle.sh")"
FRAMEWORKS="$APP/Contents/Frameworks"
mkdir -p "$FRAMEWORKS"
ditto "$SPARKLE/Sparkle.framework" "$FRAMEWORKS/Sparkle.framework"
clang -fobjc-arc -dynamiclib -mmacosx-version-min=12.0 \
  -framework Foundation -framework AppKit -framework Sparkle -F "$SPARKLE" \
  -Wl,-rpath,@loader_path -install_name @rpath/ExcavatorUpdater.dylib \
  "$ROOT/native/updater.m" -o "$FRAMEWORKS/ExcavatorUpdater.dylib"
