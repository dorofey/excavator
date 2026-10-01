#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
cargo build --locked
bundle="dist/Excavator.app"
mkdir -p "$bundle/Contents/MacOS"
cp target/debug/excavator "$bundle/Contents/MacOS/Excavator.next"
# Keep any running app's executable inode intact while rebuilding.
mv -f "$bundle/Contents/MacOS/Excavator.next" "$bundle/Contents/MacOS/Excavator"
sh scripts/build-macos-icon.sh "$bundle/Contents/Resources/Excavator.icns"
cat > "$bundle/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Excavator</string>
<key>CFBundleDisplayName</key><string>Excavator</string>
<key>CFBundleIdentifier</key><string>local.excavator</string>
<key>CFBundleExecutable</key><string>Excavator</string>
<key>CFBundleIconFile</key><string>Excavator.icns</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>CFBundleVersion</key><string>1</string>
<key>LSMinimumSystemVersion</key><string>11.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
codesign --force --sign - "$bundle"
printf 'Built %s\n' "$bundle"
