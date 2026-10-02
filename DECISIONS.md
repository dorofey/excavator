# Foundation decisions and evidence

## Milestone 0 — 2026-10-01

The starting directory contained six Markdown starter files and a ZIP, with no
source, Cargo manifest, Git repository, nested instructions, or existing checks.
The starter documents are preserved. No Git repository has been created.

- Initial target: macOS on Apple Silicon (`aarch64-apple-darwin`). This machine
  runs macOS 27.2; Rust and Cargo are 1.96.0. Command Line Tools are selected.
- Provisional minimum macOS target: 11.0 for Apple Silicon. Older-OS support is
  unverified; packaging must enforce and exercise the final deployment target.
- Dependency: exactly `gpui-kit = 0.7.0`, with its default components and assets.
  Its published source confirms `application`, `init`, `open_window`, and its
  GPUI facade. Cargo's resolved tree contains one GPUI family: `gpui-pre 0.3.7`.
  `Cargo.lock` preserves the resolved dependency set. No second UI kit is added.
- Async strategy: use GPUI foreground/background executors for future provider
  work. No separate runtime is added for this shell. Blocking filesystem work
  must run off the UI thread when browsing is implemented.
- Persistence strategy: future versioned, non-secret preferences in macOS
  Application Support, with atomic replacement and corrupt-file recovery.
  This milestone writes no user preferences and handles no credentials.
- Review boundary: one titled, centered window with a minimum size, initialized
  GPUI Kit root, startup failure reporting, and Command-Q. Two panes and file
  browsing belong to the following milestones.

## Checks

- `cargo check --offline`: passed.
- `cargo fmt --check`: passed.
- `cargo tree --offline -i gpui-pre`: confirms one compatible GPUI family.
- `cargo build --offline`: passed (53.95 seconds).
- `./target/debug/excavator`: started and remained running with no startup
  error output. Rendered inspection could not run: the native computer-use
  service returned `Sky Computer Use native pipe startup failed`. Window
  appearance and Command-Q therefore remain unverified. The app is left open
  for manual inspection.
- Cargo reports a future Rust compatibility warning in transitive dependency
  `block 0.1.6`; it does not fail the current check.

## CodeGraph

The CLI was absent from PATH. The requested initialization succeeded with
`npx --yes @colbymchenry/codegraph init -i`: one source file, five nodes, five
edges. Usage telemetry was then disabled using `codegraph telemetry off` via
the same npm runner. The index is local and excluded from version control.
CodeGraph MCP tools are not connected to this session; initialization alone
does not establish agent tool access.

API reference: <https://docs.rs/gpui-kit/0.7.0/gpui_kit/>.

## Local-files MVP — 2026-10-01

The user requested a goal to make the app work and authorized subagents. The
goal covers the local-files MVP, not remote providers or release readiness.
Bounded UI and provider/transfer agents owned separate source files; a read-only
reviewer checked state races, filesystem safety, and confirmation flows. The
coordinator owned manifests, entrypoint, preferences, packaging, integration
checks, and the checklist. Codex CLI 0.159.3 and session subagents were available.

### Implemented

- Dark compact shell, hideable favorites sidebar, Kit resizable panes, responsive
  metadata columns, active-pane focus, status and transfer drawer.
- App-owned local locations, native paths, typed errors and provider interface.
  Listings execute on GPUI background workers with cancellation, stable tab IDs,
  and request generations to reject stale results.
- Independent tabs/history, editable paths, sorting, keyboard/pointer selection,
  favorites, hidden files, and searchable commands with shortcut labels.
- Non-secret preferences use native Unix path bytes, serialized background saves,
  staging and atomic replacement. Corrupt/future-version files are preserved.
- A bounded single transfer worker handles create, rename, copy, verified move,
  system Trash, cancellation, batch conflicts and partial journals. Regular
  copies stage then install without clobbering. macOS replacement uses atomic
  exchange and retains a journaled backup. Directory/link replacement is rejected.
- Explicit default-app launch uses argument-safe macOS `open` in background.
- A repeatable ad-hoc signed development bundle in `dist/Excavator.app`.

### Verification

- `cargo check --locked`, `cargo build --locked`, `cargo fmt --check`: passed.
- `cargo clippy --locked --all-targets`: passed with unused provider API and minor
  style warnings; no clippy errors. Upstream `block 0.1.6` future warning remains.
- `cargo run --locked --example verify_local`: passed empty/hidden/Unicode files,
  metadata, a 5 GiB sparse file, dangling links, cancellation, missing paths,
  permission-denied handling, native-path preferences, and corrupt/future settings
  preservation. Raw-byte filename creation was unavailable on this filesystem;
  path serialization retains bytes.
- `cargo run --locked --example verify_transfers`: passed recursive copy/move,
  links, byte verification, Ask/KeepBoth/Skip/Replace, backup content, aliases,
  descendant guards, create/rename, invalid names, pre/live cancellation, partial
  failures, and concurrent enqueue/drain checks.
- A disposable 64 MiB APFS image exercised actual distinct-device recursive copy
  and verified move, including dangling links. The optional mounted-volume
  fixture passed; the image was detached afterward.
- The optional `EXCAVATOR_CHECK_TRASH=1` fixture passed actual macOS Trash,
  source removal, and journal checks. Disposable fixture files remain recoverable
  in system Trash. An initial assertion was corrected for `/var` versus
  `/private/var` canonical spelling; it was a fixture error, not a Trash failure.
