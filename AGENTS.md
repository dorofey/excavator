# Agent instructions

## Product intent
Build a keyboard-first, two-pane file manager in Rust using GPUI and GPUI Kit. Use ForkLift for the information architecture and Zed for the compact, calm, developer-focused interaction language. Do not copy either product's branding or exact visual details. Consult `SPEC.md`, `CHECKLIST.md`, and `ORCHESTRATION.md` before changing product behavior.

## Durable engineering rules
- Keep the application usable with local files before adding remote providers.
- Keep UI state, provider operations, transfer execution, and credential storage behind clear boundaries. UI code must not perform blocking filesystem or network I/O.
- Define an application-owned `FileSystem` abstraction. Do not let a third-party provider crate's types leak across the app boundary.
- Use async/cancellable provider operations and bounded concurrency. Preserve errors and progress as typed state; never silently swallow failures.
- Treat paths and remote object keys as provider-specific values. Do not assume every backend has POSIX paths, stable inode IDs, atomic rename, or case sensitivity.
- Use safe defaults: copy is the default drop operation; show the source and destination and ask before destructive overwrite/delete; never follow symlinks outside the selected operation's intended semantics without making that explicit.
- Store secrets in the operating system credential store when available. Persist non-secret connection metadata separately. Never log credentials, private keys, tokens, or secret-bearing URLs.
- Model empty, loading, loaded, and failed states explicitly. Keep each pane's location, tabs, selection, sort and history independent.
- Prefer GPUI Kit components when they fit, but confirm the API in the version actually pinned by the repository. Avoid mixing incompatible GPUI versions or layering a second component crate without checking its dependency graph.
- Keep visual density deliberate: compact rows, readable names, clear active-pane/focus cues, restrained separators, predictable keyboard focus, and accessible labels/tooltips.
- All important actions must be reachable from a command palette and have discoverable shortcuts. Show shortcuts in menus/tooltips. Do not make drag and drop the only way to transfer files.
- Avoid speculative abstraction. Implement the next unchecked checklist milestone as a small, reviewable vertical slice.

## Codex workflow
1. Read this file and `START.md`, `SPEC.md`, `CHECKLIST.md`, and `ORCHESTRATION.md`.
2. Inspect repository status, existing architecture, dependency versions, and applicable nested `AGENTS.md` instructions before editing. Preserve user changes.
3. Select the earliest feasible unchecked milestone in `CHECKLIST.md`. State assumptions and an implementation plan before broad changes.
4. Delegate only bounded, independent work when the installed Codex CLI supports the needed mechanism and integration cost is lower than the benefit. Follow ownership and handoff rules in `ORCHESTRATION.md`.
5. Implement the milestone and run the checks appropriate to the changed code. Never claim a rendered UI, OS integration, provider, or hardware behavior was verified unless it was actually exercised.
6. Update checklist boxes and relevant docs only when evidence supports completion. Record blocked or environment-dependent checks without marking them complete.
7. Stop at the milestone's review boundary; summarize changed files, checks and outcomes, limitations, and the next unchecked milestone.

## Scope boundaries
Do not add FTP/SFTP/S3 credentials, network access, telemetry, destructive file actions, auto-update, or broad platform support as an incidental part of the local-files MVP. Ask before expanding product scope beyond `SPEC.md`.
