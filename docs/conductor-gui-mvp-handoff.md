# Archductor GUI MVP Handoff

This is the source-of-truth handoff for the corrected GUI-first MVP. Older
notes that imply a backend-first or CLI-first product are stale.

## Product Target

Archductor is a Linux and Windows desktop control plane for parallel coding agents. V1 must
make the normal loop usable from GTK:

1. Add or clone a project repository.
2. Configure project settings that affect workspaces and agents.
3. Create one or more workspaces as Git worktrees.
4. Run Shell, Codex, or Claude Code sessions inside a workspace.
5. Inspect terminal/session state, logs, todos, diffs, checks, comments, and PR
   state.
6. Send review/check/comment context back to an agent.
7. Create, refresh, merge, archive, restore, and review history.

The CLI is fallback and automation surface. A CLI path without a GTK path does
not complete the MVP.

## Required Concepts

Use these terms exactly:

- `Project`: Archductor entry for one codebase.
- `Repository`: Git codebase behind the project.
- `Workspace`: isolated task copy of the repository.
- `Branch`: Git branch checked out in the workspace.
- `Working tree`: files on disk for that workspace.
- `Running environment`: terminals, agents, scripts, tests, servers, and other
  processes inside the workspace.
- `Turn`: the actions a coding agent takes after one user message and before
  the next user message in the same chat thread. One tool call or file write is
  not a turn.

Relationships:

- `1 project contains 1 repository`
- `1 repository contains many workspaces`
- `1 workspace maps to 1 branch`
- `1 branch has 1 working tree`
- `1 workspace can run many processes`
- `1 turn can contain many tool calls and file writes`

## V1 Must-Haves

- Project onboarding from an existing local repository and Git clone.
- Repository settings editing for scripts, check commands, environment,
  prompts, prompt pack metadata, Git behavior, provider paths, terminal,
  shortcuts, notification labels, workspace defaults, and safe advanced TOML.
- Workspace creation from branch, prompt, GitHub issue, GitHub PR, and Linear
  issue where credentials are available.
- Durable session/process records that survive app restart.
- Provider-native agent sessions for Codex app-server and Claude stream-json,
  with canonical provider events persisted separately from diagnostic transport
  logs.
- Workspace terminal, setup/run/stop controls, logs, and process list.
- Changes/checks/review/todos/conflicts/PR panels with visible failure states.
- Debug-only PTY Inspector gated by `ARCHDUCTOR_DEBUG`.
- Archive/restore/history surfaces.
- Release docs and manual test checklist aligned with actual verified behavior.

## Known Large Risks

- `crates/core/src/workspace.rs` still holds too many service responsibilities.
- GTK view files are too large and mix rendering, state, and runtime actions.
- Runtime ownership must keep converging on archcar/local daemon APIs.
- Codex unsafe approval/sandbox bypass needs explicit policy before broad
  public launch.
- Manual Linux and Windows GUI validation remains required before announcing
  the corresponding public package.

## Non-Goals For V1

- Remote-control/mobile app.
- Archductor Cloud.
- Full Conductor visual parity.
- Full terminal-emulator correctness.
- Publishing any package channel that has not passed install, launch, upgrade,
  checksum, and rollback/yank validation.

## Verification Rule

Do not call a feature done unless current evidence proves the right layer:

- Core support proves only core behavior.
- CLI support proves fallback/automation behavior.
- GTK support proves product behavior only when connected to real core behavior.
- Live provider behavior requires authenticated `gh` or `LINEAR_API_KEY`.
- Packaging support requires Linux artifact validation, not only Rust tests.