- Native computer use became available. Rendered layouts were inspected at
  approximately 805 px, 1712 px, and full-screen width. Exercised pane switching,
  searchable commands, hidden files, folder navigation, back history, new/switch
  tabs, editable paths, missing-path errors/recovery, keyboard create/rename/move
  confirmations, copy and KeepBoth resolution, queue completion and journals.
  Copied fixture bytes were compared. UI checks used temporary directories and
  isolated preferences.
- Bundle plist lint and strict codesign verification passed. Mach-O inspection
  confirms Apple Silicon, minimum macOS 11.0, and SDK 27.0. The signed bundle
  actually launched on this host.

### Review corrections and limits

Review corrected startup races, inactive-tab refresh, keyboard scrolling, native
filename preservation, modal/palette focus, keyboard confirmation/conflict
actions, worker wakeup, case aliases, per-source removal journals, replacement
backup visibility, and narrow-column readability.

This is a local MVP. Journals and jobs are session-only; termination during work
does not provide crash recovery. Directory copies can leave journaled partial
destinations. Replacement backups remain for manual recovery. File permissions
are preserved where supported; full metadata preservation is not promised.
Cancellation cannot interrupt every syscall, and hostile external mutation is
not fully eliminated. Actual divider dragging, default-app content rendering,
VoiceOver, full accessibility, large-directory performance, multiple-item and
outbound drag/drop, native choosers, sandbox bookmarks, older OS versions,
clean-user installation, and notarization remain unverified or unimplemented as
indicated in the checklist.

At that point the next unchecked milestone was macOS interaction and visual
polish (milestone 5); its follow-up evidence is recorded below.

Packaging now replaces the executable inode atomically, preserving any running
app while rebuilding. A blank screenshot observed after a rebuild did not recur
in a fresh final bundle. Pointer-divider automation was inconclusive: the pinned
toolkit expects multiple held movements with a repaint, while the native tool
offers a single from/to drag. Keyboard divider adjustment is available through
Ctrl-Alt-Left/Right and searchable commands; no toolkit workaround was added.
The final rendered build passed keyboard divider resizing with responsive column
changes, Shift-arrow range selection, and keyboard favorite removal/addition.
The development build is ready for normal local browsing on this host.
Final first-run verification (no preferences file) found a fast startup race.
Bootstrap now uses GPUI's deferred window callback after publication, and initial
panes show Loading. A fresh normal-HOME launch displayed 40 home entries and 17
root entries, confirming the corrected startup path. The app is left open with
normal local locations.

## Milestones 6–7: remote connections (2026-10-01)

The user explicitly prioritized 6 and 7 ahead of the remaining macOS polish and
selected **FTPS required**. Plain FTP and TLS downgrade are unavailable.
Three bounded agents handled provider/transfer implementation, connection UI,
and credential persistence/security review. The coordinator integrated domain,
dependencies, fixtures, packaging, rendered verification, and evidence.

### Dependency and runtime decision

Pinned `ssh2 0.9.6` (MIT/Apache-2.0, libssh2), `suppaftp 12.1.1`
(MIT/Apache-2.0, native-tls), `rust-s3 0.37.2` (MIT, sync-native-tls),
`security-framework 3.7.0` (MIT/Apache-2.0), `url 2.5.8`, `quick-xml 0.38.4`,
and the existing resolved `chrono 0.4.45`. Registry versions and actual APIs
were inspected before integration. Alternative Russh/Russh-SFTP and official
AWS SDK options were researched. The chosen synchronous adapters run exclusively
on GPUI background workers, retaining the existing executor and one-worker queue.
No protocol operation is polled on the UI thread. Cancellation is cooperative
between chunks and requests; network operations have 15-second timeouts. DNS
resolution and some library calls cannot be interrupted immediately, and a
connection may try multiple resolved addresses.

