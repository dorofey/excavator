# Multi-agent orchestration guide

Use parallel agents for independent discovery or well-separated implementation slices. The coordinator owns integration, shared contracts, and the checklist. Do not assume every Codex CLI installation supports subagents, model selection, or identical flags: check `codex --help`, the installed version, and configured agent features first. If unavailable, use the same role prompts sequentially or run independent work in separate branches/worktrees managed by the user.

## Roles and capability levels
Model labels vary by account and release. Choose from models actually available in the user's CLI; capability levels below describe the work, not exact model names.

| Role | Capability | Owns | Avoids |
|---|---|---|---|
| Coordinator / architect | Strong reasoning, repository-wide context | milestone selection, interface contracts, integration, checklist, review | broad implementation in files delegated to others |
| GPUI UI | Strong Rust + UI framework experience | view composition, pane/sidebar/tabs/listing UI, theme and keyboard interaction | provider semantics, transfer engine |
| Filesystem/providers | Strong Rust async and filesystem/API knowledge | domain types, `FileSystem` contract, local provider, later remote adapters | GPUI view state |
| Transfers/concurrency | Strong async systems and failure semantics | job model, scheduler, progress/cancel/conflicts, transfer planning | provider-specific UI |
| macOS integration | Strong macOS/Rust platform knowledge | Keychain, dialogs, drag/drop, volumes, packaging constraints | app-wide redesign |
| Tests/reviewer | Independent critical reviewer | focused verification, edge cases, API/security review, visual acceptance plan | editing implementation unless explicitly assigned a fix |

Use a stronger model for cross-cutting design, async/security review, and integration. A faster/lower-cost model can handle bounded file-local tasks, documentation edits, or straightforward UI components after interfaces are settled. Do not choose a model by name until the installed CLI reports it is available.

## Ownership and parallelism
- Coordinator is sole owner of `AGENTS.md`, `SPEC.md`, `CHECKLIST.md`, and shared API decisions during a milestone.
- Assign non-overlapping files or directories. For example, UI agent owns `src/ui/**`; provider agent owns `src/providers/**` and `src/domain/**`; transfer agent owns `src/transfers/**` only after agreeing to domain interfaces.
- Avoid concurrent edits to `Cargo.toml`, app entrypoints, shared state types, or lockfiles. Coordinator integrates dependency changes one at a time.
- If a shared type must change, pause dependent work, propose the change in writing, and let the coordinator approve/integrate it.
- Start with one or two parallel agents. More agents increase merge and API coordination costs.
- Keep secrets, production credentials, and personal files out of agent prompts and fixtures.

## Handoff format
Each delegated task should include: objective, allowed files, files that are off-limits, relevant milestone, accepted interfaces, checks to run, and expected handoff. Return:
1. Summary of implementation and design choices.
2. Changed files.
3. Commands/checks and exact outcomes.
4. Known gaps, assumptions, and risks.
5. Integration notes or conflicts.

The coordinator reviews the diff, checks compatibility with `SPEC.md`, runs integration checks, and only then updates checklist status.

## Gates
1. **Discovery gate:** repository/toolchain and actual GPUI APIs are known.
2. **Contract gate:** domain, provider, pane, and transfer interfaces are agreed before parallel implementation depends on them.
3. **Build gate:** integrated tree builds/checks using repository-prescribed commands.
4. **Behavior gate:** relevant interactions work in a launched app or provider-backed environment; compile output alone is insufficient.
5. **Security gate:** credential handling, host-key behavior, destructive operations, and error redaction have been reviewed before remote connections or destructive operations ship.
6. **Milestone gate:** checklist and docs reflect evidence, unresolved issues, and a clear next slice.

## Codex CLI workflow
First inspect what this installation supports:

```text
codex --help
codex --version
```

Then start in the repository root and provide `START.md` as the task prompt (paste its contents or use the CLI's supported file-prompt mechanism). If supported, use the CLI's documented subagent/agent configuration to launch distinct roles. Model overrides and agent syntax are version/configuration dependent; consult installed help/docs rather than copying hypothetical flags from this file.

Example coordinator prompt:

> Read AGENTS.md, START.md, SPEC.md, CHECKLIST.md, and ORCHESTRATION.md. Inspect the current repository and choose the earliest feasible unchecked milestone. Propose a small plan and identify any shared interface decisions. Delegate only bounded independent work if this CLI supports it and file ownership is clear. Implement and verify the milestone, update evidence-backed checklist items, then stop at the milestone review boundary.

Example bounded UI task:

> Implement only the two-pane shell and active-pane focus for checklist milestone 1, using the repository's pinned GPUI/GPUI Kit APIs. Own `src/ui/**`; do not edit domain/provider types, Cargo manifests, or shared docs. Keep the app buildable. Report changed files, checks and exact results, and anything requiring coordinator integration.

Example provider task:

> Inspect the repository and implement only the local provider contract and directory listing slice for checklist milestone 2. Own `src/domain/**` and `src/providers/local/**`; do not edit UI or dependency manifests without first proposing a dependency. Preserve provider-specific paths and typed errors. Report checks and unresolved API choices.

If agents are not available, the coordinator can execute these as sequential focused passes. If separate worktrees are supported, assign one task per worktree and integrate reviewed commits; do not have multiple agents write the same checkout concurrently.
