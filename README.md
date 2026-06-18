# linux-conductor

A Linux-native parallel coding-agent workflow tool built around Git worktrees.
Each task gets its own isolated workspace: a branch, a directory, a terminal,
agent sessions, run scripts, diffs, checks, and a pull request path.

Inspired by [Conductor](https://conductor.build). Built for Ubuntu, Fedora,
Arch Linux, and other common Linux distributions.

---

## Install

### AppImage (fastest release artifact)

```bash
curl -Lo linux-conductor.AppImage \
  https://github.com/pranavkannepalli/conductor-arch/releases/latest/download/linux-conductor-x86_64.AppImage
chmod +x linux-conductor.AppImage
sudo mv linux-conductor.AppImage /usr/local/bin/linux-conductor
```

The MVP AppImage launches the GTK GUI with no arguments and passes arguments
through to the CLI. It expects common GTK4/libadwaita runtime libraries to be
available on the host; use native packages if your distro does not provide them
by default.

### GTK4 GUI (native desktop app)

The GUI binary is included in the AppImage and native packages.
Build it from source with GTK4 and libadwaita installed:

```bash
# Arch Linux
sudo pacman -S gtk4 libadwaita

# Ubuntu / Debian
sudo apt install libgtk-4-dev libadwaita-1-dev

# Fedora
sudo dnf install gtk4-devel libadwaita-devel

# Build and run
cargo build --release
./target/release/linux-conductor-gtk
```

### Build from source (CLI only)

```bash
# Install Rust if needed
curl https://sh.rustup.rs -sSf | sh

# Clone and build
git clone https://github.com/pranavkannepalli/conductor-arch
cd conductor-arch
cargo build --release
sudo cp target/release/linux-conductor /usr/local/bin/
# Optionally: sudo cp target/release/linux-conductor-gtk /usr/local/bin/
```

---

## Required dependencies

| Tool | Purpose |
|------|---------|
| `git` | Worktree creation and branch management |
| `gh` | GitHub PR creation, checks, and merge |
| `sqlite3` | Local state (bundled, no install needed) |
| `openssh` | Remote repository access |

Optional: `codex`, `claude` (agent sessions), `code`/`cursor` (editor launch).

### Install required tools by distro

Run `linux-conductor doctor` to detect your distro and get exact commands.

**Ubuntu / Debian:**
```bash
sudo apt update && sudo apt install git gh sqlite3 openssh-client
```

**Fedora:**
```bash
sudo dnf install git gh sqlite openssh-clients
```

**Arch Linux:**
```bash
sudo pacman -S git github-cli sqlite openssh
```

**openSUSE:**
```bash
sudo zypper install git gh sqlite3 openssh
```

---

## Quickstart

### 1. Add a repository

```bash
linux-conductor repo add ~/src/my-app
```

This registers the repository, detects the default branch, and sets up a
workspace parent at `~/conductor/workspaces/my-app/`.

### 1b. Launch the GUI

```bash
linux-conductor-gtk
```

The GTK app is currently a prototype toward the real GUI-first Conductor MVP.
It has navigable Dashboard, Projects, History, and Workspace pages, but it is
not yet a finished Conductor clone.

Current GUI capabilities:
- Dashboard with workspace columns.
- Sidebar workspace search/grouping.
- Projects page for adding local repos, cloning Git URLs, listing projects, and
  creating workspaces.
- Workspace page with basic Shell/Codex/Claude/Cursor launch actions, run/stop,
  open-folder, archive/restore/discard, and rough tabs for chats, changes,
  checks, todos, and processes.
- History page that can read old macOS Conductor chats when
  `~/Library/Application Support/com.conductor.app/conductor.db` exists.

Still missing from the real MVP:
- Embedded Conductor-native agent chat.
- Embedded terminal.
- Full project settings editor.
- Rich diff/review/comment UI.
- GUI-first GitHub PR/check/merge workflow.
- Polished Conductor visual parity.

For the corrected MVP spec and handoff, read
[`docs/conductor-gui-mvp-handoff.md`](docs/conductor-gui-mvp-handoff.md).

Launch the GUI pre-selecting a workspace:
```bash
linux-conductor-gtk --workspace berlin
```

### 2. Create workspaces

```bash
linux-conductor workspace create my-app --name berlin --branch feat/search-refactor
linux-conductor workspace create my-app --name tokyo  --branch feat/ui-polish
```

Each workspace gets:
- A dedicated Git worktree on its own branch
- `.context/` with `brief.md`, `agent-notes.md`, and `todos.md`
- Gitignored files copied if listed in `.worktreeinclude` or `file_include_globs`
- A stable port range (`berlin` → 3000, `tokyo` → 3010, …)

### 3. Launch agents or a shell

Interactive sessions open in your terminal emulator and use your existing local
`codex` / `claude` authentication:

```bash
linux-conductor session open berlin --kind codex
linux-conductor session open tokyo  --kind claude
linux-conductor session open berlin --kind shell
```

For supervised background sessions with captured logs:

```bash
linux-conductor session start berlin --kind codex
linux-conductor session start tokyo  --kind claude
linux-conductor session start berlin --kind shell
```

Sessions run from the workspace directory with Conductor-compatible environment
variables (`CONDUCTOR_PORT`, `CONDUCTOR_WORKSPACE_NAME`, etc.).

Open in an editor:
```bash
linux-conductor open berlin --editor code
linux-conductor open tokyo  --editor cursor
```

### 4. Run the app

```bash
linux-conductor run berlin   # starts scripts.run in the background
linux-conductor logs berlin --run
linux-conductor stop berlin
```

### 5. Review changes

```bash
linux-conductor diff berlin
linux-conductor diff berlin --name-only
linux-conductor diff berlin --file src/search.ts
```

### 6. Manage todos

```bash
linux-conductor todo add berlin "add unit tests for search"
linux-conductor todo list berlin
linux-conductor todo done 1
```

### 7. Create and track a pull request

```bash
linux-conductor pr create berlin --title "feat: search refactor"
linux-conductor pr view berlin
linux-conductor pr checks berlin
```

### 8. Check workspace health at a glance

```bash
linux-conductor checks berlin
```

Prints: changed files, run/session status, PR number + state, open todos.

### 9. Merge and archive

```bash
linux-conductor pr merge berlin --method squash
linux-conductor workspace archive berlin --remove-worktree
```

Archive stops running processes, runs `scripts.archive` if configured, marks
the workspace archived, and optionally removes the worktree directory.

Restore a workspace later:
```bash
linux-conductor workspace restore berlin
```

---

## Repository settings

Place `.conductor/settings.toml` in your repository root:

```toml
"$schema" = "https://conductor.build/schemas/settings.repo.schema.json"

file_include_globs = """
.env*
config/*.local.json
"""

[scripts]
setup = "pnpm install"
run   = "pnpm dev --port $CONDUCTOR_PORT"
archive = "./script/workspace-archive.sh"
run_mode = "concurrent"

[environment_variables]
API_BASE_URL = "http://localhost:3000"

[prompts]
general    = "Prefer small, reviewable changes and run focused tests."
code_review = "Focus on correctness, behaviour changes, and missing tests."
create_pr  = "Write concise PR descriptions with test evidence."
```

Override locally (untracked) in `.conductor/settings.local.toml`.

### run_mode

- `concurrent` (default) — multiple workspaces can run simultaneously
- `nonconcurrent` — only one workspace per repository may have an active run
  script; `linux-conductor run` will refuse to start if another is running

### Environment variables available in scripts

| Variable | Value |
|----------|-------|
| `CONDUCTOR_WORKSPACE_NAME` | workspace name |
| `CONDUCTOR_WORKSPACE_PATH` | absolute path to the worktree |
| `CONDUCTOR_ROOT_PATH` | absolute path to the main repository |
| `CONDUCTOR_DEFAULT_BRANCH` | repository default branch |
| `CONDUCTOR_PORT` | base port for this workspace |
| `CONDUCTOR_IS_LOCAL` | always `1` |

---

## All commands

```
linux-conductor doctor

linux-conductor repo add <path> [--name <name>] [--default-branch <branch>]
linux-conductor repo list
linux-conductor repo doctor [<name>]

linux-conductor import conductor [--source <path-to-conductor.db>]

linux-conductor workspace create <repo> --name <name> --branch <branch> [--base <ref>]
linux-conductor workspace create <repo> --from-issue <number> [--branch-prefix <prefix>]
linux-conductor workspace list
linux-conductor workspace list --active
linux-conductor workspace archive <name> [--remove-worktree]
linux-conductor workspace restore <name>
linux-conductor workspace discard <name>
linux-conductor workspace rename <name> <new-name>

linux-conductor run <workspace>
linux-conductor runs <workspace>
linux-conductor stop <workspace>
linux-conductor logs <workspace> --run|--session

linux-conductor session start <workspace> --kind shell|codex|claude|cursor
linux-conductor session open <workspace> --kind shell|codex|claude|cursor [--terminal <terminal>]
linux-conductor session open <workspace> --kind shell|codex|claude|cursor --print-command
linux-conductor session stop <workspace>
linux-conductor session list <workspace>

linux-conductor open <workspace> [--editor code|cursor|vim|...]

linux-conductor diff <workspace> [--name-only] [--file <path>]

linux-conductor todo add <workspace> <text...>
linux-conductor todo list <workspace>
linux-conductor todo done <id>
linux-conductor todo sync <workspace>

linux-conductor pr create <workspace> [--title <t>] [--body <b>] [--draft] [--from-context]
linux-conductor pr view <workspace>
linux-conductor pr checks <workspace>
linux-conductor pr merge <workspace> [--method squash|merge|rebase]

linux-conductor checks <workspace>
linux-conductor status
linux-conductor conflicts <workspace>
linux-conductor archive <workspace> [--remove-worktree]
linux-conductor discard <workspace>
linux-conductor mcp status <workspace>

linux-conductor review add <workspace> <file> [--line <n>] <body...>
linux-conductor review list <workspace>
linux-conductor review resolve <id>

linux-conductor checkpoint create <workspace> [--session <id>] <message...>
linux-conductor checkpoint list <workspace>
linux-conductor checkpoint restore <workspace> <id>
```

---

## Data locations (XDG)

```
~/.config/linux-conductor/config.toml
~/.local/share/linux-conductor/linux-conductor.db
~/.local/state/linux-conductor/logs/<workspace>/
~/.cache/linux-conductor/
~/conductor/workspaces/<repo>/<workspace>/
```

---

## Tested distributions

| Distribution | Status |
|-------------|--------|
| Ubuntu 24.04 LTS | Tier 1 target |
| Debian 12 | Tier 1 target |
| Fedora Workstation 40+ | Tier 1 target |
| Arch Linux | Tier 1 target |
| openSUSE Tumbleweed | Tier 2, AppImage recommended |
| Linux Mint / Pop!_OS | Tier 2, AppImage recommended |
| Manjaro / EndeavourOS | Tier 2, AppImage recommended |
| NixOS | Not yet tested |
| Alpine | Not yet tested |
| WSL | Not yet tested |

## Manual testing

Before cutting a public release, run the MVP smoke path in
[docs/manual-testing-checklist.md](docs/manual-testing-checklist.md). It covers
the CLI demo, GTK GUI workflow, PR/checks path, and package smoke checks.

For a complete local deployment walkthrough, including Claude Code, Codex, and Cursor
interactive sessions, see [docs/deploy-and-local-test.md](docs/deploy-and-local-test.md).

---

## Known limits

- **No embedded terminal or native agent chat yet.** The GUI can launch Shell,
  Codex, Claude Code, and Cursor through the current process/terminal path, but
  the real Conductor-style embedded chat/terminal experience is still MVP work.
  Background `session start` remains available when you want supervised process
  records and captured logs.
- **`gh` required for PR operations.** `pr create`, `pr checks`, and `pr merge`
  shell out to the `gh` CLI. Run `gh auth login` before using these commands.
- **Flatpak is experimental.** The Flatpak build uses `--filesystem=host` for
  arbitrary repository access. Install via AppImage or native package for the
  most reliable experience; the Flatpak manifest is provided for reference.
- **Checkpoint restore is destructive.** `checkpoint restore` hard-resets the
  workspace and removes untracked files. Commit or copy anything you need before
  restoring.
- **Single SQLite database.** Concurrent CLI invocations against the same
  database may occasionally contend; this is fine for interactive use but not
  for scripted parallelism.
- **`run_mode = nonconcurrent` is advisory.** If you start a run script
  outside `linux-conductor run`, the tool cannot detect it.

---

## License

MIT