Primary references: [SSH API](https://docs.rs/ssh2/0.9.6/ssh2/),
[rename flags](https://libssh2.org/libssh2_sftp_rename_ex.html),
[SuppaFTP](https://docs.rs/suppaftp/12.1.1/suppaftp/),
[rust-s3](https://docs.rs/rust-s3/0.37.2/s3/),
[Keychain bindings](https://docs.rs/security-framework/3.7.0/security_framework/),
[S3 key semantics](https://docs.aws.amazon.com/AmazonS3/latest/userguide/object-keys.html),
[multipart completion](https://docs.aws.amazon.com/AmazonS3/latest/API/API_CompleteMultipartUpload.html).

### Implemented behavior

- Provider-qualified locations and IDs keep local paths, remote filesystem paths,
  and S3 object/prefix values separate. Both panes retain independent navigation.
- Connections provide add/edit/remove/test/connect commands, masked credential
  fields, optional FTPS CA file, sidebar shortcuts, and keyboard field navigation.
  Secrets submitted to Test are cleared; re-enter them before Save if needed.
- Non-secret records and host trust live in `connections.json`. Credentials use
  direct macOS Keychain APIs under a derived account ID, with no plaintext fallback.
  Corrupt/future metadata is preserved. Metadata write failures attempt credential
  compensation and report recovery failures without secret contents.
- SSH SHA256 fingerprints are checked before password authentication. Unknown
  hosts require explicit review; changed keys are rejected. Accepting trust
  re-probes the fingerprint. Forgetting old trust and trusting a new key are
  separate actions.
- FTPS verifies certificate/hostname and protects the data channel before login.
  Passive data connections use the control peer's IP, never arbitrary PASV IPs.
- SFTP supports browsing, metadata, bounded reads, exclusive staged writes,
  no-clobber rename, folder creation, and confirmed nonrecursive deletion.
- FTPS supports browsing/metadata/download, folder creation, and confirmed
  regular-file/empty-folder deletion. Upload and rename are refused because
  STOR/RNTO cannot guarantee no-clobber installation.
- S3 lists paginated objects and prefixes, performs ETag-conditional ranged reads,
  and writes through bounded 8 MiB multipart parts or small conditional PUTs.
  Completion uses If-None-Match, positively checks XML success, and checks size.
  Abort failures retain upload IDs in recovery details. Safe read/list/part
  requests retry transient failures at most three attempts.
- S3 prefixes are labeled Prefix in the UI. There is no folder creation or
  atomic rename. Current-object deletion may create a versioned delete marker;
  older versions are not browsed or permanently purged.
- Remote copies support regular files and directory trees with a 256 KiB stream
  buffer and partial journals. Moves/replacement are unavailable. Symlinks and
  special files are rejected rather than deliberately traversed. Cross-connection
  directory copies of the same protocol are conservatively refused because
  different connection IDs may address the same backing tree.
- The pinned S3 library strips leading slashes and normalizes period-only path
  segments. Such keys are explicitly rejected before requests, preserving their
  original domain values and protecting neighboring objects.

### Verification and environment limits

`cargo check --locked`, build/package, formatting, and all-target clippy passed.
Clippy retains style/dead API warnings and the pre-existing upstream `block`
future incompatibility warning. Local, transfer, connection metadata, and location
fixture executables passed; local safeguards remain on their original executor.

`verify_remote` passed against isolated loopback Paramiko 5.0.0 SFTP,
pyftpdlib 2.2.0 FTPS with PyOpenSSL 26.4.0, and Moto 5.2.3 S3. Checks included:
all providers' list/stat/download; SFTP 9 MiB upload/readback, conflicts,
mkdir/rmdir/rename/delete; FTPS certificate rejection and mkdir/rmdir/delete;
S3 1005-object pagination, 9 MiB multipart roundtrip and 16 MiB conflicting
multipart preservation; unknown/changed host and bad-auth rejection; unsafe-key
rejection; transient retries, truncated reads, failed multipart parts, active
cancellation, partial journals, symlink refusal, and connection-alias guards.
Independent Boto3 readback confirmed versioning/delete markers, zero outstanding
multipart uploads, preserved original bytes, and at least three retry requests.

Native computer use verified the production connection editor, masked password,
first-contact fingerprint matching independent `ssh-keygen`, explicit trust,
connection Test success, metadata Save, and visible missing-credential recovery.
A separate explicit in-memory fixture registry (not a production fallback)
verified actual SFTP/FTPS/S3 pane listings, 1005 S3 rows, keyboard F5/Enter copies,
conflict Keep Both, completed jobs/progress/journals, and refreshed destination
panes. Copied local bytes were independently compared. Bundle plist and strict
signature verification passed; the final production bundle launches on this host.

**Physical credential acceptance — 2026-10-01:** the signed production app saved
a generated disposable SFTP password and a separate pair of fake S3 keys. The
app then removed each exact connection, confirmed that its stored Keychain
credentials were removed, and showed an empty saved-connections list. This
exercises Keychain save, retrieval as part of removal, and cleanup through the
production app for both provider credential shapes. The fake values were not
used against a real provider. An earlier CLI verifier returned OSStatus -60006;
a retry after app authorization hung in its synchronous Keychain call because
the CLI and app have different ad-hoc signing identities. We interrupted it,
verified its generated account was absent, and removed its exact temporary home.

Real AWS, external SSH/FTPS servers, private-key authentication,
production IAM/endpoint policies, older OS releases, release signing/notarization,
and full accessibility remain unverified. FTPS MLSD buffers inside the library
before the application listing cap. Hostile concurrent remote symlink replacement
can race lstat/open; the implementation does not claim race-proof no-follow.

The development bundle is `dist/Excavator.app`. Section 5 still has live
outbound/multi-item drag and accessibility review outstanding. Sections 6–7
have completed the app-level physical Keychain roundtrip; real AWS and external
provider interoperability remain outside the tested fixtures.

## Milestone 4a — Appearance settings and preferences

Implemented a versioned JSON preferences schema in the existing Application
Support file, preserving workspace values during version 1 migration. Defaults
apply when the file is missing. Invalid, unknown, or future settings are
reported, retained, and never replaced on load. Explicit saves validate the
schema and use a staged atomic replacement on the background executor. If a
save fails, the in-memory appearance remains active; the message clears after
a later successful save. Connection secrets remain outside this file.

Appearance settings follow the supplied CleanShot reference with a full-page,
scrolling two-column layout, fixed heading, separate searchable light/dark theme
selectors, system/light/dark modes, validated font family/size, density, and
reset. All file manager surfaces use semantic tokens. Opening the page sets
focus to its first field; Tab and Shift-Tab cycle the searchable/font fields;
mode, theme, density, and reset actions are also in the command palette.

**Checks passed:** `cargo check --locked`, `cargo fmt --check`,
`cargo clippy --locked --all-targets` (style/dead-code warnings remain),
`cargo run --locked --example verify_settings`, and
`cargo run --locked --example verify_appearance`. The settings fixtures checked
missing files, version 1 migration, byte-preserving paths, favorites/workspace
retention, version 2 roundtrip in a fresh process, invalid/future/unknown-key
preservation, validation, redacted diagnostics, and atomic-write failure with
no leftover staging files. Appearance fixtures checked fixed/System resolution,
all four palettes' contrast, and density heights.

A separate UI fixture, restricted to `.appearance-fixture-data/home`, rendered
the Settings page without opening real connections. It showed loaded choices
and persisted font/density after restart; Light and Dark; both System transitions
using app-scoped native appearance changes; font family and size 20; density;
command palette search and Return; and the preserved left/right fixture panes
when the page closed. A read-only fixture preferences directory visibly showed
a permission error while keeping the page usable and the saved JSON unchanged.
The app-scoped native appearance exercise did not change the macOS-wide
appearance preference. Real OS-wide preference toggling was not performed.
The production bundle was rebuilt; `plutil -lint` and strict deep signature
verification passed. The rebuilt app launched, Cmd+, and the native app-menu
Settings action opened the rendered page with the version 2 defaults. The app is
left open on that page for inspection.

## Native folder picker — 2026-10-01

The first macOS polish slice adds **Choose Folder for Active Pane…** to the
File menu, command palette, and `⌘O`. It uses the pinned GPUI native path
prompt with directory selection enabled and single selection. The action is
local-pane-only; it captures the invoking pane and tab across the asynchronous
native dialog, then navigates through the existing history/persistence path.
Cancellation leaves pane state untouched, and picker errors appear in the app
notice. No standalone open-file or save workflow currently needs a file dialog.

**Verification:** the signed `dist/Excavator.app` opened the native macOS
folder panel from the File menu and `⌘O`; the panel presented “Choose Folder”.
Cancel preserved both fixture pane locations. Go to Folder selection navigated
the left fixture pane and persisted its new path while retaining the right pane.
The command-palette entry also opened the panel. These checks used an isolated
`HOME` and disposable `.choose-folder-fixture` paths. Native rendering and
folder navigation were exercised on this host; sandbox security-scoped bookmark
behavior remains unevaluated and is recorded under the remaining access-control
checklist work. The isolated fixture app was closed and the normal app was
reopened with its existing left Downloads and right root locations.

## Pane-to-pane drag copy — 2026-10-01

Pane drag/drop had not been implemented: listing rows only handled selection
and opening, and panes had no drop targets. File rows now carry a payload with
the source pane/tab and either the dragged item or the full selection. The
opposite pane accepts that payload, highlights as a drop target, and submits a
copy plan to the existing bounded transfer queue with `ConflictPolicy::Ask`.
Dragging copies; it never removes source items. Stale source or destination tabs
are rejected. Starting a drag from an already selected row preserves a
multi-selection rather than collapsing it to one item. External-app drag/drop
is still unimplemented.

**Verification:** `cargo check --locked` passed. The rebuilt signed app was
exercised against isolated source/destination directories. A single-file drop
completed with verified queue status and bytes. Selecting both fixture files
and dragging one selected row copied both: the queue showed `2/2 items` and
`54 B copied`, and the destination contained both files. The isolated app was
closed and the fixture removed after byte verification; the production bundle
was then relaunched with the existing local pane locations.

## Finder-to-pane drops — 2026-10-01

GPUI's pinned macOS backend delivers inbound Finder paths as `ExternalPaths`.
The pane drop target now accepts that type alongside Excavator's internal
`PaneDrag` payload and converts each OS-provided path into a local copy source.
Copies enter the bounded transfer queue with `ConflictPolicy::Ask`; the target
pane receives a drag-over highlight and remains the destination. The raw GPUI
path API does not carry security-scoped bookmarks, so sandboxed operation still
needs separate design and verification. Outbound drag support is documented
below and is limited to local-only selections.

**Verification:** `cargo fmt --check`, `cargo check --locked`, the macOS app
build, plist lint, and strict bundle-signature verification passed. The pinned
GPUI source confirms its macOS backend creates `ExternalPaths` on drag entry and
dispatches typed drop listeners at the target. At implementation time, a live
Finder drag could not be visually exercised because the computer-use screenshot
surface returned a blank frame. The user later confirmed the behavior in the
rendered app, as recorded below.

**Follow-up:** The user subsequently confirmed that dragging from Finder into
Excavator works. This closes the general rendered-app verification; separate
coverage for multiple items and folders was not recorded. The explicit
Return/open-selection path also routes local files to the macOS default app
through `/usr/bin/open` on the background executor; remote files show a clear
copy-to-local notice instead.

## Outbound file drag — 2026-10-01

Listing rows now attach GPUI's external file payload resolver to the existing
drag gesture. The payload contains real local paths and directory flags already
available from the listing, so starting a drag performs no filesystem I/O.
Remote and mixed selections produce no external payload rather than exporting
only part of the selection. Internal pane-to-pane drag remains on the same
gesture and continues to use the copy queue.

**Verification:** `cargo fmt --check`, `cargo check --locked`, macOS bundle
build, plist lint, and strict bundle-signature verification passed. The pinned
GPUI/macOS implementation constructs native file URLs, but live drag-out to
Finder or another app has not been exercised; this remains open in CHECKLIST.md.

## Milestone 5 follow-up — 2026-10-01

The rendered production bundle showed the compact sidebar, tab, and transfer
queue controls with visible text or symbols. New-tab and close-tab buttons were
exercised, as were the sidebar toggle and transfer queue drawer. With one tab
remaining, invoking its close button left it open, confirming the disabled
behavior; transient hover/pressed and keyboard-focus button visuals remain
unverified. The mounted
volume route navigated to `/Volumes` and listed `Recovery` and `Macintosh HD`.
A mode-000 local directory produced a visible permission-denied message and
Retry action; the temporary fixture was removed and pane locations restored.

Existing inbound Finder drop evidence is the user's confirmation of a basic
drop. Multi-item/folder inbound drops and actual outbound drags to Finder or
another application were not exercised in this session. Outbound drag support
is implemented for all-local selections only; mixed or remote selections do
not offer a native file drag payload.

Custom toolbar actions and the command-palette opener now declare button roles,
keyboard focusability, and theme-based active/focus-visible styling; GPUI Kit
buttons provide their own active style and focus ring. The pinned GPUI source
also confirms Return/Space activation for focused click handlers and AccessKit
Click actions; this does not replace rendered verification. The rendered button
states still need macOS verification. The rendered accessibility tree exposes
named controls and the command UI visibly lists shortcuts, but that does not
verify spoken VoiceOver output. Reduced-motion behavior has not been verified
under the macOS setting. These checks remain open in CHECKLIST.md.
Packaging and strict signature verification do not substitute for those
accessibility or drag/drop interactions.

The follow-up Finder session created and cleaned disposable file/folder fixtures
but could not arrange Finder and Excavator side by side through the available
automation surface. No multi-item, folder, or outbound drag was completed in
that session. A repeat CUA pass found both apps running but could not bind
either window (`cgWindowNotFound`); no fixtures were created in that pass. An
earlier AppleScript attempt to read native window bounds stalled on System
Events accessibility control and was stopped. No permission or windowing
setting was changed.

## Zed-reference control refinement — 2026-10-01

The supplied 16:16 Zed screenshot establishes the requested interaction pattern:
tab-close icons reveal on hover, and panel toggles sit at the bottom right.
Excavator now reserves a 22-pixel close-button slot in each tab and reveals its
Close icon on tab-group hover or keyboard focus. The sole-tab disabled invariant
is unchanged. Sidebar and transfer-drawer controls use compact PanelLeft and
PanelBottom icon buttons at the far right of the status bar, with selected and
accessibility toggle state, labels, and shortcut tooltips. The job count remains
status text; the redundant top Hide/Show button was removed.

The first rendered pass exposed missing SVG assets. Startup now registers the
pinned GPUI Kit embedded AssetSource, restoring actual component icons without
adding assets or dependencies. The rebuilt signed bundle screenshot confirms
both bottom-right panel icons render, the top toggle is gone, and idle tab-close
icons are hidden. CUA input still fails with `noWindowsAvailable` even when a
window screenshot is available, so hover/focus reveal and the moved controls'
click behavior remain unverified. The corresponding checklist review is open.

`cargo fmt --check`, `cargo check --locked`, the macOS bundle build, and strict
code-signature verification passed. The same six existing warnings and upstream
`block 0.1.6` future-incompatibility notice remain.

## Section 9 — additional pane splits (2026-10-01)

The first enhancement adds recursive right/down splits while retaining two
original side groups. Stable, non-reused pane slots prevent closed panes from
being confused with later replacements. Each leaf owns tabs, history, selection,
sort, path input, focus, scrolling, and its input subscription. Closing cancels
its listing tokens, releases pane state, collapses the parent split, and focuses
an adjacent surviving pane; the final pane on either original side cannot close.
Async listings and folder-picker results, captured row/tab callbacks, and drag
handles check live pane and tab identities before applying changes. Every split
retains a size-change observer so metadata columns adapt during divider dragging.

Split Right/Down, Close Pane Split, and next/previous focus are registered in the
Pane menu, command palette, and keybindings. Focus uses Tab/Shift-Tab or
Cmd-Option-]/[; split uses Cmd-Option-Right/Down; close uses Cmd-Option-W.
Ctrl-Alt-Left/Right adjusts the nearest divider. Keyboard transfers target the
most recently focused other live pane, with a surviving alternative as fallback;
operation review shows its location. Completion/hidden-file refresh covers all
live panes. Native drag/drop retains explicit destination-pane semantics.
Legacy left/right location preferences map to the first surviving pane in each
original side; added panes and divider geometry are session-only.

The isolated real GPUI verifier checks production command paths: independent
history/selection/tab identity, nested split/close/focus, adjacent close focus,
root collapse, monotonically increasing IDs, cancellation of a closed pending
listing, stale-source drag rejection, and a Copy preview targeting pane ID 2.
It bypasses saved preferences/connections/credentials and disables persistence.
An independent review caught and corrected missing split-state observations and
stale row/new-tab callbacks before acceptance.

Rendered CUA verification exercised a nested four-pane window, adding a tab in
one nested pane, independent location entry plus back/forward navigation, both
split shortcuts, close-split, forward focus, and palette execution of Split
Right. Both side-by-side and above/below divider dragging were exercised; a
narrowed pane hid metadata columns appropriately. The final fixture also
rendered at 1100 pixels wide and accepted the revised pane-focus shortcuts.
Menu registrations were checked in source; every native menu item was not
individually clicked. CUA binding to the production bundle still returned
`timeoutReached`; rendered interaction checks used the isolated production
Workspace fixture. Extreme nesting at the 640-pixel minimum, physical remote
provider close during delayed I/O, and restart persistence of extra splits are
not claimed (the latter is intentionally unsupported).

`cargo fmt --check`, `cargo check --locked --all-targets`, the isolated verifier
assertions, the signed macOS bundle build, and strict signature verification
passed. Existing binary/example unused-code warnings and the upstream
`block 0.1.6` future-incompatibility notice remain. The production artifact is
`dist/Excavator.app`. Next section-9 milestone: integrated terminal; optional
Vim mode remains an evaluation item.

## Section 9: integrated terminal implementation (2026-10-01)

Added an app-owned PTY boundary in `src/terminal.rs`, a GPUI terminal screen in
`src/ui/terminal.rs`, and workspace launch/actions in
`src/ui/terminal_commands.rs`. The bottom-right terminal icon, Terminal menu,
and palette expose show/hide, focus, new session, end session, and return to the
active file pane. Hiding retains the shell; ending drops the session and stops
its shell/foreground process. Terminal and transfer drawers share one bottom
area. This first version has one session, retained only for the app session.

Local sessions use the user's absolute `$SHELL` (fallback `/bin/zsh`) as a login
shell with an explicit working directory. Browser navigation never injects
`cd` into a running shell. SFTP uses `/usr/bin/ssh` with separate arguments for
host, port and user and a single-quoted remote startup directory. System SSH
owns authentication and `known_hosts` prompts; provider Keychain passwords and
app-managed host trust are not reused. FTPS/S3 creation is explicitly unsupported;
an existing terminal can still be shown while either provider is active.
No terminal output or session state is persisted or logged by production code.

Pinned `portable-pty = 0.9.0` and `vt100 = 0.16.2`, both MIT licensed, plus
`libc = 0.2.189` for Unix process-group lifecycle checks. These add no GPUI
version or UI component dependency; the dependency tree still has
`gpui-pre 0.3.7` and GPUI Kit/base/component 0.7.0. References:
[portable-pty](https://docs.rs/portable-pty/0.9.0/portable_pty/),
[vt100](https://docs.rs/vt100/0.16.2/vt100/).
PTY setup, read/write, parsing, resize, signaling and reaping run on bounded
workers. UI input uses nonblocking bounded queues and screen snapshots.
Cancellation checks terminal-derived foreground process groups against session
identity and excludes the app's process group. A gate prevents signals after
natural-exit reaping. ANSI output includes colors/styles, cursor, wide cells,
alternate screen, application cursor mode, bracketed paste, bounded scrollback
and terminal replies. Clipboard paste removes control characters other than
newlines/tabs before bracketed wrapping. Terminal raw keys are excluded from
Workspace Tab/Escape/tab-cycle/divider bindings. Palette activation preserves
terminal focus; failed creation restores file-pane focus.

Verification:
- `cargo run --locked --example verify_terminal` passed against a real,
  profile-free `/bin/sh` PTY in a disposable folder: cwd, ANSI color cells,
  alternate screen/input modes, interactive input, `stty` resize, Ctrl-C,
  final output and exit status 7, and shutdown of a real foreground sleep job
  with process disappearance and shell reaping.
- `verify_terminal_ui` launched the production Workspace in an isolated window
  without loading personal preferences, connections or Keychain. Assertions
  exercised toggle/focus/session retention, active-provider semantics and
  unsupported S3 creation. Its asynchronous UI snapshot received parsed ANSI
  output and the real shell cwd. A fixture-only `/var` versus `/private/var`
  comparison was corrected by canonicalizing its folder before UI startup.
- `cargo fmt --check`, `cargo check --locked --all-targets`, the macOS bundle
  build and strict signature verification passed. Existing warnings and the
  upstream `block 0.1.6` future-compatibility notice remain.

Native CUA capture repeatedly returned `cgWindowNotFound` for this terminal
fixture and the previously working split fixture, including after a CUA reset.
Rendered layout, physical keyboard/paste/focus interactions, and live SSH shell
behavior are therefore not claimed. Terminal output selection/copy and mouse
reporting remain unsupported. The terminal checklist box remains unchecked
pending native acceptance; next section-9 work after acceptance is optional
Vim-mode evaluation.

## ForkLift import correction and pane-owned shells (2026-10-01)

The earlier checklist marked ForkLift import complete, but this checkout had no
parser or import UI. That entry was premature. The importer now exists in
`src/forklift.rs` and `src/ui/connections.rs`, with a single atomic, additive
metadata batch in `src/connections.rs`. File menu, palette, Cmd+Option+I and
connection-management controls open a selection preview. Duplicate endpoint
checks run both in preview and against the latest stored records at commit;
existing passwords and host trust are preserved. Unsupported/malformed entries
have explicit skipped reasons. SFTP ports default to 22 and paths to `/` when
ForkLift metadata omits them. This version supports ForkLift 4 SFTP records;
S3 requires explicit bucket/region setup, and plain FTP is never upgraded.

Actual ForkLift 4 data was inspected structurally: `Favorites.sqlite` stores
binary NSKeyedArchiver dictionaries in `ZFAVORITE.ZDATA`. UID references are
resolved as data with bounded depth/size; no archive classes are instantiated.
Only connection metadata fields are allowed through. Reads use a read-only
SQLite transaction. Pinned `plist = 1.10.1` and bundled `rusqlite = 0.40.2` are
MIT licensed and add no GPUI version. Primary API references:
[plist](https://docs.rs/plist/1.10.1/plist/),
[rusqlite](https://docs.rs/rusqlite/0.40.2/rusqlite/).
[Official ForkLift support](https://binarynights.com/support) identifies its
ForkLift 4 group container. Direct filesystem access to that container was
OS-denied here. Native ForkLift navigation/copy produced a private temporary
copy of its database plus WAL/SHM; its SQLite integrity check passed. No OS
protection was weakened and ForkLift's original database was not changed.
Its Desktop/Favorites view was restored after copying.

The actual preview contained 37 supported SFTP entries, no duplicates, 14
local/group/tag entries and 2 skipped S3 entries with incomplete configuration.
The user-authorized import added all 37 SFTP metadata records, verified each by
readback, and a second preview reported 37 duplicates with no new candidates.
Passwords/private keys were not copied and no production connection was opened.
Credentials must be supplied through the existing editor/Keychain flow; SSH
trust still requires independent verification. The temporary database copy was
removed after verification. The generated archive verifier passed defaults,
explicit ports, hostname-case duplicate handling, protocol skips, malformed
archives, duplicate keys and secret exclusion. Native import selection/picker/
confirmation remains a separate unchecked acceptance item.

At the user's request, terminals moved from the shared drawer into pane-owned
tabs. Each tab owns its view, PTY session, launch origin and subscription.
Switching to a file tab preserves the shell and browser navigation. New terminal
tabs and terminal-right/below splits create independent sessions. Splitting an
existing terminal starts at its original launch location; it does not clone a
process or infer a later shell `cd`. Closing a tab/split drops only its owned
sessions; End Session restores a file tab. File operations and drops reject
terminal tabs, and transfer targeting prefers other visible file panes. Terminal
launch directories are not persisted as browser preferences. Tab labels are
capped and the tabstrip scrolls horizontally for narrow/many-tab panes.

`verify_terminal_ui` passed real independent `/bin/sh` PTY assertions and UI
snapshot checks for separate output/cwd, retained browser history and session
identity across tab switches, and isolation of close/end/split ownership.
Standalone PTY lifecycle verification remains the proof of process cancellation
and reaping; command/snapshot assertions do not prove native physical input.
The latest all-target compilation, formatting, macOS bundle build and strict
signature check passed. Existing warnings and the upstream `block 0.1.6`
future-compatibility notice remain. Native rendered terminal and importer
interaction acceptance is still pending because GPUI fixture capture returned
`cgWindowNotFound`; ForkLift's own native copy/navigation was exercised.


Native terminal follow-up: capture resumed for the final signed acceptance
fixture. An initial window below the production minimum height exposed a wrapped
terminal header; the fixture now enforces the same 640×420 minimum, and headers
truncate to a single line. The rendered window showed two terminal panes nested
beside file panes. CUA typed a harmless printf command into one terminal and
observed its output while the other retained its independent output. Tab/Escape
stayed within the terminal, Cmd+Option+F switched to a file tab, and
Cmd+Option+J restored the same shell/output. Cmd+Option+K ended only that tab;
the other terminal remained Running. Cmd+Option+Shift+Right created a fresh
terminal split, and Cmd+Option+W closed it and restored the preceding shell pane.
A clipboard paste reported a CUA clipboard-conflict error; the input was
cancelled with Ctrl-C before execution, so full paste acceptance remains open.
This closes the integrated-terminal milestone's rendered keyboard requirement;
live SSH and the separate ForkLift preview/picker/confirmation checks remain
unverified. The next section-9 item is optional Vim-mode evaluation.

## Grouped keyboard help

Keyboard help uses the pinned GPUI Kit base Dialog, with a viewport-bounded
scrolling catalog grouped by general actions, navigation, panes, tabs, operations,
connections, terminals, and text fields. Shared action labels and shortcuts come
from the command palette; contextual keys and Quit are included separately.
Cmd+? toggles help from the workspace or modal; plain ? opens it only in a file
list, preserving terminal and input typing. Help menu and palette entries expose
the same action. The modal captures underlying app actions and saves/restores
the exact focus handle without closing settings, operation forms or the palette.
Cmd+L now uses the existing file-command guard to avoid focusing a hidden path
field from a terminal tab.

Source review covered all registered app bindings and modal action guards.
Locked offline check, formatting and builds passed. An isolated native PTY
fixture displayed the grouped modal; Cmd+? toggled it closed, Cmd+Option+K was
blocked while open, and a harmless printf command afterward proved that focus
returned to the same running shell. Final capture then returned stale frames
and a fresh fixture timed out, so lower-group scrolling, narrow-window layout,
text-field restoration and native Help-menu activation remain acceptance gaps.
The signed macOS bundle was rebuilt with the existing app icon.

## Section 10: Zed-style interaction refinements (2026-10-01)

User feedback on the rendered app produced one integrated UI pass. New files:
`src/ui/tree.rs` (shared expansion state), `src/ui/sidebar.rs`,
`src/ui/palette.rs`, `src/ui/file_icons.rs`.

### Implemented

- Closing the last tab of a pane now closes that pane's split, matching the
  user's protected-pane choice: the last pane on each original side keeps its
  tab and `⌘W` reports `Keep at least one pane on each side.` The close icon on a
  sole-tab pane is enabled only while that pane can actually close.
- Terminals resolve missing code points through installed Nerd Font families:
  `Font.fallbacks` is built once from `text_system().all_font_names()` filtered to
  families containing "Nerd Font", preferring symbols-only and monospaced variants
  (three entries). No font files or new dependencies were added.
- The sidebar is one vertically scrolling column inside a resizable
  `h_resizable` panel (200 px default, 120–480 px range) that replaces the fixed
  160 px column. Favorites, `Local disk`, `Mounted volumes` and saved connections
  are expandable folder trees: one cancellable provider `list` per expanded node,
  loading/failed/empty states inline, folder children only. It is keyboard
  operable: `⌘⌥S` focuses it, `↑`/`↓` move, `→` expands or steps in, `←` collapses
  or steps out, `Return` opens the node in the active pane, `Esc` returns to the
  panes. Rows carry `TreeItem` roles, names and `aria-selected`; the container is
  a focusable `Tree` with its own key context and an accent focus border.
- Shift-click on a pane's + button opens a terminal tab (`ClickEvent::modifiers`),
  documented in its tooltip next to `⌘⌥T`.
- Listings show file-type icons: a selected subset of bundled Lucide SVGs
  (`icon_assets!`, 21 extras) registered with the default component assets through
  `AppAssets`, so no full-catalog embed and no copied Zed assets. Mapping is by
  extension and well-known names, with separate light/dark colors.
- File listings are trees. Rows keep a depth and indent guides; folder rows carry
  a chevron (click or `→`/`←`) that loads children asynchronously through the
  provider registry with its own cancellation token, generation guard and stale
  rejection. Selection, anchor and cursor follow locations across rebuilds;
  navigating clears expansions; `⌘R` reloads expanded folders in tree order.
  Operations use the selected rows' real locations, collapse parent/child
  duplicates, and rename inside the item's own folder.
- The separate toolbar row is gone. The window now uses the pinned GPUI Kit
  `TitleBar` with `TitleBar::window_options()` (transparent title bar, traffic
  lights at 9/9, `app_owns_titlebar_drag`), holding back/forward/parent icon
  buttons and a search-command entry that swallows mouse-down so dragging still
  moves the window.
- The command palette is a floating modal: dimmed backdrop, centered card, search
  field with icon, bounded scrolling list, highlighted selection with `↑`/`↓`
  (captured above the input so single-line inputs do not consume them), `Return`
  runs, `Esc` or a backdrop click dismisses, and focus returns to the active pane.

### Checks

- `cargo fmt --check`, `cargo check --locked --all-targets`,
  `cargo clippy --locked --all-targets`: no errors; the pre-existing dead-code and
  style warnings and the upstream `block 0.1.6` notice remain.
- `verify_settings`, `verify_appearance`, `verify_locations`, `verify_forklift`,
  `verify_local`, `verify_transfers`, `verify_credentials`: passed.
- `verify_splits_ui` passed with added assertions for last-tab split closing, tree
  row/selection rebuild, and stale-expansion rejection.
- `verify_terminal_ui` passed again (independent PTYs, retained browser state).
- Bundle build, `plutil -lint` and strict deep signature verification passed.

### Rendered evidence (signed `dist/Excavator.app`, this host)

- Title bar with traffic lights, back/forward/parent icons and the command search
  entry; no Refresh row.
- Palette opened with `⌘⇧P` as a centered modal over a dimmed window; typing
  "split" filtered the list, and `Split pane right` executed from the palette.
- `→` expanded a folder in the Documents pane (36 rows, indented child with an
  indent guide, open-folder glyph); `←` collapsed it again (35 rows).
- A real Powerline prompt (segments, apple, clock) and explicit
  `print('\ue0b0\ue0b2\uf07c\uf013\uf1c9\uf418')` rendered as glyphs, not
  missing-glyph boxes.
- `⌘⌥T` opened a terminal tab in the active pane; `⌘W` closed that tab; `⌘W` on a
  pane with one tab removed the split (two panes remained); the next `⌘W` showed
  `Keep at least one pane on each side.`
- Sidebar: `⌘⌥S` focused the tree, `↓↓↓` moved to Mounted volumes, `→` expanded it
  and showed `Recovery` indented; favorites, locations and connections render with
  folder/server/cloud icons.

### Not verified

- Shift-click on + could not be exercised: the available pointer automation sends
  plain clicks without modifiers, and the branch is one modifier test.
- Sidebar divider dragging by pointer was not exercised; keyboard focus, expansion,
  scrolling and the palette are covered above.
- VoiceOver speech, reduced motion, large-directory performance, clean-user
  install and notarization remain unverified as recorded in `CHECKLIST.md`.

## Section 10 follow-up: plain title bar and one top tab row (2026-10-01)

User feedback: remove the back/forward/parent icons and the command search entry
from the title bar, and move the tabs to the very top. Refresh was removed
entirely; ⌘R and the palette command remain.

- `render_title_bar` now draws only the Kit `TitleBar` background and border, so
  the row holds nothing but the macOS window controls. Navigation actions are
  unchanged as keybindings, palette commands and menu items.
- Pane tab strips moved out of the pane bodies into one full-width row directly
  under the title bar. `Layout::geometry` walks the resizable tree once per frame
  and returns `(leaf, left offset, width)` for every pane, using the same
  `ResizableState` sizes the layout itself uses; each strip is absolutely
  positioned at that offset and width, so nested right/down splits stay aligned.
  The row is the width of the main area (window minus the sidebar divider) and
  keeps each pane's own background, active-tab accent, hover-revealed close icons,
  reserved close-button space, `+` button and ⇧-click terminal behavior.
  `render_pane_tabs` is the shared strip renderer used only by the row.
- `verify_splits_ui` asserts the geometry contract: strips cover every leaf in
  layout order, the first starts at the row edge, offsets never move backwards,
  the last strip stays inside the row with positive width, and no strip
  references a closed pane.

### Checks

- `cargo fmt --check`, `cargo check --locked --all-targets`,
  `cargo clippy --locked --all-targets`: no errors; only the pre-existing
  dead-code/style warnings and the upstream `block 0.1.6` notice.
- `verify_splits_ui` passed including the new top tab-row assertions.
- Bundle rebuild, `plutil -lint` and strict deep signature verification passed.

### Not verified

The rendered screenshot for this change could not be captured: `screencapture`
started returning all-black frames and rejected rectangle captures on this host
after the rebuild, and System Events reported zero windows for the process while
the binary itself stayed alive until killed. Layout alignment is therefore
covered by the geometry assertions above, not by a screenshot.
