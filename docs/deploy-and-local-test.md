# Deploy And Test Locally

This guide validates Archductor on a local machine. It covers both the GTK
app and the CLI backend used by the app.

The happy path is app-first: add a repository, create workspaces, run agent
sessions, review changes, create/merge a GitHub PR, archive, and repeat. The
CLI commands below are useful for setup and fallback checks.

## 1. Install Dependencies

Ubuntu / Debian:

```bash
sudo apt update
sudo apt install -y git gh sqlite3 openssh-client pkg-config \
  libgtk-4-dev libadwaita-1-dev
```

Fedora:

```bash
sudo dnf install -y git gh sqlite openssh-clients pkgconf-pkg-config \
  gtk4-devel libadwaita-devel
```

Arch:

```bash
sudo pacman -S --needed git github-cli sqlite openssh pkgconf gtk4 libadwaita
```

Install Rust if needed:

```bash
curl https://sh.rustup.rs -sSf | sh
source "$HOME/.cargo/env"
```

## 2. Confirm Local Auth

```bash
gh auth status
codex --version
claude --version
```

GitHub operations use local `gh` auth. Codex and Claude sessions use your local
CLI authentication.

## 3. Build

```bash
cargo fmt --all -- --check
cargo test -p archductor-core -p archductor -p archductor-gtk
cargo build --workspace --release --locked
```

Binaries:

```text
target/release/archductor
target/release/archductor-gtk
```

Optional install:

```bash
sudo install -Dm755 target/release/archductor /usr/local/bin/archductor
sudo install -Dm755 target/release/archductor-gtk /usr/local/bin/archductor-gtk
```

## 4. Launch The App

```bash
archductor-gtk
```

Or preselect a workspace:

```bash
archductor-gtk --workspace berlin
archductor-gtk --workspace berlin --tab checks
archductor-gtk 'archductor://workspace/berlin?tab=review'
archductor-gtk 'archductor://history'
```

Validate the app path with
[manual-testing-checklist.md](manual-testing-checklist.md).

## 5. Minimal Repository Settings

For a repository that needs setup/run commands, add this at the repository root:

```toml
# .archductor/settings.toml
"$schema" = "https://conductor.build/schemas/settings.repo.schema.json"

file_include_globs = """
.env*
"""

spotlight_testing = false

[scripts]
setup = "true"
run = "true"
test = "true"
lint = "true"
typecheck = "true"
build = "true"
run_mode = "concurrent"
```

Replace `true` with real project commands. Commit shared `.archductor` files
that should travel across workspaces or PCs. Keep workspace `.context/` files
gitignored because they are per-workspace scratch context. Put machine-local
overrides and secrets in `.archductor/settings.local.toml`.
If `.worktreeinclude` exists, it takes precedence over `file_include_globs`.
Check scripts are exposed as workspace terminal command presets today. A
first-class local check runner UI remains a known gap.

Repository prompts are part of the customization surface and should be editable
from the Projects settings UI:

```toml
[prompts]
new_workspace = "Create a short plan before changing code."
general = "Prefer small, reviewable changes."
continue_work = "Inspect current changes before editing."
summarize_session = "Summarize verification and remaining risk."
handoff = "Leave context, changed files, tests, and next steps."
code_review = "Focus on correctness and missing tests."
create_pr = "Write concise PR descriptions with test evidence."
fix_errors = "Fix failing checks with the smallest safe change."
resolve_merge_conflicts = "Preserve both sides when possible and explain tradeoffs."
rename_branch = "Use short kebab-case branch names."
commit_generation = "Use conventional commits and include tests run."
test_fixing = "Reproduce the failing test before changing production code."
refactor_style = "Keep behavior-preserving refactors separate."
setup_script = "Infer setup commands from repo files."
run_script = "Infer run commands and required ports/env."

[prompt_pack]
active = "default"
version = "v1"
path = ".archductor/prompt-packs/default.toml"
```

Other customization areas should be representable in settings even when the GUI
only exposes the most common controls:

```toml
[git]
branch_prefix_type = "custom"
branch_prefix = "lc"
archive_on_merge = true

[customization.naming]
branch_template = "{prefix}/{type}-{slug}"
workspace_name_style = "city"
commit_style = "conventional"
pr_title_template = "{type}: {summary}"
pr_body_sections = ["Summary", "Tests", "Risk", "Rollback"]
default_merge_method = "squash"

[customization.automation]
auto_setup = true
auto_start_agent = "codex"
required_local_files = [".env"]
test_command = "pnpm test"
lint_command = "pnpm lint"
typecheck_command = "pnpm typecheck"
build_command = "pnpm build"

[customization.agent_profiles.default]
agent = "codex"
model = "gpt-5.6-sol"
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
terminal_font = "JetBrains Mono 13"
terminal_scrollback = 5000
command_palette_presets = ["test", "lint", "Preview=pnpm dev"]
diff_preference = "unified"
```

