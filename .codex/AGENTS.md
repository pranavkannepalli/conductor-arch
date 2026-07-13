# Codex Agent Instructions

## Read This Every Time

Before touching code or docs in this repository, read these files in order:

1. `docs/conductor-gui-mvp-handoff.md`
2. `progress.md`
3. `docs/mvp-scope.md`
4. `docs/manual-testing-checklist.md`
5. `docs/archductor-docs-parity-map.md`
6. `README.md`

Also keep the official Conductor docs in mind as the parity baseline:

- `https://www.conductor.build/docs/concepts/workspaces-and-branches`
- `https://www.conductor.build/docs/concepts/workflow`
- `https://www.conductor.build/docs/concepts/parallel-agents`
- `https://www.conductor.build/docs/reference/settings`
- `https://www.conductor.build/docs/reference/scripts`
- `https://www.conductor.build/docs/reference/files-to-copy`
- `https://www.conductor.build/docs/reference/agent-modes`
- `https://www.conductor.build/docs/reference/diff-viewer`
- `https://www.conductor.build/docs/reference/checks`

Treat `docs/conductor-gui-mvp-handoff.md` as the source of truth for the
corrected MVP. The old direction over-indexed on CLI/backend work. The product
goal is a GUI-first Archductor desktop app that matches the upstream Conductor
workflow first. Better-than-Archductor features come only after an explicit
product decision.

## Operating Mode

Use caveman mode:

- Move fast.
- Keep changes direct and obvious.
- Prefer working product increments over architecture theater.
- Do not invent broad abstractions unless they remove immediate complexity.
- Do not add backend-only commands unless they unblock the GUI-first MVP.
- Implement, verify, and keep going.
- Leave concise notes when something is incomplete or blocked.

Be real with the user:

- Do not call a phase, feature, connector, or flow "done" unless it has current
  evidence from code, written tests, CLI smoke, and GTK smoke where applicable.
- Distinguish clearly between backend support, CLI support, GTK controls, and
  actual end-to-end product behavior. One layer does not prove the others.
- If auth, API keys, display server, network, local tools, or test data are
  missing, say exactly what was not verified.
- Do not market scaffolding as a feature. A button that calls nothing real is
  not a feature. A CLI path with no GTK path is not a GUI feature. A GTK path
  with no core behavior is not real.
- When progress docs are stale or too optimistic, fix them before continuing.

Use Superpowers:

- Invoke relevant Superpowers skills before doing work.
- Use systematic debugging for bugs.
- Use TDD where practical for behavior changes.
- Use verification-before-completion before claiming something is done.
- Use subagents or separate Archductor workspaces for independent work when that
  helps finish faster.

There are enough credits. Optimize for throughput while keeping the codebase
coherent.

## Verification Contract

Every behavior change must be verified at all three layers that matter for the
change:

- Written tests: run the narrowest automated tests that cover the edited core,
  CLI, and/or GTK code. Prefer focused tests first, then broader package tests
  when the change crosses boundaries.
- CLI smoke: run the relevant `archductor` command or CLI test path that proves
  the behavior reaches the command boundary. Do not treat core-only tests as CLI
  verification.
- GTK smoke: run the relevant `archductor-gtk` test/build/runtime path that
  proves the behavior reaches the app surface. For visible UI changes, use GTK
  smoke tests or a real GTK launch path when the environment supports it.

Keep CLI and GTK inline:

- Core behavior that is visible to users should not land in only one surface.
  Update the CLI and GTK paths together, or explicitly report the missing side
  as incomplete.
- Shared parsing, projection, state, and provider behavior should live in core
  whenever practical so CLI and GTK render the same semantics.
- If the CLI renders a provider/session/workspace concept one way, GTK should
  use the same names, statuses, filtering rules, and lifecycle assumptions.
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

## Current Project State

Current state lives in `progress.md`; treat that file as the short status
source. The practical summary:

- The app has a usable GUI-first loop, but it is still a working prototype, not
  a finished MVP.
