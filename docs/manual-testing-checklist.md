# Linux Conductor Manual Testing Checklist

This checklist has two jobs:

1. Define the acceptance path for the corrected GUI-first Conductor MVP.
2. Smoke-test the current foundation/prototype while that MVP is unfinished.

The current app is not expected to pass the target MVP acceptance section yet.
For the full target spec, see
[`docs/conductor-gui-mvp-handoff.md`](conductor-gui-mvp-handoff.md).

Use this checklist on a machine with `git`, `gh`, Rust, GTK4, and libadwaita
development packages installed. Run `gh auth login` before GitHub tests. Set
`LINEAR_API_KEY` before Linear issue tests.

## Build And Launch

- [ ] `cargo build --workspace --release --locked`
- [ ] `./target/release/linux-conductor doctor` prints distro guidance.
- [ ] `./target/release/linux-conductor-gtk` opens the GTK app.

## Target GUI-First MVP Acceptance

These checks describe the product that must exist before calling the MVP done.
They are intentionally stricter than the current prototype smoke path.

- [ ] A user can add an existing repository from the GUI.
- [ ] A user can clone a Git repository from the GUI.
- [ ] The app detects the default/base branch and fetches the latest remote
  before workspace creation.
- [ ] A user can create a workspace from the GUI with branch/base options.
- [ ] A user can create a workspace from a branch, pull request, GitHub issue,
  Linear issue, or prompt.
- [ ] GitHub issue and pull request workspace creation fails clearly when
  `gh auth status` is not authenticated.
- [ ] GitHub pull request workspace creation fetches the PR head ref before
  creating the worktree.
- [ ] Linear issue workspace creation fails clearly when `LINEAR_API_KEY` is
  missing, and creates from real Linear issue data when it is present.
- [ ] Prompt workspace creation writes the prompt into `.context/brief.md`.
- [ ] Workspace creation creates a real Git worktree and maps one workspace to
  one branch.
- [ ] The app prevents or explains branch/worktree conflicts when a branch is
  already checked out elsewhere.
- [ ] Workspace names are durable friendly identifiers while branch/issue/PR
  remains the primary work label.
- [ ] The workspace page shows project, repository, branch, path, run state,
  active agent sessions, PR/check state, changed files, todos, comments, and
  conflicts.
- [ ] Workspace page shows ports, process state, logs, terminal state, and
  current `CONDUCTOR_*` context.
- [ ] The app supports monorepo directory selection or clearly marks it as a
  remaining MVP blocker.
- [ ] The app supports linked workspace/directory context for multi-repository
  work or clearly marks it as a remaining MVP blocker.
- [ ] Claude Code, Codex, and Cursor sessions launch from the workspace page and
  appear as app-native session rows.
- [ ] Agent output is readable in a Conductor-native chat surface, not only in
  external terminals.
- [ ] Chat composer supports attachments or selected review/comment context.
- [ ] Multiple agent sessions can run in one workspace.
- [ ] Multiple workspaces can run sessions in parallel.
- [ ] Supported harness controls are visible where applicable: Plan Mode, Fast
  Mode, reasoning/effort, Codex personality, Codex goals, checkpoints, skills,
  and tool approvals.
- [ ] Provider/auth status is visible for Claude Code, Codex, and Cursor.
- [ ] MCP status is visible for Claude Code and Codex, with Cursor MCP status
  delegated/documented correctly.
- [ ] The workspace page includes an embedded terminal scoped to that workspace.
- [ ] Big Terminal Mode or equivalent full-center terminal workflow is present
  or explicitly deferred as a known MVP gap.
- [ ] Setup, run, and stop actions are available from the GUI and stream logs.
- [ ] The app shows active processes, ports, exit codes, and
  `CONDUCTOR_*` environment context.
- [ ] `scripts.run_mode = "concurrent"` and `"nonconcurrent"` behavior is
  honored.
- [ ] Spotlight testing is configurable for projects that must run from the
  repository root.
- [ ] The changes/review view shows changed files with additions/deletions,
  git status, recent commits, and a usable unified or side-by-side diff.
- [ ] Inline review comments can be added and sent back to a selected agent.
- [ ] GitHub review comments and local comments appear in the review/checks
  flow where possible.