The exact advanced schema may evolve. The product direction is stable: prompts
and common workflow controls belong in the UI; deep theme/view/layout,
keybinding, notification, hook, and command-preset options can be file-editable.
The GTK Projects settings page includes an advanced customization TOML block for
these `[customization]` sections. Workspace creation currently consumes
`customization.workspace_defaults.base_branch`, `branch_prefix`, and
`port_block_size`. Runtime setup/run/archive scripts, terminal commands, and
agent sessions consume `working_directory` as a relative path inside the
worktree. PR merge consumes `customization.naming.default_merge_method` and
merge blockers for open todos, open local review comments, failed checks, and
pending checks. GTK workspace startup and sidebar selection consume
`default_visible_tab` unless an explicit launch tab is provided, and apply the
configured `theme`, `accent_color`, and `density` as stylesheet classes. GTK
also consumes `keybindings` for global refresh, sidebar, and command-palette
shortcuts, applies `terminal_font` plus `terminal_scrollback` to terminal
surfaces, and expands `command_palette_presets` into terminal preset buttons
from known aliases or `Label=command` entries. Configured `scripts.test`,
`scripts.lint`, `scripts.typecheck`, and `scripts.build` are prepended to those
terminal presets. Prompt pack metadata is saved in `.archductor/settings.toml`;
config bootstrap also seeds `.archductor/prompt-packs/default.toml` for new and
existing projects when missing. Richer pack switching/import/export and session
snapshots remain known gaps. The other fields are merged, saved, and preserved
for workflow surfaces that use them.

## 6. CLI Smoke Path

Register a repository:

```bash
archductor doctor
archductor repo add /path/to/repo --name demo
archductor repo list
archductor repo settings demo export --output /tmp/demo-settings.toml
archductor repo settings demo import /tmp/demo-settings.toml
archductor repo settings demo import /tmp/demo-settings.toml --local
```

Create two workspaces:

```bash
archductor workspace create demo --name berlin --branch lc/berlin-demo
archductor workspace create demo --name tokyo --branch lc/tokyo-demo
archductor workspace list
archductor workspace link-dir berlin tokyo
archductor workspace linked-dirs berlin
```

Open sessions:

```bash
archductor session open berlin --kind codex
archductor session open tokyo --kind claude
archductor session start berlin --kind shell
archductor session list berlin
archductor session stop berlin
archductor history list --workspace berlin
archductor history show <process-id>
```

Run scripts and inspect work:

```bash
archductor run berlin
archductor logs berlin --run
archductor stop berlin

archductor diff berlin
archductor checks berlin
archductor conflicts berlin
archductor workspace source-preflight
archductor pr summary berlin
archductor pr resolve-thread berlin <thread-id>
archductor pr reopen-thread berlin <thread-id>
```

For a workspace created with `workspace create <repo> --from-pr <number>`, `pr
summary` should work even when the local workspace branch was renamed, because
the app records and reuses the PR number. Review-thread resolve/reopen needs a
GitHub review thread node ID from `pr summary`.

Todos and checkpoints:

```bash
archductor todo add berlin "manual smoke todo"
archductor todo list berlin
archductor todo done <id>

archductor checkpoint create berlin "manual smoke checkpoint"
archductor checkpoint list berlin
```

GitHub PR flow:

```bash
cd ~/archductor/workspaces/demo/berlin
echo "archductor smoke $(date)" >> archductor-smoke.txt
git add archductor-smoke.txt
git commit -m "test: archductor smoke"

archductor pr create berlin --title "test: archductor smoke" \
  --body "Manual Archductor smoke test"
archductor pr view berlin
archductor pr checks berlin
```

Merge only in a disposable repository:

```bash
archductor pr merge berlin --method squash
archductor workspace archive berlin --remove-worktree
```

For a non-disposable repository, close the PR manually and discard the
workspace:

```bash
archductor workspace discard berlin
```

## 7. Import macOS Conductor Data

On macOS, existing Conductor repositories/workspaces can be imported:

```bash
archductor import conductor
archductor repo list
archductor workspace list
```

The importer reads
`~/Library/Application Support/com.conductor.app/conductor.db`, preserves
archived workspace state, and renames duplicate workspace names with repository
prefixes so CLI commands remain unambiguous.

## 8. Build Release Artifacts

Packaging proves artifact creation; it does not prove product readiness. Run the
manual checklist before publishing.

Install `nfpm`:

```bash
curl -fsSL https://github.com/goreleaser/nfpm/releases/download/v2.38.0/nfpm_2.38.0_Linux_x86_64.tar.gz \
  | sudo tar -xz -C /usr/local/bin nfpm
```

Create `.deb` and `.rpm` packages:

```bash
mkdir -p dist
VERSION=0.1.0 nfpm package --packager deb --target dist/
VERSION=0.1.0 nfpm package --packager rpm --target dist/
```

Create an AppImage:

```bash
sudo curl -fsSL -o /usr/local/bin/appimagetool \
  https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
sudo chmod +x /usr/local/bin/appimagetool

install -Dm755 target/release/archductor \
  packaging/appimage/archductor.AppDir/usr/bin/archductor
install -Dm755 target/release/archductor-gtk \
  packaging/appimage/archductor.AppDir/usr/bin/archductor-gtk

appimagetool --appimage-extract-and-run \
  packaging/appimage/archductor.AppDir \
  dist/archductor-0.1.0-x86_64.AppImage
```

Smoke the AppImage:

```bash
./dist/archductor-0.1.0-x86_64.AppImage
./dist/archductor-0.1.0-x86_64.AppImage doctor
```
