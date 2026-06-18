# Conductor GUI MVP Handoff

This document is the source of truth for the corrected MVP direction.

The previous implementation work over-indexed on a CLI/backend workflow and
underbuilt the actual Conductor product experience. The target MVP is a
GUI-first Conductor-style desktop app, not a CLI tool with a dashboard.

## Product Definition

Conductor is a local desktop control plane for parallel coding agents.

The app lets a user add a repository, create isolated Git worktree workspaces,
run Claude Code, Codex, and Cursor inside those workspaces, inspect the work,
review changes, manage GitHub PRs/checks, and archive completed work without
juggling many terminal windows.

The key product value is that agent coordination happens inside the Conductor
GUI.

## Core Model

| Term | Meaning | Relationship |
| --- | --- | --- |
| Project | App-level entry for one codebase. Holds repo settings, scripts, instructions, and workspaces. | 1 project contains 1 repository |
| Repository | Local Git codebase behind the project. Can be an existing folder or cloned source. | 1 repository contains many workspaces |
| Workspace | Isolated copy of a project/repo for one task, issue, experiment, or PR. | 1 workspace maps to 1 branch |
| Branch | Git branch checked out inside a workspace. This is the review/PR unit. | 1 branch has 1 worktree |
| Working tree | Files on disk for one workspace, created by Git worktree. | 1 working tree belongs to 1 workspace |
| Running environment | App/server/watchers/tests/terminals running inside a workspace. | 1 workspace can run many processes |
| Agent session | Claude Code, Codex, or Cursor process attached to a workspace. | 1 workspace can have many sessions |

Workspace isolation is development isolation, not a security boundary. Agents
and commands still run on the user's machine with the user's permissions.

## MVP User Flow

1. User opens the Conductor-style GUI.
2. User adds an existing local repo or clones a Git repo.
3. App creates a project entry and shows repo settings/workspaces.
4. User creates a workspace for one task/issue/branch/PR.
5. App creates a Git worktree and branch.
6. User clicks into the workspace.
7. User starts Claude Code, Codex, or Cursor from the workspace page.
8. Agent output appears in a Conductor-native chat UI, not as raw terminal
   output in an unrelated window.
9. User can run setup scripts, run app scripts, open a workspace terminal, view
   logs, and inspect active processes from the workspace page.
10. User reviews changes, checks, todos, comments, and GitHub PR state in the
    same workspace view.
11. User creates/updates/merges a PR and archives the workspace when complete.
12. Archived workspaces and old chats remain available from History.

## Required MVP Features

### 1. GUI Shell

- Native desktop window.
- Left sidebar with projects, active workspaces, archived/history entry, search,
  and status badges.
- Main area with navigable pages: Dashboard, Project, Workspace, History.
- Keyboard shortcuts can come later, but basic navigation must be clickable.
- App should not require normal workflow coordination through CLI commands.

### 2. Project/Repository Onboarding

- Add existing local repository from GUI.
- Clone repository from Git URL from GUI.
- Detect default/base branch.
- Fetch latest remote before workspace creation.
- Show repository path, remote, default branch, workspace parent.
- Project settings screen for:
  - setup script
  - run script
  - archive script
  - run mode
  - environment variables
  - files to copy / include globs
  - durable instructions/prompts

### 3. Workspace Management

- Create a workspace from GUI.
- Workspace creation creates a real Git worktree.
- Workspace maps to one branch.
- Workspace page shows:
  - project/repo
  - branch
  - path
  - run state
  - active agent sessions
  - PR/check status
  - changed file count
  - todos/comments/conflicts
- Archive, restore, discard, and rename from GUI.
- Destructive actions require confirmations.

### 4. Agent Coordination

- Supported agents: Claude Code, Codex, Cursor.
- Agents use the user's existing local auth.
- Agent sessions are launched from the workspace page.
- Sessions are visible in the workspace page.
- Multiple agents can run in one workspace.
- Multiple workspaces can run agents in parallel.
- Agent output should be formatted in a Conductor-native chat surface.
- MVP should avoid forcing users into many external terminals.
- If the first implementation uses PTY-backed processes, the UI should still
  render a readable app-native transcript and session state.

### 5. Terminal And Runtime

- Workspace page includes terminal access scoped to that workspace.
- Run setup script from GUI.
- Run app/dev script from GUI.
- Stop run script from GUI.
- Show process list.
- Show logs.
- Show ports and `CONDUCTOR_*` environment context.

### 6. Git And Review

- Show changed files.
- Show git status.
- Show recent commits.
- Show unified diff.
- Diff viewer should evolve into:
  - file tree
  - side-by-side or inline diff
  - inline comments
  - send selected comments back to agent
- Detect conflicting active workspaces that changed the same files.

### 7. GitHub Integration

Use local `gh` auth for MVP.

Required GUI workflows:

- Push branch / detect upstream state.
- Create PR.
- View PR title, number, URL, state.
- Show CI/check status.
- Surface failing checks.
- Show basic PR review/comment state.
- Merge PR.
- Archive workspace after merge.

The backend can call `gh`; the user should not need to run `gh` manually for
normal Conductor flow.

### 8. Checks/Todos/Attention

Workspace page should answer:

- What changed?
- Is an agent running?
- Is the app/dev server running?
- Are there failing checks?
- Are there open todos?
- Are there unresolved comments?
- Are there conflicts?
- Is this ready to PR/merge/archive?

Todos should be editable from GUI and syncable from `.context`.

### 9. History

- History page shows archived workspaces.
- History page shows old chats.
- User can inspect old chats and restore archived workspaces.
- Existing macOS Conductor DB import/read support is useful for migration, but
  the app needs its own unified local history model.

## What Exists Now

### Strong Foundation

