# Changelog

## 0.3.0 — 2026-10-08

### Added
- Standalone macOS TUI with local and saved remote browsing, expandable file trees, Nerd Font icons, command palette and Vim-style keyboard controls.
- Reviewed copy/move to another browser or an idle Herdr shell's current directory; queued jobs, conflict decisions, cancellation, Log and a bottom progress gauge.
- Compact dialogs, a combined Copy review with local shortcuts and favorites, and connection editing/group management.
- Favorite add/remove, ascending/descending sorting, Return to local, and explicit opening of regular local files in their default app.

### Fixed
- Preserve shared favorites when an already-open desktop app saves its workspace after a TUI edit. Serialize favorite edits across TUI processes and preserve unrelated preferences.

### Verification
- Thirty TUI/shared-library tests and native disposable-config terminal checks passed, including restart persistence, simultaneous favorite saves, every sort field in both directions, group edits and terminal restoration.
- GUI/TUI release builds, desktop bundle signing, archive checksums and the Sparkle Ed25519 signature passed. Fresh remote transfer checks remain limited by coordination-socket restrictions; native desktop interaction for the shared-save change and default-app launching were not rechecked.
- Apple Silicon prerelease; desktop bundle remains ad hoc signed and not notarized.

## 0.2.3 — 2026-10-06

### Added
- Drop files into the terminal to insert shell-quoted absolute paths at the cursor without executing them. SSH terminals accept paths from the same SFTP connection.
- Drag to select terminal output and press Cmd+C to copy. Ctrl+C continues to interrupt the shell. Selected output remains stable until input, scrolling, resizing, or a click clears it. Copying visual rows inserts newlines, including at wrapped lines.

### Changed
- Rename Transfers to Log because it also contains deletion entries. Open it only on user request.

### Fixed
- Preserve selection during listing refresh and select the closest surviving file after deletion instead of jumping to the top.

### Verification
- Compilation, macOS bundle build, and existing PTY checks passed. Native selection, clipboard, and file-drop acceptance remain pending.
- This remains an Apple Silicon prerelease, ad hoc signed and not notarized.

## 0.2.2 — 2026-10-02

### Added
- CPU and resident RAM in the status bar, with a click-to-open graph popup and command-palette access. Graphs collect every two seconds while open and keep at most 120 samples in memory.

### Fixed
- Bound split-panel contents to prevent the lower pane being pushed out of view.
- Isolate usage redraws from the file-browser workspace.
- Avoid idle terminal grid copies, share paint snapshots, skip default cell backgrounds, and reduce polling.

### Verification
- User confirmed Trash/restoration, SFTP caching/failure recovery, tab dragging and modal interaction.
- Build checks pass; the new split correction and CPU improvements still require live acceptance. This remains an ad hoc signed, non-notarized prerelease.

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
