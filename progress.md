# Progress

## Current State

This project has completed the Phase 0 documentation reset, Phase 1 app
architecture cleanup slice, Phase 2 project settings slice, and a usable Phase
3 Workspace Command Center slice. Phase 4 Embedded Runtime, Phase 5 Agent
Sessions, Phase 6 Git/Diff/Review, and a first Phase 7 GitHub Workflow slice
are now in place for the GUI-first MVP path. Phase 3 still lacks live connector proof for
GitHub/Linear credentials, but the source creation code path is no longer
placeholder-only.

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

Latest verification on 2026-06-20: `cargo test` passes across the CLI, core,
GTK, and doctest suites.

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
- Core can skip no-op Spotlight syncs and sync only when the active workspace
  patch differs from the active root patch.
- Core can scan all active Spotlight sessions and sync only the sessions whose
  workspace patch changed.
- Spotlight stop/sync/switch now refuse to reverse the active patch when the
  repository root has extra edits outside that patch.
- Shell/Codex/Claude/Cursor session launch primitives.
- Codex and Claude launches honor configured executable paths from repository
  settings.
- Git status/diff/log helpers.
- Todo, review comment, checkpoint, conflict, and checks-summary commands.
- GitHub PR create/view/checks/merge through local `gh` auth.
- PR merge now blocks open todos and open local review comments, and can archive
  the workspace after merge when repository Git settings enable it.
- PR checks output can be parsed into failing-check prompts for agents.
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
- Agent panel can start PTY-backed Shell, Codex, Claude, and Cursor sessions,
  persist transcripts, send input, stop selected sessions, create checkpoints,
  show harness metadata, surface provider/auth/MCP status, and stage open
  review comments into the selected live session.
- Changes tab now has a selectable changed-file tree, per-file unified diff
  preview, full-diff fallback, file-scoped inline comments, recent commits,
  branch push state, git status, and safe tracked-file revert.
- Review tab can add local file/line comments, resolve open comments, and stage
  open comments for the selected agent session.
- Checks tab can create/refresh PR state, inspect raw PR checks and PR
  comments/reviews, stage failing checks or PR comments/reviews for the
  selected agent session, show merge blockers, merge a PR, and archive after
  merge through the existing repository setting.
- History page can read old chats from the macOS Conductor database when
  available.

## What Is Not Done

The actual GUI-first Conductor MVP is not complete.

MVP-critical missing work:

- Polished Conductor-native Claude/Codex/Cursor chat. A PTY-backed app-native
  session surface exists, but it is still transcript-oriented rather than a
  rich structured chat surface.
- Polished PTY terminal UX. A real PTY-backed workspace shell now exists, but
  it is still largely transcript-oriented, not a full terminal emulator with
  richer cursor-state/session emulation and polished history/scrollback browsing.
- Polished Big Terminal Mode. Current Terminal tab is the first full-width
  direction/preset slice.
- More polished project settings/onboarding layout.
- Monorepo directory selection and linked-directory workflows.
- Fully polished repository/workspace creation flows.
- Rich diff/review/comment surface beyond the current file tree, unified diff,
  local inline comments, comment staging, and safe tracked-file revert.
- Structured GitHub review-thread sync and richer PR/check/deployment
  aggregation.
- Command palette, keyboard shortcuts, and deep links.
- Monorepo sparse-checkout controls and linked-directory workflows.
- Rich message rendering with attachments and stronger in-app session history.
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
- Lifecycle action failures now show both inline status text and app toasts.
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
- Core PTY sessions can resize the child terminal.
- Core now records PTY terminal shells as process rows and can mark them stopped
  with the same signal-style exit code used by other stopped runtime processes.
- Core now gives each recorded PTY terminal shell its own log file instead of
  sharing one `terminal-active.log`, which prevents multiple shell records from
  pointing at the same transcript path.
- Core can search persisted terminal transcript logs for a workspace and return
  matching process ids, log paths, line numbers, and lines.
- Core can reconcile terminal process rows whose recorded PTY shell PID is no
  longer alive, marking those stale rows exited instead of leaving them running.
- Core now tracks setup script runs as a separate process kind and can read the
  latest setup log.
- GTK terminal panels now execute real workspace commands asynchronously instead
  of queuing placeholder text.
- GTK terminal panels now have Start Shell/Stop Shell controls for a PTY-backed
  workspace shell.
- GTK terminal panels now show clickable shell tabs for live PTY shells, with
  running/stopped labels; command input and Stop Shell target the selected tab.
- GTK terminal panels add a Close Shell control to remove closed tabs after stopping
  shell processes and refreshes the active tab/session strip accordingly.