- Rust workspace with core, CLI, and GTK crates.
- SQLite-backed repository/workspace/process state.
- Repository add/list/update/doctor.
- Import from macOS Conductor DB.
- Workspace create/list/archive/restore/discard/rename.
- Git worktree creation.
- `.context` initialization.
- setup/run/archive script plumbing.
- per-workspace port allocation.
- run/stop/logs for background scripts.
- Shell/Codex/Claude/Cursor session launch primitives.
- diff/status/log helpers.
- todos, review comments, checkpoints.
- PR create/view/checks/merge through `gh`.
- conflict detection and checks summaries.
- Packaging scaffolding for AppImage, deb, rpm, AUR, Flatpak.

### Rough GTK Prototype

- GTK window with sidebar and pages.
- Dashboard columns.
- Projects page can add local repo, clone repo, list projects, and create a
  workspace.
- Workspace page has basic metadata/actions/tabs.
- History page reads old macOS Conductor chats directly from the Conductor DB.
- Some workspace actions are wired to core APIs.

## What Is Partial Or Misleading

- GUI is functional prototype quality, not MVP-complete.
- Current UI does not match Conductor closely enough.
- Agent sessions still mostly behave like launched external tools.
- There is no true embedded Conductor chat UI.
- There is no embedded terminal.
- Changes/checks/todos/process panels are basic, not review-grade.
- GitHub integration is backend/CLI-heavy and not a real GUI workflow.
- History reads old Conductor DB but is not a first-class app history model.
- Project settings editor is missing.
- Error handling, progress, confirmations, refresh, and toasts are incomplete.

## What Is Not Built Yet

- Full GUI-first agent coordination.
- Native formatted Claude/Codex/Cursor chat.
- Resumable in-app agent sessions.
- Embedded terminal panes.
- Real Conductor-like visual design.
- Rich workspace creation from GitHub issue, branch, PR, or Linear issue.
- Project settings editor.
- Diff viewer with inline comments.
- GUI PR/check/review/merge flow.
- Agent status model comparable to Conductor.
- Safe, polished destructive action flows.
- Release-ready packaging validation.

## Evaluation Of `progress.md`

`progress.md` currently overstates completion.

Incorrect or misleading claims:

- "MVP complete" is not accurate for the GUI-first product.
- "GUI MVP complete" is not accurate.
- "Remaining nice-to-haves" lists features that are actually MVP-critical:
  embedded terminal, real agent UI, GUI GitHub workflow, settings, review
  surfaces.
- Packaging exists, but release readiness should wait until the GUI workflow is
  real.
- Several historical entries describe an older richer GTK layout that was later
  replaced or changed; current docs should describe current behavior only.

Correct interpretation:

- CLI/backend foundation is substantially implemented.
- GTK app is an early prototype.
- The true MVP remains unfinished because the product is GUI-first.

## Implementation Plan

### Phase 0: Documentation Reset

- Rewrite README around the corrected GUI-first MVP.
- Rewrite `progress.md` current-state section.
- Keep this file as the full handoff spec.
- Clearly mark current implementation as foundation/prototype.

### Phase 1: App Architecture Cleanup

- Split `crates/gtk-app/src/main.rs` into modules:
  - `app_shell`
  - `dashboard`
  - `projects`
  - `workspaces`
  - `agents`
  - `terminal`
  - `diff`
  - `checks`
  - `settings`
  - `history`
- Define app state:
  - selected project
  - selected workspace
  - active page
  - active workspace tab
  - running sessions/processes
- Replace ad hoc refresh closures with an explicit refresh/event model.

### Phase 2: Project Onboarding And Settings

- Build polished add/clone repo flow.
- Add project settings page.
- Persist settings edits to `.conductor/settings.toml` or local settings as
  appropriate.
- Validate setup/run/archive scripts.
- Show file-copy settings and environment variables.

### Phase 3: Workspace Command Center

- Build real workspace page layout:
  - header/status
  - agents panel
  - runtime panel
  - changes/checks/review tabs
  - chat/terminal split
- Create workspace from GUI with branch/base options.
- Archive/restore/discard/rename with confirmations and visible progress.

### Phase 4: Embedded Runtime

- Add embedded terminal support.
- Run setup/run scripts from GUI and stream logs.
- Show processes and ports.
- Stop processes reliably.
- Surface errors and exit codes.

### Phase 5: Agent Sessions

- Create PTY/process layer for Claude Code, Codex, and Cursor.
- Persist session metadata and transcript.
- Render app-native messages.
- Track status: idle, working, waiting, errored, done.
- Support multiple sessions per workspace.
- Support history/resume where possible.

### Phase 6: Git/Diff/Review

- Build changed-file tree.
- Build usable diff viewer.
- Add inline comments.
- Send comments back to selected agent.
- Show conflicts with sibling workspaces.
- Show commits and branch state.

### Phase 7: GitHub Workflow

- GUI PR create/view/checks/merge.
- Show CI failures.
- Show PR comments/review state.
- Let user ask agents to fix failing checks or review comments.
- Archive after merge.

### Phase 8: History And Restore

- Create unified local history model.
- Import existing Conductor sessions/workspaces into that model where possible.
- Browse archived workspaces.
- Restore workspace with chats/context.

### Phase 9: Release Readiness

- Manual smoke tests for the full GUI workflow.
- Visual QA against Conductor screenshots.
- Packaging validation on target Linux distros.
- Remove stale docs/claims.
- Cut release only after GUI workflow is usable without CLI coordination.

## Next Recommended Task

Do Phase 0 only first:

1. Rewrite README.
2. Rewrite `progress.md` current-state section.
3. Update manual testing checklist to reflect GUI-first MVP requirements.
4. Do not add more features until the docs/spec match the intended product.
