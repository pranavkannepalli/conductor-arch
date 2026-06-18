# Deploy And Test The Current Prototype Locally

This guide validates the current backend/CLI foundation and rough GTK
prototype. It does not describe a finished GUI-first Conductor MVP. For the
target product spec, see
[`docs/conductor-gui-mvp-handoff.md`](conductor-gui-mvp-handoff.md).

This guide assumes `claude`, `codex`, `gh`, and `git` are already installed and
authenticated on the machine where you test.

## 1. Install System Dependencies

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

These commands should work before launching Linux Conductor:

```bash
gh auth status
codex --version
claude --version
```

If `codex` or `claude` opens normally from your shell, Linux Conductor will use
the same local authentication because it launches those CLIs on your machine.

## 3. Build Locally

From the repository root:

```bash
cargo fmt --all -- --check
cargo test -p linux-conductor-core -p linux-conductor
cargo build --workspace --release --locked
```

The binaries are:

```text
target/release/linux-conductor
target/release/linux-conductor-gtk
```

Optional local install:

```bash
sudo install -Dm755 target/release/linux-conductor /usr/local/bin/linux-conductor
sudo install -Dm755 target/release/linux-conductor-gtk /usr/local/bin/linux-conductor-gtk
```

## 4. Register A Repository

Use any Git repository you can safely create test branches in:

```bash
linux-conductor doctor
linux-conductor repo add /path/to/repo --name demo
linux-conductor repo list
```

On macOS, import existing repositories and workspaces from the Conductor app:

```bash
linux-conductor import conductor
linux-conductor repo list
linux-conductor workspace list
```

The importer reads
`~/Library/Application Support/com.conductor.app/conductor.db`, preserves
archived workspace state, and renames duplicate workspace names with repository
prefixes so CLI commands remain unambiguous.

If the target repo needs workspace setup or run scripts, add:

```toml
# /path/to/repo/.conductor/settings.toml
"$schema" = "https://conductor.build/schemas/settings.repo.schema.json"

file_include_globs = """
.env*
"""

[scripts]
setup = "true"
run = "true"
run_mode = "concurrent"
```

Replace `true` with the repo's real setup and dev-server commands when testing a
real app.

## 5. Create Workspaces

```bash
linux-conductor workspace create demo --name berlin --branch lc/berlin-demo
linux-conductor workspace create demo --name tokyo --branch lc/tokyo-demo
linux-conductor workspace list
```

Each workspace should have:

```text
.context/brief.md
.context/agent-notes.md
.context/todos.md
```

## 6. Codex Session

Open Codex interactively in the `berlin` workspace:

```bash
linux-conductor session open berlin --kind codex
```

This launches your default terminal emulator, changes directory into the
workspace, applies `CONDUCTOR_*` environment variables, and runs `codex`.

If no terminal emulator is detected, print the command and run it manually:

```bash
linux-conductor session open berlin --kind codex --print-command
```

Useful checks inside the Codex terminal:

```bash
pwd
echo "$CONDUCTOR_WORKSPACE_NAME"
echo "$CONDUCTOR_PORT"
git branch --show-current
```

## 7. Claude Code Session

Open Claude Code interactively in the `tokyo` workspace:

```bash
linux-conductor session open tokyo --kind claude
```

This uses your existing local Claude Code login. If your terminal is not
auto-detected:

```bash
linux-conductor session open tokyo --kind claude --terminal gnome-terminal
linux-conductor session open tokyo --kind claude --print-command
```

Supported terminal names include `gnome-terminal`, `kgx`, `konsole`,
`alacritty`, `kitty`, `xterm`, `tilix`, `terminator`, and `xfce4-terminal`.

## 8. Supervised Background Sessions

Use this path when you want process records and captured logs instead of an
interactive terminal:

```bash
linux-conductor session start berlin --kind shell
linux-conductor session list berlin
linux-conductor logs berlin --session
linux-conductor session stop berlin
```

## 9. Run Scripts, Diffs, Todos, And Checks

```bash
linux-conductor run berlin
linux-conductor logs berlin --run
linux-conductor stop berlin

linux-conductor todo add berlin "manual smoke todo"
linux-conductor todo list berlin
linux-conductor checks berlin

linux-conductor diff berlin
linux-conductor status
```

## 10. Pull Request Flow

Make and commit a small test change in a workspace first:

```bash
cd ~/conductor/workspaces/demo/berlin
echo "linux-conductor smoke $(date)" >> linux-conductor-smoke.txt
git add linux-conductor-smoke.txt
git commit -m "test: linux conductor smoke"
```

Then create and inspect the PR:

```bash
linux-conductor pr create berlin --title "test: linux conductor smoke" \
  --body "Manual Linux Conductor smoke test"
linux-conductor pr view berlin
linux-conductor pr checks berlin
```

Merge only if this is a disposable test repository:

```bash
linux-conductor pr merge berlin --method squash
linux-conductor archive berlin --remove-worktree
```

For a non-disposable repo, close the PR manually on GitHub and discard the test
workspace:

```bash
linux-conductor discard berlin
```

## 11. GTK Prototype

Launch the app:

```bash
linux-conductor-gtk
```

Or preselect a workspace:

```bash
linux-conductor-gtk --workspace berlin
```

Manual GUI smoke:

- Confirm the sidebar shows Dashboard, History, repository sections, workspace
  rows, and search.
- Confirm the Dashboard shows project tabs and columns for Backlog, In progress,
  In review, and Done.
- Confirm the Projects page lists repositories and can create a workspace.
- Confirm the Workspace page opens from the sidebar and shows rough Chats,
  Changes, Checks, Todos, and Processes tabs.
- Confirm Shell, Codex, Claude Code, and Cursor actions use the current
  external-process launch path.
- Confirm History lists old Conductor chats when the macOS Conductor database is
  available.
- Use `Ctrl+R` to force-refresh workspace state.

Known GUI MVP gaps:

- No embedded Conductor-native agent chat yet.
- No embedded terminal yet.
- No full project settings editor yet.
- No rich diff/review/comment GUI yet.
- No full GUI-first GitHub PR/check/merge workflow yet.

## 12. Build Release Artifacts

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
./dist/linux-conductor-0.1.0-x86_64.AppImage doctor
./dist/linux-conductor-0.1.0-x86_64.AppImage
```

## 13. Publish

Push the final branch, then tag a release:

```bash
git tag v0.1.0
git push origin v0.1.0
```

The `Publish` workflow builds tarball, `.deb`, `.rpm`, and AppImage artifacts
on Ubuntu 24.04 and attaches them to the GitHub release.
