# Archductor MVP Scope

Archductor V1 is Linux-first and GUI-first. The MVP is the smallest product that
lets one developer coordinate parallel local coding-agent work without juggling
many terminals.

## In Scope

### Project Setup

- Add an existing Git repository as a project.
- Clone a Git repository as a project.
- Store project settings in `.archductor/settings.toml` and local overrides in
  `.archductor/settings.local.toml`.
- Surface missing tool/auth states for `git`, `gh`, Codex, Claude Code, Cursor,
  and Linear.

### Workspace Lifecycle

- Create, duplicate, rename, archive, restore, and delete workspaces.
- Model each workspace as one Git worktree and one branch.
- Preserve workspace timeline/history events.
- Keep workspace operations recoverable after app restart.

### Runtime And Agents

- Start and stop Shell, Codex, Claude Code, and Cursor sessions.
- Persist raw PTY chunks, parsed session events, chat threads/messages, process
  records, and logs.
- Keep multiple sessions in one workspace separate from multiple workspaces.
- Reconcile stale processes on restart.

### Review And PR Flow

- Show changed files, diffs, todos, comments, conflicts, checks, PR state, and
  readiness blockers.
- Stage review/check/comment context into a selected agent session.
- Create, refresh, merge, and archive GitHub PRs through local `gh` auth.
- Show failures instead of silent empty states.

### Settings And Customization

- Support scripts, check commands, environment variables, prompts, prompt pack
  metadata, provider paths, Git defaults, file-copy rules,
  monorepo working directory, merge rules, workspace defaults, terminal
  preferences, view preferences, shortcuts, notification rules, and
  import/export where implemented.
- Treat advanced TOML as acceptable for power-user settings that do not yet have
  dedicated GUI controls.

### Release

- Pass `cargo fmt`, clippy, workspace tests, release-readiness script tests, and
  Linux build/package gates that match the announced channel.
- Complete the manual GTK checklist on a real Linux desktop before public
  release.

## Out Of Scope For V1

- Cloud sync, teams, hosted remote execution, or account model.
- Remote/mobile control plane.
- Native Windows packaging or process model.
- Full terminal emulator replacement.
- Full visual parity with upstream Conductor.
- Unvalidated package channels.

## Ship Criteria

V1 can ship when the GUI path handles the core loop end to end:

`project -> workspace -> agent/runtime -> review -> PR -> merge/archive -> history`

Known rough edges are acceptable only if they are documented in release notes
and do not corrupt repositories, lose session state, expose secrets, or make
dangerous actions easy to trigger accidentally.
