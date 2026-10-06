# Incremental implementation checklist

Work top to bottom. Each milestone should leave the app buildable and end with a short review note. Do not mark a box done without evidence. The first milestone can be adjusted after repository inspection.

## 0. Repository and build baseline
- [x] Inspect repository, nested `AGENTS.md`, current branch/status, toolchain, GPUI/GPUI Kit versions, and existing build/run/check commands.
- [x] Record platform target, minimum macOS version, async runtime, persistence strategy, and known environment limits in a short decision note.
- [x] Build and launch the existing baseline if a project exists; record exact outcome. If no app exists, create the smallest buildable GPUI shell.
- [x] Confirm pinned GPUI and GPUI Kit APIs from docs/examples and ensure dependency versions are compatible.

Evidence and limitations: see `DECISIONS.md`. Milestones 1–4 implement the
user-authorized local-files MVP; OS polish and release readiness remain separate.

## 1. Window shell and two-pane interaction
- [x] Establish theme tokens for dense rows, typography, spacing, borders, selection, hover, and active focus.
- [x] Compose compact top bar, hideable sidebar, resizable two-pane workspace, and bottom status area.
- [x] Give each pane a visible active/inactive focus state and route a small set of actions to the active pane.
- [x] Add placeholder tabs and path bars for both panes; keep pane state independent.
- [x] Verify layout at narrow and wide window sizes and keyboard focus order in the rendered app.

## 2. Local filesystem browsing
- [x] Define app-owned provider/domain types and typed errors.
- [x] Implement the local filesystem provider and capability reporting.
- [x] List directory entries asynchronously; sort directories/files predictably and handle hidden files according to a clear preference.
- [x] Render name, kind, size, and modified time with loading, empty, permission-denied, and error states.
- [x] Add row selection, multi-selection, open-folder behavior, and path/breadcrumb navigation.
- [x] Prevent stale listing responses from replacing a more recent location.
- [x] Verify against a test fixture folder containing empty directories, hidden files, Unicode names, large files, and inaccessible entries when available.

## 3. Navigation, tabs, favorites, and commands
- [x] Implement independent back/forward history in each pane.
- [x] Implement add, activate, close, and reorder tabs per pane.
- [x] Add favorites in the sidebar and navigate into the selected pane.
- [x] Add editable location/path entry with validation and clear errors.
- [x] Add command palette search for implemented actions and locations.
- [x] Add keybindings for pane focus, navigation, tab actions, selection, and command palette; show bindings in UI.
- [x] Add grouped keyboard-shortcuts help with Cmd+? toggle, file-list ? entry, Help menu and palette access; trap focus and preserve the previous view.
- [ ] Finish keyboard-help native scroll, narrow-window and text-field restoration acceptance; native capture became stale during the final fixture check.
- [x] Persist non-secret favorites and workspace preferences; handle corrupt or older settings safely.

## 4. Safe local file operations and transfer queue
- [x] Define operation plans and destination conflict decisions.
- [x] Implement create folder, rename, copy, move, and delete with explicit confirmation where needed.
- [x] Implement bounded transfer scheduling, progress, cancellation, and typed failure reporting.
- [x] Ensure cross-volume move copies and verifies before source deletion; preserve partial-failure details.
- [x] Add overwrite/keep-both/skip/cancel decisions and batch application where safe.
- [x] Build transfer drawer with queued, running, completed, and failed views; expose aggregate status in status bar.
- [x] Verify interrupted, cancelled, conflicting, and partial transfer outcomes using safe temporary fixtures.

## 4a. Theming and JSON-backed settings
- [x] Define a typed, versioned non-secret settings schema backed by JSON in the app's macOS Application Support directory; extend existing persistence without losing favorites or workspace preferences.
- [x] Load defaults for missing settings, migrate supported older versions, and report invalid or unsupported settings clearly without overwriting the original file.
- [x] Save settings atomically outside the UI thread; surface read/write failures and retain usable in-memory preferences.
- [x] Add a keyboard-accessible settings page with an Appearance section, reachable from the app menu and command palette via `Cmd+,`; include clear labels, descriptions, and reset-to-default controls.
- [x] Support a fixed theme or dynamic light/dark theme selection, with Light, Dark, and System appearance modes; react to OS appearance changes in System mode.
- [x] Provide built-in light and dark themes, separate light/dark selections, and searchable theme selectors inspired by the supplied Appearance screenshot.
- [x] Apply semantic theme tokens consistently to panes, sidebar, tabs, listings, palette, dialogs, transfer drawer, and settings; preserve readable contrast and distinct hover, selection, and active-focus states.
- [x] Expose UI font family, font size, and row-density preferences with validated ranges and immediate preview; apply appearance changes without restarting or disturbing pane state.
- [x] Document the settings JSON location, supported keys, defaults, migration behavior, and when manual edits take effect; keep credentials and secret-bearing values out of the file.
- [x] Verify settings round trips, restart persistence, missing/malformed/older JSON, and write failures using temporary fixtures; review light/dark themes, System switching, and keyboard-only settings interaction in the rendered app.

