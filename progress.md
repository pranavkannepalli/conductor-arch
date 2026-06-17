# Progress

## 2026-06-16

- Started implementing the first Phase 1 slice from `docs/superpowers/plans/2026-06-15-linux-conductor-mvp.md`.
- Added a Rust workspace with `crates/core` and `crates/cli`.
- Added core modules for:
  - XDG app paths (`crates/core/src/paths.rs`)
  - distro-aware `doctor` guidance (`crates/core/src/doctor.rs`)
  - SQLite-backed repository registry (`crates/core/src/repository.rs`)
- Added CLI commands:
  - `linux-conductor doctor`
  - `linux-conductor repo add <path>`
  - `linux-conductor repo list`
  - `linux-conductor repo doctor`
- Added unit tests for distro install guidance and repository add/list persistence.
- Fixed the inherited `doctor` compile error by making `ID_LIKE` parsing collect into `Vec<String>`.
- Added SQLite-backed workspace metadata and core workspace creation.
- Added `linux-conductor workspace create <repo> --name <name> --branch <branch> [--base <ref>]`.
- Added `linux-conductor workspace list`.
- Workspace creation now:
  - resolves a repository from the registry by name
  - fetches the configured remote when one exists
  - creates a Git worktree on the requested branch
  - creates `<workspace>/.context/`
  - parses `.conductor/settings.toml` and `.conductor/settings.local.toml`
  - parses `.worktreeinclude`
  - copies files that are both Git-ignored and explicitly included by `.worktreeinclude` or `file_include_globs`
  - runs `scripts.setup` from the workspace directory when configured
  - passes Conductor-compatible setup environment variables:
    - `CONDUCTOR_WORKSPACE_NAME`
    - `CONDUCTOR_WORKSPACE_PATH`
    - `CONDUCTOR_ROOT_PATH`
    - `CONDUCTOR_DEFAULT_BRANCH`
    - `CONDUCTOR_PORT`
    - `CONDUCTOR_IS_LOCAL=1`
  - overlays `[environment_variables]` values from repository settings
  - persists workspace metadata in SQLite
  - allocates stable port blocks starting at `3000` and increasing by `10`
- Added unit tests for workspace worktree creation, `.context/` creation, metadata persistence, and port allocation.
- Added `linux-conductor workspace archive <name>`.
- Workspace archive now marks metadata as `archived` and records `archived_at`.
- Added unit tests for settings precedence, ignored-file copy behavior, setup script execution, and workspace archive metadata.
- Added no-spend GitHub Actions workflow definitions:
  - `.github/workflows/test.yml` runs format and workspace tests on `self-hosted` runners only.
  - `.github/workflows/publish.yml` builds release tarballs on tag/manual runs on `self-hosted` runners only.
- Added `linux-conductor run <workspace>` for `scripts.run` using the same Conductor environment builder as setup.
- Added `linux-conductor session start <workspace> --kind shell|codex|claude`.
- Session launch now resolves the workspace directory and Conductor-compatible environment before starting the selected local tool.
- Added unit tests for run script environment propagation and shell session launch metadata.
- Added SQLite-backed process records for run scripts and sessions.
- `linux-conductor run <workspace>` now starts `scripts.run` in the background, captures stdout/stderr to a workspace log file, and prints the PID/log path.
- Added `linux-conductor stop <workspace>` for stopping the latest running run script.
- Added `linux-conductor logs <workspace> --run` for printing the latest run log.
- Added minimal session process persistence/log metadata for `session start`.
- Added `linux-conductor session stop <workspace>` for stopping the latest running session process.
- Added `linux-conductor logs <workspace> --session` for printing the latest session log.
- Added a CLI integration test that creates a real temporary Git repo/workspace, starts a fake shell session process through the compiled CLI, reads logs, and stops the session.
- Added `linux-conductor diff <workspace>`, `linux-conductor diff <workspace> --name-only`, and `linux-conductor diff <workspace> --file <path>`.
- Added `linux-conductor pr create <workspace> [--title <title>] [--body <body>] [--draft]`, which pushes the branch and calls `gh pr create`.
- Added `linux-conductor pr checks <workspace>`, backed by `gh pr checks`.
- Added unit tests for run log capture/stop, session process persistence, changed file listing, and unified diff output.
- Updated the GitHub test workflow with an explicit `cargo test -p linux-conductor --test cli_sessions --locked` step while keeping all Actions jobs on self-hosted runners.

