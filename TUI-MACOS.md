# Single-pane macOS TUI — implementation notes

## Scope and build

The shared library exports domain, providers, transfers, connections, credentials,
persistence and platform modules. The GUI imports those same modules. Appearance,
ForkLift import, the integrated PTY view, GPUI views and updater remain in the GUI
binary. Provider and transfer implementations are unchanged. Shared connection metadata now uses cross-process locking and reviewed mutation checks. Keychain keeps its
existing service and connection IDs.

The default `gui` feature and `default-run = "excavator"` preserve existing build
and launch commands. GPUI Kit remains pinned at 0.7.0 and optional. GUI examples
require `gui`. The optional `tui` feature uses Ratatui 0.30.2 and Crossterm 0.29.0
(MIT), plus signal-hook 0.3.18 (MIT/Apache-2.0). Ratatui's explicit Crossterm 0.29
backend matches the directly pinned event API. The installed compiler is rustc
1.96.0; Ratatui requires Rust 1.88 or newer. No asynchronous runtime was added.
Dependencies remain locked; TUI builds contain no GPUI dependency.

```sh
cargo run --locked --no-default-features --features tui --bin excavator-tui -- /path/to/folder
cargo run --locked --bin excavator
```

The current entrypoint retains a byte-safe absolute path in shared `Location`.
Directory validation and listing run on the browser worker; failures appear in
the terminal with retry and parent navigation. q exits. Ctrl+C cancels a loading
request, closes an overlay, or exits from a loaded/error screen. Resize redraws
the screen; idle polling does not redraw it. An RAII guard covers
normal errors and partial initialization, and Ratatui installs a restoration panic
hook. SIGINT/SIGTERM request orderly exit. A noninteractive terminal is rejected.
The local browser described below is implemented in milestone 2.

## herdr adapter contract