Scope: use the supplied Appearance screenshot as an interaction reference, adapted
to this file manager. Custom theme imports and independent icon-theme packs are
follow-up decisions. Review boundary: working appearance settings with JSON
persistence; evidence and the scoped native appearance limitation are recorded
in DECISIONS.md.

## 5. macOS interaction and visual polish
- [x] Add bottom-right sidebar/transfer-drawer icon controls and hover/focus-revealed tab-close icons, with accessible labels and shortcut tooltips; additional pane-splitting controls remain future work.
- [ ] Re-verify new-tab, hover-revealed close-tab, sidebar, and transfer-queue interactions after the Zed-reference control refinement. Idle icon placement/readability was rendered; earlier control interactions were verified before this change.
- [x] Verify closing the sole remaining tab is disabled.
- [ ] Verify hover, pressed, and keyboard-focus visuals on action buttons in the rendered app.
- [x] Verify control accessibility names in the rendered accessibility tree and visible shortcut labels in the command UI.
- [ ] Verify spoken VoiceOver labels and reduced-motion behavior on macOS.
- [x] Add a native directory picker for choosing the active pane's local folder.
- [x] Add native file open/save dialogs for relevant file actions when those
  workflows exist. No app-managed file-open or save-as workflow exists; selected
  files use the explicit open-in-default-app action, and transfers use the queue.
- [x] Add open-in-default-app action with explicit user initiation.
- [x] Add drag/drop between panes with copy-by-default semantics and conflict
  review.
- [x] Implement inbound Finder file/folder drops as queued copies with conflict review.
- [x] Verify a basic Finder-to-pane drop in the rendered app; the user confirmed it works.
- [ ] Verify Finder drops with multiple items and folders in the rendered app.
- [x] Add outbound drops to external apps where supported (local paths only).
- [ ] Verify outbound drops in the rendered macOS app with Finder, another app, a multi-selection, and a folder.
- [x] Add a Mounted volumes sidebar route to `/Volumes`; keep local provider permission failures typed and visible. `verify_local` exercises permission-denied listing; Finder drops do not supply security-scoped bookmarks, and the app is currently unsandboxed.
- [x] Verify mounted-volume navigation and access-denied presentation in the rendered app.
- [x] Review the actual rendered app against the written design brief and attached reference screenshots if supplied to the coding environment.
- [x] Verify packaging and launch on the intended macOS architecture; document unverified hardware/OS behavior.

## 6. Remote provider preparation and SFTP
- [x] Re-check maintained Rust SFTP options, license, async runtime compatibility, host-key support, and platform constraints before choosing a crate.
- [x] Define non-secret connection metadata and OS credential-store adapter; keep secret references out of logs/settings.
- [x] Implement add/edit/remove/test connection flow with Keychain-backed secret storage and missing-secret recovery; the connection editor flow was rendered.
- [x] Verify physical Keychain save/readback/cleanup with generated disposable SFTP and S3 connection IDs through the signed app UI.
- [x] Add import of saved connections from ForkLift with a preview, duplicate handling, and clear reporting of unsupported providers. ForkLift 4 SFTP metadata import implemented and 37 actual entries read back; passwords/private keys are not copied and missing credentials use the existing editor/Keychain flow. See DECISIONS.md.
- [ ] Verify ForkLift preview selection, file picker and confirmation with native keyboard/mouse interaction.
- [x] Implement known-host storage, first-connect fingerprint confirmation, changed-key rejection, and explicit recovery.
- [x] Implement SFTP list/stat/read/write/rename/delete according to advertised capabilities.
- [x] Add an “Open SSH in Terminal” action for SFTP connections using the user's default terminal app and the connection's host, port, user, and supported SSH identity metadata; expose it in connection menus and the command palette without passing secrets in command arguments.
- [x] Verify remote browsing and transfer behavior against a disposable test server; record provider/library versions and gaps.