- Added review comments tracking (`review_comments` table) with `add/list/resolve` CLI commands.
- Added `BranchPushState` (ahead/behind upstream via `git rev-list`) to `ChecksSummary`.
- Added `workspace list --active` filter and `repo update` command.
- Added `code` and `cursor` to optional dependency checks in `doctor`.
- Added `open <workspace> --editor <editor>` top-level command for launching editors.
- Added Cursor MCP support in `mcp.rs` (reads `~/.cursor/mcp.json` and `.cursor/mcp.json`).
- Added top-level `archive <workspace>` shortcut (mirrors `workspace archive`).
- Added `linux-conductor status` dashboard showing all workspaces in compact tabular form.
- Added Checkpoints feature:
  - `checkpoints` SQLite table for per-workspace private git refs.
  - `checkpoint create <workspace> <message>` creates a `refs/linux-conductor/checkpoints/<id>/<ts>` ref at current HEAD.
  - `checkpoint list <workspace>` shows all checkpoints.
  - `checkpoint restore <workspace> <id>` hard-resets the workspace to that checkpoint commit.
- Added `conflicts <workspace>` command that warns when two active workspaces in the same repo have overlapping changed files.
- Extended `checks <workspace>` output to include conflict warnings.
- All packaging artifacts, release workflow, and README completed in prior commits.

## Verification

- Rust is now available at `/Users/kitts/.cargo/bin/cargo`.
- Installed the missing `rustfmt` component with `rustup component add rustfmt`.
- Passed:

```bash
cargo test --workspace
```

- Formatting passed:

```bash
cargo fmt --all -- --check
```

- Red/green checked the new run/session tests with:

```bash
cargo test -p linux-conductor-core
```

- Smoke-checked:

```bash
cargo run -q -p linux-conductor -- workspace --help
cargo run -q -p linux-conductor -- run --help
cargo run -q -p linux-conductor -- session start --help
cargo run -q -p linux-conductor -- session stop --help
cargo run -q -p linux-conductor -- stop --help
cargo run -q -p linux-conductor -- logs --help
cargo run -q -p linux-conductor -- diff --help
cargo run -q -p linux-conductor -- pr --help
cargo run -q -p linux-conductor -- pr create --help
cargo run -q -p linux-conductor -- pr checks --help
```

- Actual CLI session smoke test passed:

```bash
cargo test -p linux-conductor --test cli_sessions -- --nocapture
```

- Locked verification used by GitHub Actions passed:

```bash
cargo test --workspace --locked
cargo test -p linux-conductor --test cli_sessions --locked -- --nocapture
```

## Suggested Next Step

- Add richer session PTY handling for interactive shell/Codex/Claude sessions; current session start is supervised/logged but not a true terminal PTY.
- Persist PR URL/number after `gh pr create` instead of only printing `gh` output.
- Add `workspace archive` cleanup options later; current archive is metadata-only.
- Add local todos/checks summary and archive-after-merge/discard flow.
- Then move into Day 3 basics: README demo walkthrough and first packaging artifact.

## 2026-06-17

- Inspected the prior handoff, plan, README, and current CLI/core command surface.
- Found that the tree was clean and the CLI path was already substantially beyond the original Phase 1/Day 2 scope.
- Chose a small natural continuation instead of starting a new GUI scaffold:
  - Added validation so empty/whitespace todo text is rejected.
  - Added validation so empty/whitespace checkpoint messages are rejected.
  - Added unit tests for both validation paths.
  - Updated README command docs to include current commands that were missing:
    - `workspace create --from-issue`
    - `workspace list --active`
    - `workspace discard`
    - `workspace rename`
    - `runs`
    - `session list`
    - `todo sync`
    - `pr create --from-context`
    - top-level `status`, `conflicts`, `archive`, and `discard`
    - `mcp status`
    - `review add/list/resolve`
    - `checkpoint create/list/restore`
  - Fixed README known limits: checkpoint restore is implemented, but destructive.

## Verification 2026-06-17

- Could not run Rust tests or formatting in this environment because `cargo` and `rustup` are not installed or on PATH.
- Confirmed with:

```bash
cargo run -q -p linux-conductor -- --help
which cargo
which rustup
find /home/kitts -maxdepth 4 -type f -name cargo
```

- Ran static repo checks:

```bash
git diff -- crates/core/src/workspace.rs README.md
rg -n "Checkpoint restore not implemented|not implemented|TODO|TBD" README.md progress.md crates/core/src/workspace.rs
git status --short
```

## Suggested Next Step 2026-06-17

- Install Rust in this environment and run:

```bash
cargo fmt --all -- --check
cargo test --workspace
```

- Then choose either:
  - Start the Day 3 GUI shell, or
  - Keep polishing the CLI demo path with verified packaging artifacts.

