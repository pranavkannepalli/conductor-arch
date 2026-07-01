# Claude Agent Instructions

## Read This First

Before touching code or docs in this repository, read:

1. `README.md`
2. `progress.md`
3. `docs/manual-testing-checklist.md`
4. `docs/archductor-docs-parity-map.md`
5. `docs/deploy-and-local-test.md`

Use official Conductor behavior as the parity baseline:

- `https://www.conductor.build/docs/concepts/workspaces-and-branches`
- `https://www.conductor.build/docs/concepts/workflow`
- `https://www.conductor.build/docs/concepts/parallel-agents`
- `https://www.conductor.build/docs/reference/settings`
- `https://www.conductor.build/docs/reference/scripts`
- `https://www.conductor.build/docs/reference/files-to-copy`
- `https://www.conductor.build/docs/reference/agent-modes`
- `https://www.conductor.build/docs/reference/diff-viewer`
- `https://www.conductor.build/docs/reference/checks`

## Product Baseline

Linux Archductor is a local desktop control plane for parallel coding agents.
The core loop should work in the app: add or clone a repository, create
workspaces, run multiple chats/sessions, review work, create/merge PRs, archive,
and repeat for the same repository.

Do not market scaffolding as a feature. Distinguish clearly between core, CLI,
GTK controls, and verified end-to-end app behavior.

## Product Structure

Use this structure consistently when editing code, docs, or UI copy:

- `Project`: the Archductor entry for one codebase. It owns repository-level
  settings, scripts, instructions, and the list of workspaces.
- `Repository`: the Git codebase behind the project.
- `Workspace`: one isolated copy of the repository for one task, issue,
  experiment, or PR.
- `Branch`: the Git branch checked out inside the workspace.
- `Working tree`: the files on disk for that workspace.
- `Running environment`: terminals, agent sessions, setup/run scripts, tests,
  watchers, and app processes inside that workspace.

Relationship model:

- `1 project contains 1 repository`
- `1 repository contains many workspaces`
- `1 workspace maps to 1 branch`
- `1 branch has 1 working tree`
- `1 working tree belongs to 1 workspace`
- `1 workspace can run many processes`

Practical rule:

- Workspaces are the task/review unit.
- Sessions are part of the running environment inside a workspace.
- Multiple chats in one workspace do not mean multiple workspaces.

## Operating Mode

- Move fast, but keep changes direct and obvious.
- Prefer working product increments over architecture theater.
- Do not invent broad abstractions unless they remove immediate complexity.
- Do not add backend-only commands unless they unblock the app workflow.
- Implement, verify, and keep going.
- Fix stale docs when you find them.
- Do not call a feature done without current evidence from code, tests, CLI
  smoke, or GUI/runtime verification.
- If auth, API keys, display server, network, local tools, or test data are
  missing, say exactly what was not verified.

## Current State

The app has a usable but rough Archductor loop:

- Projects can add/clone repositories, edit shared/local settings, and create
  branch/prompt/GitHub/Linear workspaces.
- Workspaces are real Git worktrees with `.context` files and stable port
  ranges.
- The workspace page can start Shell, Codex, Claude, and Cursor sessions,
  terminal shells, setup/run scripts, review tabs, checks, todos, and lifecycle
  actions.
- The Checks tab can create/refresh PRs, read checks/comments, stage context for
  agents, merge, and archive after merge when configured.

Known rough edges:

- Agent sessions run PTY-backed harnesses.
- Codex chat now depends on:
  - PTY-accurate enter behavior
  - rendered screen parsing
  - `chat_threads`
  - `chat_messages`
  - native resume ID capture from Codex rollout metadata
- GTK chat is being migrated from process-first selection to thread-first
  selection.
- Terminal rendering is not a full terminal emulator.
- Broad shortcuts, deep links, monorepo directory selection, linked directories,
  richer GitHub review-thread sync, and unified local history are incomplete.
- GitHub-backed flows require local `gh` auth.
- Linear-backed flows require `LINEAR_API_KEY`.

## Repository Structure

Before changing architecture, know where things live:

- `crates/core`
  - repository/workspace/process state
  - PTY handling
  - Codex TUI parsing
  - harness argument building
  - thread/message persistence
- `crates/gtk-app`
  - workspace UI
  - chat/session surface
  - history view
  - terminal view
  - app state
- `crates/cli`
  - fallback CLI
  - session helper flows used by app/runtime paths
- `docs`
  - parity target
  - implementation plans/specs
  - manual testing and deployment notes

Default design direction:

- GUI-first
- workspace-centered
- PTY-backed
- thread-first chat persistence
- Linux-first quality

## Engineering Rules

- Work from this workspace unless explicitly told otherwise.
- Target branch is `origin/main`; do not rename the current branch.
- Check `git status --short` before edits and before final response.
- Do not revert user or other-agent changes unless explicitly asked.
- Use `rg`/`rg --files` for search.
- Keep changes scoped to the requested task.
- Run the narrowest useful verification for the change.
- If a frontend/GTK change affects visible UI, run or build enough to prove it
  still works.
