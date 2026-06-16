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
