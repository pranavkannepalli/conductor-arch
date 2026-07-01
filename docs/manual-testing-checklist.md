# Linux Archductor Manual Testing Checklist

Use this checklist before calling the app flow healthy or cutting a public
artifact. It focuses on the real Archductor loop: one repository, many
workspaces, multiple agent sessions, review, GitHub PR, merge, archive, repeat.

Run on a machine with `git`, `gh`, Rust, GTK4, libadwaita, and any agent CLIs
you want to test. Run `gh auth login` before GitHub checks. Set
`LINEAR_API_KEY` before Linear checks.

## Build And Launch

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo test -p linux-archductor-core -p linux-archductor -p linux-archductor-gtk`
- [ ] `cargo build --workspace --release --locked`
- [ ] `./target/release/linux-archductor doctor` prints distro guidance.
- [ ] `./target/release/linux-archductor-gtk` opens the GTK app.

## Repository Setup

- [ ] Add an existing local repository from the Projects page.
- [ ] Clone a Git repository from the Projects page.
- [ ] Confirm the repository row shows path, remote/default branch metadata, and
  workspace parent.
- [ ] Edit shared `.archductor/settings.toml` from Projects.
- [ ] Edit local `.archductor/settings.local.toml` from Projects.
- [ ] Export shared settings with
  `linux-archductor repo settings <name> export --output <file>`.
- [ ] Import that file back into shared and local settings with
  `linux-archductor repo settings <name> import <file>` and `--local`.
- [ ] Configure setup, run, archive, run mode, Spotlight testing, Files to copy,
  environment variables, provider executable fields, prompts, and Git behavior.
- [ ] Confirm all repository action prompts are editable from the GUI: general,
  code review, create PR, fix errors, resolve conflicts, rename branch, commit
  generation, test fixing, and refactor style.
- [ ] Confirm prompt profiles or prompt packs can be represented in the
  advanced customization TOML block even if the first UI only edits the active
  values.
- [ ] Confirm final assembled agent prompts can be previewed or exported before
  launch, or mark prompt preview as a known gap.
- [ ] Configure branch naming, workspace naming, commit style, PR title/body
  template, default merge strategy, and archive-after-merge defaults through
  settings or the advanced customization TOML block.
- [ ] Confirm repository setup can be fully automated with setup/run/archive
  scripts and no manual terminal-only step for a normal workspace.
- [ ] Confirm repository automation can represent required local file checks,
  test/lint/build presets, and pre/post hooks, or mark unsupported hooks as a
  known gap.
- [ ] Configure default agent, agent profile, approval mode, reasoning/effort,
  Codex personality/goals, and MCP visibility.
- [ ] Configure merge blockers and a repository-specific definition of done.
- [ ] Confirm PR merge honors configured default merge method, open todo/comment
  blockers, failed-check blockers, and pending-check blockers.
- [ ] Configure workspace defaults: base branch, workspace parent, branch
  prefix, working directory, port block size, auto-open, checkpoint timing, and
  default visible tab/panel. Confirm new workspace creation honors configured
  base branch, branch prefix, and port block size.
- [ ] Confirm `.worktreeinclude` wins over Files to copy settings and is shown
  as a read-only preview when present.
- [ ] Confirm shared settings do not encourage committing secrets.
- [ ] Confirm advanced view/theme/customization settings are documented as
  config-file editable when they do not have GUI controls.
- [ ] Confirm file-editable settings cover light/dark/system theme, accent
  color, density, sidebar layout, diff preference, terminal font/scrollback,
  transcript display, dashboard columns, notification rules, keybindings,
  command palette presets, and settings import/export, or mark missing areas as
  known gaps.

## Workspace Creation

- [ ] Create a branch/base workspace from the GUI.
- [ ] Create a prompt workspace and confirm `.context/brief.md` contains the
  prompt.
- [ ] Create a GitHub issue workspace with authenticated `gh`.
- [ ] Create a GitHub PR workspace with authenticated `gh`; confirm the PR head
  ref is fetched before the worktree is created.
- [ ] For a PR-sourced workspace whose local branch name differs from the remote
  PR branch, run `linux-archductor pr summary <workspace>` and confirm it uses
  the stored PR number instead of failing branch inference.
- [ ] Confirm GitHub source creation fails clearly when `gh auth status` is not
  authenticated.
- [ ] Click Check Sources on the Projects page and confirm GitHub reports ready
  only when `gh` is installed and authenticated.
- [ ] With authenticated `gh`, click through GitHub issue and GitHub PR
  workspace creation in GTK and confirm the created workspaces include source
  context in `.context/brief.md`.
- [ ] Create a Linear issue workspace with `LINEAR_API_KEY`.
- [ ] Confirm Linear source creation fails clearly without `LINEAR_API_KEY`.
- [ ] Click Check Sources on the Projects page and confirm Linear reports ready
  only when `LINEAR_API_KEY` is set.
- [ ] Confirm each workspace maps to one branch and one Git worktree.
- [ ] Confirm each workspace has `.context/brief.md`, `.context/agent-notes.md`,
  and `.context/todos.md`.
- [ ] Confirm two workspaces in the same repository receive different
  `ARCHDUCTOR_PORT` ranges.
- [ ] Confirm branch/worktree conflicts are surfaced clearly.

## Agent Sessions And Terminal

- [ ] Start Shell, Codex, Claude Code, and Cursor sessions from the workspace
  page.
- [ ] Confirm sessions run from the workspace directory with `ARCHDUCTOR_*`
  environment variables.
- [ ] In a monorepo, set `customization.workspace_defaults.working_directory`
  to a tracked subdirectory and confirm setup/run scripts, terminal commands,
  and agent sessions run from that directory while `ARCHDUCTOR_WORKSPACE_PATH`
  still points at the worktree root and `ARCHDUCTOR_WORKING_DIRECTORY` points at
  the subdirectory.
- [ ] Start multiple sessions in one workspace.
- [ ] Start sessions in two workspaces for the same repository.
- [ ] Select a saved or running agent session and confirm the session surface
  shows kind, status, runtime state, attachment state, harness metadata,
  command, event counts, and labeled transcript events for user input, review
  prompts, system notices, and agent output.
- [ ] Send input to a selected live session.
- [ ] Stage a review/check/comment prompt and send it to the selected session.
- [ ] Stop the selected session and confirm the process row updates.
- [ ] Confirm Plan/Fast mode and Codex harness controls affect new sessions.
- [ ] Confirm provider/auth/MCP status text appears where applicable.
- [ ] Link one workspace to another from the workspace page and confirm
  `.context/linked-directories/<target>` points at the target workspace.
- [ ] Run `linux-archductor workspace linked-dirs <workspace>` and confirm the
  same target and symlink path are listed.
- [ ] Start a session in the source workspace and confirm
  `ARCHDUCTOR_LINKED_DIRECTORIES` and `ARCHDUCTOR_LINKED_DIRECTORY_<NAME>` are
  available to the agent process.
- [ ] Run a one-shot terminal command and confirm stdout, stderr, and exit code.
- [ ] Start multiple PTY shells, select one, send input to it, and stop only
  that shell.
- [ ] Run a full-screen TUI command that enters/exits the alternate screen and
  confirm the transcript view returns to the normal shell output afterward.
- [ ] Run cursor-heavy shell output or a TUI transcript and confirm CSI cursor
  variants such as forward, vertical absolute, and vertical relative movement
  do not leave duplicate or misplaced text.
- [ ] Confirm stopped/exited terminal process rows reconcile after restart.
- [ ] Confirm terminal transcripts are persisted, searchable, and reloadable.

## Runtime

- [ ] Run setup from the workspace page and confirm logs/process status.
- [ ] Run the app script and confirm logs/process status.
- [ ] Stop the run script and confirm exit status.
- [ ] Confirm `scripts.run_mode = "concurrent"` allows two workspace run scripts
  in one repository.
- [ ] Confirm `scripts.run_mode = "nonconcurrent"` blocks a second run script in
  the same repository.
- [ ] With `spotlight_testing = true`, confirm Spotlight On applies tracked
  workspace changes to a clean repository root.
- [ ] Confirm Spotlight Sync updates the active root patch after tracked
  workspace edits.
- [ ] Confirm Spotlight Off restores the root.
- [ ] Confirm root-only edits block Spotlight Off/Sync until repaired or cleaned.
- [ ] Confirm Repair Spotlight discards root-only edits and reapplies the active
  workspace patch only after explicit user action.

## Review And Merge

- [ ] Make a small change in a workspace.
- [ ] Confirm Changes shows git status, recent commits, changed-file list,
  additions/deletions, and a file diff.
- [ ] Add a local review comment.
- [ ] Resolve a local review comment.
- [ ] Revert a tracked changed file from the UI.
- [ ] Confirm unsafe untracked-file revert attempts fail visibly.
- [ ] Create two workspaces that edit the same file and confirm sibling
  conflict detection.
- [ ] Copy or inspect conflicting sibling files from the conflict panel.
- [ ] Add and complete todos from the GUI.
- [ ] Create a PR from the Checks tab.
- [ ] Refresh PR state.
- [ ] View raw PR checks.
- [ ] Stage failing PR checks for the selected agent.
- [ ] View raw PR comments/reviews.
- [ ] Stage PR comments/reviews for the selected agent.
- [ ] View the structured PR summary and confirm it includes review decision,
  aggregate rollup counts, latest reviews, comments, review threads, status
  rollup checks, and deployment rollup entries when GitHub returns them.
- [ ] For a PR with a deployment attached to the head commit, confirm the
  structured PR summary includes the deployment environment, latest status, and
  environment/log URL even when it is absent from `statusCheckRollup`.
- [ ] For a PR with legacy commit statuses attached to the head commit, confirm
  the structured PR summary includes success, failure, pending, and error
  statuses even when they are absent from `statusCheckRollup`.
- [ ] Confirm duplicate check/status entries are collapsed when the same status
  appears in both `statusCheckRollup` and the PR-head commit status API result.
- [ ] Confirm the structured PR summary promotes requested changes, unresolved
  review threads, failing/pending checks, and failing/pending deployments into
  the attention section.
- [ ] Confirm cancelled check/deployment states are treated as attention items
  when GitHub returns them in provider rollups.
- [ ] Confirm skipped check states are visible in the raw detail list when
  GitHub returns them; skipped is currently treated as neutral rather than
  promoted to attention.
- [ ] For a PR with review threads, confirm the structured PR summary includes
  GitHub review thread node IDs.
- [ ] Run `linux-archductor pr resolve-thread <workspace> <thread-id>` and
  `linux-archductor pr reopen-thread <workspace> <thread-id>` against a real
  review thread and confirm GitHub updates the thread state.
- [ ] Enter the same thread ID in the Checks tab and confirm Resolve Thread and
  Reopen Thread update the GitHub thread state.
- [ ] Stage the structured PR summary for the selected agent.
- [ ] Confirm merge is blocked by open todos or open local review comments.
- [ ] Merge the PR with squash, merge, or rebase.
- [ ] Confirm `archive_on_merge = true` archives after merge.
- [ ] Archive, restore, rename, and discard workspaces from the GUI.
- [ ] Repeat the create-work-review-merge-archive loop for the same repository.

## History And Navigation

- [ ] Sidebar search finds repositories and workspaces.
- [ ] Dashboard groups active and archived workspaces.
- [ ] History shows archived workspaces.
- [ ] Start a local Shell/Codex/Claude/Cursor session, send at least one
  composer message, refresh History, and confirm the saved Linux session appears.
- [ ] Run `linux-archductor history list --workspace <name>` and confirm the
  saved session row appears with message count and preview.
- [ ] Run `linux-archductor history show <process-id>` and confirm the saved
  transcript is labeled as You, Agent, System, or Review Prompt.
- [ ] History reads old macOS Conductor chats when
  `~/Library/Application Support/com.conductor.app/conductor.db` exists.
- [ ] `Ctrl+K` opens the command palette.
- [ ] Type in the command palette and confirm commands filter by label,
  shortcut, and aliases such as `ci`, `diff`, `chat`, and `terminal`.
- [ ] `Ctrl+R` refreshes the visible workspace state.
- [ ] `Ctrl+B` toggles the sidebar.
- [ ] Use the command palette to navigate Dashboard, Projects, History,
  Workspace, Changes, Checks, Review, Chat/Terminal, Big Terminal, Todos,
  Processes, and Checkpoints.
- [ ] Confirm command palette workspace-tab commands are hidden until a
  workspace is selected.
- [ ] Launch `linux-archductor-gtk --workspace <name> --tab checks` and confirm
  the workspace opens on Checks.
- [ ] Launch `linux-archductor-gtk 'linux-archductor://workspace/<name>?tab=review'`
  and confirm the workspace opens on Review.
- [ ] Launch `linux-archductor-gtk 'linux-archductor://history'` and confirm
  History opens directly.
- [ ] Set `customization.workspace_defaults.default_visible_tab = "checks"` and
  confirm opening/selecting that workspace lands on Checks when no explicit tab
  is passed.
- [ ] Set `customization.view.theme = "light"`, `accent_color = "green"`, and
  `density = "compact"` and confirm selecting the workspace changes the GTK
  stylesheet.
- [ ] Set `customization.view.keybindings = "vim"` and confirm Ctrl+P opens the
  command palette while the palette shows configured Refresh/Sidebar shortcuts.
- [ ] Set `customization.view.terminal_font` and
  `customization.view.terminal_scrollback`, then confirm workspace terminal
  surfaces show the font/scrollback summary and trim output at the configured
  line budget.
- [ ] Set `customization.view.command_palette_presets = ["test",
  "Preview=pnpm dev"]` and confirm the terminal preset row shows Test and
  Preview buttons with the expected commands in their tooltips.

## Known Gaps To Keep Visible

- [ ] Terminal rendering is not a full terminal emulator.
- [ ] Exhaustive per-command shortcut customization is not complete.
- [ ] Deeper layout/theme coverage is not complete.
- [ ] Full naming-template, hook, notification, shortcut, prompt-pack, and
  non-terminal preset customization is not complete.
- [ ] GitHub review-thread resolution has CLI and GTK GraphQL controls with a
  live-proven CLI mutation path; check/deployment aggregation has live-proven
  CLI coverage for success/failure/pending/error commit statuses,
  success/failure/pending/inactive deployments, PR-head deployment API
  coverage, rollup/head-status de-duplication, and real provider
  cancelled/skipped check rollups.
- [ ] Visual parity with Archductor is not complete.

## Packaging Smoke

- [ ] `VERSION=0.1.0 nfpm package --packager deb --target dist/`
- [ ] `VERSION=0.1.0 nfpm package --packager rpm --target dist/`
- [ ] AppImage launches GUI with no args:
  `./dist/linux-archductor-0.1.0-x86_64.AppImage`
- [ ] AppImage forwards CLI args:
  `./dist/linux-archductor-0.1.0-x86_64.AppImage doctor`
- [ ] Flatpak build status is documented if it fails because of sandbox or
  dependency limitations.
- [ ] Tag-driven publish pipeline creates or updates GitHub release artifacts,
  AppImage, APT `.deb` repository metadata, DNF/zypper `.rpm` repository
  metadata, AUR package state, and Flatpak release state.
- [ ] Checksums, provenance, and rollback/yank steps are documented for each
  supported Linux package channel where the channel supports them.
- [ ] Install, upgrade, and launch smoke tests pass from each supported package
  channel, not only from locally built files.
- [ ] Website build for the Linux product subset of `perceo.ai` succeeds and
  publishes matching release downloads, install instructions, supported targets,
  known limits, and GitHub release links.
