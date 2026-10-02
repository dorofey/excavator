# Changelog

## 0.2.1 — 2026-10-02

### Fixed
- Vim `dd` now reviews moving local files to macOS Trash instead of requesting unsupported permanent deletion. Remote files retain explicit permanent-deletion confirmation.
- Local delete requests use Trash consistently, with accurate confirmation text.

### Verification
- Tab dragging reported working by the user.
- Update installation/relaunch and the corrected Trash interaction are being checked; this remains an ad hoc signed, non-notarized prerelease.

## 0.2.0 — 2026-10-02

### Added
- Optional Vim navigation and selection, search, history, tab and split bindings, reviewed operations, and a settings toggle.
- Drag tabs between panes and splits while preserving terminal sessions and file history.
- Clickable path breadcrumbs and a compact loading indicator.
- SFTP directory caching with background revalidation, symlink-directory navigation, SSH key passphrase and agent authentication support.
- Connection groups and modal connection management/editing.
- In-app update checks, downloads, signed-archive verification, installation and relaunch through Sparkle.

### Changed
- Align sidebar search and tab strips with their panes, including nested splits and hidden sidebar.
- Separate file and terminal tab buttons; Option-click splits down and Shift-Option-click splits right.
- Compact sidebar hierarchy, improve input/tab padding, and distinguish remote tabs with icons.
- Present operation reviews and transfer conflict decisions as centered modals; replacement needs explicit confirmation.
- Require macOS 12 for the update-enabled Apple Silicon bundle.

### Release status
This is a prerelease, ad hoc signed and not notarized. The updater framework and
app build compile; live update installation, native Vim acceptance, clean-user
installation and large-directory performance remain unverified. Remote fixture
coverage does not establish interoperability with every external provider.
