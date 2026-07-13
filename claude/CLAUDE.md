# Claude Agent Instructions

## Read This First

Before touching code or docs in this repository, read:

1. `docs/conductor-gui-mvp-handoff.md`
2. `progress.md`
3. `docs/mvp-scope.md`
4. `docs/manual-testing-checklist.md`
5. `docs/archductor-docs-parity-map.md`
6. `README.md`

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

Archductor is a local desktop control plane for parallel coding agents.
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
  smoke, and GTK smoke where applicable.
- If auth, API keys, display server, network, local tools, or test data are
  missing, say exactly what was not verified.

## Verification Contract

Every behavior change must be verified at the layers it touches:

- Written tests: run the narrowest automated tests that cover the edited core,
  CLI, and/or GTK code. Use focused tests first and broader package tests when
  the change crosses boundaries.
- CLI smoke: run the relevant `archductor` command or CLI test path that proves
  the behavior reaches the command boundary.
- GTK smoke: run the relevant `archductor-gtk` test or runtime path that
  proves the behavior reaches the app surface. For visible UI changes, use GTK
  smoke tests or a real GTK launch path when the environment supports it.

Keep CLI and GTK inline:

- User-visible core behavior should not land in only one surface. Update CLI and
  GTK together, or report the missing side as incomplete.
- Shared parsing, projection, state, and provider behavior should live in core
  when practical so CLI and GTK render the same semantics.
- CLI and GTK should use the same names, statuses, filters, and lifecycle
  assumptions for providers, sessions, workspaces, and runtime events.
- Before final response, name the written tests, CLI smoke, and GTK smoke that
  ran. If any layer was skipped, say why.

## Always-on Project Rules

Linear:

- Use the Linear tool freely.
- Archductor work lives in the `Archductor` project.
- When pulling tasks from Linear, query the `Archductor` project specifically.
- Be specific with Linear queries: project, status, assignee, issue key, labels,
  and relevant text.
- When starting a Linear task, move it to `In Progress`.
- When finishing a Linear task, move it to `In Review` so the user can review
  and push.

Cubic:

- Use the Cubic wiki freely.
- Pull project context from Cubic because it is current and useful.

Superpowers and communication:

- Use relevant Superpowers skills before work when available.
- Use caveman mode by default: short plan, short updates, short summary.

## Current State

Current state lives in `progress.md`; treat that file as the short status
source. The practical summary:

- The app has a usable GUI-first loop, but it is still a working prototype, not
  a finished MVP.
- Projects can add/clone repositories, edit shared/local settings, and create
  branch/prompt/GitHub/Linear workspaces.
- Workspaces are real Git worktrees with `.context` files, timelines, linked
  directories, stable port ranges, runtime controls, review surfaces, and
  archive/restore/history paths.
- The workspace page can start Shell, Codex, Claude, and Cursor session launch
  paths from GTK. CLI session commands currently support Shell, Codex, and
  Claude.
- Agent/session work is PTY/provider-event backed. Prefer structured session
  events, `chat_threads`, `chat_messages`, and native provider IDs over raw
  terminal-log inference.
- Terminal rendering is useful but not a full terminal emulator.
- GitHub-backed flows require local `gh` auth. Linear-backed flows require
  `LINEAR_API_KEY`.
- Packaging is not release-ready until the manual Linux app checklist and
  target package-channel validation pass.

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
  - MVP target
  - parity map
  - manual testing, deployment, release notes, and UI sketches

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
- Run written tests plus relevant CLI and GTK smoke for the change.
- If a frontend/GTK change affects visible UI, run or build enough to prove it
  still works.
