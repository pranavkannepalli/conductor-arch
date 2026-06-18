# Progress

## Current State

This project has completed the Phase 0 documentation reset, Phase 1 app
architecture cleanup slice, Phase 2 project settings slice, and a usable Phase
3 Workspace Command Center slice. Phase 4 Embedded Runtime is now active. Phase
3 still lacks live connector proof for GitHub/Linear credentials, but the
source creation code path is no longer placeholder-only.

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
- Background setup scripts and logs.
- Background setup/run/session process rows now update to `exited` after
  natural process completion and record exit codes when the OS reports one.
- Stopped background processes now record signal-style exit code `143` and
  suppress expected `kill` fallback noise.
- Workspace-scoped terminal command execution with captured stdout, stderr, and
  exit code.
- PTY-backed shell session primitive using `portable-pty`, with input writes
  and streamed output reads.
- Terminal shell process records, so started/stopped PTY shells are visible in
  the existing process model with pid, status, timestamps, and stop exit code.
- First Spotlight testing slice: when enabled in repository settings, the app
  can apply a workspace's tracked changes to the repository root and later
  reverse that patch.
- Spotlight start now creates a checkpoint commit for the tracked workspace
  state before applying it to the repository root.
- Spotlight start can switch the active Spotlight workspace by restoring the
  previous workspace patch and applying the newly selected workspace patch.
- Spotlight sync can manually refresh the active root checkout from the active
  workspace's latest tracked changes and create another checkpoint.
- Shell/Codex/Claude/Cursor session launch primitives.
- Codex and Claude launches honor configured executable paths from repository
  settings.
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
  workspaces from branch/base, GitHub issue, GitHub PR, Linear issue, or
  prompt. GitHub-backed sources require working local `gh` auth. Linear-backed
  sources require `LINEAR_API_KEY`.
- Workspace page has a command center layout with status header, agents panel,
  runtime panel, changes/checks/review tabs, chat/terminal split, todos,
  processes, and lifecycle controls.
- Workspace page has a basic embedded command terminal and a larger Terminal
  tab with presets for Conductor environment, git status, diff, and file list.
- Terminal panels can start/stop a PTY-backed workspace shell. Presets and
  typed commands go through the PTY while a shell is active, with the previous
  one-shot command path still used when no PTY is running.
- Terminal shell records now appear in the workspace Processes tab, so the app
  no longer treats embedded PTY shells as invisible runtime state.
- Runtime panel can start setup/run scripts, stop run scripts, and show latest
  setup/run log tails.
- Runtime panel can start/sync/stop the first Spotlight testing slice and show
  the active Spotlight patch/status.
- History page can read old chats from the macOS Conductor database when
  available.

## What Is Not Done

The actual GUI-first Conductor MVP is not complete.

MVP-critical missing work:

- Embedded Conductor-native Claude/Codex/Cursor chat.
- Polished PTY terminal UX. A real PTY-backed workspace shell now exists, but
  it is still a basic transcript/input surface, not a terminal emulator with
  cursor control, resize handling, scrollback management, or multiple terminal
  sessions.
- Polished Big Terminal Mode. Current Terminal tab is the first full-width
  direction/preset slice.
- More polished project settings/onboarding layout.
- Monorepo directory selection and linked-directory workflows.
- GUI-visible MCP status. The core can inspect known MCP config files, but the
  GUI does not expose or validate live MCP reachability yet.
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
- Codex and Claude session launch now uses configured provider executable paths
  when present.
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

## Phase 3 Workspace Command Center Status

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
  GitHub PR, Linear issue, or prompt while reusing current core workspace APIs.
  GitHub PR creation fetches the PR head ref before creating the worktree.
  GitHub issue, GitHub PR, Linear issue, and prompt creation write source
  context into `.context/brief.md`. Linear creation calls Linear's API through
  `LINEAR_API_KEY`; without that key it fails with a visible error instead of
  creating a fake workspace.

Still needs Phase 3 proof:

- Manual or automated GTK click-through for source creation and lifecycle
  controls.
- Live GitHub issue/PR verification after `gh auth login`.
- Live Linear issue verification after `LINEAR_API_KEY` is configured.

Verified Phase 3 evidence so far:

- Core tests cover prompt, GitHub issue, and GitHub PR workspace source
  creation. GitHub tests use fake `gh` output plus a real local git remote and
  PR head ref.
- Full workspace tests pass.
- CLI prompt creation smoke created a real worktree and wrote the prompt to
  `.context/brief.md`.
- CLI Linear creation without `LINEAR_API_KEY` fails clearly before creating a
  fake workspace.
- GTK app launches on `DISPLAY=:0` and stays alive until a 5-second smoke-test
  timeout with isolated XDG state.

## Phase 4 Embedded Runtime Status

- Core now has a `terminal_command` API that runs arbitrary shell commands from
  the workspace directory with Conductor environment variables and returns
  stdout, stderr, timestamps, and exit code.
- Core now has a PTY session primitive that starts a shell in a workspace,
  accepts input after spawn, and streams output back to the app.
- Core now records PTY terminal shells as process rows and can mark them stopped
  with the same signal-style exit code used by other stopped runtime processes.
- Core now tracks setup script runs as a separate process kind and can read the
  latest setup log.
- GTK terminal panels now execute real workspace commands asynchronously instead
  of queuing placeholder text.
- GTK terminal panels now have Start Shell/Stop Shell controls for a PTY-backed
  workspace shell.
- GTK terminal panels create terminal process records on Start Shell and mark
  them stopped on Stop Shell or panel teardown.
- GTK terminal presets expose `CONDUCTOR_*` environment, git status, git diff,
  and a short file list; when a PTY shell is active, presets are sent into that
  shell.
- Workspace tabs now include a full-width Terminal tab as the first Big
  Terminal Mode direction slice.
- Runtime panel now has Setup, Run, Stop, Open Folder actions plus latest
  setup/run process and log previews.
- Process rows and runtime summaries show exit codes after background processes
  exit naturally.
- Core can start/stop a Spotlight session when `spotlight_testing = true`,
  requiring a clean repository root, applying the workspace's tracked patch to
  the root checkout, recording the patch, creating a workspace checkpoint
  commit, and reversing the patch on stop.
- Starting Spotlight for a different workspace in the same repository now
  replaces the root checkout state with that workspace's tracked changes.
- GTK Runtime now exposes Spotlight On/Sync/Off controls and active status.

Still needs Phase 4 work:

- Terminal emulator polish: resize events, cursor/ANSI handling beyond raw text
  transcript, multiple terminal sessions, persisted terminal history, and
  stronger automatic reconciliation for PTY processes if the whole app crashes.
- Full Spotlight parity: automatic file watching, automatic repeated checkpoint
  sync after changes, and stronger root dirty-state recovery. Current support
  is manual checkpoint/apply/restore/switch/sync of tracked changes.
- Toasts and richer error/progress state.

## Next Step

Do not continue adding backend-only commands unless they unblock the GUI-first
MVP.

Recommended next work:

1. Keep local docs aligned with the official Conductor docs before adding
   better-than-Conductor product ideas.
2. Continue polishing project onboarding and settings validation.
3. Finish live Phase 3 connector proof when `gh` auth and `LINEAR_API_KEY` are
   available.
4. Continue Phase 4 toward a true PTY terminal and Spotlight testing.
5. Start Phase 5 only after the runtime base can support app-native
   Claude/Codex/Cursor session streams.
