# Progress

## Current State

This project has completed the Phase 0 documentation reset, Phase 1 app
architecture cleanup slice, Phase 2 project settings slice, and a Phase 3
workspace command center slice for the corrected GUI-first MVP plan. The next
implementation phase is embedded runtime and app-native sessions.

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
  workspaces from branch/base, GitHub issue, GitHub PR ref, Linear issue slug,
  or prompt slug.
- Workspace page has a command center layout with status header, agents panel,
  runtime panel, changes/checks/review tabs, chat/terminal split, todos,
  processes, and lifecycle controls.
- History page can read old chats from the macOS Conductor database when
  available.

## What Is Not Done

The actual GUI-first Conductor MVP is not complete.

MVP-critical missing work:

- Embedded Conductor-native Claude/Codex/Cursor chat.
- PTY-backed embedded workspace terminal.
- Big Terminal Mode direction.
- More polished project settings/onboarding layout.
- Monorepo directory selection and linked-directory workflows.
- MCP status.
- Agent controls: Plan Mode, Fast Mode, reasoning/effort, Codex personality,
  Codex goals, checkpoints, skills, and tool approvals where supported.
- Fully polished repository/workspace creation flows.
- Real diff/review/comment surface.
- GUI-first GitHub PR/check/review/merge flow.
- Command palette, keyboard shortcuts, and deep links.
- Monorepo sparse-checkout controls and linked-directory workflows.
- Agent status model and resumable in-app session history.
- Unified local history model for archived workspaces and chats.
- Toast-backed progress/error states and deeper per-action refresh polish.
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

## Phase 2 Project Settings Slice Done

- Core settings now load and save Conductor-compatible repository settings,
  including scripts, run mode, Spotlight testing, Files to copy, environment
  variables, durable action prompts, provider executable/provider fields, and
  Git behavior flags.
- Core settings inspection now reports shared/local settings presence and
  `.worktreeinclude` precedence, with `.worktreeinclude` winning over
  `file_include_globs`.
- Settings saves validate run mode and environment variable names before
  writing repository settings.
- The GTK Projects page now has a repository settings editor for shared
  `.conductor/settings.toml` and local `.conductor/settings.local.toml`, plus a
  visible settings-layer and file-copy precedence summary.
- When `.worktreeinclude` exists, the GUI shows its active patterns as the
  read-only Files to copy source.
- Add/clone repository onboarding now shows the default clone location, reports
  detected base branch and workspace parent after success, and carries the
  selected repository into workspace creation and settings editing.
- Repository rows now show default branch metadata, which helps users create
  workspaces from the correct base.

## Phase 3 Workspace Command Center Slice Done

- Workspace detail now renders through a focused command center module instead
  of the old inline page.
- The workspace page shows header/status metrics, agents, runtime, workspace
  lifecycle controls, work tabs, and a chat/terminal split.
- Agents panel launches Shell, Codex, Claude, and Cursor sessions through the
  existing session surface.
- Runtime panel can run/stop configured workspace scripts, open the workspace
  folder, and show latest process state.
- Work tabs separate changes, checks, review comments, todos, and processes.
- Lifecycle controls support rename, archive, restore, and discard with visible
  progress text and confirmation gating for destructive archive/discard actions.
- The Projects page can create workspaces from branch/base, GitHub issue,
  GitHub PR ref, Linear issue slug, or prompt slug while reusing current core
  workspace APIs.

## Next Step

Do not continue adding backend-only commands unless they unblock the GUI-first
MVP.

Recommended next work:

1. Keep local docs aligned with the official Conductor docs before adding
   better-than-Conductor product ideas.
2. Continue polishing project onboarding and settings validation.
3. Replace the session/terminal foundations with PTY-backed streaming and
   bidirectional app-native chat/terminal I/O.
4. Add richer runtime logs, agent controls, PR/check actions, toasts, and
   visible blockers on top of the command center.