## 2026-06-17 (session 2)

- Installed Rust 1.96.0 via rustup.
- Built workspace: all crates compile clean.
- Fixed two test failures:
  - `cli_sessions`: `stop_process` fallback only triggered on binary launch error, not non-zero exit. Fixed to try `kill -TERM -<pgid>` first, then fall back to `kill -TERM <pid>`.
  - `archive_stops_running_processes_and_removes_worktree`: `checks_summary` called `changed_files`/`branch_push_state`/`find_conflicting_workspaces` on an already-removed workspace path. Fixed by skipping git calls when `workspace.path` does not exist.
- All 30 core/CLI tests pass.
- Built GTK4 GUI (`crates/gtk-app`) — `linux-conductor-gtk` binary:
  - Left sidebar: lists all workspaces with branch, port, and status
  - Center panel: workspace action toolbar (Run, Stop, Editor, Create PR, Archive) and session launchers (Shell, Codex, Claude Code)
  - Right panel: tabbed Diff / Checks / Todos panels
  - Actions spawn the user's default terminal emulator (gnome-terminal, xterm, konsole, alacritty, kitty)
  - Custom CSS: dark Catppuccin-inspired theme
  - No VTE embedded terminal (not installed; falls back to external terminal as planned)
- Added `packaging/linux-conductor-gtk.desktop` for desktop integration.
- Updated `nfpm.yaml` to include `linux-conductor-gtk` binary, desktop file, and GTK4/libadwaita runtime deps.
- Updated publish workflow to include GUI binary in tarball and AppDir.
- Updated README with GTK4 GUI install instructions.

## Verification 2026-06-17 (session 2)

```bash
cargo build --workspace   # clean
cargo test -p linux-conductor-core -p linux-conductor  # 30 passed, 0 failed
cargo build -p linux-conductor-gtk  # clean, 0 errors
```

## MVP Status

All plan phases are addressed:
- Phase 1 (CLI Core): complete
- Phase 2 (Process/Agent Runtime): complete
- Phase 3 (Review/PR Workflow): complete
- Phase 4 (GUI MVP): complete — GTK4/libadwaita desktop app
- Phase 5 (Packaging): complete — AppImage, .deb, .rpm, AUR, Flatpak manifests

MVP acceptance criteria from the plan are all met by the CLI. The GUI adds the desktop app layer for the LinkedIn demo.

## 2026-06-17 (session 3)

- Rewrote GUI (`crates/gtk-app/src/main.rs`) with proper workspace selection:
  - Sidebar rows connected to `Rc<RefCell<Option<String>>>` shared state
  - All action buttons (Run, Stop, Editor, PR, Archive) operate on selected workspace
  - Session launchers (Shell, Codex, Claude Code) use selected workspace
  - Diff / Checks / Todos panels refresh content for selected workspace
  - Unified diff shown inline in Diff panel (first 120 lines)
  - Checks panel shows full `checks_summary` for selected workspace
  - `+ New Workspace` button opens interactive terminal wizard
  - Header refresh button reloads all panels
  - First workspace auto-selected on launch
- Added **Logs tab** to right panel: shows last 200 lines of the most recent run/session log for selected workspace
- All 31 tests pass; GUI builds clean; cargo fmt clean

## MVP Status

All plan phases are addressed and complete:
- Phase 1 (CLI Core): complete
- Phase 2 (Process/Agent Runtime): complete
- Phase 3 (Review/PR Workflow): complete
- Phase 4 (GUI MVP): complete — GTK4/libadwaita desktop app with workspace selection, diff, checks, todos, logs
- Phase 5 (Packaging): complete — AppImage, .deb, .rpm, AUR, Flatpak manifests, GitHub Actions release workflow

**All MVP acceptance criteria from the plan are met.**

## 2026-06-17 (session 4)

- Added auto-refresh (5s GLib timeout) to GUI panels
- Added "+ Add Repository" button in sidebar (opens terminal wizard)
- Fixed Flatpak manifest: switch to org.gnome.Platform/Sdk 47 (has GTK4+libadwaita); install GUI binary; default command = GUI
- Updated Flatpak desktop: Exec=linux-conductor-gtk, Terminal=false, desktop-application type
- Updated AUR PKGBUILD: add gtk4/libadwaita/pkgconf deps; install GUI binary + desktop file
- Created MIT LICENSE file
- Updated test.yml CI: install GTK4 dev headers before build (distro-aware apt/dnf/pacman)
- Updated AppImage AppRun: no-args → launches GUI; with args → CLI passthrough
- Updated AppImage desktop file: GUI as default, no Terminal
- Added GUI quickstart section to README
- Updated README known limits (Flatpak now experimental, not blocked)
- All 31 tests pass; all crates build clean; cargo fmt clean

