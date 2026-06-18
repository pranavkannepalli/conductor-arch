# Progress

## Current State

This project has completed the Phase 0 documentation reset and the first Phase
1 app architecture cleanup slice for the corrected GUI-first MVP plan. The next
implementation phase is polished GUI product surfaces.

The codebase is being redirected from a CLI-heavy worktree tool into a
GUI-first Conductor-style desktop app. The CLI and core backend are useful
foundation, but they are not the product experience by themselves.

The previous progress log overstated the GUI as "MVP complete." That was
incorrect. The corrected MVP definition is in:

- [`docs/conductor-gui-mvp-handoff.md`](docs/conductor-gui-mvp-handoff.md)
- [`docs/mvp-scope.md`](docs/mvp-scope.md)

Phase 0 now uses the official Conductor docs as the parity baseline. Match
Conductor's documented workflow first: repository setup, isolated workspaces,
agent sessions, runtime, diff review, checks, todos, PR flow, archive/history,
settings, command palette, shortcuts, deep links, provider settings, MCP, and
security/privacy posture.

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
- Big Terminal Mode direction.
- GUI-first project settings editor.
- Files to copy / `.worktreeinclude` UI and settings-layer visibility.
- Spotlight testing.
- Provider settings and MCP status.
- Agent controls: Plan Mode, Fast Mode, reasoning/effort, Codex personality,
  Codex goals, checkpoints, skills, and tool approvals where supported.
- Polished repository/workspace creation flows.
- Workspace creation from branch, PR, GitHub issue, Linear issue, and prompt.
- Real diff/review/comment surface.
- GUI-first GitHub PR/check/review/merge flow.
- Command palette, keyboard shortcuts, and deep links.
- Monorepo sparse-checkout controls and linked-directory workflows.
- Agent status model and resumable in-app session history.
- Unified local history model for archived workspaces and chats.
- Robust confirmations, progress, error states, refresh, and toasts.
- Visual parity with Conductor.
- Release-ready packaging validation.

## Phase 1 Architecture Cleanup Done

- `crates/gtk-app/src/main.rs` now has explicit app state for selected
  workspace, active page/tab, selected session, process attention state, and
  settings layer direction.
- Active GTK surfaces are split into focused modules for dashboard, sidebar,
  projects, history, session surface, terminal foundation, state, and refresh
  events; the old unused prototype shell has been removed.
- GTK refresh wiring now goes through a small refresh/event hub instead of each
  shell control manually carrying every refresh closure.
- Workspace detail now has an app-native agent session surface that starts
  supervised Shell, Codex, Claude, and Cursor sessions through existing core
  APIs and shows the latest captured session log.
- Workspace detail now has an embedded terminal foundation tab scoped to the
  workspace, ready for PTY-backed execution.

## Next Step

Do not continue adding backend-only commands unless they unblock the GUI-first
MVP.

Recommended next work:

1. Keep local docs aligned with the official Conductor docs before adding
   better-than-Conductor product ideas.
2. Build polished project onboarding and repository settings, including setup,
   run, archive, files-to-copy, `.worktreeinclude`, Spotlight, environment, and
   layered settings visibility.
3. Turn the workspace page into the command center: confirmations, progress,
   targeted refresh, richer runtime logs, agent controls, review/check/PR
   surfaces, and visible blockers.
4. Replace the session/terminal foundations with PTY-backed streaming and
   bidirectional app-native chat/terminal I/O.
5. Continue extracting the workspace detail helpers as that surface is made
   product-grade.
