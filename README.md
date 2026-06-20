# linux-conductor

Linux Conductor is being built as a GUI-first desktop control plane for
parallel coding agents. The intended product is a Conductor-style app where a
user adds a repository, creates isolated Git worktree workspaces, starts Claude
Code, Codex, or Cursor from those workspaces, reviews changes/checks/PRs, and
archives completed work without coordinating the normal workflow through many
terminal windows.

The current codebase is not a finished GUI-first MVP. It has a strong
Rust/SQLite/Git worktree foundation, a broad CLI, and an early GTK prototype.
The corrected MVP direction and phase plan live in
[`docs/conductor-gui-mvp-handoff.md`](docs/conductor-gui-mvp-handoff.md).
The Conductor docs parity map lives in
[`docs/conductor-docs-parity-map.md`](docs/conductor-docs-parity-map.md).

Inspired by [Conductor](https://conductor.build). Target platforms are Ubuntu,
Fedora, Arch Linux, and other common Linux distributions.

---

## Product Direction

The MVP should match the documented Conductor workflow before adding speculative
extensions:

- Add or clone a repository from the GUI.
- Configure project settings: setup/run/archive scripts, run mode, Spotlight
  testing, Files to copy, `.worktreeinclude`, environment variables, provider
  settings, durable prompts, and Git behavior.
- Create one Git worktree workspace per shippable unit, or run multiple agent
  sessions in one workspace when they must share one branch.
- Run Claude Code, Codex, and Cursor inside the workspace with app-native chat,
  agent status, controls, checkpoints, approvals, provider status, and MCP
  status where supported.
- Run workspace terminals, setup scripts, run scripts, logs, processes, tests,
  and `CONDUCTOR_*` environment context from the workspace page.
- Review changed files, diffs, comments, checks, todos, conflicts, and PR state
  in the app.
- Create/update/merge PRs and archive completed workspaces from the GUI.
- Restore archived workspaces and chats from History.

Useful Conductor docs for parity:

- <https://www.conductor.build/docs/concepts/workspaces-and-branches>
- <https://www.conductor.build/docs/concepts/workflow>
- <https://www.conductor.build/docs/concepts/parallel-agents>
- <https://www.conductor.build/docs/reference/settings>
- <https://www.conductor.build/docs/reference/scripts>
- <https://www.conductor.build/docs/reference/files-to-copy>
- <https://www.conductor.build/docs/reference/diff-viewer>
- <https://www.conductor.build/docs/reference/checks>
- <https://www.conductor.build/docs/reference/agent-modes>

---

## Install

### AppImage (fastest release artifact)

```bash
curl -Lo linux-conductor.AppImage \
  https://github.com/pranavkannepalli/conductor-arch/releases/latest/download/linux-conductor-x86_64.AppImage
chmod +x linux-conductor.AppImage
sudo mv linux-conductor.AppImage /usr/local/bin/linux-conductor
```

The current prototype AppImage launches the GTK GUI with no arguments and
passes arguments through to the CLI. It expects common GTK4/libadwaita runtime
libraries to be available on the host; use native packages if your distro does
not provide them by default.

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

### Build from source (CLI/backend foundation)

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

## Current Prototype Quickstart

Start with the GTK app when evaluating the product direction:

```bash
linux-conductor-gtk
```

The GTK app is currently a prototype toward the real GUI-first Conductor MVP,
not the finished MVP. It has navigable Dashboard, Projects, History, and
Workspace pages.

Current GUI capabilities:
- Dashboard with workspace columns.
- Sidebar workspace search/grouping.
- Projects page for adding local repos, cloning Git URLs, listing projects, and
  creating workspaces.
- Projects page includes a repository settings editor for shared/local
  `.conductor` settings: setup/run/archive scripts, run mode, Spotlight
  testing, Files to copy, `.worktreeinclude` precedence preview, environment
  variables, provider executable/provider hints, durable action prompts, and
  basic Git behavior flags. Saves validate run mode and environment variable
  names.
- Workspace page with PTY-backed Shell/Codex/Claude/Cursor session launch,
  readable app-native transcripts, send/stop/checkpoint controls, Plan/Fast
  mode and Codex harness controls, provider/auth/MCP status text, setup/run/stop,
  Spotlight On/Off, open-folder, archive/restore/discard, and tabs for chats,
  terminal, changes, checks, review, todos, checkpoints, and processes. Runtime
  and lifecycle action failures now show both inline status text and app toasts.
- Basic embedded terminal scoped to the workspace. It can run one-shot commands
  with `CONDUCTOR_*` environment variables and can start PTY-backed workspace
  shells that accept input after launch and stream output. The terminal panel
  has clickable live-shell tabs with running/stopped labels, so Start Shell can
  create more than one live PTY shell and Stop Shell targets the selected tab.
  After a tab is stopped, the terminal automatically selects another running
  shell tab when one exists. Started/stopped
  embedded shells refresh into the Processes tab immediately, and the app marks
  stale terminal process rows exited on startup and during periodic runtime
  refresh when their recorded shell PID is no longer alive. Each recorded shell
  gets its own log path, and PTY command/output chunks are appended to that raw
  transcript log. The terminal panel can search persisted terminal transcript
  logs and append matching process/line results with one-line before/after
  context, list recorded terminal sessions/logs with status counts, line/byte
  counts, and last-output previews newest first, keep the transcript selector in
  the same newest-first order, load a selected past transcript into the terminal
  view, and restore the latest transcript into the terminal view after app
  restart.
  Active PTY shells are resized from the GTK terminal allocation.
  The visible transcript strips common ANSI control sequences and applies
  carriage-return, backspace, cursor-up, cursor-left/right overwrite, and
  saved-cursor restore plus erase-line and clear-screen/home redraws. The
  on-screen scrollback is capped while raw transcript logs stay complete, but
  this is not a polished terminal emulator yet.
- Changes tab includes recent commits, git status, branch push state, a
  selectable changed-file tree, a file-level diff summary with
  additions/deletions including untracked files, per-file unified diff preview,
  full-diff fallback, file-scoped inline comments, and safe tracked-file revert
  back to `HEAD`.
- Review tab can add local file/line/body comments, resolve open local comments,
  stage open comments as an agent-ready prompt, and send the staged prompt into
  the selected live agent session when one is attached.
- Checks tab can create and refresh PR state, view raw `gh pr checks` output
  and raw PR comments/reviews, stage failing checks or PR comments/reviews into
  the selected agent session, show merge blockers, merge the PR, and archive
  after merge when repository Git settings enable it.
- First-slice Spotlight testing can apply tracked workspace changes to a clean
  repository root when `spotlight_testing = true`, then reverse that patch on
  stop. Starting Spotlight creates a checkpoint commit for the tracked workspace
  state, and starting Spotlight for another workspace switches the root checkout
  to that workspace's tracked changes. Spotlight Sync manually refreshes the
  root checkout from the active workspace; the selected workspace page also
  polls and auto-syncs changed tracked patches. The app shell also polls active
  Spotlight sessions across pages and watches active Spotlight workspace trees
  while the app is open, so nested file edits can trigger sync before the next
  polling interval. Stop/Sync/Switch refuse to proceed when the root has extra
  edits outside the active Spotlight patch. Repair Spotlight can explicitly
  discard root-only edits and reapply the active patch, and dirty-root Spotlight
  failures prioritize root-only affected paths and warn that Repair Spotlight
  discards root-only edits before reapplying the active patch. Runtime status
  also shows whether the active Spotlight root is clean or has extra root edits
  before the user clicks Stop/Sync.
- History page that can read old macOS Conductor chats when
  `~/Library/Application Support/com.conductor.app/conductor.db` exists.

Still missing from the real MVP:
- Polished Conductor-native agent chat. A PTY-backed app-native session surface
  exists, but it is still transcript-oriented rather than a rich structured
  chat surface.
- Polished PTY terminal emulator behavior.
- More polished project onboarding and settings layout.
- Full Spotlight testing parity beyond app-open file watching/checkpoint sync.
- Command palette, shortcuts, and deep links.
- Richer resumable structured session history beyond saved transcripts and
  best-effort PTY reattach.
- Monorepo directory selection and linked-directory workflows.
- Rich diff/review/comment UI beyond the current file tree, unified diff,
  local inline comments, staged review prompts, local comment resolution, and
  safe tracked-file revert.
- Structured GitHub review-thread sync and richer checks/deployment aggregation
  beyond the first-slice PR controls.
- Polished Conductor visual parity.

Launch the GUI pre-selecting a workspace:
```bash
linux-conductor-gtk --workspace berlin
```

The CLI remains useful for validating the backend foundation while the GUI is
brought up to the corrected MVP:

### 1. Add a repository

```bash
linux-conductor repo add ~/src/my-app
```

This registers the repository, detects the default branch, and sets up a
workspace parent at `~/conductor/workspaces/my-app/`.

### 2. Create workspaces from the backend foundation

```bash
linux-conductor workspace create my-app --name berlin --branch feat/search-refactor
linux-conductor workspace create my-app --name tokyo  --branch feat/ui-polish
```

Each workspace gets:
- A dedicated Git worktree on its own branch
- `.context/` with `brief.md`, `agent-notes.md`, and `todos.md`
- Gitignored files copied if listed in `.worktreeinclude` or `file_include_globs`
- A stable port range (`berlin` → 3000, `tokyo` → 3010, …)

### 3. Launch agents or a shell through the current process path

Interactive sessions currently open in your terminal emulator and use your
existing local `codex` / `claude` authentication:

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

Conductor-style settings are layered:

1. Managed settings.
2. Local project override: `.conductor/settings.local.toml`.
3. Repository shared settings: `.conductor/settings.toml`.
4. User shared settings.
5. Built-in defaults.

Place shared project settings in `.conductor/settings.toml` at your repository
root and commit them when teammates should get the same workflow:

```toml
"$schema" = "https://conductor.build/schemas/settings.repo.schema.json"

file_include_globs = """
.env*
config/*.local.json
"""

spotlight_testing = false

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

Override locally, untracked, in `.conductor/settings.local.toml`. Do not commit
secrets.

Use `.worktreeinclude` when the project should share Files to copy patterns:

```text
.env*
config/*.local.json
certs/local/**
```

Only gitignored files are copied into new local workspaces. Generated files,
dependencies, and fetched secrets usually belong in `scripts.setup` instead.

### run_mode

- `concurrent` (default) — multiple workspaces can run simultaneously
- `nonconcurrent` — only one workspace per repository may have an active run
  script; `linux-conductor run` will refuse to start if another is running
- Spotlight testing — use when the project must run from the repository root or
  one shared local stack instead of one app process per workspace. Current
  prototype support manually checkpoints and applies/restores/switches/syncs
  tracked workspace changes, and the selected workspace page polls for changed
  active patches; the app shell also polls active Spotlight sessions across
  pages. Repair Spotlight can explicitly discard root-only edits and reapply the
  active patch, and dirty-root failures list affected root paths with a
  destructive repair warning. The GTK app also watches active Spotlight
  workspace trees while open and uses file
  events to trigger sync, with polling as a fallback.

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

## Current Backend/CLI Commands

These commands expose the current backend foundation and are useful for testing
or fallback automation while the GUI catches up. They are not the intended
normal product workflow.

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

## Manual Testing

Before cutting a public release, the target GUI-first MVP acceptance path in
[docs/manual-testing-checklist.md](docs/manual-testing-checklist.md) must pass.
The same checklist also keeps a separate current-prototype smoke path for the
existing CLI/backend foundation and rough GTK app.

For a local prototype validation walkthrough, including Claude Code, Codex, and
Cursor interactive sessions, see
[docs/deploy-and-local-test.md](docs/deploy-and-local-test.md).

---

## Known limits

- **Agent chat and terminal polish are still limited.** The GUI has PTY-backed
  Shell/Codex/Claude/Cursor sessions with app-native transcripts, send/stop,
  checkpoints, harness controls, provider/auth/MCP status, staged review prompt
  sending, a PTY-backed workspace shell, and a one-shot command runner. It is
  still transcript-oriented rather than a rich structured chat UI, and broader
  cursor/session polish plus a polished history/scrollback browser beyond
  summarized session listing/transcript search are still MVP work. Latest
  transcript restore is built.
  Background `session start` remains available when you want supervised process
  records and captured logs.
- **Conductor app controls are incomplete.** Command palette, shortcut coverage,
  deep links, richer Big Terminal Mode, richer checkpoint/history browsing, and
  resumable structured chat history are still MVP work.
- **Spotlight is partial.** It can manually apply and restore tracked workspace
  changes against a clean repository root and creates a checkpoint when
  Spotlight starts. Starting Spotlight for a different workspace switches the
  root checkout to that workspace's tracked changes, and Spotlight Sync refreshes
  the active patch manually. The selected workspace page polls for active patch
  changes and auto-syncs them; the app shell also polls active sessions across
  pages and watches active Spotlight workspace trees while the app is open. It
  refuses to reverse the patch when root-only edits are present, shows affected
  root paths when Git can identify them, and Repair Spotlight can explicitly
  discard those root-only edits and reapply the active patch.
- **Project setup UI is functional but not polished.** The Projects page can
  edit shared/local repository settings and preview `.worktreeinclude`
  precedence, but monorepo directory selection, linked-directory flows, and
  Conductor-level visual polish remain.
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
