# Progress

## Current State

This project is being redirected from a CLI-heavy worktree tool into a
GUI-first Conductor-style desktop app.

The previous progress log overstated the GUI as "MVP complete." That was
incorrect. The corrected MVP definition is in:

- [`docs/conductor-gui-mvp-handoff.md`](docs/conductor-gui-mvp-handoff.md)
- [`docs/mvp-scope.md`](docs/mvp-scope.md)

## What Exists

### Backend/Core Foundation

- Rust workspace with core, CLI, and GTK crates.
- SQLite-backed repository, workspace, process, PR, todo, review, and checkpoint
  state.
- Repository add/list/update/doctor.
- Import from the macOS Conductor database.
- Workspace create/list/archive/restore/discard/rename.
- Real Git worktree creation.
- `.context` initialization.
- setup/run/archive script plumbing from `.conductor/settings.toml`.
- Stable per-workspace port allocation.
- Background run scripts and logs.
- Shell/Codex/Claude/Cursor session launch primitives.
- Git status/diff/log helpers.
- Todo, review comment, checkpoint, conflict, and checks-summary commands.
- GitHub PR create/view/checks/merge through local `gh` auth.
- Packaging scaffolding for AppImage, deb, rpm, AUR, and Flatpak.

### GTK Prototype

- Native GTK window with navigable Dashboard, Projects, History, and Workspace
  pages.
- Sidebar workspace search/grouping.
- Dashboard columns.
- Projects page can add local repos, clone Git URLs, list projects, and create
  workspaces.
- Workspace page has basic actions and rough tabs for chats, changes, checks,
  todos, and processes.
- History page can read old chats from the macOS Conductor database when
  available.

## What Is Not Done

The actual GUI-first Conductor MVP is not complete.

MVP-critical missing work:

- Embedded Conductor-native Claude/Codex/Cursor chat.
- Embedded workspace terminal.
- GUI-first project settings editor.
- Polished repository/workspace creation flows.
- Real diff/review/comment surface.
- GUI-first GitHub PR/check/review/merge flow.
- Agent status model and resumable in-app session history.
- Robust confirmations, progress, error states, refresh, and toasts.
- Visual parity with Conductor.
- Release-ready packaging validation.

## Next Step

Do not continue adding backend-only commands until the docs and app architecture
are aligned around the GUI-first MVP.

Recommended next work:

1. Finish the documentation reset.
2. Refactor `crates/gtk-app/src/main.rs` into page/component modules.
3. Define the app state model for projects, workspaces, agent sessions, and
   selected UI tabs.
4. Build the embedded agent session and terminal foundation.