## 7. FTP and S3 decision and implementation
- [x] Decide whether plain FTP is acceptable; evaluate FTPS support and clearly communicate transport security limitations.
- [x] Re-evaluate S3 as object storage: prefixes, pagination, versioning, delete markers, multipart uploads, and no directory/rename equivalence.
- [x] Add only provider features that have a concrete UX and error model.
- [x] Store access keys through OS credential storage; do not persist them in plaintext settings. The shared Keychain adapter has no plaintext fallback.
- [x] Verify physical S3 key save/readback/cleanup through macOS Keychain using generated disposable credentials in the signed app UI.
- [x] Verify pagination, retries, cancellation, conflicts, and partial transfers against a disposable account/bucket.

Evidence: see the milestones 6–7 section of `DECISIONS.md`. Disposable loopback
protocol and rendered checks passed; real AWS/external-provider interoperability
is not claimed.

## 8. Release readiness
- [ ] Review architecture boundaries, capability fallbacks, error redaction, and destructive-action confirmations.
- [ ] Check large-directory responsiveness and memory use; virtualize only where measurements justify it.
- [ ] Review keyboard-only usage and all keybindings for conflicts/platform conventions.
- [ ] Verify clean build/package and launch from a clean user-facing install location.
- [ ] Update README, release notes, and this checklist with evidence and remaining limitations.

## 9. Future interaction enhancements
- [x] Allow splitting a pane horizontally or vertically into additional independently navigable panes with resizable dividers; expose split, close-split, and focus actions through menus, the command palette, and discoverable shortcuts, preserving each pane's tabs, history, and selection. Command assertions and rendered nested-pane interaction evidence are recorded in DECISIONS.md; extra splits are session-only.
- [x] Add an integrated terminal window with keyboard-accessible show/hide and focus actions; define how local working directories and SSH sessions relate to the active pane. Per-pane terminal tabs and fresh terminal splits passed real PTY, multi-session Workspace and rendered typing/focus/split/end checks. Live SSH and full clipboard-paste acceptance remain unverified. See DECISIONS.md.
- [x] Define and implement optional Vim mode for listing navigation, counts, visual/marked selection, search, history, tabs, tree expansion, directional pane focus/splits, and reviewed file operations; persisted Settings → Interaction toggle defaults off. Bindings appear in shortcut help and the status bar shows NORMAL/VISUAL or search/command entry.
- [ ] Complete native Vim acceptance across all bindings, inputs/terminals/modals, settings restart persistence, and split geometry. Build and code review passed; native preview automation returned mismatched screenshots and then an inactive-surface error, so rendered acceptance is not claimed.

## 10. Zed-style interaction refinements (user feedback, 2026-10-01)
- [x] Closing the last tab of a pane closes that pane split; the sole pane on each original side keeps its last tab (protected, with a notice). Verified in the rendered app and in `verify_splits_ui`.
- [x] Render Nerd Font / Powerline glyphs in terminals via installed Nerd Font fallbacks instead of missing-glyph boxes. A real Powerline prompt and explicit `nf-*` code points rendered in the signed app.
- [x] Make the sidebar vertically scrollable and horizontally resizable (120–480 px divider); keep long names on one line with ellipsis. Keyboard focus, expansion and scrolling verified; pointer drag of the divider is not yet exercised.
- [x] New-tab (+) supports Option-click to split down and Shift-Option-click to split right, with tooltip hints. Terminal creation has a separate button. Build passed; modifier branches are not pointer-verified.
- [x] Show Zed-like folder and file-type icons in listings and sidebar, using selected bundled Lucide icons (no full-catalog embed, no copied Zed assets).
- [x] Show file listings as a tree: chevron click or →/← expands/collapses folders inline with indent guides; double-click/Enter still navigates into a folder; expansions load asynchronously, reject stale results, and survive refresh; operations act on the selected rows' real locations (rename stays in the item's own folder). Expand/collapse and indent verified in the rendered app.
- [x] Make sidebar favorites and saved connections expandable trees of subfolders (asynchronous, cancellable on collapse, errors shown inline); clicking a node opens it in the active pane. Keyboard tree navigation (⌘⌥S, ↑↓, →←, Return, Esc) verified in the rendered app.
- [x] Collapse the top toolbar into a Zed-like transparent title bar beside the traffic lights (compact back/forward/parent icons and a command search entry); drop the separate Refresh button row (⌘R and palette remain).
- [x] Present the command palette as a floating, centered modal with a bounded scrolling list, highlighted selection, ↑/↓ navigation, Enter to run, Esc/backdrop to dismiss, and shortcut labels. Verified in the rendered app, including running a command from it.
- [x] Plain title bar (window controls only) with one full-width tab row at the very top: every pane's strip aligned above its pane from live resizable sizes; Refresh removed entirely. Geometry asserted in `verify_splits_ui`; the rendered screenshot for this change could not be captured (screen capture returned black frames on this host).
- [ ] Verify the remaining pointer-only items in the rendered app: sidebar divider drag, ⌥-click and ⇧⌥-click on +, and title-bar window drag/double-click.

