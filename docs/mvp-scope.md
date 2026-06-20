# MVP Scope And Current Feature State

For the full corrected GUI-first handoff spec, see
[`docs/conductor-gui-mvp-handoff.md`](conductor-gui-mvp-handoff.md).
For the official Conductor docs map used by Phase 0, see
[`docs/conductor-docs-parity-map.md`](conductor-docs-parity-map.md).

This project is being refocused from a CLI-heavy Linux worktree tool into a
full Conductor-style desktop app. The previous docs overstated the GUI as
"MVP complete"; that was inaccurate.

## Product Goal

The MVP should match the core Conductor loop before adding speculative
better-than-Conductor features:

1. Add or clone a repository.
2. Configure project setup, run, archive, Files to copy, environment, provider,
   and durable prompt settings.
3. Create one isolated workspace per shippable unit, or use multiple sessions in
   one workspace when the agents must share a branch.
4. Run Claude Code, Codex, or Cursor inside the workspace.
5. Run setup, app/dev scripts, terminal commands, tests, and Spotlight testing
   where appropriate.
6. Review diffs, comments, checks, todos, conflicts, and PR state.
7. Create/update/merge the PR and archive the workspace.
8. Restore archived workspaces and old chats from History.

The GUI is the product. CLI coverage is useful foundation and fallback, but the
MVP is not complete until the normal workflow can be driven from the app.

## Target MVP Product Surfaces

### App Shell

- Native GTK/libadwaita desktop window.
- Sidebar with projects, active workspaces, archived/history entry, search, and
  attention badges.
- Main pages for Dashboard, Project, Workspace, Diff/Review, Settings, and
  History.
- Command palette and shortcuts for core actions.
- Planned deep-link architecture for prompts, repository paths, issues, and
  async plans.

### Project Setup

- Add local repositories and clone Git URLs from the GUI.
- Detect default/base branch and fetch latest remote before workspace creation.
- Project settings editor for setup/run/archive scripts, run mode, Spotlight
  testing, Files to copy, `.worktreeinclude` precedence, environment variables,
  provider settings, action prompts, and Git behavior.
- Layer settings like Conductor: managed, local project override, repository
  shared, user shared, built-in defaults.

### Workspace Command Center

- Create a workspace from a task, branch, pull request, GitHub issue, Linear
  issue, or prompt.
- Keep one workspace mapped to one Git worktree and branch.
- Show project, branch, path, run state, active sessions, changed files,
  PR/check state, todos, comments, conflicts, ports, and process state.
- Archive, restore, discard, and rename with confirmations and progress.
- Support monorepo directory selection and linked workspace/directory context
  where needed for multi-repository changes.

### Agents And Runtime

- Launch Claude Code, Codex, and Cursor from the workspace page.
- Render sessions in app-native chat surfaces with status, transcript, composer,
  attachments, and resumable history.
- Support multiple sessions per workspace and multiple active workspaces.
- Surface relevant agent controls: Plan Mode, Fast Mode, effort/reasoning,
  Codex personality, Codex goals, checkpoints, skills, approvals, provider
  status, and MCP status.
- Include embedded terminal access and Big Terminal Mode direction.
- Run setup/run/archive scripts, stream logs, stop processes, show exit codes,
  and expose `CONDUCTOR_*` context.

### Review, Checks, And Merge

- Changed-file tree, git status, recent commits, and unified or side-by-side
  diff.
- Inline local comments and GitHub review comments.
- Send selected comments, failing checks, conflicts, and todos back to an agent.
- Checks tab aggregates git status, PR metadata, CI/status checks, deployments,
  GitHub comments/review threads, and todos.
- Treat failing checks, unresolved comments, open todos, and conflicts as merge
  blockers until explicitly resolved or overridden.
- GUI PR create/view/update/merge/archive-after-merge workflow through local
  `gh` auth for MVP.

### History And Safety

- Unified local history model for archived workspaces and chats.
- Restore archived workspace state and chats.
- Clear security/privacy posture: agents run locally with user permissions,
  approval prompts can gate risky actions, model traffic goes to configured
  providers, and enterprise data privacy disables external-AI/custom-MCP
  features where applicable.

## Implemented Today

### CLI/Core

- Repository registration with `repo add/list/update/doctor`.
- Import from the macOS Conductor database with `import conductor`.
- Workspace create/list/archive/restore/discard/rename using Git worktrees.
- Workspace `.context` initialization.
- Setup/run/archive script support through `.conductor/settings.toml`.
- Stable per-workspace `CONDUCTOR_PORT` allocation.
- Run/stop/logs for background workspace processes.
- Setup script runs and log capture as workspace runtime processes.
- Background setup/run/session processes update to exited after natural
  completion and store exit codes when available.
- Workspace-scoped terminal command execution with captured stdout, stderr, and
  exit code.
- PTY-backed workspace shell primitive with post-spawn input and streamed
  output.
- Terminal shell process records for embedded PTY shells, including pid,
  running/stopped status, timestamps, and stop exit code.
- First Spotlight testing slice: apply a workspace's tracked changes to the
  clean repository root when `spotlight_testing = true`, then reverse that patch
  on stop, with a start-time checkpoint commit of the tracked workspace state
  and manual sync/switching between active Spotlight workspaces. Stop/sync/switch
  refuse to reverse the active patch when the root has extra edits and dirty-root
  failures prioritize root-only affected paths plus a destructive Repair
  Spotlight warning when Git can identify paths. Runtime status also shows
  clean vs extra-root-edit state for the active Spotlight root before Stop/Sync.
  The selected workspace page polls for active patch changes and auto-syncs
  them; the app shell also polls active Spotlight sessions across pages.
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
  Cursor, Setup, Run, Stop, Open Folder, Archive, Restore, and Discard.
