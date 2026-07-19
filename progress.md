# Progress

Current as of 2026-07-16.

## Current State

Archductor has a usable but rough GUI-first loop for one local repository:

1. Add or clone a repository as a project.
2. Edit app Shared defaults, repository-committed settings, and Local project
   overrides.
3. Create branch, prompt, GitHub issue, GitHub PR, or Linear workspaces.
4. Run setup/run scripts, terminal commands, and Shell/Codex/Claude/Cursor
   sessions inside a workspace.
5. Review diffs, todos, local comments, sibling conflicts, GitHub PR checks,
   PR comments, review threads, deployments, and merge blockers.
6. Stage review/check/comment/readiness context into the selected agent session.
7. Create, refresh, merge, and optionally archive GitHub PRs through local
   `gh` auth.
8. Restore archived workspaces and inspect saved Linux session history.

The app is not MVP-complete. Treat it as a working prototype with real product
paths and known rough edges.

## Implemented Surfaces

### Core And CLI

- SQLite-backed project, workspace, process, PR, todo, review, checkpoint,
  timeline, chat, and history state.
- Repository add/list/update/doctor plus shared/local settings import/export.
- App Shared settings import/export and effective settings precedence from
  built-in defaults through app Shared, repository-committed settings
  (including prompt packs), and Local project overrides.
- Workspace create/list/archive/restore/discard/delete/rename/duplicate.
- Workspace creation from branch/base, prompt, GitHub issue, GitHub PR, and
  Linear issue.
- Git worktree creation with `.context` initialization and stable
  per-workspace port allocation.
- Workspace branch create/checkout/rename/delete.
- Workspace timeline records for creation, duplication, branch changes, session
  lifecycle, PR/check actions, archive, restore, delete, and related events.
- `.worktreeinclude` precedence over settings file-copy globs.
- Repository settings for scripts, prompts, prompt pack metadata, environment,
  provider paths, Git behavior, merge rules, workspace defaults, view defaults,
  terminal preferences, keybindings, notification labels, and advanced TOML.
- Prompt pack files are bootstrapped under `.archductor/prompt-packs`.
- Effective prompt routing for first-chat General instructions, continue work,
  PR creation, commit/push, blocker resolution, setup/run assistants, code
  review staging, and local/PR check fixing. Shared/Local prompt saves refresh
  managed prompt snapshots for existing live workspaces.
- Monorepo working-directory defaults for scripts, terminal commands, editor
  launches, and agent sessions.
- Linked workspace directories with persisted records, symlinks under
  `.context/linked-directories`, and `ARCHDUCTOR_LINKED_*` environment.
- PTY-backed shell/session primitives, transcript logs, provider events, and
  stale-process reconciliation.
- Shell, Codex, and Claude CLI session commands. Cursor is available from GTK
  launch paths where configured, not from the current CLI `session --kind`
  enum.
- Immediate Codex delivery from GTK Ctrl+Enter and CLI `session send`/`archcar
  send --immediate`, using active-turn steer with transparent new-turn fallback.
- Archcar-managed Claude Code stream-json sessions with local auth readiness,
  persistent input delivery, resumable controls, process-group interrupt,
  native hook settings, provider interaction records, and common CLI commands
  for listing/resolving provider interactions.
- Managed Codex and Claude descriptors report contract version 1 with the full
  required baseline in written conformance tests. Codex goals are native;
  Claude goals return a structured unsupported reason.
- Git status/diff/log, todos, review comments, checkpoints, conflicts, checks,
  PR summary, PR checks, PR thread resolve/reopen, PR merge, and history
  commands.
- GitHub-backed flows use local `gh` auth. Linear-backed workspace creation
  uses `LINEAR_API_KEY`.
- Release packaging scaffolding for tarball, AppImage, `.deb`, `.rpm`, AUR, and
  experimental Flatpak, plus a portable native Windows ZIP.
- Cross-platform process/path/shell boundaries for Windows, including AppData
  storage, `cmd.exe` scripts, Windows Terminal launch, `taskkill` process-tree
  shutdown, and loopback archcar IPC.
- CI compile gates for native Windows, glibc Linux, musl Linux, and GTK builds
  on Debian, Fedora, Arch, openSUSE, and Alpine families.

### GTK App

- GTK/libadwaita app with Dashboard, Projects, History, and Workspace pages.
  Settings, Dashboard filters, and History tabs
  reuse the standard close-free workspace chat-tab presentation.
- Project onboarding from local repository path or Git clone URL.
- Scope-aware Settings page: Shared applies machine-wide defaults to every
  project without a project selector; Local selects one project and edits only
  its project/workspace overrides. Local prompt editors show inherited values
  without copying them into the override until edited.
- Repository settings for committed team configuration remain available through
  the Projects surface and remain between app Shared and Local in effective
  precedence.
- Workspace creation from branch/base, prompt, GitHub issue, GitHub PR, and
  Linear issue with source preflight feedback.
- Workspace command center with status header, agents panel, runtime panel,
  Changes, Checks, Review, Chat/Terminal, Big Terminal, Todos, Processes,
  Branch, Timeline, Checkpoints, lifecycle actions, and linked-directory panel.
- Workspace delete lifecycle jobs own artifact cleanup and retry behavior; GTK
  surfaces invoke the lifecycle job instead of starting detached duplicate
  cleanup.
- Agent/session surface for Shell, Codex, Claude, and Cursor session launch
  paths, transcript persistence, selected-session input, staged review prompts,
  provider/auth/MCP status, harness metadata, prompt preview, profile selector,
  and stop notifications.
