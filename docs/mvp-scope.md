# MVP Scope And Current Feature State

For the full corrected GUI-first handoff spec, see
[`docs/conductor-gui-mvp-handoff.md`](conductor-gui-mvp-handoff.md).

This project is being refocused from a CLI-heavy Linux worktree tool into a
full Conductor-style desktop app. The previous docs overstated the GUI as
"MVP complete"; that was inaccurate.

## Product Goal

The MVP should support the core Conductor loop:

1. Add a repo. The app registers or clones a Git repository and keeps all work
   local.
2. Deploy agents. Each Claude Code, Codex, or Cursor session runs inside an
   isolated Git worktree workspace.
3. Conduct. The app shows who is working, what needs attention, code changes,
   checks, todos, chat history, and workspace lifecycle actions.

## Implemented Today

### CLI/Core

- Repository registration with `repo add/list/update/doctor`.
- Import from the macOS Conductor database with `import conductor`.
- Workspace create/list/archive/restore/discard/rename using Git worktrees.
- Workspace `.context` initialization.
- Setup/run/archive script support through `.conductor/settings.toml`.
- Stable per-workspace `CONDUCTOR_PORT` allocation.
- Run/stop/logs for background workspace processes.
- Interactive terminal launch for Shell, Codex, Claude Code, and Cursor.
- Supervised background sessions with process records and logs.
- Git diff/status/log helpers.
- Todo add/list/done/sync.
- PR create/view/checks/merge through `gh`.
- Review comments, checkpoints, MCP status, conflict detection, and status
  summaries.

### GTK App

- Native GTK/libadwaita window with sidebar and page navigation.
- Dashboard page with workspace columns.
- Sidebar workspace search and grouping by repository.
- Projects page that can add a local repo, clone a Git URL, list projects, and
  create a workspace.
- Workspace detail page with metadata and actions for Shell, Codex, Claude Code,
  Cursor, Run, Stop, Open Folder, Archive, Restore, and Discard.
- Workspace tabs for Chats, Changes, Checks, Todos, and Processes.
- History page that reads prior Conductor chats/messages from the macOS
  Conductor database when available.
- Imported Conductor repositories/workspaces are visible in the app.

## Partial Or Rough

- The GTK layout is functional but not yet a close visual clone of Conductor.
- History is read-only and sourced directly from the macOS Conductor database;
  it is not yet a unified local chat model.
- Agent sessions launch in external terminals. There is no embedded terminal or
  in-app chat composer yet.
- Projects/workspace creation forms are basic text fields, not polished
  Conductor flows.
- Run/Stop and lifecycle buttons perform actions but need confirmation,
  progress, error toasts, and automatic targeted refresh.
- Changes/Checks/Todos/Processes are basic panels, not full Conductor review
  surfaces.

## Not Yet Built

- Exact Conductor UI parity.
- In-app chat threads for new Claude/Codex/Cursor sessions.
- Resuming old chats inside the app.
- Rich workspace creation from GitHub issue, branch, PR, or Linear issue.
- Diff viewer with inline comments that can be sent back to an agent.
- Full checks tab with CI/deployment/comment/todo aggregation.
- PR composer/review/merge workflow in the GUI.
- Embedded terminal panes.
- Project/repository settings editor.
- Agent status model comparable to Conductor's live session state.
- Safe confirmation flows for destructive actions.
- Polished packaging/release readiness.

## Current MVP Definition

The current MVP is an early functional slice:

- Worktree-backed repository/workspace engine.
- CLI for the complete workflow.
- GTK app with basic navigation and enough controls to create/select
  workspaces, launch agents externally, inspect changes/checks/history, and run
  lifecycle actions.

It is not yet a full-fledged Conductor app. The next work should prioritize
turning the rough GTK surfaces into the actual Conductor workflow experience
instead of adding more backend-only commands.
