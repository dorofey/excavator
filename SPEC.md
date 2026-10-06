# Product and architecture specification

## Product
A native-feeling desktop file manager for people who move and organize files across local folders and remote storage. Its core is a ForkLift-style two-pane workspace with Zed-like visual restraint, keyboard navigation, command discovery, and clear focus.

The initial target is macOS, while keeping provider and UI boundaries portable where GPUI supports them. Screenshots referenced in the planning conversation were not available to the starter-pack author; treat the written brief as the design source and add screenshots to the repo later if pixel-level review is wanted.

## Experience and layout
- Global compact top bar: navigation/back-forward, location or focused-pane path, search/command entry, and app-level actions.
- Persistent, hideable sidebar: Favorites, local volumes/locations, and named remote connections. Favorites are shortcuts, not a second file tree.
- Main workspace: two independently navigable panes with a draggable divider. Each pane has its own tabs, breadcrumb/path field, back/forward history, listing, sort, selection, and active-pane state. Reveal each tab's close icon on hover or keyboard focus, reserving its space to keep tab labels stable.
- Listing: dense table/list with name, kind, size, modified time; sortable columns, multi-select, inline loading/empty/error states, and virtualization when measured data size warrants it.
- Bottom status bar: active location, selected item count/size, provider state, and transfer summary. Compact icon buttons at the bottom right toggle the sidebar and transfer drawer, with active-state cues and shortcut tooltips. Expand the transfer drawer for queued/running/completed/failed jobs.
- Command palette: search actions and locations, show keybindings, and invoke pane, file, transfer, favorite, and connection actions.
- Drag/drop: files between panes and external app surfaces when the platform API permits. Make the operation explicit; default to copy for ambiguous cross-provider drops. Keep keyboard/menu alternatives.
- Keyboard help: a grouped, scrollable shortcuts modal toggled by `Cmd+?`, also available through Help and the command palette. Preserve typing in inputs and terminals, block underlying actions while open, and restore the previous focus when dismissed.

## Optional Vim interaction

Settings → Interaction exposes a persisted, default-off Vim mode scoped to
file listings. NORMAL and VISUAL state, pending sequences/counts and inline
search/command entry appear in the status bar. The keymap in README.md covers
navigation, selection, search, history, tabs, folder trees, directional pane
focus/splits and reviewed file operations. Inputs, terminals and modals retain
their normal keys; existing macOS shortcuts remain available. Vim delete and
copy use the existing review/confirmation flows.

## Additional pane splits

The workspace starts with two side-by-side pane groups. Split the focused pane
to the right or below to create an independently navigable pane at the same
location, preserving the original pane's tabs, history, selection, and sort.
Splits may be nested and have resizable dividers. Closing a split removes only
that pane and collapses its parent divider; each original side keeps at least
one pane. Focus navigation visits all surviving panes in layout order.

Keyboard copy/move targets the most recently focused different surviving pane,
with a surviving alternate pane as fallback. Operation review shows the actual
source and destination; drag/drop always uses its explicit destination pane.
Extra panes and split geometry are session-only. Existing left/right location
preferences describe the first surviving pane on each original side.

## Integrated terminal

Terminals are tabs owned by individual panes. A pane can switch between file
and terminal tabs without stopping either terminal sessions or file navigation
state. Creating a terminal tab starts an independent session; splitting a
terminal creates a fresh session at the original launch location rather than
copying its process or input. Explicit terminal-right/below actions can also
start a shell beside a file tab. Closing a terminal tab ends only that session;
closing a split ends its owned terminal sessions. End Session returns that pane
to a file tab. The sole remaining tab remains protected from ordinary Close Tab.

Local sessions start the user's login shell with its working directory set to
the active file pane. Later browser navigation never sends commands into a
live shell. SFTP sessions use system SSH with the saved host, port and user and
a quoted startup directory. System SSH owns authentication and `known_hosts`,
independently of the provider's Keychain password and host-key store. Failure
to change remote directory ends the session; FTPS and S3 have no shell.