- GTK uses managed harness descriptors for Codex and Claude live controls:
  provider/model/thinking are baseline controls, Codex-only goals remain
  visible, and Claude hides unsupported goals.
- Plain Enter follow-up queueing, Ctrl+Enter immediate Codex delivery, and
  queue-row reconciliation isolated from streaming chat refreshes.
- GTK keeps hot workspace/chat UI state in watched AppState slices for
  selection, refresh requests, pending workspace phases, pending chat targets,
  and queued composer input. Workspace and chat creation publish optimistic
  status, keep the composer usable, and drain queued input after the real
  workspace/chat thread and agent session are ready.
- GTK refreshes use typed events for routine runtime, review, workspace
  inventory, terminal, and chat changes; `RefreshScope::All` is reserved for
  explicit manual refresh and startup reconciliation.
- GTK background sync samples persisted running chat markers off the main timer
  callback, coalesces lifecycle refreshes by workspace, and avoids loading
  hidden full chat timelines for off-focus work.
- Running chat sessions are sampled in the background with lightweight ids and
  sequence markers so sidebar/dashboard/history and chat tab state can update
  while another workspace or thread is selected.
- Terminal surfaces for one-shot commands, PTY shell tabs, transcript
  persistence/search/reload, basic ANSI/control redraw handling, alternate
  screen restoration, configured terminal font, configured scrollback, and
  command preset buttons.
- Runtime controls for setup/run/stop scripts, log tails, and the current
  Spotlight testing slice.
- Changes/review/checks surfaces for changed files, diffs, recent commits,
  branch push state, local comments, safe tracked-file revert, PR create/refresh,
  PR checks/comments/reviews, PR readiness summary, review-thread actions,
  merge blockers, merge, and archive-after-merge.
- Dashboard cards open workspaces and group them as Ready, Running, Review, or
  Archived, with filters for All projects and every registered project.
- History defaults to a Workspaces tab with All/Active/Archived filters and also
  provides a Chats tab for saved Linux sessions and older macOS Conductor chats
  when the upstream database exists.
- Command palette, global refresh/sidebar shortcuts, tab/deep-link navigation,
  view defaults, theme/accent/density classes, and terminal presets.

## Known Gaps

- GTK app polish and visual parity are incomplete.
- Terminal rendering handles common ANSI/control redraws but is not a full
  terminal emulator.
- Project onboarding/settings need more polish and clearer managed/user setting
  separation.
- Prompt pack switching/import/export, naming templates, hooks, local check
  runner UI, richer notifications, and deeper layout/theme controls are not
  fully surfaced in the GUI.
- `new_workspace`, `summarize_session`, `handoff`, `rename_branch`, and
  `refactor_style` prompts remain editable inherited defaults without dedicated
  surfaced actions.
- Runtime ownership is now Archcar-managed for Codex and Claude Code in written
  tests. Live Claude Code auth/session/interaction smokes still need to be run
  on a machine with an authenticated Claude CLI before calling the parity slice
  manually validated.
- Codex unsafe approval/sandbox bypass needs explicit product policy before a
  broad public launch.
- Live GitHub validation requires authenticated `gh`; live Linear validation
  requires `LINEAR_API_KEY`.
- Native Windows is a preview target. The workspace compiles there and the
  release workflow assembles a portable ZIP, but real Windows install/launch,
  GTK runtime, PTY, provider, upgrade, and checksum smoke remain required
  before calling the package release-ready.
- Linux remains the manually validated primary product target. CI covers GNU
  and musl plus representative distro families; individual package channels
  still require install/launch/upgrade validation.
- Release packaging still needs full manual validation on target distros before
  public launch.

## Agent Reading Order

Coding agents should read these durable docs before changing behavior or docs:

1. `.codex/AGENTS.md` or `claude/CLAUDE.md`, depending on agent.
2. `docs/conductor-gui-mvp-handoff.md`
3. `progress.md`
4. `docs/mvp-scope.md`
5. `docs/manual-testing-checklist.md`
6. `docs/archductor-docs-parity-map.md`
7. `README.md`

Old one-off implementation plans and specs were pruned from `docs/superpowers`
because they were dated task artifacts, not current product or agent
instructions.

## Verification Standard

Keep docs grounded in current evidence. When a feature exists only in core,
CLI, or GTK, say which layer is implemented or verified.

Before calling behavior done, name:

- written tests
- CLI smoke
- GTK smoke

If one layer is skipped, say exactly why.

## Recent Verification

Claude Code Archcar parity written verification on 2026-07-16:

- Passed `cargo fmt --all -- --check`.
- Passed `cargo clippy -p archductor-core -p archductor -p archductor-gtk --all-targets -- -D warnings`.
- Passed `cargo test -p archductor-core archcar::harness_conformance --lib`.
- Passed `cargo test -p archductor-core`.
- Passed `cargo test -p archductor`.
- Passed `cargo test -p archductor-gtk`.
- Passed `cargo build -p archductor-gtk`.
- `claude auth status` succeeded with local first-party `claude.ai` auth; no API
  key prompt was required.
- GTK Xvfb launch reached startup under `timeout 8 xvfb-run -a
  target/debug/archductor-gtk`, but emitted existing runtime warnings for DRI3,
  unsupported libadwaita dark-theme setting, CSS `text-align`, and missing
  accessibility bus. Treat GTK runtime smoke as started-with-warnings, not clean.

Not yet manually smoke-verified in this branch:

- Live Claude first-send/follow-up through Archcar.
- Two simultaneous Claude native thread IDs.
- Live queue/immediate/interrupt/model/effort/permission-mode behavior.
- Live permission/question/plan interaction cards in GTK.
- Archcar restart with a pending Claude interaction.
