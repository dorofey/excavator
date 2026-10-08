#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release --locked --no-default-features --features tui --bin excavator-tui
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
package="dist/tui/excavator-tui-$version-macos-arm64"
archive="$package.tar.gz"
mkdir -p "$package"
cp target/release/excavator-tui "$package/excavator-tui"
codesign --force --sign - "$package/excavator-tui"
codesign --verify --strict "$package/excavator-tui"
cp TUI-MACOS.md "$package/TUI-MACOS.md"
cp CHANGELOG.md "$package/CHANGELOG.md"
tar -czf "$archive" -C dist/tui "$(basename "$package")"
(cd dist/tui && shasum -a 256 "$(basename "$archive")" > "$(basename "$archive").sha256")
printf 'TUI archive: %s\n' "$archive"