2026-10-02 alignment correction: the sidebar search width now subtracts the
title bar's existing traffic-light inset, and tab geometry uses the remaining
workspace width. `cargo check --locked --offline` and macOS bundle build passed;
rendered acceptance of the corrected alignment remains pending.

2026-10-02 tab controls: new-file-tab and new-terminal buttons sit at each
pane strip's right edge with a separately scrollable tab list. Tabs show
file-tree/terminal icons and location-only labels. The macOS bundle build
passed; rendered placement and button interaction acceptance remain pending.

Lower split panes now render their own tab strips; only panes touching the
workspace top edge use the title bar. Terminal creation supports the same
Option-click down / Shift-Option-click right split modifiers. The terminal
path/Files/End header is removed. Rendered acceptance remains pending.

2026-10-02 sidebar and connection modals: compact collapsible Favorites →
Folder, Connected Disks → Disk, Connections → Group → Connection hierarchy;
mounted disks load asynchronously. Connection manager supports adding/renaming
groups and choosing a group in the editor, including groups from older metadata.
Popup click isolation and stable focus corrected connection editing. Native
capture confirmed the hierarchy, manager/editor/group modals and typing in an
existing connection name and group draft; both drafts were cancelled. Build
passed. Group save/rename persistence and final focus restoration are not
yet exercised through the native UI.

Connection editor inputs now use explicit 32 px height, 10 px horizontal /
4 px vertical padding and the configured UI font size, with even field spacing.
The signed bundle build passed; final rendered padding acceptance is pending.

SFTP authentication now tries the SSH agent if loading/authenticating with the
selected private key fails, after host verification. Build passed; the user's
external SSH login succeeds, but Excavator's agent-backed login remains unverified.

Explicit Enter/double-click on local/SFTP folder symlinks resolves the target
asynchronously and navigates to its actual path; stale responses are discarded.
Transfers retain their no-follow behavior. Path inputs use explicit height,
padding and UI font size to avoid clipped text. Bundle build passed; live
symlink navigation and final rendered path-input acceptance remain pending.

Pane path bars now show clickable provider-aware ancestor breadcrumbs. Remote
roots use the saved connection name; Cmd+L or the ellipsis button reveals the
editable path. Bundle build passed; native breadcrumb interaction is pending.

SFTP root listings now use a session-only stale-while-revalidate cache keyed
by location/connection and hidden-file preference. Revisits display cached
rows immediately and refresh in the background; refresh failures retain rows
with a visible stale/error notice. Selection survives replacement by location.
The cache is bounded to 64 listings / 50,000 entries, skips listings over
10,000 entries, and clears after operation completion or connection changes.
Generation and cache-epoch checks prevent obsolete results from repopulating it.
Build passed; user confirms SFTP caching works (2026-10-02).
Refresh-failure recovery is user-verified: cached rows remained visible with
a timeout/retry notice while offline, and refresh succeeded after reconnecting
Wi-Fi (2026-10-02).

Directory loading and SWR progress use a spinner in a reserved path-bar slot
beside Edit Path. Removed loading/refreshing text rows to keep listing geometry
stable; refresh errors remain visible. Bundle build passed; native spinner
animation/placement acceptance is pending.

Tabs can be dragged between panes/splits, dropped before another tab to reorder,
or dropped onto a pane/tab strip to append. Existing terminal entities and tab
state move intact; asynchronous listing/tree responses locate the current pane
by stable tab ID. Moving the last tab collapses an extra split or leaves a fresh
file tab on a protected original side. Command palette includes Move tab to next
pane. Build passed; the user reports tab dragging works. Terminal-session continuity
after moving a terminal tab remains unverified.

Operation review now uses a centered confirmation modal with a dimmed backdrop,
source/destination details, explicit confirm/cancel buttons and Enter/Esc.
Underlying pane commands are blocked until review closes; validation errors
stay inside the modal. Bundle build passed; user reports the modal works
properly (2026-10-02).

2026-10-02 confirmation audit: transfer conflicts now automatically open a
centered decision modal with source/destination, Keep both, Skip, Cancel job and
separate replacement review. The drawer retains a compact Review conflict
entry. Copy/move/delete/Trash/rename/create-folder, ForkLift import, SSH trust,
connection removal and saved-host reset already use modals. Native conflict
interaction remains pending; no destructive operation was exercised.

