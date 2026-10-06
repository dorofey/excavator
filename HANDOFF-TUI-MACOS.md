# Excavator single-pane TUI: macOS / herdr implementation handoff

## Objective and authorization

Build a single-pane terminal file browser for Excavator on macOS. Run multiple
instances in herdr panes and copy or move selections to another instance's
current location. Support the existing remote providers through the same
provider and transfer engine used by the desktop application.

The user requested this alternate frontend and explicitly chose macOS and
**herdr first**. tmux is a later adapter, not a prerequisite. This document is
an implementation plan, not evidence that any feature is implemented.

Read AGENTS.md, START.md, SPEC.md, CHECKLIST.md and ORCHESTRATION.md first.
For this task, work through the milestones below rather than unrelated GUI
acceptance items in CHECKLIST.md. Keep the desktop application buildable and
preserve its behavior. Do not replace the GPUI frontend or mark its pending
acceptance checks complete.

## Repository evidence and reuse points

- `src/domain.rs`: app-owned Location, Entry, capabilities and typed errors.
  Locations already distinguish local paths, SFTP paths, FTPS paths and S3
  bucket/key/prefix values. Do not flatten these into universal path strings.
- `src/providers/mod.rs`: FileSystem trait, boxed provider futures and cooperative
  cancellation. Futures may perform blocking calls when polled; run them on
  background workers, never in the terminal event/render loop.
- `src/providers/remote.rs`: ProviderRegistry and existing remote operations.
- `src/transfers/mod.rs` and `remote.rs`: OperationPlan, conflict decisions,
  job snapshots, progress, cancellation and completion journal. The current
  manager executes one worker at a time. Preserve this bound initially.
- `src/connections.rs`: saved metadata, secret lookup and known-host handling.
- `src/credentials.rs`: macOS Keychain implementation. Reuse its service and
  connection IDs; do not introduce plaintext secret persistence.
- `src/main.rs`: currently a GPUI binary with private module declarations.
  Shared modules need a library boundary to be reused cleanly by another binary.
- `Cargo.toml`: GPUI Kit 0.7.0; ssh2 0.9.6; suppaftp 12.1.1;
  rust-s3 0.37.2; security-framework 3.7.0 on macOS. Recheck actual pinned
  versions and lockfile before making dependency choices.

At handoff preparation, the worktree contained user edits to Cargo.toml,
README.md, connections, credentials, persistence and platform modules, plus
an untracked Linux build script. Reinspect on the Mac; never discard or
overwrite those changes as part of this task.

## Product behavior

Proposed invocation: `excavator-tui [local-directory]`, with a saved-connection
picker inside the TUI. Keep launch syntax simple before adding remote flags.

Each instance owns its browsing location, selection, sort, history and loading
state. herdr owns layout, resizing, focus, workspace and terminal persistence.
The TUI provides listing navigation, multi-selection, hidden-file toggle,
refresh, path editing, searchable commands, shortcut help and transfer review.

All transfers have keyboard commands and an explicit destination display.
Provide Copy/Move left, right, up and down, plus Choose destination. A single
eligible browser in a requested direction is selected automatically. Multiple
eligible browsers open a picker. Never silently wrap to the opposite side.
Ordinary Copy/Move may use the last explicitly chosen surviving destination;
otherwise show the picker. Final keybindings must be checked against herdr,
macOS terminal behavior and text-entry controls; show them in help.

An adjacent shell/editor is not a file destination. Do not infer its directory
from terminal output or inject `cp`, `mv` or shell commands into another pane.
Only registered, responsive Excavator instances qualify for automatic targeting.
Manual destination entry remains available outside herdr.

Remote panes run locally on the same Mac. One pane can browse local files and
another SFTP, FTPS or S3. Running the TUI inside SSH on a different host is not
part of this implementation's coordination protocol.

## Architecture and contracts

### Shared core and frontend

Expose the existing domain/provider/transfer/connection/credential modules
through a library. Keep GPUI views, appearance and updater out of the TUI path.
Use a small terminal frontend, provisionally Ratatui + Crossterm, after checking
current compatible versions, licensing and macOS support. Prefer an optional
GUI dependency/feature if needed to make TUI-only builds independent of GPUI;
retain the existing default GUI build and commands.