- [ ] Resolved comments can be marked resolved from the GUI.
- [ ] Selected files can be reverted from the review flow where safe.
- [ ] The app detects sibling workspaces that changed the same files.
- [ ] The GitHub workflow is usable from the GUI: push branch, create PR, view
  PR state, show checks, surface failures, show comments/reviews, merge PR, and
  archive after merge.
- [ ] Failing checks, unresolved comments, open todos, and conflicts can be sent
  to an agent with the relevant context.
- [x] Project settings can be viewed and edited from the GUI: setup/run/archive
  scripts, run mode, environment variables, files to copy/include globs, and
  durable prompts/instructions.
- [ ] Project settings display the full settings layer precedence: managed,
  local project override, repository shared, user shared, built-in defaults.
  The GUI currently edits local/shared repository layers and shows whether each
  file exists.
- [x] `.worktreeinclude` takes precedence over Files to copy settings and is
  shown read-only when present.
- [x] Shared settings do not encourage committing secrets; local overrides are
  used for machine-specific secrets.
- [ ] Provider executable/provider fields and action prompts are discoverable
  from Projects settings. Codex and Claude executable fields affect session
  launch. User-only model defaults still need the Settings page.
- [ ] Todos are editable from the GUI and sync with `.context`.
- [ ] Open todos, unresolved comments, failed checks, and conflicts block or
  strongly discourage merge until resolved or explicitly overridden.
- [ ] Archive, restore, discard, and rename actions have confirmations,
  progress, error states, and targeted refresh.
- [ ] History shows archived workspaces and old chats from the app's unified
  local history model.
- [ ] Archived workspaces can be inspected and restored from History.
- [ ] Command palette covers add repository, create workspace, open workspace,
  run/stop, show diff, show checks, create PR, merge PR, archive, and settings.
- [ ] Keyboard shortcuts cover the core navigation, workspace, chat, terminal,
  review, Git, and settings flows.
- [ ] Deep links are implemented or architecturally ready for prompt,
  prompt-plus-path, issue, and async-plan flows.
- [ ] Security/privacy UI states that agents run locally with user permissions,
  approvals can gate risky actions, model traffic goes to configured providers,
  and enterprise data privacy disables external-AI/custom-MCP behavior where
  applicable.
- [ ] The visual design is close enough to Conductor for the core workflow to
  feel like a desktop control plane rather than a CLI dashboard.

## Current Prototype Smoke Path

The remaining sections validate what exists today. Passing them means the
foundation is healthy; it does not mean the GUI-first MVP is complete.

## CLI Foundation

- [ ] Add a test repository:
  `linux-conductor repo add <repo-path> --name demo`
- [ ] Create two workspaces:
  `linux-conductor workspace create demo --name berlin --branch lc/berlin-demo`
  `linux-conductor workspace create demo --name tokyo --branch lc/tokyo-demo`
- [ ] Confirm each workspace has `.context/` and a unique port block:
  `linux-conductor workspace list`
- [ ] Open interactive shell, Codex, and Claude Code sessions:
  `linux-conductor session open berlin --kind shell`
  `linux-conductor session open berlin --kind codex`
  `linux-conductor session open tokyo --kind claude`
- [ ] Print a manual session command for fallback terminal testing:
  `linux-conductor session open berlin --kind codex --print-command`
- [ ] Start and stop a supervised background shell session:
  `linux-conductor session start berlin --kind shell`
  `linux-conductor session list berlin`
  `linux-conductor session stop berlin`
- [ ] Run and stop the repository run script:
  `linux-conductor run berlin`
  `linux-conductor logs berlin --run`
  `linux-conductor stop berlin`
- [ ] Make a small committed change in `berlin`, then confirm:
  `linux-conductor diff berlin`
  `linux-conductor checks berlin`
- [ ] Add and complete a todo:
  `linux-conductor todo add berlin "manual smoke todo"`
  `linux-conductor todo list berlin`
  `linux-conductor todo done <id>`
- [ ] Create and inspect a checkpoint:
  `linux-conductor checkpoint create berlin "manual smoke checkpoint"`
  `linux-conductor checkpoint list berlin`
