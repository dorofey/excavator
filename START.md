# Start here: Codex CLI master prompt

You are building the two-pane Rust file manager described in this repository's starter pack. Treat `AGENTS.md`, `SPEC.md`, `CHECKLIST.md`, and `ORCHESTRATION.md` as the current product and workflow guidance. The written product brief is ForkLift's two-pane information architecture with Zed's dense, developer-focused design language, built with Rust, GPUI, and GPUI Kit.

At the start of each session:

1. Read the four guidance files and inspect the repository, nested instructions, working tree, pinned dependencies, and existing build/run/check workflow. Preserve unrelated user changes.
2. Choose the earliest feasible unchecked milestone in `CHECKLIST.md`. Do not jump ahead to remote providers while local browsing/operations remain incomplete unless repository evidence requires a prerequisite.
3. Give a concise plan, name assumptions and the specific review boundary. Confirm current GPUI/GPUI Kit APIs from the pinned version before coding.
4. Delegate only if the installed Codex CLI supports the required agent workflow and work can be split across clear file ownership. Follow `ORCHESTRATION.md`; otherwise work in focused sequential passes.
5. Implement a real, small vertical slice. Keep the app buildable, preserve typed errors and provider boundaries, and avoid speculative scaffolding.
6. Run relevant repository checks and report exact outcomes. For visual, OS, Keychain, drag/drop, or remote-provider behavior, distinguish what was actually exercised from what remains unverified.
7. Update checklist/docs only when supported by evidence. Stop after the milestone review boundary and report changed files, checks, limitations, and the next unchecked milestone.

Do not assume a specific model name, subagent flag, async runtime, crate API, or platform capability until verified in the installed environment or repository. Ask a focused question only if a product decision blocks safe implementation; otherwise state a reasonable assumption and proceed.