- Workspace tabs for Chats, Changes, Terminal, Checks, Todos, and Processes.
- Changes tab shows recent commits, git status, a file-level additions/deletions
  summary including untracked files, and the raw unified diff.
- Basic embedded terminal with PTY-backed shell controls plus presets for
  Conductor env, git status, diff, and file list.
- Processes tab lists embedded terminal shells alongside setup, run, and agent
  session processes, and terminal Start/Stop triggers workspace refresh so
  terminal process state appears immediately.
- Runtime controls for Setup, Run, Stop, and first-slice Spotlight On/Sync/Off.
  Runtime button failures surface as inline status text and app toasts.
- History page that reads prior Conductor chats/messages from the macOS
  Conductor database when available.
- Imported Conductor repositories/workspaces are visible in the app.

## Partial Or Rough

- The GTK layout is functional but not yet a close visual clone of Conductor.
- History is read-only and sourced directly from the macOS Conductor database;
  it is not yet a unified local chat model.
- Agent sessions now launch in the workspace page through PTY-backed local
  session panels with persisted transcripts, basic send/stop/checkpoint
  controls, harness metadata, provider/auth/MCP status, and best-effort
  reattach for still-running prior sessions. The chat surface is still
  transcript-oriented rather than a richer Conductor-style message UI with
  attachments.
- Embedded terminal support now has a PTY-backed shell, but the UI is still a
  transcript/input surface rather than a polished terminal emulator with
  cursor/session management. Terminal process records are created, stopped, and
  reconciled to exited on app startup and periodic refresh when their recorded
  shell PID is no longer alive. Active PTY shells are resized from the GTK
  terminal allocation. Each recorded shell gets a distinct log path, and PTY
  command/output chunks are appended to that raw transcript log. The visible
  transcript strips common ANSI/OSC escape sequences, applies carriage-return,
  backspace, cursor-up, cursor-left/right overwrite, saved-cursor restore, and
  erase-line plus clear-screen/home redraws, and caps on-screen scrollback while
  keeping persisted logs raw. It can search persisted transcript logs with
  one-line before/after context, list recorded terminal sessions/logs with
  status counts, line/byte counts, and last-output previews newest first, keep
  the transcript selector in the same newest-first order, load a selected past
  transcript into the terminal view, and restore the latest transcript into the
  terminal view after app restart.
  The terminal panel has clickable live-shell
  tabs so multiple live PTY shells can run, Stop Shell targets the selected tab,
  and stopping one tab automatically selects another running tab when available.
  Broader cursor/session emulation, richer terminal tab lifecycle controls, a more polished terminal history/scrollback
  browser are still missing.
- Spotlight support is manual checkpoint/apply/restore/switch/sync with
  dirty-root refusal before patch reversal, explicit destructive root repair,
  review-style affected-path dirty-root guidance in the Runtime panel,
  app-wide polling sync, and app-open recursive filesystem watching for active
  Spotlight workspace trees.
- Projects/workspace creation forms are basic text fields, not polished
  Conductor flows.
- Run/Stop and lifecycle buttons perform actions, and Runtime/lifecycle button
  failures have first-slice error toasts, but richer progress and automatic
  targeted refresh still need polish.
- Changes/Checks/Todos/Processes are basic panels, not full Conductor review
  surfaces. Local review comments can be added and resolved in the Review tab,
  and the Checks tab can view/stage raw PR comments/reviews, stage failing PR
  checks to an agent, show merge blockers, merge, and archive after merge.
  Inline diff anchoring, structured GitHub review-thread sync, and richer
  checks/deployment aggregation are still missing.
- Settings, command palette, keyboard shortcut, deep-link,
  Spotlight, monorepo, linked-directory, and rich History surfaces are missing
  or incomplete.

## Not Yet Built

- Exact Conductor UI parity.
- In-app chat threads for new Claude/Codex/Cursor sessions.
- Rich resumable chat threads inside the app.
- Live GitHub/Linear workspace creation verification with real credentials.
- Diff viewer with inline comments that can be sent back to an agent.
- Full checks tab with CI/deployment/GitHub comment/todo aggregation.
- Structured GitHub review-thread sync beyond first-slice PR controls.
- Polished PTY-backed embedded terminal panes.
- Project/repository settings editor.
- Agent status model comparable to Conductor's live session state.
- Command palette, shortcut coverage, and deep links.
- Polished Big Terminal Mode beyond the first full-width terminal tab/presets.
- Full Spotlight testing parity.
- Monorepo sparse-checkout controls and linked-directory workflows.
- Unified local history model.
- Safe confirmation flows for destructive actions.
- Polished packaging/release readiness.

## Current Foundation Definition

The current implementation is an early functional slice:

- Worktree-backed repository/workspace engine.
- CLI coverage for the backend workflow.
- GTK app with basic navigation and enough controls to create/select
  workspaces, launch agents externally, inspect changes/checks/history, and run
  lifecycle actions.

This is not the GUI-first MVP. The next work should prioritize turning the
rough GTK surfaces into the actual Conductor workflow experience instead of
adding more backend-only commands.