- Stop Shell now keeps the stopped tab visible but automatically selects the
  next running shell tab when one exists, so follow-up commands do not stay
  pointed at a stopped shell.
- GTK terminal panels propagate terminal view size changes to the active PTY
  shell, so child processes can observe the resized row/column grid.
- GTK terminal panels create terminal process records on Start Shell and mark
  them stopped on Stop Shell or panel teardown.
- GTK terminal Start Shell/Stop Shell now trigger a workspace refresh so the
  Processes tab picks up terminal process state immediately instead of waiting
  for polling/manual refresh.
- GTK terminal panels append PTY command echoes and output chunks into the
  terminal process log, so recorded shell logs contain a usable raw transcript
  instead of only an empty placeholder file.
- GTK terminal panels expose a basic terminal history search field that searches
  persisted transcript logs and appends matching process/line results with
  one-line before/after context into the terminal transcript.
- GTK terminal panels expose a basic Show History control that lists recorded
  terminal sessions, status, pid, exit code, log file, start time, and command.
- GTK terminal history now shows session counts by status and sorts listed
  sessions newest first.
- GTK terminal history selector now uses the same newest-first ordering as the
  displayed history list, so Load Transcript defaults to the newest session.
- GTK terminal history now fills a session selector and can load the selected
  persisted transcript into the terminal view.
- Core can read the latest terminal transcript, and GTK terminal panels restore
  that latest transcript into the initial terminal view after app restart.
- GTK terminal display now strips common ANSI/OSC escape sequences and applies
  carriage-return progress-line updates in the visible transcript while keeping
  the persisted terminal logs raw.
- GTK terminal display now handles backspace redraws for spinner-style terminal
  output that rewrites the previous character.
- GTK terminal display now handles simple cursor-up plus clear-line redraws for
  common progress/status output that rewrites the previous line.
- GTK terminal display now handles cursor-left/right inline overwrites for
  terminal output that rewrites characters within the current line.
- GTK terminal display now handles CSI saved-cursor restore for terminal output
  that returns to an earlier cursor position before writing more text.
- GTK terminal display now handles erase-line modes for terminal output that
  clears part or all of the current line before redrawing it.
- GTK terminal display now handles full-screen redraws that clear the visible
  screen and move the cursor home before writing fresh output.
- GTK terminal display caps the on-screen scrollback and shows a trim marker
  while preserving complete raw terminal transcript logs on disk.
- GTK app shell reconciles stale terminal process records once during startup
  and then periodically while the app is open, so crashed or externally-ended
  shells stop showing as running without waiting for a manual refresh.
- GTK terminal presets expose `CONDUCTOR_*` environment, git status, git diff,
  and a short file list; when a PTY shell is active, presets are sent into that
  shell.
- Workspace tabs now include a full-width Terminal tab as the first Big
  Terminal Mode direction slice.
- Runtime panel now has Setup, Run, Stop, Open Folder actions plus latest
  setup/run process and log previews.
- Runtime panel button failures for Setup, Run, Stop, and Spotlight controls now
  show both inline status text and app toasts.
- Spotlight dirty-root Runtime failures now show targeted guidance that points
  to Repair Spotlight or manually cleaning/saving root changes instead of only
  echoing the raw backend error. Core dirty-root failures also include the
  changed root paths when Git can identify them, prioritizing root-only paths
  over active Spotlight patch paths, and GTK shows those affected paths as a
  review-style inline list with an explicit warning that Repair Spotlight
  discards root-only edits.
- Runtime Spotlight status now passively shows `root clean` or root-only extra
  edit paths for the active Spotlight root before the user clicks Stop/Sync.
- Terminal history now uses core-provided session summaries so Show History
  includes line counts, byte counts, and last-output previews alongside status
  counts and newest-first ordering.
- Changes tab now includes a file-level diff summary with additions/deletions
  and untracked-file counts before the raw unified diff.
- Changes tab now has a first-slice file tree with selectable per-file diff
  preview, a full-diff fallback, recent commits, branch push state, and safe
  revert for tracked files back to `HEAD`.
- The selected diff file now shows file-scoped inline comments in the same
  review area and can add another file/line comment without switching tabs.
- Open review comments can now be converted into an agent-ready staged review
  prompt from the agent panel; this is not live bidirectional agent chat yet.
- Local review comments now render as actionable rows in the GTK Review tab,
  can be added from the GUI with file/line/body fields, and open comments can be
  marked resolved from the GUI.
- Review comments can now be staged from the Review tab and sent to the
  selected live agent session from the session surface without leaving the app.