Shell input, ANSI colors, cursor movement, alternate screen, bracketed paste,
scrollback, resize, mouse-drag output selection and Cmd+C copy are supported.
Ctrl+C still sends a shell interrupt. Selection holds a stable output snapshot
until typing, scrolling, resizing or a click clears it. Copy joins visual rows
with newlines. Terminal mouse reporting is follow-up work. Sessions and output are never persisted.
File transfers require visible file tabs; they never use a terminal launch
directory as an implicit destination. Dropping files onto a terminal inserts
shell-quoted absolute paths at its cursor without sending Enter. Local terminals
accept local paths; SSH terminals accept paths from the same SFTP connection.
Saved left/right locations come from file tabs.

## MVP definition
MVP is local filesystem only. It must launch into two useful panes, navigate folders, show metadata, select files, open folders, create folders, rename, copy, move, and delete with confirmation; support tabs/history, favorites, keyboard commands, and a visible transfer queue. Treat cross-volume move as copy then delete only after a verified copy; surface partial failures and offer recovery information. SFTP/FTP/S3 are later phases.

## State and component model
Suggested modules (adapt to the repository, do not create empty scaffolding for all of these upfront):

- `app`: application startup, window composition, global actions and settings.
- `workspace`: sidebar, split layout, pane identity and active-pane routing.
- `pane`: tabs and per-tab browser state.
- `browser`: listing model, selection, sort, path editing and navigation history.
- `domain`: `Location`, `Entry`, `EntryId`, `Metadata`, `ProviderId`, typed errors, operation capabilities.
- `providers`: `FileSystem` trait and local implementation; later remote adapters.
- `transfers`: job model, planner, bounded scheduler, progress events, cancellation and conflict decisions.
- `credentials`: secret references and OS credential-store adapter.
- `persistence`: non-secret preferences, sidebar, favorites, connections and workspace state.

Suggested state ownership:
- `AppState`: sidebar visibility, palette/dialog state, transfer drawer visibility, preferences, provider registry.
- `WorkspaceState`: left/right pane IDs, active pane, divider position.
- `PaneState`: ordered tabs and active tab.
- `TabState`: location, history stack/cursor, listing request generation, entries, selection, sort, loading/error state.
- `TransferManager`: jobs and bounded workers, independent of views; UI subscribes to snapshots/events.

Use stable app-owned IDs. A late listing response must not overwrite a newer navigation: associate requests with a generation/token and discard stale results. GPUI entities should own UI state; pass snapshots/IDs to background work and deliver results back through documented GPUI update mechanisms. Confirm exact APIs against pinned GPUI/GPUI Kit versions.

## Provider boundary
Own a trait along these lines, adapting signatures to the chosen async runtime and GPUI integration:

```rust
trait FileSystem: Send + Sync {
    fn provider_id(&self) -> ProviderId;
    fn capabilities(&self) -> Capabilities;
    async fn list(&self, location: &Location, options: ListOptions) -> Result<Page<Entry>, FsError>;
    async fn metadata(&self, location: &Location) -> Result<Metadata, FsError>;
    async fn create_dir(&self, location: &Location) -> Result<(), FsError>;
    async fn rename(&self, from: &Location, to: &Location) -> Result<(), FsError>;
    async fn delete(&self, location: &Location, kind: DeleteKind) -> Result<(), FsError>;
    async fn open_read(&self, location: &Location) -> Result<Box<dyn AsyncRead + Unpin + Send>, FsError>;
    async fn create_write(&self, location: &Location, mode: WriteMode) -> Result<Box<dyn AsyncWrite + Unpin + Send>, FsError>;
}
```

This is illustrative, not a required compile-ready trait. In Rust, async trait object support/runtime bounds need an explicit dependency decision; use boxed futures or an established compatible pattern if necessary. Avoid pretending one universal path string works for local paths, SFTP paths, and S3 bucket/key locations. Capabilities must report rename, server-side copy, trash, permissions, symlinks, seek, and resumability where relevant. Surface unsupported operations plainly.

A provider registry maps `ProviderId` to implementations and constructs typed `Location`s. Start with `LocalFileSystem`; keep app code unaware of OS-specific file handles except within provider/platform adapters.

