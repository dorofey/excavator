#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
profile="${EXCAVATOR_BUILD_PROFILE:-debug}"
if [ "$profile" = release ]; then
    cargo build --locked --release
elif [ "$profile" = debug ]; then
    cargo build --locked
else
    printf 'Invalid build profile: %s\n' "$profile" >&2; exit 1
fi
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
bundle="dist/Excavator.app"
mkdir -p "$bundle/Contents/MacOS"
cp "target/$profile/excavator" "$bundle/Contents/MacOS/Excavator.next"
# Keep any running app's executable inode intact while rebuilding.
mv -f "$bundle/Contents/MacOS/Excavator.next" "$bundle/Contents/MacOS/Excavator"
sh scripts/build-macos-icon.sh "$bundle/Contents/Resources/Excavator.icns"
cat > "$bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Excavator</string>
<key>CFBundleDisplayName</key><string>Excavator</string>
<key>CFBundleIdentifier</key><string>local.excavator</string>
<key>CFBundleExecutable</key><string>Excavator</string>
<key>CFBundleIconFile</key><string>Excavator.icns</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>CFBundleVersion</key><string>$version</string>
<key>LSMinimumSystemVersion</key><string>12.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
if [ "$profile" = release ] && [ -z "${EXCAVATOR_UPDATE_PUBLIC_KEY:-}" ]; then
    EXCAVATOR_UPDATE_PUBLIC_KEY=$(cat assets/update-public-key.txt)
fi
if [ -n "${EXCAVATOR_UPDATE_PUBLIC_KEY:-}" ]; then
    /usr/libexec/PlistBuddy -c "Add :SUFeedURL string ${EXCAVATOR_UPDATE_FEED_URL:-https://raw.githubusercontent.com/dorofey/excavator/main/appcast.xml}" "$bundle/Contents/Info.plist"
    /usr/libexec/PlistBuddy -c "Add :SUPublicEDKey string $EXCAVATOR_UPDATE_PUBLIC_KEY" "$bundle/Contents/Info.plist"
    /usr/libexec/PlistBuddy -c "Add :SUEnableAutomaticChecks bool true" "$bundle/Contents/Info.plist"
    bash scripts/build-updater.sh "$bundle"
fi
codesign --force --deep --sign - "$bundle"
printf 'Built %s\n' "$bundle"