- [ ] Create a PR from the workspace branch:
  `linux-conductor pr create berlin --title "manual smoke" --body "Manual MVP smoke test"`
  `linux-conductor pr view berlin`
  `linux-conductor pr checks berlin`
- [ ] Archive or discard the workspace:
  `linux-conductor workspace archive berlin`
  `linux-conductor workspace restore berlin`
  `linux-conductor discard tokyo`

## GTK Prototype

- [ ] Sidebar shows Dashboard, History, repository grouping, workspace rows, and
  search.
- [ ] Dashboard shows project tabs and Backlog, In progress, In review, and Done
  columns.
- [ ] Projects page can list registered repos.
- [ ] Projects page can create a workspace from a registered repo.
- [ ] Projects page can create a prompt workspace and the generated
  `.context/brief.md` contains the prompt.
- [ ] GitHub issue/PR workspace creation reports local `gh` auth errors when
  the host is not authenticated.
- [ ] Linear issue workspace creation reports missing `LINEAR_API_KEY` instead
  of creating a fake workspace.
- [ ] Workspace page opens when selecting a workspace.
- [ ] Workspace page shows metadata and rough tabs for Chats, Changes, Checks,
  Todos, and Processes.
- [ ] Workspace page buttons can launch Shell, Codex, Claude Code, and Cursor
  through the current external-process path.
- [ ] Setup, Run, and Stop buttons call the current runtime process APIs, show
  latest setup/run log previews, and show an app toast when the button action
  fails.
- [ ] A short setup/run script that exits naturally changes from running to
  exited and shows its exit code in runtime/process views.
- [ ] Terminal tab runs a short one-shot command, shows stdout/stderr, and
  reports the exit code without freezing the app.
- [ ] Terminal tab can start a workspace shell, accept typed input after the
  shell starts, stream output, and stop the shell.
- [ ] Terminal tab can start multiple PTY shells, select the active shell, send
  input to the selected shell, and stop only the selected shell.
- [ ] Stopping one selected shell leaves its tab visible as stopped and
  auto-selects another running shell when one exists.
- [ ] Processes tab shows the embedded terminal shell as running after Start
  Shell and stopped with exit code `143` after Stop Shell without waiting for
  the global polling refresh.
- [ ] Starting more than one embedded shell records distinct terminal log paths
  in the Processes tab instead of reusing one shared terminal log.
- [ ] PTY shell command echoes and output appear in that terminal process log,
  not only in the on-screen transcript.
- [ ] Terminal history search finds matching persisted transcript lines and
  appends process/line matches with one-line before/after context into the
  terminal transcript.
- [ ] Terminal Show History lists recorded terminal sessions with status, pid,
  exit code, log file, start time, line/byte counts, last-output preview, and
  command.
- [ ] Terminal Show History displays running/stopped/exited counts and orders
  terminal sessions newest first.
- [ ] Load Transcript selector uses the same newest-first ordering as the
  rendered history list.
- [ ] Selecting a recorded terminal session and clicking Load Transcript
  replaces the terminal view with that session's persisted transcript.
- [ ] Reopening the GTK app after terminal output exists restores the latest
  terminal transcript into the terminal view.
- [ ] Terminal commands that emit ANSI color/control sequences show readable
  text in the GTK transcript instead of raw escape codes.
- [ ] Terminal commands that redraw progress with carriage returns show the
  latest progress line instead of raw `\r` artifacts.
- [ ] Terminal commands that redraw a spinner with backspaces show the latest
  spinner character instead of raw backspace artifacts.
- [ ] Terminal commands that redraw one line using cursor-up plus clear-line
  sequences show the latest line instead of stale duplicate lines.
- [ ] Terminal commands that save and restore the cursor position overwrite at
  the restored location instead of appending at the end.
- [ ] Terminal commands that use erase-line modes clear the intended part of
  the visible transcript line instead of appending stale text.
- [ ] Terminal commands that clear the screen and move the cursor home show only
  the fresh screen contents instead of stale previous output.
- [ ] Resizing the GTK terminal area updates the active PTY shell size; `stty
  size` from the shell reflects the resized grid.
- [ ] Long terminal output caps the on-screen transcript with a scrollback
  trimmed marker while the persisted raw terminal log remains complete.
