# linux-conductor

A Linux-native parallel coding-agent workflow tool built around Git worktrees.
Each task gets its own isolated workspace: a branch, a directory, a terminal,
agent sessions, run scripts, diffs, checks, and a pull request path.

Inspired by [Conductor](https://conductor.build). Built for Ubuntu, Fedora,
Arch Linux, and other common Linux distributions.

---

## Install

### AppImage (fastest, any distro)

```bash
curl -Lo linux-conductor.AppImage \
  https://github.com/pranavkannepalli/conductor-arch/releases/latest/download/linux-conductor-x86_64.AppImage
chmod +x linux-conductor.AppImage
sudo mv linux-conductor.AppImage /usr/local/bin/linux-conductor
```

### Build from source

```bash
# Install Rust if needed
curl https://sh.rustup.rs -sSf | sh

# Clone and build
git clone https://github.com/pranavkannepalli/conductor-arch
cd conductor-arch
cargo build --release
sudo cp target/release/linux-conductor /usr/local/bin/
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

linux-conductor workspace create <repo> --name <name> --branch <branch> [--base <ref>]
linux-conductor workspace list
linux-conductor workspace archive <name> [--remove-worktree]
linux-conductor workspace restore <name>

linux-conductor run <workspace>
linux-conductor stop <workspace>
linux-conductor logs <workspace> --run|--session

linux-conductor session start <workspace> --kind shell|codex|claude
linux-conductor session stop <workspace>

linux-conductor open <workspace> [--editor code|cursor|vim|...]

linux-conductor diff <workspace> [--name-only] [--file <path>]

linux-conductor todo add <workspace> <text...>
linux-conductor todo list <workspace>
linux-conductor todo done <id>

linux-conductor pr create <workspace> [--title <t>] [--body <b>] [--draft]
linux-conductor pr view <workspace>
linux-conductor pr checks <workspace>
linux-conductor pr merge <workspace> [--method squash|merge|rebase]

linux-conductor checks <workspace>
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

---

## Known limits

- **No PTY / interactive terminal.** Sessions are spawned as background
  processes with stdout/stderr captured to log files. Interactive terminal
  multiplexing (Codex inline diff, Claude conversation UI) requires launching
  from a real terminal window. Use `linux-conductor session start … --kind shell`
  to get a shell in the workspace, then run `codex` or `claude` manually for
  the full interactive experience.
- **`gh` required for PR operations.** `pr create`, `pr checks`, and `pr merge`
  shell out to the `gh` CLI. Run `gh auth login` before using these commands.
- **Flatpak not yet supported.** The sandbox conflicts with arbitrary repository
  paths and process supervision. Install via AppImage or native package.
- **Checkpoint restore not implemented.** The plan includes private Git refs for
  per-turn checkpoints; this is deferred past the MVP.
- **Single SQLite database.** Concurrent CLI invocations against the same
  database may occasionally contend; this is fine for interactive use but not
  for scripted parallelism.
- **`run_mode = nonconcurrent` is advisory.** If you start a run script
  outside `linux-conductor run`, the tool cannot detect it.

---

## License

MIT
