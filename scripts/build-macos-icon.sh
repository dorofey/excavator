#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
icon_source="assets/excavator-icon.png"
icon_target="${1:-dist/Excavator.app/Contents/Resources/Excavator.icns}"
icon_work=$(mktemp -d "${TMPDIR:-/tmp}/excavator-icon.XXXXXX")
trap 'rm -rf "$icon_work"' EXIT HUP INT TERM
icon_set="$icon_work/Excavator.iconset"
mkdir -p "$icon_set" "$(dirname "$icon_target")"
for icon_size in 16 32 128 256 512; do
    sips -z "$icon_size" "$icon_size" "$icon_source" \
        --out "$icon_set/icon_${icon_size}x${icon_size}.png" >/dev/null
    icon_retina=$((icon_size * 2))
    sips -z "$icon_retina" "$icon_retina" "$icon_source" \
        --out "$icon_set/icon_${icon_size}x${icon_size}@2x.png" >/dev/null
done
iconutil --convert icns "$icon_set" --output "$icon_work/Excavator.icns"
mv -f "$icon_work/Excavator.icns" "$icon_target"
