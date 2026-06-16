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

## Verification

- Rust is now available at `/Users/kitts/.cargo/bin/cargo`.
- Installed the missing `rustfmt` component with `rustup component add rustfmt`.
- Passed:

```bash
cargo test --workspace
```

- Smoke-checked:

```bash
cargo run -q -p linux-conductor -- workspace --help
```

## Suggested Next Step

- Continue Day 1 by adding `session start <workspace> --kind shell`, either by launching the user's shell in the workspace or by introducing the first process/session table.
- Add `run <workspace>` for `scripts.run` with the same environment builder used by setup.
- Add `workspace archive` cleanup options later; current archive is metadata-only.
- Then move into Day 2 basics: changed file list, unified diff command, branch push, PR creation through `gh`, and checks through `gh pr checks`.