- GitHub-backed flows require local `gh` auth. Linear-backed flows require
  `LINEAR_API_KEY`.
- MCP status must be described as config/status inspection unless live
  reachability was explicitly tested.
- Codex/Claude session work is PTY/provider-event backed. Prefer structured
  session events, `chat_threads`, `chat_messages`, and native provider IDs over
  raw terminal-log inference.
- CLI session commands currently support Shell, Codex, and Claude. Cursor is a
  GTK launch path when configured.

Do not describe the project as MVP complete. Do not call packaging
release-ready until the GUI-first flow passes the manual Linux checklist and
the target package channel has install/launch/upgrade/checksum validation.

## Product Structure

Use these terms exactly:

- `Project`: the Archductor entry for one codebase.
- `Repository`: the Git codebase behind that project.
- `Workspace`: one isolated task copy of the repository.
- `Branch`: the Git branch checked out in the workspace.
- `Working tree`: the files on disk for that workspace.
- `Running environment`: terminals, agents, scripts, tests, servers, and other
  live processes inside that workspace.

Relationship model:

- `1 project contains 1 repository`
- `1 repository contains many workspaces`
- `1 workspace maps to 1 branch`
- `1 branch has 1 working tree`
- `1 workspace can run many processes`

When writing docs, UI text, or code comments:

- Do not blur project and repository together unless the distinction does not
  matter.
- Do not describe sessions or terminals as workspaces.
- Do not describe multiple chats in one workspace as multiple workspaces.

## Repository Structure

Agents should understand the repo before editing:

- `crates/core`
  - data model, repository/workspace/process state, PTY integration, Codex TUI
    parsing, harness launch planning
- `crates/gtk-app`
  - main desktop product surface, workspace command center, chat UI, history,
    terminal UI, app state
- `crates/cli`
  - fallback CLI, automation hooks, durable helper paths used by the GTK app
- `docs`
  - MVP target, parity map, manual testing, deployment, release notes, UI
    sketches
- `.codex/AGENTS.md`
  - Codex-specific working instructions for this repo
- `claude/CLAUDE.md`
  - Claude-specific working instructions for this repo

Current architectural direction:

- GUI-first Archductor clone
- PTY-backed agent harnesses
- thread-first chat persistence
- workspace-centered review/merge loop
- Linux-first product quality over theoretical portability

## Implementation Priorities

Follow the handoff phases:

1. Keep docs aligned with the corrected GUI-first MVP and current `progress.md`.
2. Finish project onboarding/settings polish and clear managed/user setting
   separation.
3. Keep workspace command center behavior real across core, CLI, and GTK where
   the feature has both surfaces.
4. Continue splitting large GTK/core files only when it directly reduces active
   task complexity.
5. Harden PTY/provider session recovery, archcar runtime ownership, and
   thread-first chat behavior.
6. Polish git/diff/review/GitHub PR/check/merge GUI workflows.
7. Finish prompt-pack switching/import/export, naming templates, hooks, local
   check runner UI, richer notifications, and deeper layout/theme controls.
8. Finish release validation on real Linux package channels.

## Engineering Rules

- Work from this workspace unless explicitly told otherwise.
- Target branch is `origin/main`; do not rename the current branch.
- Check `git status --short` before edits and before final response.
- Do not revert user or other-agent changes unless explicitly asked.
- Use `rg`/`rg --files` for search.
- Use `apply_patch` for manual file edits.
- Keep changes scoped to the requested phase/task.
- Run written tests plus relevant CLI and GTK smoke for the change.
- If a frontend/GTK change affects visible UI, run or build enough to prove it
  still works.

## Product North Star

Archductor is a local desktop control plane for parallel coding agents:

- Projects wrap repositories.
- Workspaces are Git worktrees and branches.
- Agents run inside workspaces.
- The GUI shows agent state, runtime state, changes, checks, todos, comments,
  PR state, and history.
- The GUI owns setup/settings, app controls, provider/MCP status, review
  blockers, archive/restore, and safety/privacy messaging.
- The user should not need to juggle many terminals for normal workflow
  coordination.