Do not implement a second provider or transfer engine. Introduce only the small
frontend-neutral helpers needed to replace UI-specific orchestration. Inspect
existing examples before changing exports or dependency features.

The terminal event loop consumes background results with request generations,
discarding stale listings. Bound worker queues and all coordination messages.
Always restore terminal mode/cursor on ordinary exit, errors and unwind.
Handle resize, narrow terminals, Ctrl-C and interrupted startup explicitly.

### herdr adapter: verify before coding

Read the official API and inspect the installed herdr version on the Mac:

- https://herdr.dev/docs/cli-reference/
- https://github.com/motionharvest/herdr

Confirm how to obtain the current instance's pane/workspace identity, enumerate
panes, obtain layout relationships or geometry, distinguish active workspace
and tab, and handle pane relocation/closure. Environment IDs may be launch-time
identifiers; verify whether aliases remain valid after relocation. Record exact
commands/API fields and installed version in a decision note.

Use structured API output when available. Never scrape terminal screen output.
Do not assume a directional API or invent field names. If herdr exposes only
pane identities, ship the explicit destination picker first and record the
directional-discovery limitation. Use geometry for direction only when actually
available: restrict scope to the current visible workspace/tab, rank candidates
by direction and orthogonal overlap, and prompt when selection is ambiguous.

Define a narrow Multiplexer adapter returning app-owned pane identity and
optional geometry/adjacency. Implement herdr first. Outside herdr, discovery
is unavailable but browsing and manual destinations still work. Add tmux only
after herdr acceptance, using the same interface.

### Coordination between instances

Start with per-instance Unix-domain sockets in a private per-user runtime
directory. A small bounded registry maps live instance IDs to socket addresses
and multiplexer identity. Keep registry updates atomic and reconcile stale
records through handshake/liveness checks. Avoid a central daemon initially.
Verify macOS socket path-length limits and cleanup only owned runtime artifacts.

Version the protocol. Publish only non-secret data:

- instance ID and startup identity, herdr scope/pane identity;
- current typed Location and monotonically increasing location generation;
- optional short display label and protocol version.

Support handshake, read-current-location and refresh/invalidate notifications.
The protocol must not accept arbitrary shell commands or file-operation requests.
Restrict socket/directory permissions to the owner; verify peer identity using
macOS-supported facilities where available. Bound frame size, timeout and
concurrent requests; reject malformed/version-incompatible messages visibly.

Define an explicit wire location representation. Local paths can contain
non-UTF-8 bytes; preserve them with a byte-safe encoding. Display escaping must
not change the value used for I/O. Remote values keep their provider semantics.
Do not send credentials, keys, tokens or secret-bearing endpoint URLs.

At review, capture source selections and the destination Location, instance ID
and generation. At confirmation, recheck the target: if it exited or navigated,
require renewed review. Once queued, the destination remains immutable even if
the other pane navigates. Execution belongs to the source process; a destination
exit after enqueue does not cancel a valid accepted transfer.

Source owns progress, cancellation and conflict review. Notify all reachable
instances viewing affected locations after completion or partial changes;
notification failure must not turn a successful transfer into a failed transfer.
The destination can refresh manually. Show that exiting the source terminates
its workers; a durable daemon/job-recovery service is a later scope decision.

## Remote connection behavior

First expose existing saved connections and reuse Keychain lookup and known-host
storage. Add a terminal add/edit connection flow only after saved connections
work; it must offer parity for required fields without copying GPUI widgets.
Masked secret input must not enter logs, history, CLI arguments or runtime
registry. Loading credentials and authentication run off the event loop.

SFTP first-connect fingerprint review, changed-key rejection and explicit
recovery are required. Cancellation/timeout and missing/locked Keychain states
must be visible. Audit shared error messages before displaying them.

Resolve both source and destination connection IDs from the shared local
metadata store. A target's successful login does not imply the source process
has loaded its credentials: initialize the needed provider registry in the
executing process. Do not borrow secret material through pane coordination.

