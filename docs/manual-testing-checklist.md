# Linux Conductor Current Manual Testing Checklist

This checklist tests the current foundation/prototype. It is not a claim that
the GUI-first Conductor MVP is complete. For the target MVP, see
[`docs/conductor-gui-mvp-handoff.md`](conductor-gui-mvp-handoff.md).

Use this checklist on a machine with `git`, `gh`, Rust, GTK4, and libadwaita
development packages installed. Run `gh auth login` before PR tests.

## Build And Launch

- [ ] `cargo build --workspace --release --locked`
- [ ] `./target/release/linux-conductor doctor` prints distro guidance.
- [ ] `./target/release/linux-conductor-gtk` opens the GTK app.

## CLI Demo Path

- [ ] Add a test repository:
  `linux-conductor repo add <repo-path> --name demo`
- [ ] Create two workspaces:
  `linux-conductor workspace create demo --name berlin --branch lc/berlin-demo`
  `linux-conductor workspace create demo --name tokyo --branch lc/tokyo-demo`
- [ ] Confirm each workspace has `.context/` and a unique port block:
  `linux-conductor workspace list`
- [ ] Open interactive shell, Codex, and Claude Code sessions:
  `linux-conductor session open berlin --kind shell`
  `linux-conductor session open berlin --kind codex`
  `linux-conductor session open tokyo --kind claude`
- [ ] Print a manual session command for fallback terminal testing:
  `linux-conductor session open berlin --kind codex --print-command`
- [ ] Start and stop a supervised background shell session:
  `linux-conductor session start berlin --kind shell`
  `linux-conductor session list berlin`
  `linux-conductor session stop berlin`
- [ ] Run and stop the repository run script:
  `linux-conductor run berlin`
  `linux-conductor logs berlin --run`
  `linux-conductor stop berlin`
- [ ] Make a small committed change in `berlin`, then confirm:
  `linux-conductor diff berlin`
  `linux-conductor checks berlin`
- [ ] Add and complete a todo:
  `linux-conductor todo add berlin "manual smoke todo"`
  `linux-conductor todo list berlin`
  `linux-conductor todo done <id>`
- [ ] Create and inspect a checkpoint:
  `linux-conductor checkpoint create berlin "manual smoke checkpoint"`
  `linux-conductor checkpoint list berlin`
- [ ] Create a PR from the workspace branch:
  `linux-conductor pr create berlin --title "manual smoke" --body "Manual MVP smoke test"`
  `linux-conductor pr view berlin`
  `linux-conductor pr checks berlin`
- [ ] Archive or discard the workspace:
  `linux-conductor workspace archive berlin`
  `linux-conductor workspace restore berlin`
  `linux-conductor discard tokyo`

## GUI Demo Path

- [ ] Sidebar shows Dashboard, History, repository grouping, workspace rows, and
  search.
- [ ] Dashboard shows project tabs and Backlog, In progress, In review, and Done
  columns.
- [ ] Projects page can list registered repos.
- [ ] Projects page can create a workspace from a registered repo.
- [ ] Workspace page opens when selecting a workspace.
- [ ] Workspace page shows metadata and rough tabs for Chats, Changes, Checks,
  Todos, and Processes.
- [ ] Workspace page buttons can launch Shell, Codex, Claude Code, and Cursor
  through the current external-process path.
- [ ] Run and Stop buttons call the current run process APIs.
- [ ] Archive, Restore, and Discard buttons call the current lifecycle APIs.
- [ ] History page lists old Conductor chats if the macOS Conductor database is
  available.
- [ ] `Ctrl+R` refreshes the visible workspace state.

## Known GUI MVP Gaps

- [ ] Embedded Conductor-native agent chat is not implemented.
- [ ] Embedded workspace terminal is not implemented.
- [ ] Full project settings editor is not implemented.
- [ ] Rich diff viewer with inline comments is not implemented.
- [ ] GitHub PR/check/merge workflow is not fully available from the GUI.
- [ ] Visual parity with Conductor is not complete.

## Packaging Smoke

- [ ] `VERSION=0.1.0 nfpm package --packager deb --target dist/`
- [ ] `VERSION=0.1.0 nfpm package --packager rpm --target dist/`
- [ ] AppImage launches GUI with no args and CLI with args:
  `./dist/linux-conductor-0.1.0-x86_64.AppImage`
  `./dist/linux-conductor-0.1.0-x86_64.AppImage doctor`
- [ ] Flatpak manifest builds or its failure is documented as a known sandbox or
  dependency limitation.