2026-10-02 release preparation: 0.2.0 changelog, versioned optimized macOS
packaging and Sparkle 2.10.0 integration added. Archive signing key generated
in Keychain; only its public key is committed. In-app Check for Updates is
available from the app menu and palette. Update-enabled bundles require macOS
12. Developer ID/notarization, native install/relaunch and release-readiness
acceptance gates remain unchecked.

0.2.0 prerelease published at https://github.com/dorofey/excavator/releases/tag/v0.2.0.
Optimized build and deep bundle-signature verification passed. Signed update
archive and SHA256 checksum uploaded. Native download/install/relaunch remains
unverified; this first release cannot establish an upgrade from an older
Sparkle-enabled app. Source commit: 5f116fe.

- [x] Verify tab dragging in normal use; user reports it works (2026-10-02).
- [ ] Verify terminal-session continuity after dragging a terminal tab between panes/splits.
- [ ] Verify file-to-terminal drops in the rendered app: multiple files, spaces/apostrophes, insertion at the shell cursor, same-connection SFTP paths, and no execution before Enter. Path insertion is implemented; compilation and existing PTY checks passed (2026-10-05).

## 11. Release update acceptance (2026-10-02)
- [x] Publish 0.2.1 with the local Trash/Vim fix and signed ZIP/delta archives.
- [x] Exercise native update detection from the running 0.2.0 app; Sparkle showed 0.2.1 and its changelog.
- [x] Exercise download, Install and Relaunch; the relaunched app's Check for Updates reported “Excavator 0.2.1 is currently the newest version available.”
- [x] Verify the corrected local Move to Trash flow in 0.2.1; user confirms it works (2026-10-02).
- [x] Verify restoring a trashed item from macOS Trash; user confirms recovery works (2026-10-02).

Source release commit: 476b690. Published release:
https://github.com/dorofey/excavator/releases/tag/v0.2.1. Signing remains ad hoc;
Developer ID and notarization are still pending.

- [x] Verify SFTP caching in normal use; user confirms it works (2026-10-02).
- [x] Verify operation modal flow in normal use; user reports it works properly (2026-10-02).
- [x] Verify SFTP refresh-failure recovery: user screenshot shows retained cached rows and a timeout/retry notice; user confirms refresh works after reconnecting Wi-Fi (2026-10-02).

## 12. Local process usage monitor
- [x] Show process CPU and resident RAM in the status bar with asynchronous two-second sampling.
- [x] Add a click-to-open graph popup and command-palette entry; bound in-memory history to 120 samples and pause collection on popup close.
- [ ] Verify rendered popup positioning, changing metrics/graphs, keyboard dismissal, and pause/resume. Bundle build passed; native UI automation failed with “Sky Computer Use native pipe startup failed.”

2026-10-02 CPU/split regression correction: usage sampling now updates an
independent child view, avoiding full-workspace redraws; closed graphs allocate
no history vectors and unchanged displayed metrics do not redraw. Terminal
polling checks revision before cloning, painting shares an Arc snapshot, and
default cell backgrounds avoid individual quads. Worker/cancellation polling
is reduced. Down-split panels use bounded, clipped contents to prevent intrinsic
listing/terminal height pushing the lower pane outside the viewport. Integrated
cargo check passed; native split behavior and CPU improvement remain unverified.

0.2.2 prerelease published with CPU/RAM graphs, idle terminal optimizations and
the Down-split layout correction. Optimized build and deep bundle signature
verification passed; signed ZIP and deltas uploaded. Native split/CPU acceptance
remains pending. Source commit: b32cd36.
https://github.com/dorofey/excavator/releases/tag/v0.2.2

- [ ] Verify terminal text selection in the rendered app: forward/reverse and multirow drags, wide Unicode, scrollback, Cmd+C copy, Ctrl+C interrupt, and selection clearing on input/scroll/resize. Selection is implemented; native interaction remains pending.

## 0.2.3 release preparation (2026-10-06)
Optimized macOS bundle build, deep/strict code signature verification, compilation,
existing PTY and transfer checks passed. Release ZIP length, SHA256 and Ed25519
signature were verified against the committed Sparkle public key. The feed uses
the full archive and preserves all prior release entries. Native selection/drop
acceptance and update installation/relaunch remain pending. Signing is ad hoc;
the bundle is not notarized.

0.2.3 prerelease published with the signed full ZIP and checksum:
https://github.com/dorofey/excavator/releases/tag/v0.2.3
Source tag v0.2.3 points to 9baea04. Uploaded archive SHA256 matches the local
verified archive. Live upgrade installation/relaunch remains unverified.