Support local↔SFTP first, then SFTP↔SFTP, then existing FTPS and S3. Let existing
capabilities choose execution. Stream remote-to-remote transfers through the Mac
when necessary. Do not promise server-side copy without implemented capability
and verification. S3 prefixes are not POSIX directories or atomic renames.
Move must use safe rename when supported, or verified copy before source deletion;
inspect and preserve the existing verification definition and failure reporting.
Retain explicit review for move, overwrite, delete and host-trust changes.

## Milestones and review boundaries

1. **Discovery and core boundary.** Inspect dirty status, nested instructions,
   modules, examples, macOS toolchain and installed herdr. Document the exact
   adapter contract. Introduce the library and minimal TUI entrypoint. Acceptance:
   GUI still builds; TUI-only build works; entering/exiting restores the terminal.

2. **Standalone local browser.** Implement async listing states, generation
   checks, metadata columns, navigation/history, selection, path entry, refresh,
   help and commands. Acceptance: actual macOS terminal exercise with empty,
   Unicode/non-UTF-8, inaccessible and large temporary directories, resize and
   cancellation; no render-loop filesystem I/O.

3. **herdr discovery and coordination.** Register two or more instances,
   retrieve live locations, implement picker and verified directional targeting.
   Acceptance: real herdr panes, resize/rearrange/move/close, shell neighbors,
   stale registry and incompatible protocol; no accidental opposite-side target.

4. **Local cross-pane transfers.** Wire existing queue, source/destination
   review, bounded execution, progress, conflict decisions, cancellation and
   refresh notifications. Acceptance: fixture copies/moves, conflicts, symlinks,
   partial failures and destination navigation before/after confirmation.
   Test cross-volume moves only if an actual second volume is available.

5. **SFTP saved connections.** Add picker, Keychain/auth states and host-key
   review; local↔SFTP and SFTP↔SFTP transfers. Acceptance: disposable server on
   the Mac with first/changed keys, denied permissions, interruption and
   cancellation. Never exercise destructive checks on personal remote data.

6. **Remote parity.** Add terminal connection editing, then existing FTPS/S3
   browsing and transfers with capability-aware commands. Acceptance: disposable
   fixtures, pagination/prefix semantics, conflicts and partial failure. Record
   which external services have actually been exercised.

7. **macOS delivery.** Document installation/build, herdr launch workflow,
   shortcuts, credential requirements, runtime cleanup and process-exit behavior.
   Verify release binary in a fresh terminal session. Finish herdr acceptance
   before considering tmux or a persistent transfer daemon.

Stop at each milestone's review boundary with changed files, exact checks and
remaining limitations. Keep this document's progress/evidence separate from the
desktop checklist until the coordinator explicitly incorporates the new scope.

## Checks and acceptance evidence

Use repository-prescribed formatting, locked checks and clippy, adjusted for
the actual feature/binary layout. Run existing relevant verification examples
for local providers, transfers, locations, credentials and remote fixtures.
Compile affected GUI targets after shared changes. Do not add a blanket new
test suite; follow the repository's instruction about adding tests and use
existing checks plus documented fixture/manual exercises.

For every milestone record exact commands, exit outcomes, installed versions
and what was manually exercised. Compilation does not verify herdr geometry,
terminal restoration, Keychain interaction or real remote behavior. Never
report unavailable checks as passing. Use generated disposable credentials
and temporary fixtures; remove only resources created for verification.

## Copy/paste implementation prompt

You are implementing Excavator's macOS single-pane terminal frontend with
herdr integration first. Read HANDOFF-TUI-MACOS.md and the repository guidance.
The user has authorized this alternate frontend and remote support using the
existing providers. Preserve the GPUI app and unrelated user changes.

Start with milestone 1 only. Inspect the installed herdr API and current pinned
dependencies before designing the adapter. Extract a minimal reusable library
boundary and create a TUI entrypoint that builds independently of GPUI if
feasible while retaining the default GUI workflow. Do not implement tmux first,
invent herdr APIs, rewrite provider/transfer engines, or put secrets in CLI
arguments or coordination messages. State assumptions and a small plan before
editing. Complete the milestone's checks, report exact evidence and limitations,
then stop for review. On subsequent instructions continue the next milestone
from this handoff. Use sequential work unless independent file ownership makes
delegation clearly worthwhile; no particular model or agent CLI is required.
