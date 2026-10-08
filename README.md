# Excavator

A keyboard-first, two-pane file manager for macOS, built with Rust and
GPUI Kit. Browse folders with independent tabs and history, sort and select
files, manage favorites, and create, rename, copy, move, or send items to Trash.
The transfer drawer shows progress, conflicts, failures, and recovery details.
Remote connections support SFTP, encrypted FTPS, and S3 object storage.
Either initial pane can be split repeatedly to the right or below, with
independent tabs, history, selection, and resizable dividers.

The window opens straight into a full-width tab row under a plain title bar:
every pane's tabs sit above that pane, aligned to the pane's live width. The
title bar keeps only the window controls; back, forward, parent, refresh and the
command palette stay on ⌘[, ⌘], ⌘↑, ⌘R and ⌘⇧P, and the palette is a floating
modal (↑/↓, Return, Esc). Folders expand inline as a tree — click the chevron or
press →/←, while double-click or Return still opens the folder. Listings and the
sidebar show file-type icons.

The sidebar scrolls vertically, resizes by dragging its divider (120–480 px),
and expands favorites, locations and connections into folder trees. Focus it
with ⌘⌥S; ↑/↓ move, →/← expand and collapse, Return opens in the active pane and
Esc returns to the panes. Sidebar and transfer-drawer toggles are compact panel
icons at the bottom right of the status bar. Tab-close icons appear when a tab is
hovered or the close button has keyboard focus; their space stays reserved so tab
labels do not shift. Use the separate terminal button for a terminal tab. Option-click either new-tab
button to split down; Shift-Option-click splits right.

## Run and build

```sh
cargo run --locked
./scripts/build-macos-app.sh
open dist/Excavator.app
```

The script creates an ad-hoc signed Apple Silicon development bundle, not a
notarized release. It converts `assets/excavator-icon.png` into the standard
macOS icon sizes and bundles `Excavator.icns` with its plist registration.
GPUI Kit is pinned to 0.7.0; the lockfile pins its compatible
GPUI family. The binary targets macOS 12.0 or later; older OS versions are untested.

## Keyboard