Installed CLI: `herdr 0.9.1`; caller reports `HERDR_ENV=1`.
References: [official CLI](https://herdr.dev/docs/cli-reference/),
[official source](https://github.com/motionharvest/herdr).
The installed CLI's bundled schema was inspected with
`herdr api schema --output /private/tmp/excavator-herdr-schema.json`.

Verified read-only commands and result fields:

- `herdr pane current --current`: `result.pane` has `workspace_id`, `tab_id`,
  `pane_id`, `terminal_id`. It resolves correctly from a freshly created pane.
- `herdr workspace list`: `result.workspaces` contains `workspace_id`,
  `active_tab_id`, `focused`; never substitute focused workspace for the caller.
- `herdr pane list --workspace ID`: `result.panes` contains live pane identity,
  including `terminal_id` and `tab_id`. Foreground cwd is not a transfer target.
- `herdr pane layout --pane ID`: `result.layout` contains `workspace_id`,
  `tab_id`, `area`, `panes`, `splits`, `zoomed`, `focused_pane_id`.
  Each pane has `pane_id` and `rect` with integer `x`, `y`, `width`, `height`.
- `herdr pane neighbor --pane ID --direction right`: `result.neighbor` has
  `pane_id`, `neighbor_pane_id`, `direction`, and `layout`. This exposes herdr's
  immediate neighbor; it does not determine which registered browsers qualify.

The coordinator inherited stale launch IDs: `--current` and workspace queries
returned `pane_not_found` / `workspace_not_found`. Explicit live IDs worked.
Fresh pane `--current` resolved correctly. A within-workspace relocation was later exercised: the previous review was
rejected and the moved pane could be closed through its original alias. Other
relocation variants remain unverified. A failed caller lookup must disable automatic
routing with a visible error, never fall back to the user's focused pane.

The app-owned adapter returns a freshly resolved identity
(workspace/tab/pane/terminal IDs) and optional rectangles for that tab. Retain
terminal ID as the session-local reconciliation key; resolve its live pane/scope
before each review and confirmation, failing closed on ambiguity or absence.
Only registered, responsive Excavator instances qualify. Outside herdr return
unavailable and retain manual destination entry. A shell neighbor is ineligible.
Directional ranking must use the visible scope, direction and orthogonal overlap;
multiple eligible candidates require a picker, with no opposite-side wrapping.
The CLI has geometry, so explicit-picker-only fallback is unnecessary on this
version. Discovery/coordination code is reserved for milestone 3.

## Acceptance evidence — 2026-10-07

- `cargo build --locked --no-default-features --features tui --bin excavator-tui`: passed.
- `cargo build --locked --bin excavator`: passed.
- `cargo check --locked --all-targets`: passed.
- `cargo check --locked --all-targets --no-default-features --features tui`: passed.
- `cargo clippy --locked --no-default-features --features tui --bin excavator-tui`:
  passed with 11 pre-existing shared-core warnings; no TUI warnings.
- Existing `verify_local`, `verify_locations`, `verify_credentials` and
  `verify_transfers`, each via `cargo run --locked --no-default-features --example NAME`:
  passed. Credential checks cover metadata/redaction/trust; they do not establish
  a new live Keychain UI acceptance result.
- Disposable macOS PTYs exercised q, Esc, Ctrl+C, SIGINT, SIGTERM and a 12-column /
  4-row resize. Each exited with code 0, restored the exact initial termios flags,
  left the alternate screen and restored the cursor. Non-TTY and nonexistent
  directory startup exited with code 1 before terminal initialization.
  The throwaway driver is `/private/tmp/excavator-tui-lifecycle.py`, not a new
  repository test suite. An initial driver raced event-source initialization;
  the corrected driver waits for setup before sending resize/exit input.
- In an owned real herdr pane, screen output was inspected and q / Ctrl+C exits
  returned to the shell with identical `stty -g` values and exit code 0.
  Fresh caller identity resolved correctly. The owned pane was closed afterwards.
  No user pane was closed or repurposed.
- Existing `verify_remote` passed against freshly generated loopback SFTP/FTPS/S3
  fixtures and fault proxy. It exercised listing/download/upload, conflicts,
  host-key review, invalid authentication, TLS rejection, pagination, retries,
  cancellation and partial-transfer reporting. Run command:
  `target/debug/examples/verify_remote` from the fresh fixture working directory
  `/private/tmp/excavator-tui-fixtures-z65iu6rk`. The initial attempt had no running
  servers; the next attempt used an expired (Oct 3) old fixture certificate.
  Fresh fixtures resolved those prerequisites. Servers and their owned data were
  removed after verification. Existing fixture data was preserved.
- Panic-hook restoration is established by the pinned implementation, not an
  injected-panic exercise. Forced termination (SIGKILL) cannot run cleanup.

## Current status

The TUI now implements Herdr coordination, source-owned reviewed transfers and
saved/editable remote connections. The evidence below records exercised paths;
it does not imply every platform or provider scenario passed.

## Milestone 2 — standalone local browser (2026-10-07)

`src/tui/browser.rs` owns per-instance location, sort, history, cursor, selection,
loading/loaded/error/cancelled state. One worker drives the existing local provider
future and sorts results. Requests and responses each have one coalesced mailbox
slot. New requests cancel superseded work and increase the request generation;
responses from older generations cannot replace current state. Selection and
cursor restoration use real `Location` values. A worker-built membership index
keeps refresh reconciliation from becoming quadratic after Select all.

The event/render loop performs no provider/filesystem operations. It drains at
most one result, responds to input, and renders only visible rows. Name, kind,
size and UTC modified time are displayed; narrow terminals omit metadata columns.
Provider cancellation is cooperative between syscalls. Sorting is checked before
and after cancellation; a blocking OS syscall cannot be forcibly interrupted.
Exit cancels outstanding work without joining a potentially blocked worker.

Display escapes invalid bytes, control characters and backslashes while retaining
original paths for I/O. Enter navigates directories only. Files and symbolic links
are not automatically opened or followed; explicitly entered directory paths use
the existing provider's path semantics. Path entry accepts absolute UTF-8 paths,
`~` and `~/…`. A non-UTF-8 current path is not lossily prefilled: path entry starts
empty with a notice, and row navigation keeps the original byte path.

Keyboard controls:

| Action | Keys |
| --- | --- |
| Move / page / first / last | ↑↓ or j/k; PageUp/PageDown; Home/End or g/G |
| Open directory / parent | Enter or →; Backspace or ← |
| History | Alt+Left / Alt+Right |
| Toggle / range / all / clear selection | Space; Shift+↑↓; Ctrl+A; Esc |
| Refresh / hidden / cycle sort | r; .; s |
| Edit path | /; Ctrl+U clears; Enter accepts; Esc cancels |
| Search commands | : or Ctrl+P; ↑↓ select; Enter runs; Esc cancels |
| Shortcut help | ?; arrows scroll vertically/horizontally; Esc or q closes |
| Cancel listing / exit | Ctrl+C during loading; q exits |

Path and palette input remain separate from browser commands. Bracketed paste is
enabled and disabled with terminal cleanup. Paste containing control characters
is rejected as a whole, so pasted text cannot silently select a different path.
Inputs are bounded to 8192 UTF-8 bytes. Unsupported control/super combinations
are ignored; repeated Ctrl+C cannot cancel a load and immediately exit.

### Checks and live acceptance

- TUI build, TUI all-target check, default GUI all-target check, and TUI clippy
  pass with locked offline dependencies. Clippy retains 11 pre-existing shared
  core warnings and reports no warnings in the new TUI code.
- `git diff --check` and targeted rustfmt checks pass.
- Actual macOS PTYs exercised directory Enter/parent/history, toggled/range/all/
  cleared selection, sorting, Unicode names, hidden files, refresh after an
  external fixture change, path editing, help, searchable commands and execution.
- Empty directories, permission-denied and missing-path errors were rendered.
  A 6-row/24-column resize and a 20,000-file directory were exercised.
- Loading cancellation was observed and showed “Listing cancelled. Press r to
  retry.” Rapid large→empty navigation retained the empty result after 1.3 seconds.
- Exit restored original termios flags, alternate screen and cursor. The disposable
  driver and captures are `/private/tmp/excavator-tui-browser-check.py` and
  `/private/tmp/excavator-tui-browser-captures.json`; no repository test suite was added.
- Final targeted PTY checks passed: Ctrl+Q/Ctrl+J do not trigger browser actions,
  control-character bracketed paste is visibly rejected, Unicode path paste is
  accepted, and help scroll at 6×24 reaches Quit. Exit restored terminal state.
  Captures: `/private/tmp/excavator-tui-input-captures.json`.
- Native non-UTF-8 file/directory navigation could not be exercised: this macOS
  volume rejects creation of those names with errno 92 (Illegal byte sequence).
  Byte-safe representations and display are implemented; native acceptance remains
  pending on a filesystem that supports such names.

The local browser implementation is ready for review. Cross-instance destination
selection, transfers, remote browsing, and connection editing remain later milestones.

## Integration and delivery evidence — 2026-10-07

- Real Herdr sibling browsers: directional copy review, byte comparison and peer
  refresh passed. Navigating the destination after review rejected confirmation;
  no file was created at the unreviewed location. Resize preserved the reviewed
  target; zoom rejected confirmation; a shell neighbor was never selected.
  Moving the destination to a new tab rejected the old review; closing that
  browser removed it from directional discovery.
  Directional eligibility is rechecked before enqueue, including after credential
  loading. Test tabs were closed and the
  original tab restored.
- Native macOS PTYs: manual copy/move, explicit overwrite review, Keep Both,
  review/conflict cancellation and terminal restoration passed. The shared
  engine intentionally retains replaced-file backups; the Log shows journal
  descriptions and backup locations.
- Saved connections: disposable loopback SFTP, FTPS with a fresh CA, and S3
  listings passed. First SFTP host-key review and local↔SFTP transfers passed.
  SFTP→SFTP and SFTP→S3 passed; S3 content was compared with a prefix query.
  Changed keys rejected Enter; explicit reset and separate replacement review
  were required. Blocked SSH read dismissal stayed responsive and discarded the
  eventual error; reopening explained the pending read.
  FTPS upload returned the existing exclusive-creation capability error safely.
- Editor: complete field traversal, masked bracketed paste, validation, secret-free
  save review, 12×42 resizing/paging and cancellation passed. Blank secret fields
  preserve stored values; explicit clearing requires review.
- Separate processes concurrently patched fixture credentials: both changes and
  the original password survived. Stale metadata save/remove reviews were rejected.
  All metadata mutations serialize through a private `.connections.lock`; contention
  returns a visible error after 15 seconds. Trust reset checks the reviewed endpoint
  and fingerprint atomically.
- Three native exits removed their owned socket/registry files. Shutdown cancels
  active transfers and prevents late enqueue after the source frontend exits.
- GUI all-target checking and TUI Clippy passed; Clippy reports 11 existing shared
  core warnings. Disposable credentials/records were removed, original records
  were verified unchanged, and owned fixture servers were stopped. The release TUI was built without GPUI and exercised in a fresh PTY.

### Operation and limits

Build with the release command in README. In Herdr, split a shell pane and run
`excavator-tui /path/to/folder` in each pane. Ordinary F5/F6 uses an explicit picker
or a previously reviewed live target. Directional commands use current geometry;
failed identity lookup, hidden/zoomed scope or stale generations reject routing.
Confirmation rechecks target liveness, location and connection metadata. Once queued,
the destination is immutable. The source owns progress, conflicts and cancellation.

`c` opens the shared connection picker; `n/e/d` create, edit or review removal.
Editor Tab/Shift+Tab traverses fields, Ctrl+U clears the current input, and Ctrl+D
reviews clearing a secret field. Enter reviews before saving. Changed SSH keys
reject direct connection; `f` opens an explicit trust-reset review, followed by a
separate replacement-key review. No credentials travel through pane IPC.

Known acceptance gaps: native non-UTF-8 filename navigation (the macOS fixture
filesystem rejected creation), cross-workspace pane relocation,
cross-volume moves, interrupted large remote operations and locked-Keychain prompts.
Network calls have shared-provider timeouts; dismissal discards late results but
cannot immediately interrupt an already blocked call. SIGKILL/crashes can leave
stale registry files; discovery requires a successful live handshake and never
uses their recorded location. Native screenshot inspection was blocked because
Computer Use denied Ghostty; PTY and Herdr text captures are interaction evidence.
No TUI GitHub release, signing or clean-user installation was performed.

### Expandable tree and icons

The TUI Name column now includes Nerd Font folder/file icons and an indented tree;
the Kind column is removed. Right expands a directory on demand or enters its
first visible child. Left collapses the directory or selects its visible parent;
at the root it stays put. Enter opens a directory as the current location, while
Backspace navigates to the parent location. Expand/collapse commands are available
in the command palette. Symbolic links are displayed as links and are not expanded.

Each child listing runs on the existing background worker. Loading, failure,
cancellation and empty folders are shown beside the folder. Refresh reloads expanded
branches; collapsed cached selections are cleared when their entries cannot be
revalidated. Collapsing alone retains selected children. Transfers include selected
cached children and omit descendants of selected directories, and wait for pending
listings before capturing sources.

Disposable native PTY checks covered nested expansion/collapse, stable collapsed
selection, refresh/hidden handling, Enter/Backspace navigation, narrow layouts,
terminal restoration, copying a selected collapsed child and avoiding duplicate
parent/child transfer sources. Glyph emission was checked; visual glyph rendering
requires a Nerd Font Mono configured in the terminal and remains unverified because
Computer Use previously denied Ghostty access. Desktop tree behavior is unchanged.

### Copy to an idle shell

F5 destination discovery and directional Copy commands now include idle local
shells in the caller's visible, nonzoomed Herdr tab. The picker labels Shell and
Browser targets. Review shows the shell's actual process working directory.
Confirmation rechecks shell PID, terminal/pane identity, scope and cwd before and
after provider preparation. A changed cwd rejects the old review. Foreground
programs, editors and SSH sessions are excluded. Move still requires a browser.
Excavator performs the copy itself; it sends no command or input to the shell.

Real Herdr disposable-fixture checks passed file-content comparison, cwd-change
rejection, discovering the new cwd, foreground sleep exclusion and Move exclusion.
TUI debug/release builds and combined GUI/TUI all-target checks passed. Existing
SFTP-to-local provider support is reused; this change's native fixture used local
sources and did not copy the user's selected remote files.

### File operations, filename search and job queue

F7 creates a folder in the current location; F2 renames one selected item in its
own parent, including expanded tree children. Inputs accept one name, reject
path separators, controls and dot components, then show a separate operation
review. F8 reviews system Trash for local items. For remote items it explicitly
reviews permanent deletion; the shared provider engine supports nonrecursive
removal of files and empty directories. S3 folder creation and FTPS/S3 native
rename remain unsupported and report that limit before review.

`f` opens literal, case-insensitive filename search in visible tree rows. Enter
finds a match; `n` and `N` wrap forward/backward. Searching moves the cursor and
preserves selected locations. Path editing now uses Ctrl+L; / also searches filenames. Operations and search
are also available from the command palette.

Up to 16 jobs can be active or waiting. Reviewed jobs retain independent provider
registries and fixed sources/destinations, and execute sequentially. Log shows
unique job numbers, queued/active/completed states and operation journals. In Log,
`x` cancels the active job and `X` cancels all waiting jobs. Outside Log, `X`
cancels the active job. Exiting cancels all jobs owned by the process.

Disposable native PTY checks passed mkdir review/cancel and name validation,
local rename/content preservation, expanded-child rename parent correctness,
native Trash review/cancel/completion and Log, literal filename search and wrap,
and two reviewed copies queued during execution with FIFO completion, independent
destinations, distinct Log IDs and content comparison. Two focused search-model
tests passed. New remote rename/deletion interactions were not exercised against
a live server; they reuse the shared provider engine. Terminal screenshots were
not captured; the existing Ghostty Computer Use limitation remains.

### Vim keyboard workflow

The file listing now uses NORMAL/VISUAL mode with pending counts and sequences
shown in the status line. j/k and arrows accept counts; gg/G move to first/last,
and 5G goes to row 5. v/V anchors a range by Location; movements extend it.
Esc leaves visual or cancels a pending sequence while preserving selection;
another Esc clears selection. Space still toggles an individual item.

h/l collapse/expand the tree; za toggles, zo expands and zc collapses without
moving when already in the requested state. Ctrl+D/U moves half the current
terminal page. Ctrl+O/I navigates history (Ctrl+I is also the terminal Tab byte).
Navigation and modal entry reset pending sequences and visual mode.

/ or f searches visible filenames; n/N repeats with optional counts. Path editing
is now Ctrl+L or Edit path in the command palette. yy reviews Copy; dd reviews
local Trash or permanent remote deletion. Counts select consecutive visible rows
when no explicit selection exists. R opens Rename. F2/F5/F6/F7/F8 remain available.
Text inputs, connection dialogs and operation reviews keep their ordinary keys.
Herdr continues to own pane focus, splits and terminal tabs.

Native disposable PTY checks passed counted movement, gg/countG, visual selection,
Esc behavior, / search and n/N, Ctrl+L path input, counted copy review and byte
comparison, dd review/cancel, R rename input and terminal restoration. Combined
GUI/TUI checks and debug/release builds passed. No personal files were operated on.

### Shared modal redesign (2026-10-08)

All TUI dialogs now use a shared, terminal-palette-aware frame: a dimmed backdrop,
rounded border, accented title and separated keyboard-action footer. Heights fit
the content and clamp to the viewport. Destination and connection pickers use
selected two-line cards with the directory below the pane/connection identity;
selection remains visible when scrolling. Input fields show an actual cursor and
scroll long values horizontally. Permanent deletion, host trust/removal and
replacement reviews use danger accents. Path/search, commands, help, Log, operation
forms/reviews and connection editors use the same frame.

Inspiration: Ratatui's popup, input and table examples
(https://ratatui.rs/examples/apps/) and TachyonFX's composed effects examples
(https://github.com/ratatui/tachyonfx/tree/development/examples). The implementation
retains the pinned Ratatui dependency and event-driven redraw behavior.

Six renderer tests passed, covering destination viewport following, compact sizing,
long Unicode inputs, masked secrets, connection states, destructive reviews and
all browsing overlays. Generated buffer previews were inspected with light and
dark palettes. Native PTY checks passed search/path/palette/help/name/Log, input
visibility at 6×24, and terminal restoration. Fresh transfer interaction could
not be rechecked: the current sandbox rejected the Unix coordination socket with
“Operation not permitted.” Existing operation/credential logic remains unchanged.
Debug/release builds and combined GUI/TUI all-target checks passed.

### Combined Copy review (2026-10-08)

Ordinary and directional Copy now open one source-and-destination review with
provider direction, a scrolling source list and responsive destination cards.
Cards include detected panes, Home, Downloads, Desktop, saved local favorites,
Custom local path and Current provider path. Local favorites are read from shared
preferences on the worker and duplicate shortcuts are omitted. Custom local paths
remain local when the source is remote. Arrow keys choose cards, Tab switches
between cards and path editing, PgUp/PgDn scroll sources, Enter confirms and Esc
cancels. Sources and connection metadata are captured before confirmation;
existing peer identity, shell working-directory and metadata checks still run
before execution. Move retains its existing review flow.

All 11 TUI library tests passed, including confirmation/cancellation snapshots,
SFTP-to-local rendering, grid navigation and separate local/provider path drafts.
Combined GUI/TUI all-target checking and the release TUI build passed. Generated
terminal-buffer previews were inspected. Native copy execution remains unverified
in this sandbox because coordination socket startup is denied.

### Local navigation and file opening (2026-10-08)

`D` returns from a remote location to the latest local folder, cancels the remote
listing request and preserves navigation history. Listing providers are created
per request; this action does not cancel independent approved transfers. `B`
opens a read-only picker of shared local favorites, loaded on a worker with
loading, empty and failure states. Enter opens a chosen favorite. Favorites
management remains in the desktop app.

Enter opens folders or regular local files. File opening runs `/usr/bin/open`
with a separate argument on a worker, rechecks file kind, and reports completion
or typed failure without replacing the listing. Remote files and symbolic links
are excluded. The command palette and shortcut help expose all three actions.

Seventeen library tests passed, including return-to-local history, cancelled
favorites loading, native path bytes and asynchronous opening failures. Native
PTY checks passed favorites shortcut/palette, local command, folder opening and
terminal restoration. Favorites buffer previews were inspected. Launching a
default macOS app and disconnecting a live remote session were not exercised.

### Favorites, sort direction and connection groups (2026-10-08)

`A` saves the current local folder, including explicit directory symlink paths;
missing paths and regular files are rejected on the worker. Favorites (`B`) now
supports reviewed shortcut removal with `d`/Delete and Enter. The palette also
offers Add current folder and Remove favorite. Saves read the latest preferences,
preserve unrelated fields, retain native path bytes, refuse malformed files and
use an interprocess file lock plus atomic replacement.

`S` reverses sort order; `s` changes Name/Size/Modified while retaining direction.
Folders stay first in either order. Direction appears in status and has explicit
ascending/descending palette actions. Each expanded sibling list is sorted on
the worker, with refresh restoring the selected cursor by its actual location.

Connections (`c`) → `g`, or the Connection groups palette command, opens group
management. `n` adds; `e`/Enter renames; `d` reviews removal. Renaming updates member
records; removal leaves members Ungrouped and preserves records and credentials.
Membership remains editable through the connection editor's Group field.

The optional absolute `EXCAVATOR_CONFIG_DIR` isolates preferences and connection
metadata for fixture checks without replacing HOME or touching user settings.
Normal configuration paths and credential storage remain the defaults.

Thirty library tests passed. Native PTY checks passed favorite addition,
duplicates, navigation, cancellation, removal and restart; two-process favorite
saves; all three sort fields in both directions; group creation, renaming,
case-insensitive duplicate handling, cancellation, removal, connection-record
preservation and restart. Terminal restoration passed. Group/favorite renderer
previews were inspected; combined GUI/TUI checking passed. Fixtures used no real
credentials or remote connection attempts. ForkLift import was not added.

Desktop saves now merge current disk favorites and ordered explicit favorite
edits, so navigation in an already-open desktop window cannot replace TUI edits
with its stale cache. Shared persistence tests exercise stale snapshots, ordered
intents and corrupt-file preservation. GUI integration compiles; native desktop
interaction was not rechecked for this save-path change.

### 0.3.0 distribution (2026-10-08)

Published source tag `v0.3.0` at `0d7f563` and an Apple Silicon TUI tarball with
SHA256 checksum alongside the desktop ZIP. Both archive uploads were downloaded
and matched against the locally verified files. The packaged TUI passed native
management/restart/terminal-restoration checks. Release:
https://github.com/dorofey/excavator/releases/tag/v0.3.0.
