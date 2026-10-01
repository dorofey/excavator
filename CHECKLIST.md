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
- [ ] Evaluate an optional Vim mode for file navigation and selection; define modes, discoverable bindings, and interactions with text inputs and existing shortcuts before implementation.