Open grouped shortcut help with **⌘?** (**⌘⇧/**), the Help menu, or
the command palette. **?** also opens it from a file list. Escape or ⌘?
closes it and restores focus. Scroll or use arrow/Page Up/Page Down keys
to browse the groups; question marks remain normal text in inputs and shells.

| Action | Shortcut |
| --- | --- |
| Grouped keyboard shortcuts | ⌘? (⌘⇧/) |
| Search commands | ⌘⇧P |
| Appearance settings | ⌘, |
| Next / previous pane | Tab / ⇧Tab or ⌘⌥] / ⌘⌥[ |
| Split right / down | ⌘⌥→ / ⌘⌥↓ |
| Close active split | ⌘⌥W |
| Adjust nearest divider | Ctrl Alt ← / Ctrl Alt → |
| Select / extend selection | ↑↓ / ⇧↑↓ |
| Expand / collapse folder row | → / ← |
| Focus sidebar | ⌘⌥S |
| Sidebar move / expand / open | ↑↓ / →← / Return |
| Select all | ⌘A |
| Open selected folder or file | Return |
| Edit location | ⌘L |
| Choose folder for active pane | ⌘O |
| Back / forward / parent | ⌘[ / ⌘] / ⌘↑ |
| Refresh | ⌘R |
| New tab | ⌘T |
| New terminal tab | ⌘⌥T or ⇧-click + |
| Close tab | ⌘W (closes the split when it is the pane's last tab) |
| Next / previous tab | Ctrl Tab / Ctrl Shift Tab |
| Reorder current tab | ⌘⇧[ / ⌘⇧] |
| Toggle sidebar / hidden files | ⌘B / ⌘⇧. |
| Add / remove current favorite | ⌘D / ⌘⇧D |
| Create folder / rename | ⌘⇧N / F2 |
| Copy / move to recently focused other pane | F5 / F6 |
| Trash local selection / review permanent remote deletion | ⌘Backspace |
| Manage connections | ⌘⇧C |
| Toggle transfer drawer | ⌘J |
| Confirm / dismiss operation review | Return / Esc |
| Quit | ⌘Q |

Conflict decisions and cancellation are also searchable. Replacement requires
review, supports regular files only, and retains a journaled backup. Symlinks
are copied as links. Move verifies the copied tree before removing sources.

Split, close, and focus actions also appear in the Pane menu and command palette.
Closing a split focuses an adjacent surviving pane; each original side retains
at least one pane. F5/F6 targets the most recently focused different surviving
pane and shows the destination in operation review. Drag/drop targets the pane
under the pointer. Extra panes and divider geometry are session-only; restart
restores the two original sides using their first surviving pane's saved location.

`cargo run --locked --example verify_splits_ui` opens an isolated four-pane
acceptance window and checks real split, close, focus, state preservation,
cancellation, stale-drag rejection, transfer-destination commands, last-tab split
closing, and tree row/selection/stale-expansion handling. It does not load saved
preferences, connections, or credentials. Close it with ⌘Q.

## Checks

```sh
cargo check --locked
cargo fmt --check
cargo clippy --locked --all-targets
cargo run --locked --example verify_settings
cargo run --locked --example verify_appearance
cargo run --locked --example verify_local
cargo run --locked --example verify_transfers
cargo run --locked --example verify_credentials
cargo run --locked --example verify_locations
```

Fixture runners create their own temporary directories. Optional
`EXCAVATOR_OTHER_VOLUME=/path/to/disposable/mounted/volume` checks real
cross-volume transfers. `EXCAVATOR_CHECK_TRASH=1` sends one newly created fixture
file to system Trash and prints its recovery name.

## Current limits

- Remote copies support regular files and directory trees, with partial-tree
  journals and no symlink traversal. Remote moves and replacement are unavailable;
  unsupported actions produce explicit errors.
- FTPS requires verified TLS on control and data channels. Downloads, browsing,
  folder creation, and file/empty-folder deletion are supported; uploads and
  rename are disabled because FTP cannot guarantee no-clobber installation.
- S3 object keys starting with `/` or containing period-only path segments are
  rejected because the pinned library normalizes them. Other keys remain provider-specific.
- S3 folders are prefixes. There is no atomic rename or real folder creation.
  Deleting a current object may create a delete marker in a versioned bucket;
  older versions are not browsed or purged. Multipart uploads use conditional
  completion and abort on failure/cancellation.
- Drag selected items to the opposite pane to queue a copy. Drops never move or
  delete the source; name conflicts ask before replacing. Files and folders can
  also be dropped from Finder into either pane. Local files and folders can be
  dragged out to external apps on macOS; remote and mixed selections are not
  exported.
- Favorites, pane locations, hidden files, and sidebar visibility persist.
  Remote pane locations, extra splits, tab sets, history, and transfer journals are session-only.
- Partial copies can leave destination directories; the journal explains them.
  Replacement backups remain for manual recovery.
- Cancellation is cooperative; external changes can race OS calls.
- Expanded folders load on demand per pane. Expansions are session-only, are
  cleared when a pane navigates, and reload after ⌘R. Terminal glyphs resolve
  through installed Nerd Font families when present.
- VoiceOver speech, reduced-motion behavior, transient button states,
  multi-item Finder drops, outbound drag behavior, large-directory performance,
  clean-user installation, older macOS versions, and notarization remain outside
  the verified MVP.

`DECISIONS.md` records checks and rendered/OS evidence. `SPEC.md`, `CHECKLIST.md`,
`AGENTS.md`, `START.md`, and `ORCHESTRATION.md` remain product/workflow guidance.
CodeGraph is initialized locally with telemetry disabled.

## Connections

Use Connections (⌘⇧C) or the command palette to add, edit, test, remove, and open
connections. Empty credential fields while editing preserve existing secrets.
Passwords and S3 keys belong to macOS Keychain; metadata is stored separately in
`~/Library/Application Support/Excavator/connections.json`. Missing or locked
credentials are reported with an edit/recovery action. There is no plaintext
credential fallback.

SFTP can authenticate with a password or an OpenSSH-compatible private-key file.
The key remains at its local path; an optional key passphrase is stored in Keychain.
When a key is configured, a password credential is not required.

SFTP verifies an SHA256 host-key fingerprint before authentication. Compare the
first fingerprint through a trusted channel. Changed keys are rejected; forgetting
an old trust record and accepting a replacement are separate explicit actions.
FTPS uses explicit TLS and certificate/hostname verification with no plain FTP
or downgrade. An optional custom CA PEM file adds a trusted root for that connection.
S3 requires HTTPS except for a loopback fixture endpoint, and uses explicitly
entered credentials rather than ambient credential discovery.

Disposable protocol checks use `scripts/remote-fixtures.py` and
`scripts/remote-faults.py` with a separate Python virtual environment. These
loopback-only servers and public dummy credentials are for fixture use, never
production. See `DECISIONS.md` for exact evidence and remaining verification gaps.

To run the remote fixtures from the repository root:

```sh
python3 -m venv .remote-fixture-venv
.remote-fixture-venv/bin/pip install paramiko pyftpdlib pyopenssl 'moto[server]'
.remote-fixture-venv/bin/python scripts/remote-fixtures.py --root .remote-fixture-data
# In a second terminal:
.remote-fixture-venv/bin/python scripts/remote-faults.py
# In a third terminal:
cargo run --locked --example verify_remote
cargo build --locked --example verify_remote_ui
HOME="$PWD/.remote-fixture-data/ui-home" ./target/debug/examples/verify_remote_ui
```

Wait for the fixture server's readiness message before running checks. The UI
launcher refuses a normal HOME and uses injected public dummy credentials. Stop
the servers with Ctrl-C. The ignored fixture directories contain disposable
server keys, certificates, and data. Real AWS and external servers are unverified.
Physical Keychain save/readback/cleanup was exercised in the signed app with
disposable SFTP and S3 credentials. Ordinary connection errors surface in the
app, with no plaintext credential fallback.

## Vim mode

Enable **Settings → Interaction → Vim mode** (`Cmd+,`). It defaults off and
applies only while a file listing has focus. Inputs, terminals and dialogs keep
their usual keys. The command palette also offers **Toggle Vim mode**.

| Keys | Action |
|---|---|
| `j` / `k`, `gg` / `G` | Move down/up, first/last; counts such as `5j` and `5G` |
| `h` / `l` / Enter | Parent folder / open selected item |
| `Ctrl+d` / `Ctrl+u` | Half-page down/up |
| `za` / `zo` / `zc` | Toggle/expand/collapse folder tree |
| `v`, Space, Esc | Range selection, toggle marked item, leave mode then clear selection |
| `/`, `n` / `N` | Search visible filenames, next/previous match |
| `Ctrl+o` / `Ctrl+i` | Folder history back/forward |
| `gt` / `gT` | Next/previous tab |
| `Ctrl+w` then `h/j/k/l` | Focus a pane left/down/up/right |
| `Ctrl+w` then `s/v/c` | Split below/right, close split |
| `yy` / `dd` | Review copy to another pane / move local items to Trash (remote: permanent deletion) |
| `:rename`, `:refresh`, `:help` | Rename dialog, refresh listing, shortcut help |
| `?` | Shortcut help |

Search and commands appear in the existing status bar; Enter applies them and
Esc cancels them. Space marks survive cursor movement. File-operation
confirmations and protected original panes retain their existing behavior.

## Appearance settings

Open Settings from the Excavator menu, the command palette, or ⌘,. The page
uses the supplied Appearance reference: labels and descriptions on the left,
controls on the right. Light and Dark keep a fixed appearance; System follows
macOS using independent light and dark selections. Search a theme name and
press Return to select it. Tab and Shift-Tab move between fields; all mode,
theme, density, and reset actions are also searchable in the command palette.
Valid changes preview immediately and save in the background. Esc returns to
the panes without changing their tabs, selection, or history.

Non-secret settings live in
`~/Library/Application Support/Excavator/preferences.json`. Version 2 adds:

```json
"appearance": {
  "mode": "dark",
  "light_theme": "paper",
  "dark_theme": "graphite",
  "font_family": ".SystemUIFont",
  "font_size": 13.0,
  "row_density": "compact"
}
```

These are the defaults. Supported modes are `light`, `dark`, and `system`;
light themes are `paper` and `frost`; dark themes are `graphite` and `midnight`;
densities are `compact`, `comfortable`, and `spacious`. Font size accepts finite
numbers from 10 through 20 pixels. Font family accepts a nonempty installed
family name (at most 128 characters, no control characters); `.SystemUIFont`
selects the system font. Unavailable families use the platform's font fallback.
Reset appearance restores these defaults and retains workspace preferences.

The other supported keys are `version`, `favorites`, `left`, `right`,
`show_hidden`, `sidebar_visible`, and `vim_mode` (boolean, default `false`). Native paths are written as byte arrays
to preserve filenames that are not valid UTF-8; legacy string paths remain
readable. Version 1 migrates in memory while retaining favorites, local pane
locations, and visibility flags; the next explicit change writes version 2.
Missing files use defaults. Malformed, unsupported-version, unknown-key, or
invalid-value files are preserved, with an error and usable session defaults;
repair or move the file and restart to re-enable saving. Manual edits take
effect at the next launch. Do not add passwords, keys, tokens, or remote URLs:
connection metadata and Keychain credentials have separate stores.

Saves stage a new file and atomically replace the original outside the UI
thread. A write failure is shown while the in-memory appearance remains usable.
`verify_settings` checks migration, fresh-process reload, invalid-file retention,
validation, and failures without touching live Application Support.
`verify_appearance` checks theme resolution and text contrast.

### Integrated terminal

Drop files from a file pane onto a terminal to insert their shell-quoted full
paths at the cursor. Multiple files become separate arguments, followed by a
space; the drop does not send Enter. Local terminals accept local files, and
SSH terminals accept files from the same SFTP connection. Finder drops also
insert paths into local terminals. Unsupported sources and filenames containing
control characters show a notice.

Terminals live in pane tabs. Cmd+Option+T creates a terminal tab in the active
pane; Ctrl+backtick switches between its file and terminal tabs without stopping
the shell. Cmd+Option+J focuses a terminal and Cmd+Option+F returns to files.
Cmd+Option+Shift+Right/Down opens a fresh terminal in a new split. Ordinary pane
split commands also create a fresh shell when the active tab is a terminal.
All actions are in the Terminal menu and command palette.

Each session starts in its pane's local folder, or uses system SSH for an SFTP
location. Splitting starts at the original launch location. Browsing elsewhere
never changes an existing shell's directory. Closing a terminal tab or using
Cmd+Option+K ends only that session; closing a split ends its owned sessions.
SSH handles its own authentication and `known_hosts`; Excavator's provider
password and trust store are not reused. FTPS and S3 have no shell.

ANSI screen rendering, resize, shell control keys, Unicode input, clipboard
paste and scrollback are supported. Drag across output to select text and press
Cmd+C to copy; Ctrl+C still interrupts the shell. Typing, scrolling, or resizing
clears selection. Selected output stays stable while copying. Copying across
visual rows inserts newlines, including at wrapped lines. Mouse reporting
is not implemented. Sessions are not saved. File operations require file tabs.

Run real PTY checks with `cargo run --locked --example verify_terminal`.
`verify_terminal_ui` opens isolated production Workspace controls with independent
profile-free `/bin/sh` sessions in a disposable folder. Its command and screen
snapshot assertions verify session isolation and retained browser state. Native
rendered checks exercised typing, file/terminal switching, terminal split/close
and isolated session end. Live SSH and full clipboard-paste acceptance remain
unverified.

### Import ForkLift connections

Use **File > Import Connections from ForkLift…**, Cmd+Option+I, the command
palette, or **Manage Connections > Import from ForkLift**. Review/select entries
before importing. If macOS prevents access to ForkLift's group container, use
**Choose ForkLift database…** to select `Favorites.sqlite`, or copy it and its
adjacent `-wal`/`-shm` files using ForkLift into a private folder and select the
copy. Reads use a SQLite snapshot; originals are never modified.

This importer supports ForkLift 4's archived SFTP metadata. It adds connections
without replacing existing ones, skips duplicate endpoints and reports skipped
protocols. S3 entries need explicit bucket/region settings; plain FTP is never
converted to FTPS. Passwords, private keys and ForkLift host trust are not
imported. Edit missing credentials in Excavator and verify host fingerprints
when first connecting. Missing paths default to `/` and missing ports to 22.

`cargo run --locked --example verify_forklift` checks generated archives,
protocol skips, duplicates, malformed data and credential exclusion.

## Releases and updates

See [CHANGELOG.md](CHANGELOG.md) for user-facing changes. The Apple Silicon
prereleases are ad hoc signed and not notarized. Release builds include
Sparkle 2.10.0 for **Excavator → Check for Updates…** and the command palette.
Sparkle provides download, signature verification, installation and relaunch.
It asks about automatic update checks. Debug builds omit Sparkle unless
`EXCAVATOR_UPDATE_PUBLIC_KEY` is provided.

The public feed is `appcast.xml` on this repository's `main` branch; archives
are GitHub release assets. Archives are signed with the `excavator` Sparkle
Ed25519 key in the release maintainer's macOS Keychain. Only the public key is
committed. Preserve that Keychain key for future releases; do not export private
keys into the repository. No GitHub credential is bundled in the application.

To prepare the next release:

1. Bump the package version in Cargo.toml/Cargo.lock and update CHANGELOG.md.
2. Run `./scripts/package-release.sh`. It builds an optimized bundle, embeds
   Sparkle, creates the ZIP/checksum, signs it and updates `appcast.xml`.
   Run `./scripts/package-tui-release.sh` for the separate TUI tarball/checksum.
3. Review the changes and commit/tag the exact source used for the archive.
4. Upload the archive/checksum to a draft release with that version's tag.
5. Publish the release assets before pushing the updated appcast to `main`.
   Older appcast entries are preserved by copying `appcast.xml` into the
   packaging directory before generating the next feed.
6. Exercise update detection, installation and relaunch from the previous
   installed release. Build/signature checks alone do not verify this path.

The signing tool requires the original Keychain key. Sparkle downloads are
pinned by version and SHA256 in `scripts/fetch-sparkle.sh`. Framework licensing
and integration details: https://sparkle-project.org/ (MIT).

## CPU and memory

Click the CPU/RAM figures in the status bar to open live graphs. Figures refresh
every two seconds; history collects while the popup is open and pauses when it
closes. History stays in memory and is bounded to the most recent 120 samples.
CPU 100% means one fully used core; memory is resident RAM for Excavator itself,
excluding terminal subprocesses. No usage data is saved or sent anywhere.

### Single-pane macOS TUI

The optional Ratatui frontend has keyboard navigation, selection, saved SFTP/FTPS/S3
connections, and reviewed transfers between running browser instances. Provider,
Keychain and coordination work runs outside the terminal event loop.

```sh
cargo build --release --locked --no-default-features --features tui --bin excavator-tui
./target/release/excavator-tui /path/to/folder
```

In Herdr, `F5` copies to an Excavator browser or an idle local shell pane;
`F6` moves to an Excavator browser. A shell destination uses its detected working
directory, shown for review and rechecked before copying. Shell panes running a
foreground command (including SSH or an editor) are excluded.
`Shift+F5/F6` enters a destination in the current provider. Use `:` to find
**Copy left/right/up/down** or **Move left/right/up/down**. Directional discovery
requires the caller's visible Herdr tab. Copy can target a shell in that tab; Move
requires a browser.
Standalone terminals support the explicit browser picker and manual paths.

Dialogs use compact content-sized panels with highlighted destination rows and
a separate keyboard footer; long inputs and large lists scroll within the panel.
The Copy dialog combines the source review and destination selection. Choose
Home, Downloads, Desktop, a saved local favorite, a detected pane or a custom
local path. Arrow keys select cards, Tab switches to path editing, and Enter
confirms the copy. PgUp/PgDn scroll the source list. The current-provider path
card retains manual remote destinations; `Shift+F5` opens that path flow directly.
Copy/Move progress appears in a bottom gauge with completed item counts,
transferred bytes and final state. The gauge measures items, not byte percentage,
and hides in very small terminals to preserve browser controls.

`D` leaves remote browsing and returns to the last local folder. It cancels the
listing request while approved transfers continue independently. `B` opens saved
local favorites; choose with arrows or j/k, then Enter. `Enter` opens folders or
regular local files in their default macOS app. Remote files and symlinks remain
excluded. All three actions are also available in the command palette.

`A` saves the current local folder as a favorite. In the `B` picker, `d` or Delete
reviews removal; Enter confirms and Esc cancels. Removing a favorite removes
the shortcut. Favorites remain shared with the desktop app, and edits preserve
the other preferences.

`s` cycles Name, Size and Modified sorting; `S` reverses ascending/descending
order. The status shows ↑ or ↓, folders stay first, and the chosen direction
remains when switching fields. Expanded trees, cursor and selection survive.

In Connections (`c`), `g` opens group management: `n` adds, `e` or Enter renames,
and `d` reviews removal. Group renaming updates its members. Removing a group
moves its members to Ungrouped and preserves saved connections and credentials.
Assign a connection using the editor's Group field. Favorite add/remove, sort
direction and group management are also available in the command palette.

For isolated configuration, set `EXCAVATOR_CONFIG_DIR` to an absolute directory.
It overrides only the preferences and connection metadata directory; credentials
still use Keychain. Without it, both interfaces use the normal Application Support
directory.

The listing is an expandable tree. `Right` expands a folder; `Left` collapses it
or selects its parent. `Enter` opens a folder as the current location, and
`Backspace` navigates to the parent location. Each folder loads on demand.
Folder and file icons use [Nerd Fonts](https://www.nerdfonts.com/cheat-sheet);
choose a Nerd Font Mono in your terminal to render them. The Kind column is removed.

Use `F7` to create a folder, `F2` to rename one item, and `F8` to move local
items to Trash. Remote F8 requires a permanent-deletion review. Names stay within
the selected item's parent; folder creation uses the current location. Every
operation is reviewed before it is queued.

The TUI keyboard layer supports `j/k` and counts (`10j`), `gg/G` for first/last,
`5G` for row 5, `h/l` for the tree, and `v/V` for visual range selection.
`Esc` leaves visual mode, then clears selection on a second press. `yy` reviews
Copy and `dd` reviews Trash/local or permanent deletion/remote; counts such as
`3yy` select three rows when there is no explicit selection. `R` reviews rename.
Use `Ctrl+D/U` for half-page movement, `Ctrl+O/I` for history, and `za/zo/zc`
for toggle/expand/collapse tree. Herdr owns terminal panes and tabs.

Use `/` or `f` to find a filename in visible tree rows, then `n`/`N` for
next/previous matches. Search is literal and case-insensitive. `Ctrl+L` edits
the current path. NORMAL/VISUAL mode and pending sequences appear in the status
line. Text inputs and operation dialogs retain their normal typing controls.

Use `c` for connections, `L` for the log, `X` to cancel a transfer, `?` for help,
`Ctrl+L` to edit the current path, and `q` to quit. The log opens only on request.
Connection secrets stay in macOS Keychain. Host-key changes require explicit review.
Jobs belong to the source process; exiting cancels active and waiting jobs.
Up to 16 jobs can be active or waiting; they execute in order. `L` shows their
operation and unique job number. In Log, `x` cancels the active job and `X` cancels
waiting jobs. Outside Log, `X` cancels the active job.

FTPS browsing/downloads are supported. Uploads retain the shared provider's safe
rejection when exclusive destination creation cannot be guaranteed. See
[TUI-MACOS.md](TUI-MACOS.md) for evidence and remaining acceptance limits.
