# Linux Archductor

Linux Archductor is a desktop control plane for running coding agents across
isolated Git worktree workspaces.

Use it when one codebase has several streams of work in flight: create a
workspace, start Codex or Claude Code, review the diff, open or merge a GitHub
pull request, archive the workspace, then start the next task without leaving
the app.

Inspired by [Conductor](https://conductor.build). This project targets Linux
desktops with GTK4/libadwaita.

## Product Structure

Archductor is cloning the Conductor structure below and the docs in this repo
should use these terms consistently:

| Term | What it means in Archductor | Relationship |
| --- | --- | --- |
| `Project` | The Archductor entry for one codebase. It holds repository settings, scripts, instructions, and the list of workspaces for that codebase. | `1 project contains 1 repository` |
| `Repository` | The Git codebase behind a project. It can come from an existing local folder, Git URL, or quick-start flow. | `1 repository contains many workspaces` |
| `Workspace` | An isolated copy of a project and repository for one task, issue, experiment, or pull request. | `1 workspace maps to 1 branch` |
| `Branch` | The Git branch checked out inside a workspace. This is usually the review and PR unit. | `1 branch has 1 working tree` |
| `Working tree` | The files on disk for one workspace. Archductor creates these separate checkouts with Git worktrees. | `1 working tree belongs to 1 workspace` |
| `Running environment` | The app, server, watchers, tests, terminals, and agent processes running inside a workspace. | `1 workspace can run many processes` |

Practical rule:

- Projects own repository-level defaults and configuration.
- Workspaces are the task unit.
- Sessions, terminals, setup/run scripts, and agents are part of the running
  environment inside a workspace.

## What Works Today

The current app supports the core Archductor loop, with some rough edges:

- Add a project from an existing repository path or clone a Git URL from the
  Projects page.
- Create workspaces from a branch, prompt, GitHub issue, GitHub PR, or Linear
  issue.
- Give each workspace its own Git worktree, branch, `.context` directory, and
  stable `ARCHDUCTOR_PORT` range.
- Run multiple workspaces for the same repository in parallel.
- Start multiple Shell, Codex, Claude Code, or Cursor sessions inside one
  workspace.
- Use an embedded workspace terminal, setup/run/stop controls, logs, and process
  lists.
- Review changed files, file diffs, todos, local review comments, sibling
  workspace conflicts, PR checks, and GitHub PR comments.
- Stage review comments, failing checks, or PR comments into the selected agent
  session.
- Create, refresh, merge, and archive GitHub PRs from the workspace view through
  local `gh` auth.
- Restore archived workspaces, inspect saved Linux session history, and read
  older macOS Conductor chat history when that database is available.
- Expand Codex transcript events inline: skill reads, command/tool runs, and
  added/edited/deleted file summaries render as compact `+ name` chips that
  open into the full captured output, file preview, or structured diff body.
- Customize repository behavior with editable prompts, scripts, environment,
  provider paths, Git behavior, file-copy rules, monorepo working directories,
  and file-editable workflow defaults for naming, automation, agents, merge
  rules, workspaces, and views.

- New PTY-backed Codex chat threads now use real pseudo-terminals, rendered
  screen parsing, structured thread/message persistence, and exact resume IDs
  captured from Codex rollout metadata.

The GUI is usable, but not fully polished. Agent sessions run local PTY-backed
harnesses and render structured app-native transcript events. Terminal
rendering, exhaustive per-command shortcuts, full visual theme/layout
application, and full Archductor visual parity are still in progress.

## The Workflow

1. Open `linux-archductor-gtk`.
2. Add or clone a repository on the Projects page. This creates the Archductor
   project entry for that codebase.
3. Configure repository scripts and settings if the project needs them.
4. Create a workspace for the next task.
5. Start Codex, Claude Code, Cursor, or a shell from the workspace page.
6. Work in the agent chat or embedded terminal.
7. Review changes, todos, comments, checks, and conflicts in the workspace.
8. Create a PR from the Checks tab.
9. Send failing checks or review comments back to an agent if needed.
10. Merge the PR and archive the workspace.
11. Repeat for the same repository or another repository.

Normal work should happen in the app. The CLI remains available for automation,
debugging, and fallback workflows.

## Install

### AppImage

```bash
curl -Lo linux-archductor.AppImage \
  https://github.com/pranavkannepalli/conductor-arch/releases/latest/download/linux-archductor-x86_64.AppImage
chmod +x linux-archductor.AppImage
sudo mv linux-archductor.AppImage /usr/local/bin/linux-archductor
```

Run the app:

```bash
linux-archductor
```

The AppImage opens the GTK app with no arguments and forwards CLI arguments to
the command-line interface.

### Build From Source

Install GTK4/libadwaita and Rust first:

```bash
# Ubuntu / Debian
sudo apt update
sudo apt install git gh sqlite3 openssh-client pkg-config libgtk-4-dev libadwaita-1-dev

# Fedora
sudo dnf install git gh sqlite openssh-clients pkgconf-pkg-config gtk4-devel libadwaita-devel

# Arch Linux
sudo pacman -S --needed git github-cli sqlite openssh pkgconf gtk4 libadwaita

# Rust
curl https://sh.rustup.rs -sSf | sh
```

Build and run:

```bash
git clone https://github.com/pranavkannepalli/conductor-arch
cd conductor-arch
cargo build --workspace --release --locked
./target/release/linux-archductor-gtk
```

Or use the short `make` targets:

```bash
make gtk
make cli
make build
make build-release
make check
make release VERSION=0.1.0
make tag VERSION=0.1.0
make publish-tag VERSION=0.1.0
```

Optional install:

```bash
sudo install -Dm755 target/release/linux-archductor /usr/local/bin/linux-archductor
sudo install -Dm755 target/release/linux-archductor-gtk /usr/local/bin/linux-archductor-gtk
```

## Requirements

| Tool | Required For |
| --- | --- |
| `git` | Worktrees, branches, diffs, commits |
| `gh` | GitHub PR creation, checks, comments, merge |
| `openssh` | SSH repository access |
| `codex` | Codex sessions |
| `claude` | Claude Code sessions |
| `cursor` or `code` | Editor/session launch when configured |

Run `gh auth login` before using PR features. Codex and Claude Code use your
existing local CLI authentication.

## Repository Settings

Shared project settings live at `.archductor/settings.toml` in the repository
root. In product terms, these are project-level settings because one Archductor
project wraps one repository. Commit shared `.archductor` files except
`.archductor/settings.local.toml` when teammates or another PC should get the
same setup. Workspace `.context/` files are local scratch context and should
stay gitignored.

Every `env_file_refs` path is required when a workspace launches. Remove the
block if the file does not exist, or commit/create the referenced `.env` file.

```toml
"$schema" = "https://conductor.build/schemas/settings.repo.schema.json"

file_include_globs = """
.env*
config/*.local.json
"""

env_file_refs = """
.env
"""

spotlight_testing = false

[scripts]
setup = "pnpm install"
run = "pnpm dev --port $ARCHDUCTOR_PORT"
archive = "./script/workspace-archive.sh"
test = "cargo test --workspace"
lint = "cargo clippy --workspace"
typecheck = "cargo check --workspace"
build = "cargo build --workspace"
run_mode = "concurrent"

[environment_variables]
API_BASE_URL = "http://localhost:3000"

[prompt_pack]
active = "default"
version = "v1"
path = ".archductor/prompt-packs/default.toml"

[prompts]
new_workspace = "Create a small plan before changing code."
general = "Prefer small, reviewable changes and run focused tests."
continue_work = "Inspect current changes before editing."
summarize_session = "Summarize verification and remaining risk."
handoff = "Leave context, changed files, tests, and next steps."
code_review = "Focus on correctness, behavior changes, and missing tests."
create_pr = "Write concise PR descriptions with test evidence."
test_fixing = "Reproduce failing tests first, then make the smallest fix."
refactor_style = "Keep behavior-preserving refactors separate from feature work."
setup_script = "Infer setup commands from repo files."
run_script = "Infer run commands and required ports/env."

[git]
archive_on_merge = true

[customization.naming]
branch_template = "{prefix}/{type}-{slug}"
workspace_name_style = "city"
commit_style = "conventional"
pr_body_sections = ["Summary", "Tests", "Risk"]
default_merge_method = "squash"

[customization.automation]
auto_setup = true
auto_start_agent = "codex"
required_local_files = [".env"]
test_command = "cargo test --workspace"
lint_command = "cargo clippy --workspace"
typecheck_command = "cargo check --workspace"
build_command = "cargo build --workspace"

[customization.agent_profiles.default]
agent = "codex"
model = "gpt-5-codex"
approval_mode = "on-request"
reasoning_mode = "medium"

[customization.merge_rules]
block_on_open_todos = true
block_on_open_comments = true
block_on_failed_checks = true
block_on_pending_checks = false
definition_of_done = "Tests run, reviewer comments resolved, PR explains risk."

[customization.workspace_defaults]
base_branch = "main"
branch_prefix = "lc"
working_directory = "apps/web"
port_block_size = 10
default_visible_tab = "changes"

[customization.view]
theme = "system"
accent_color = "green"
density = "compact"
keybindings = "vim"
diff_preference = "unified"
transcript_display = "structured"
```

Local machine overrides live at `.archductor/settings.local.toml`. Do not commit
secrets.

Use `.worktreeinclude` when new workspaces should copy gitignored local files:

```text
.env*
config/*.local.json
certs/local/**
```

Only gitignored files are copied. Generated files and dependency installs
belong in `scripts.setup`.

### Script Environment

Setup, run, archive, terminal, and agent processes receive Archductor context.
When `customization.workspace_defaults.working_directory` is set, processes run
from that relative subdirectory inside the worktree.

| Variable | Value |
| --- | --- |
| `ARCHDUCTOR_WORKSPACE_NAME` | Workspace name |
| `ARCHDUCTOR_WORKSPACE_PATH` | Absolute path to the worktree |
| `ARCHDUCTOR_WORKING_DIRECTORY` | Absolute path to the selected working directory |
| `ARCHDUCTOR_ROOT_PATH` | Absolute path to the main repository |
| `ARCHDUCTOR_DEFAULT_BRANCH` | Repository default branch |
| `ARCHDUCTOR_PORT` | Base port for this workspace |
| `ARCHDUCTOR_IS_LOCAL` | `1` |
| `ARCHDUCTOR_LINKED_DIRECTORIES` | Newline-separated `workspace=/path` entries for linked workspaces |
| `ARCHDUCTOR_LINKED_DIRECTORY_<NAME>` | Absolute path for one linked workspace |

### Linked Directories

Use linked directories when one workspace needs to read or edit another related
workspace, such as a frontend and backend checked out from separate projects.
Links are stored in the app database and materialized as symlinks under
`.context/linked-directories/<target-workspace>`.

```bash
linux-archductor workspace link-dir frontend backend
linux-archductor workspace linked-dirs frontend
linux-archductor workspace unlink-dir frontend backend
```

`scripts.run_mode = "concurrent"` lets multiple workspaces run at once.
`"nonconcurrent"` allows only one active run script per repository.

Spotlight testing is available for projects that must run from the repository
root. The current implementation can checkpoint, apply, sync, switch, repair,
and restore tracked workspace changes, but it is still less polished than the
normal worktree runtime.

## Customization

Linux users should be able to make the app fit their workflow. The rule of
thumb is:

- Frequently edited workflow prompts should be editable in the app.
- Repository setup should be automated through committed setup/run/archive
  scripts.
- Advanced appearance, layout, theme, and view preferences can live in config
  files instead of crowding the UI.
- Team defaults belong in shared repository settings.
- Machine-specific preferences and secrets belong in local or user settings.

Customization areas that should be first-class:

### Prompts

Prompts are part of daily agent work, so they should be editable in the app:

- General agent instructions.
- Code review instructions.
- PR creation instructions.
- Failing check repair instructions.
- Merge conflict resolution instructions.
- Branch naming/rename instructions.
- Commit message generation instructions.
- Test-fixing instructions.
- Refactor style instructions.
- Staged prompts generated from local review comments, PR comments, checks,
  todos, conflicts, and selected diffs.
- Prompt profiles or prompt packs, such as `strict-review`, `fast-prototype`,
  `security-heavy`, or `docs-heavy`.
- Final prompt preview before launching an agent.

### Naming And Git Style

Teams should be able to encode their Git conventions once:

- Branch name templates, such as `lc/{workspace}`, `{type}/{slug}`,
  `{issue_key}-{slug}`, or `{github_issue}-{slug}`.
- Workspace name style: generated city names, prompt slug, issue key, branch
  slug, or custom templates.
- Commit message style: conventional commits, terse lowercase, team template, or
  "include tests run" format.
- PR title source: branch, first commit, issue title, prompt summary, or custom
  template.
- PR body sections: Summary, Tests, Screenshots, Risk, Rollback, Follow-ups.
- Default merge strategy: squash, merge, or rebase.
- Archive-after-merge default.

### Repository Automation

Repositories should be able to bootstrap themselves:

- Setup/run/archive scripts.
- Auto-run setup after workspace creation.
- Auto-start a preferred agent after setup.
- Required local file checks for `.env`, certs, tokens, or config files.
- Pre/post hooks for clone, workspace creation, setup, PR creation, merge, and
  archive.
- Per-workspace environment generation.
- Script presets for tests, lint, typecheck, build, seed, reset, and local
  services.

### Agent Defaults

Agent behavior should be configurable per user, repository, workspace, and
session profile:

- Default agent per repository.
- Agent profiles: planning, fast prototype, review-only, tests-first,
  refactor-only, docs-only.
- Default approval mode.
- Default reasoning or effort level.
- Default Codex personality/goals where supported.
- Default MCP visibility and status checks.
- Allowed or disallowed tools by repository or workspace.

### Review And Merge Rules

Merge readiness should match the team's definition of done:

- Configurable merge blockers for open todos, unresolved comments, failed
  checks, sibling workspace conflicts, uncommitted changes, missing tests, or
  missing PR description sections.
- Required checklist before PR creation.
- Required checklist before merge.
- Custom "definition of done" text shown in the workspace.
- Rules for when agent-generated work must be reviewed manually.

### Workspace Defaults

Workspace creation should be fast and predictable:

- Default base branch.
- Workspace parent directory.
- Monorepo working directory relative to the workspace root.
- Branch prefix and slug style.
- Default port block size.
- Files to copy policy.
- Auto-open workspace after creation.
- Auto-create checkpoints on agent start, before PR, before merge, and before
  archive.
- Default tabs/panels to show when a workspace opens.

### View, Theme, And Layout

Not every visual option needs a button. Good file-editable settings include:

- Light, dark, or system theme.
- Accent color.
- Density: compact, normal, spacious.
- Sidebar grouping and sorting.
- Default workspace tab.
- Show/hide panels.
- Unified or side-by-side diff preference.
- Terminal font, size, and scrollback.
- Agent transcript font, wrapping, and timestamps.
- Dashboard columns and status labels.

### Notifications, Shortcuts, And Commands

Power users should be able to tune attention and speed:

- Toasts vs quiet mode.
- Alerts when agents stop, checks fail/pass, PR comments arrive, or conflicts
  appear.
- User-editable keybindings.
- Custom command palette entries.
- Repository-specific terminal presets.
- Import/export for settings bundles and prompt packs.

The current implemented settings format is TOML. The Projects settings page
and Settings page edit common repository fields directly and include an
advanced TOML block for the `[customization]` sections. Workspace creation
already honors
`customization.workspace_defaults.base_branch`, `branch_prefix`, and
`port_block_size`. Runtime setup/run/archive scripts, terminal commands, and
agent sessions honor `working_directory`. PR merge honors
`customization.naming.default_merge_method` plus merge blockers for open todos,
open local review comments, failed checks, and pending checks. GTK workspace
startup and sidebar selection honor `default_visible_tab` unless an explicit
launch tab is provided, and apply the configured `theme`, `accent_color`, and
`density` as stylesheet classes. GTK also honors `keybindings` for global
refresh, sidebar, and command-palette shortcuts, including `vim` and custom
`action=shortcut` mappings, and applies `terminal_font` plus
`terminal_scrollback` to workspace terminal surfaces. `command_palette_presets`
feeds workspace terminal preset buttons; entries can be known aliases or custom
`Label=command` / `Label: command` entries. Configured `scripts.test`,
`scripts.lint`, `scripts.typecheck`, and `scripts.build` are also exposed as
terminal command presets. Prompt pack metadata is file-backed in settings today;
pack import/export and switching are tracked TODOs until that workflow exists.
Config bootstrap seeds `.archductor/prompt-packs/default.toml` for new projects
and backfills it for existing projects when missing, so prompt loading always has
a safe fallback file.
The CLI can export or import shared and local repository settings bundles. The
other customization fields are saved, merged, and preserved for workflow
surfaces that consume them.

## Platform Stance

Linux is the primary target. The code should keep a portable core where
practical, but product decisions should optimize for Linux desktop quality
first.

- Linux: primary supported platform.
- WSL: likely the best first Windows-adjacent target after Linux.
- macOS: technically possible, but lower priority because the original
  the upstream Conductor app already serves macOS and GTK packaging is less native there.
- Native Windows: possible later, but process groups, PTYs, paths, shells,
  signals, and packaging need deliberate platform abstraction before it is a
  realistic support target.

## CLI Reference

The CLI mirrors the app backend and is useful for smoke tests:

```bash
linux-archductor doctor

linux-archductor repo add <path> [--name <name>]
linux-archductor repo list
linux-archductor repo doctor [<name>]
linux-archductor repo settings <name> export [--local] [--output <path>]
linux-archductor repo settings <name> import <path> [--local]

linux-archductor workspace create <repo> --name <name> --branch <branch> [--base <ref>]
linux-archductor workspace create <repo> --from-issue <number>
linux-archductor workspace create <repo> --from-pr <number>
linux-archductor workspace create <repo> --from-linear <issue-id>
linux-archductor workspace create <repo> --prompt <prompt>
linux-archductor workspace list
linux-archductor workspace archive <name> [--remove-worktree]
linux-archductor workspace restore <name>
linux-archductor workspace discard <name>
linux-archductor workspace delete <name> [--remove-worktree] [--delete-branch]
linux-archductor workspace rename <name> <new-name>

linux-archductor session start <workspace> --kind shell|codex|claude|cursor
linux-archductor session open <workspace> --kind shell|codex|claude|cursor
linux-archductor session stop <workspace>
linux-archductor session list <workspace>

linux-archductor run <workspace>
linux-archductor stop <workspace>
linux-archductor logs <workspace> --run|--session

linux-archductor diff <workspace> [--name-only] [--file <path>]
linux-archductor checks <workspace>
linux-archductor conflicts <workspace>

linux-archductor todo add <workspace> <text...>
linux-archductor todo list <workspace>
linux-archductor todo done <id>

linux-archductor review add <workspace> <file> [--line <n>] <body...>
linux-archductor review list <workspace>
linux-archductor review resolve <id>

linux-archductor pr create <workspace> [--title <title>] [--body <body>] [--draft]
linux-archductor pr view <workspace>
linux-archductor pr checks <workspace>
linux-archductor pr merge <workspace> [--method squash|merge|rebase]

linux-archductor workspace link-dir <workspace> <target-workspace>
linux-archductor workspace linked-dirs <workspace>
linux-archductor workspace unlink-dir <workspace> <target-workspace>

linux-archductor checkpoint create <workspace> [--session <id>] <message...>
linux-archductor checkpoint list <workspace>
linux-archductor checkpoint restore <workspace> <id>
```

## Data Locations

```text
~/.config/linux-archductor/config.toml
~/.local/share/linux-archductor/linux-archductor.db
~/.local/state/linux-archductor/logs/<workspace>/
~/.cache/linux-archductor/
~/archductor/workspaces/<repo>/<workspace>/
```

## Documentation

- [Manual testing checklist](docs/manual-testing-checklist.md)
- [Local deploy and validation guide](docs/deploy-and-local-test.md)
- [Archductor docs parity map](docs/archductor-docs-parity-map.md)
- [Packaging notes](packaging/README.md)

## Known Limits

- Agent session surfaces are structured over local PTY harness logs rather than
  provider-native chat protocols.
- Terminal rendering handles common ANSI/control redraws, but it is not a full
  terminal emulator.
- GitHub PR workflows use the local `gh` CLI and require `gh auth login`.
- Linear workspace creation requires `LINEAR_API_KEY`.
- Exhaustive per-command shortcut customization and deeper layout/theme
  coverage are not finished.
- `checkpoint restore` is destructive: it resets the workspace and removes
  untracked files.
- Flatpak is experimental because arbitrary repository access needs broad
  filesystem permissions.

## License

Apache-2.0