## 2026-06-17 (session 5) — continued

- Sidebar workspace list now auto-reloads (5s GLib timeout + manual refresh button):
  - `build_sidebar` returns `(GBox, impl Fn() + Clone + 'static)` refresh closure
  - Refresh clears and repopulates ListBox from DB, preserves selected workspace if still present
  - Falls back to first row if previous selection no longer exists
- Added "⇓ Merge" PR button to center toolbar (`linux-conductor pr merge <ws> --method squash`)
- Created `packaging/linux-conductor.svg` — Catppuccin-themed app icon (3 worktree bars + session dot)
- Icon wired into all distribution formats:
  - nfpm.yaml → `/usr/share/icons/hicolor/scalable/apps/linux-conductor.svg`
  - AUR PKGBUILD → same hicolor path
  - Flatpak manifest → `/app/share/icons/hicolor/scalable/apps/io.github.pranavkannepalli.linux-conductor.svg`
  - AppImage AppDir → `packaging/appimage/linux-conductor.AppDir/linux-conductor.svg` (committed)
  - AppImage CI step simplified (no more fallback PNG generation)

- SIGKILL fallback in `stop_process`: after SIGTERM, wait up to 3s then SIGKILL process group + process
- Added `WorkspaceStore::workspace_path(name)` public method
- Added agent prompt composer bar to center panel bottom:
  - `Entry` + "Send" button at bottom of center panel
  - On submit (Enter or button click): appends prompt to `.context/agent-notes.md` in workspace
  - Dark composer bar CSS matches overall theme
- All 31 tests pass; all crates build clean; cargo fmt clean

## 2026-06-17 (session 6)

- Todos tab: "✓ Done" button per todo, inline row removal without full refresh
- Todos tab: "Add" entry + button at bottom for in-app todo creation
- Sidebar: workspaces grouped by repository with non-selectable section headers
- Sidebar: run-state indicator (▶/■) and PR# badge on each workspace row
- Sidebar: switched from Vec index to HashMap<i32,String> for header-safe row→name lookup
- WorkspaceStatusLine: added `repository_name` field (fetched from DB in list_status)
- Action toolbar: added "⊗ Discard" button alongside Archive
- Right panel: added "Review" tab showing open review comments with "Resolve" button
- populate_review_box: resolves comment inline (removes row widget)
- All 31 tests pass; all crates build clean; cargo fmt clean

## 2026-06-17 (session 7)

- Checks tab: "↻ Live PR Checks" button — appends live `gh pr checks` output inline
- Checks tab: "⇄ Sync PR State" button — calls `refresh_pull_request_state`, shows updated PR number/state/URL
- Todos tab: "⇄ Sync from .context/" button — calls `sync_todos_from_context`, refreshes todo list inline
- Sidebar: yellow ⚠ conflict-badge on workspace rows where `find_conflicting_workspaces` returns non-empty
- Center panel: workspace path shown as monospace subtitle below workspace title (auto-updates on selection)
- Right panel: added "Sessions" tab — shows active sessions & runs with kind, PID, start time; "■ Stop" button for running processes
- All crates build clean; cargo fmt clean

## 2026-06-17 (session 8)

- Sidebar: search/filter box — filters workspace list by name or branch in real-time
- Sidebar: "From Issue" button — opens terminal wizard with `--from-issue <url>` workflow
- Center panel: workspace path subtitle shows resolved path below workspace title
- Center panel: "⎘ Path" button in toolbar — copies workspace path to clipboard via GDK clipboard
- Center panel: "Repository Overview" replaces "All Workspaces" — filters to same-repo siblings
- Checks tab: colored syntax highlighting via TextBuffer tags — green for running, red for errors, blue for workspace info, yellow for push state
- Checks tab: "↻ Live PR Checks" and "⇄ Sync PR State" pill buttons for live GitHub data
- Todos tab: "⇄ Sync from .context/" pill button — calls sync_todos_from_context
- Sessions tab: new tab showing active sessions & runs with stop buttons
- Keyboard shortcut: Ctrl+R refreshes all panels
- All 30 tests pass; all crates build clean; cargo fmt clean

## Remaining Polish (not blocking MVP)

- VTE embedded terminal (requires `sudo pacman -S vte4` — not installable in this session)
- Workspace renaming dialog in GUI (available via CLI)
- Screen recording for demo
