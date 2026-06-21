# Deploy And Test Locally

This guide validates Linux Conductor on a local machine. It covers both the GTK
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
cargo test -p linux-conductor-core -p linux-conductor -p linux-conductor-gtk
cargo build --workspace --release --locked
```

Binaries:

```text
target/release/linux-conductor
target/release/linux-conductor-gtk
```

Optional install:

```bash
sudo install -Dm755 target/release/linux-conductor /usr/local/bin/linux-conductor
sudo install -Dm755 target/release/linux-conductor-gtk /usr/local/bin/linux-conductor-gtk
```

## 4. Launch The App

```bash
linux-conductor-gtk
```

Or preselect a workspace:

```bash
linux-conductor-gtk --workspace berlin
linux-conductor-gtk --workspace berlin --tab checks
linux-conductor-gtk 'linux-conductor://workspace/berlin?tab=review'
linux-conductor-gtk 'linux-conductor://history'
```

Validate the app path with
[manual-testing-checklist.md](manual-testing-checklist.md).

## 5. Minimal Repository Settings

For a repository that needs setup/run commands, add this at the repository root:

```toml
# .conductor/settings.toml
"$schema" = "https://conductor.build/schemas/settings.repo.schema.json"

file_include_globs = """
.env*
"""

spotlight_testing = false

[scripts]
setup = "true"
run = "true"
run_mode = "concurrent"
```

Replace `true` with real project commands. Commit shared settings. Put
machine-local overrides and secrets in `.conductor/settings.local.toml`.
If `.worktreeinclude` exists, it takes precedence over `file_include_globs`.

Repository prompts are part of the customization surface and should be editable
from the Projects settings UI:

```toml
[prompts]
general = "Prefer small, reviewable changes."
code_review = "Focus on correctness and missing tests."
create_pr = "Write concise PR descriptions with test evidence."
fix_errors = "Fix failing checks with the smallest safe change."
resolve_merge_conflicts = "Preserve both sides when possible and explain tradeoffs."
rename_branch = "Use short kebab-case branch names."
commit_generation = "Use conventional commits and include tests run."
test_fixing = "Reproduce the failing test before changing production code."
refactor_style = "Keep behavior-preserving refactors separate."
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
build_command = "pnpm build"

[customization.agent_profiles.default]
agent = "codex"
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
from known aliases or `Label=command` entries; the other fields are merged,
saved, and preserved for workflow surfaces that use them.

## 6. CLI Smoke Path

Register a repository:

```bash
linux-conductor doctor
linux-conductor repo add /path/to/repo --name demo
linux-conductor repo list
linux-conductor repo settings demo export --output /tmp/demo-settings.toml
linux-conductor repo settings demo import /tmp/demo-settings.toml
linux-conductor repo settings demo import /tmp/demo-settings.toml --local
```

Create two workspaces:

```bash
linux-conductor workspace create demo --name berlin --branch lc/berlin-demo
linux-conductor workspace create demo --name tokyo --branch lc/tokyo-demo
linux-conductor workspace list
linux-conductor workspace link-dir berlin tokyo
linux-conductor workspace linked-dirs berlin
```

Open sessions:

```bash
linux-conductor session open berlin --kind codex
linux-conductor session open tokyo --kind claude
linux-conductor session start berlin --kind shell
linux-conductor session list berlin
linux-conductor session stop berlin
linux-conductor history list --workspace berlin
linux-conductor history show <process-id>
```

Run scripts and inspect work:

```bash
linux-conductor run berlin
linux-conductor logs berlin --run
linux-conductor stop berlin

linux-conductor diff berlin
linux-conductor checks berlin
linux-conductor conflicts berlin
linux-conductor workspace source-preflight
linux-conductor pr summary berlin
linux-conductor pr resolve-thread berlin <thread-id>
linux-conductor pr reopen-thread berlin <thread-id>
```

For a workspace created with `workspace create <repo> --from-pr <number>`, `pr
summary` should work even when the local workspace branch was renamed, because
the app records and reuses the PR number. Review-thread resolve/reopen needs a
GitHub review thread node ID from `pr summary`.

Todos and checkpoints:

```bash
linux-conductor todo add berlin "manual smoke todo"
linux-conductor todo list berlin
linux-conductor todo done <id>

linux-conductor checkpoint create berlin "manual smoke checkpoint"
linux-conductor checkpoint list berlin
```

GitHub PR flow:

```bash
cd ~/conductor/workspaces/demo/berlin
echo "linux-conductor smoke $(date)" >> linux-conductor-smoke.txt
git add linux-conductor-smoke.txt
git commit -m "test: linux conductor smoke"

linux-conductor pr create berlin --title "test: linux conductor smoke" \
  --body "Manual Linux Conductor smoke test"
linux-conductor pr view berlin
linux-conductor pr checks berlin
```

Merge only in a disposable repository:

```bash
linux-conductor pr merge berlin --method squash
linux-conductor workspace archive berlin --remove-worktree
```

For a non-disposable repository, close the PR manually and discard the
workspace:

```bash
linux-conductor workspace discard berlin
```

## 7. Import macOS Conductor Data

On macOS, existing Conductor repositories/workspaces can be imported:

```bash
linux-conductor import conductor
linux-conductor repo list
linux-conductor workspace list
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

install -Dm755 target/release/linux-conductor \
  packaging/appimage/linux-conductor.AppDir/usr/bin/linux-conductor
install -Dm755 target/release/linux-conductor-gtk \
  packaging/appimage/linux-conductor.AppDir/usr/bin/linux-conductor-gtk

appimagetool --appimage-extract-and-run \
  packaging/appimage/linux-conductor.AppDir \
  dist/linux-conductor-0.1.0-x86_64.AppImage
```

Smoke the AppImage:

```bash
./dist/linux-conductor-0.1.0-x86_64.AppImage
./dist/linux-conductor-0.1.0-x86_64.AppImage doctor
```
