#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
EXCAVATOR_BUILD_PROFILE=release ./scripts/build-macos-app.sh
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
archive="dist/updates/Excavator-$version-macos-arm64.zip"
mkdir -p dist/updates
if [ -f appcast.xml ]; then
    cp appcast.xml dist/updates/appcast.xml
    cp appcast.xml dist/updates/.previous-appcast.xml
fi
ditto -c -k --sequesterRsrc --keepParent dist/Excavator.app "$archive"
cp CHANGELOG.md "${archive%.zip}.md"
sparkle=$(bash scripts/fetch-sparkle.sh)
"$sparkle/bin/generate_appcast" --versions "$version" --embed-release-notes --account excavator --download-url-prefix "https://github.com/dorofey/excavator/releases/download/v$version/" dist/updates
# Preserve existing release entries: generate_appcast can rewrite their URLs
# using the newest release's download prefix, even with --versions.
python3 - "$version" dist/updates/.previous-appcast.xml dist/updates/appcast.xml <<'PYFEED'
import sys
from pathlib import Path
import xml.etree.ElementTree as ET
version, previous, current = sys.argv[1:]
namespace = "http://www.andymatuschak.org/xml-namespaces/sparkle"
ET.register_namespace("sparkle", namespace)
if Path(previous).exists():
    old = {item.findtext("{" + namespace + "}version"): item
           for item in ET.parse(previous).findall("./channel/item")}
    feed = ET.parse(current)
    channel = feed.find("./channel")
    for index, item in enumerate(list(channel)):
        number = item.findtext("{" + namespace + "}version")
        if number != version and number in old:
            channel.remove(item)
            channel.insert(index, old[number])
    present = {item.findtext("{" + namespace + "}version")
               for item in channel.findall("item")}
    for number, item in old.items():
        if number not in present:
            channel.append(item)
    ET.indent(feed, space="    ")
    feed.write(current, encoding="utf-8", xml_declaration=True)
PYFEED
cp dist/updates/appcast.xml appcast.xml
(cd dist/updates && shasum -a 256 "$(basename "$archive")" > "$(basename "$archive").sha256")
printf 'Release archive: %s\n' "$archive"