- [ ] If an embedded terminal shell exits outside the app, the Processes tab
  eventually marks its terminal row exited after the app-wide reconciliation
  poller runs.
- [ ] If the app starts with stale terminal process rows from an earlier crash,
  those rows are marked exited during startup reconciliation.
- [ ] Terminal presets show `CONDUCTOR_*` env, git status, diff, and file list.
- [ ] With `spotlight_testing = true`, Spotlight On applies tracked workspace
  changes to a clean repository root and Spotlight Off restores the root.
- [ ] Spotlight On creates a checkpoint entry for the tracked workspace state.
- [ ] Starting Spotlight for a second workspace restores the first workspace's
  root patch and applies the second workspace's tracked changes.
- [ ] After editing tracked files in the active workspace, Spotlight Sync
  refreshes the root patch and creates another checkpoint.
- [ ] With that workspace page open, editing tracked files in the active
  Spotlight workspace eventually auto-syncs the root patch.
- [ ] After navigating away from the workspace page, active Spotlight sessions
  still auto-sync changed tracked patches through the app-wide poller.
- [ ] Runtime Spotlight status shows `root clean` when the active root matches
  the active Spotlight patch.
- [ ] Runtime Spotlight status shows root-only affected paths after extra root
  edits exist before clicking Spotlight Off/Sync.
- [ ] If the repository root has extra edits outside the active Spotlight patch,
  Spotlight Off/Sync fails without marking the session stopped and the Runtime
  status prioritizes root-only affected paths and warns that Repair Spotlight
  discards root-only edits while the toast points to Repair Spotlight or manual
  clean/save.
- [ ] Repair Spotlight explicitly discards root-only edits, reapplies the active
  Spotlight patch, and then Spotlight Off can restore the clean root.
- [ ] Archive, Restore, and Discard buttons call the current lifecycle APIs and
  show an app toast if the action fails.
- [ ] History page lists old Conductor chats if the macOS Conductor database is
  available.
- [ ] `Ctrl+R` refreshes the visible workspace state.

## Known GUI MVP Gaps To Keep Visible

- [ ] Embedded Conductor-native agent chat is not implemented.
- [ ] Polished PTY terminal emulation is not implemented. The current terminal
  has a PTY-backed shell but still renders as transcript text with basic
  ANSI-control stripping, carriage-return/backspace/cursor-up/cursor-left/right
  line redraws, saved-cursor restore, erase-line redraws, clear-screen/home
  redraws, and resize propagation; stale process rows reconcile at startup and while the
  app is open, and shell records get distinct searchable raw transcript logs with
  capped on-screen scrollback, latest-transcript restore, and clickable
  live-shell tabs, but this is not a full emulator/live PTY reattach model, full
  terminal tab model, or polished terminal history/scrollback browser.
- [ ] Full Spotlight parity is not implemented. The current slice manually
  checkpoints/applies/restores/switches/syncs tracked changes plus app-wide
  polling sync, app-open recursive file watching for active Spotlight workspace
  trees, and explicit destructive root repair, but it is not full
  Conductor-level Spotlight parity.
- [ ] Project settings editor is functional but still needs polish, validation,
  full user/managed layer visibility, and user-only model defaults.
- [ ] Monorepo directory selection, linked-directory workflows, and MCP status
  are not fully implemented.
- [ ] Rich diff viewer with inline comments is not implemented.
- [ ] GitHub PR/check/merge workflow is not fully available from the GUI.
- [ ] Command palette, shortcuts, deep links, polished Big Terminal Mode,
  monorepo controls, linked directories, and unified local history are not
  complete.
- [ ] Visual parity with Conductor is not complete.

## Packaging Smoke

- [ ] `VERSION=0.1.0 nfpm package --packager deb --target dist/`
- [ ] `VERSION=0.1.0 nfpm package --packager rpm --target dist/`
- [ ] AppImage launches GUI with no args and CLI with args:
  `./dist/linux-conductor-0.1.0-x86_64.AppImage`
  `./dist/linux-conductor-0.1.0-x86_64.AppImage doctor`
- [ ] Flatpak manifest builds or its failure is documented as a known sandbox or
  dependency limitation.