## Connections and credentials
Connection records contain a stable ID, display name, provider type, non-secret endpoint/region/user metadata, and a credential reference. Secrets live in macOS Keychain (and a future platform adapter), not the settings file. Support missing/locked credentials as normal states. Never persist passwords/private key contents in logs, errors, clipboard, or plain configuration. Private-key file paths may be metadata, but the key material remains owned by the OS/user. The connection editor should validate endpoints without exposing secrets.

## Transfers and concurrency
A transfer job includes ID, operation, source and destination locations, state, byte/item totals when known, current item, progress, cancellation token, conflict policy, timestamps, and typed failure details. Plan transfers through provider capabilities: server-side copy when safely available; otherwise stream through bounded buffers. Limit concurrent jobs and per-job files/streams. Support pause only if the underlying operation can be paused safely; otherwise do not show a fake pause control. Cancellation must stop scheduling new work and interrupt current I/O when supported. For move across providers, verify destination completion before deleting source. Keep a transfer journal sufficient to explain partial completion and retry safely; never claim global atomicity across providers.

Conflict decisions include replace, keep both, skip, and cancel, with batch application where safe. Preserve metadata only where the destination supports it. Progress events should be throttled for UI responsiveness.

## Security and destructive behavior
- Local root/jail behavior is not implied. Resolve symlinks and permissions according to OS behavior; avoid custom path concatenation that can escape a chosen destination.
- Confirm recursive deletes and overwrites with item/path context. Prefer platform trash where available; clearly distinguish permanent deletion.
- Do not execute/open unknown files automatically. Opening uses explicit user action and platform launch APIs.
- Sanitize error display and redact secrets/credentials from diagnostics.
- Remote host key verification for SFTP must be explicit and persistent; never silently trust changed host keys.
- TLS and endpoint validation must be enabled for supported protocols; do not downgrade silently.
- Consider OS sandbox/file access entitlements and bookmark/access-token persistence during macOS integration; document capability limits.

## Dependency and design-system policy
Before adding a crate, inspect current GPUI and GPUI Kit documentation, compatibility, license, maintenance, async runtime fit, and macOS behavior. Pin compatible versions in the actual project. GPUI Kit currently documents components such as Sidebar, Resizable, StatusBar, Toolbar, Command, Dock, Tabs, Tree, and VirtualList; use only where their APIs suit this product and the pinned version. Avoid adding a second component library or duplicate GPUI version without dependency-tree evidence. See [GPUI Kit components](https://gpui-kit.com/component/).

## Phases
1. **Foundation:** repo inspection, buildable GPUI shell, theme tokens, two-pane layout, active-pane focus, app actions.
2. **Local browsing:** provider contract + local provider, async listings, path entry/breadcrumbs, metadata columns, selection/sort, loading/empty/error states.
3. **Navigation and preferences:** tabs per pane, independent history, favorites, persistence for non-secret settings, command palette/keybindings.
4. **Local operations:** create/rename/copy/move/delete, confirmations, bounded transfer queue, progress/cancel/conflicts, partial-failure reporting.
5. **Polish and macOS integration:** drag/drop, volumes, native dialogs/open-in-default-app, accessibility, packaging, visual/performance review.
6. **SFTP:** authenticated connection flow, host-key verification, credential references, remote browsing and streaming transfers.
7. **FTP/S3:** add only after explicit product prioritization; FTP security limitations and S3 object semantics must be explained in UI and tests. Consider whether FTPS is required instead of plain FTP.

## Acceptance principles
A milestone is complete only when its behavior is implemented and checks are recorded. Compilation is not proof of visual layout, drag/drop, Keychain, host-key, or real remote-provider behavior. Use manual verification notes for OS/UI integration and provider-backed checks where available. Do not add tests unless asked by the user; if the repo's existing workflow already requires tests or a milestone calls for checks, follow that project's instructions and distinguish added tests from executed existing checks.

## Local process usage monitor

The status bar shows Excavator's process CPU and resident-memory usage, sampled
asynchronously every two seconds. Clicking it opens a compact graph popup and
starts bounded, in-memory history collection; closing pauses history collection.
CPU uses 100% per fully occupied core. Memory covers this process, excluding
terminal subprocesses. No samples are persisted or transmitted.