- Checks tab now has a first-slice PR creation form with title/body/draft
  fields wired to the existing core GitHub PR creation flow.
- Checks tab now exposes first-slice PR merge controls with squash/merge/rebase
  methods wired to the existing core GitHub merge flow and todo-blocking guard.
- Checks tab now exposes first-slice PR state refresh and raw `gh pr checks`
  output actions through the existing core GitHub view/check paths.
- Process rows and runtime summaries show exit codes after background processes
  exit naturally.
- Core can start/stop a Spotlight session when `spotlight_testing = true`,
  requiring a clean repository root, applying the workspace's tracked patch to
  the root checkout, recording the patch, creating a workspace checkpoint
  commit, and reversing the patch on stop.
- Starting Spotlight for a different workspace in the same repository now
  replaces the root checkout state with that workspace's tracked changes.
- Spotlight state changes now verify the root diff still matches the active
  Spotlight patch before reversing it, protecting root-only edits from being
  silently mixed into Spotlight cleanup.
- Core can explicitly repair an active Spotlight root by resetting/cleaning the
  repository root and reapplying the stored active Spotlight patch.
- GTK Runtime now exposes Spotlight On/Sync/Repair/Off controls and active
  status.
- GTK Runtime polls the selected active Spotlight workspace and auto-syncs when
  its tracked patch changes while that workspace page is open.
- GTK app shell also polls all active Spotlight sessions, so active workspaces
  can keep syncing while the user is on another page.
- Core now exposes active Spotlight watch targets, and the GTK app installs
  recursive filesystem watchers for active Spotlight workspace trees while the
  app is open. File events trigger the same active-session sync path, with
  polling still kept as fallback and target refresh.

Still needs Phase 4 work:

- Conductor-level parity around every checkpoint/watch edge case. Current
  support is manual checkpoint/apply/restore/switch/sync plus event-triggered
  and app-wide polling sync of tracked changes, dirty-root refusal before patch
  reversal, review-style affected-path conflict details, app-closed/active
  focus reconciliation, app background handling via active-state notifications,
  and an explicit destructive root repair action.

## Phase 5 Agent Sessions Done

- Workspace agent panel now starts PTY-backed Shell, Codex, Claude, and Cursor
  sessions inside the selected workspace and records them as session processes.
- Session transcripts persist to per-session logs, remain selectable in the
  app, and show readable app-native transcript output instead of requiring a
  separate terminal window.
- The session panel supports send, stop, and checkpoint actions and derives
  idle/working/waiting/detached/done/errored state from the recorded process
  plus recent output activity.
- Harness controls are visible where supported: Plan Mode, Fast Mode,
  approvals, reasoning, effort, Codex personality, Codex goals, and Codex
  skills.
- Launch metadata for those harness controls is stored with the session process
  record and shown for the selected session.
- Provider/auth/MCP status is visible above the session panel for Claude,
  Codex, and Cursor, including executable/auth presence and the note that
  Cursor MCP is managed by Cursor.
- Multiple sessions per workspace are supported through the session selector.
- The selected agent session now persists in app state across workspace page
  refreshes and revisits instead of resetting to the newest session each time.
- Running sessions from a previous app run can now be reattached when their
  PTY device is still available; otherwise they remain visible as detached
  saved transcripts.
- Verification: `cargo test` passes. This covers core session launch/process
  behavior, PTY input/output, GTK staged-review/session status helpers, and the
  broader GTK build. Manual live auth/provider smoke for Claude, Codex, and
  Cursor still depends on those tools being installed and authenticated locally.

## Phase 6 Git Diff Review Done

- The Changes tab now shows a selectable changed-file list instead of only a
  raw monolithic transcript dump.
- Selecting a changed file loads that file's unified diff; Show All restores
  the full workspace diff.
- The Changes tab also exposes recent commits, current branch push state, and
  current git status alongside the diff review flow.
- Tracked changed files can now be reverted from the GUI back to `HEAD`; the
  app refuses untracked-file revert attempts instead of pretending they are safe.
- The Review tab still owns local file/line comments and comment resolution.
- Open review comments can now be staged and sent into the selected live agent
  session from the app-native session surface.
- Sibling workspace conflict detection and copy/diff actions remain available
  in the Checks tab for overlapping files.
- Verification: `cargo test` passes. This covers changed files, unified diffs,
  file summaries, review comment prompts, safe tracked-file revert, conflict
  detection, GTK diff-tree/comment rendering helpers, and terminal transcript
  rendering used by the review/runtime surfaces. Live GitHub review comment
  aggregation remains Phase 7 work.

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
5. Start Phase 7 GitHub workflow polish instead of adding more backend-only
   commands.
