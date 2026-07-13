# Progress

Current as of 2026-07-13.

## Current State

Archductor has a usable but rough GUI-first loop for one local repository:

1. Add or clone a repository as a project.
2. Edit shared and local repository settings.
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
- Monorepo working-directory defaults for scripts, terminal commands, editor
  launches, and agent sessions.
- Linked workspace directories with persisted records, symlinks under
  `.context/linked-directories`, and `ARCHDUCTOR_LINKED_*` environment.
- PTY-backed shell/session primitives, transcript logs, provider events, and
  stale-process reconciliation.
- Shell, Codex, and Claude CLI session commands. Cursor is available from GTK
  launch paths where configured, not from the current CLI `session --kind`
  enum.
- Git status/diff/log, todos, review comments, checkpoints, conflicts, checks,
  PR summary, PR checks, PR thread resolve/reopen, PR merge, and history
  commands.
- GitHub-backed flows use local `gh` auth. Linear-backed workspace creation
  uses `LINEAR_API_KEY`.
- Release packaging scaffolding for tarball, AppImage, `.deb`, `.rpm`, AUR, and
  experimental Flatpak.

### GTK App

- GTK/libadwaita app with Dashboard, Projects, History, Workspace, and
  debug-only PTY Inspector pages.
- Project onboarding from local repository path or Git clone URL.
- Project settings for shared/local settings, prompts, scripts, Git, terminal,
  shortcuts, notifications, provider paths, and advanced customization TOML.
- Workspace creation from branch/base, prompt, GitHub issue, GitHub PR, and
  Linear issue with source preflight feedback.
- Workspace command center with status header, agents panel, runtime panel,
  Changes, Checks, Review, Chat/Terminal, Big Terminal, Todos, Processes,
  Branch, Timeline, Checkpoints, lifecycle actions, and linked-directory panel.
- Agent/session surface for Shell, Codex, Claude, and Cursor session launch
  paths, transcript persistence, selected-session input, staged review prompts,
  provider/auth/MCP status, harness metadata, prompt preview, profile selector,
  and stop notifications.
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
- History reads saved Linux session history and older macOS Conductor chats when
  the upstream database exists.
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
- Runtime ownership is still converging around the archcar/local daemon APIs.
- Codex unsafe approval/sandbox bypass needs explicit product policy before a
  broad public launch.
- Live GitHub validation requires authenticated `gh`; live Linear validation
  requires `LINEAR_API_KEY`.
- Linux is the only release target. WSL can be considered before native
  Windows. macOS is lower priority while upstream Conductor covers that
  platform.
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
