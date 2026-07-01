# Progress

## Current State

Linux Archductor now has a usable app-first loop for one repository:

1. Add or clone a repository.
2. Configure repository settings.
3. Create branch, prompt, GitHub issue, GitHub PR, or Linear workspaces.
4. Run multiple Shell/Codex/Claude/Cursor sessions in a workspace.
5. Use workspace terminal, setup/run scripts, logs, and process views.
6. Review diffs, todos, local comments, sibling conflicts, PR checks, and PR
   comments.
7. Stage review/check/comment context into a selected agent session.
8. Create, refresh, merge, and optionally archive GitHub PRs through local
   `gh` auth.
9. Restore archived workspaces and inspect older imported upstream Conductor chats.

The GTK app is usable but still rough. Agent sessions run PTY-backed harnesses
and render structured app-native transcript events, terminal rendering is not a
full emulator, and several app controls remain unfinished.

Latest verification on 2026-06-20: `cargo fmt --all -- --check`,
`cargo test -p linux-archductor-core --lib` (96 tests), and
`cargo test -p linux-archductor-gtk` (92 tests) passed for PTY session input,
structured PR readiness summaries, recorded-PR-number fallback,
review-thread resolve/reopen mutations, PR readiness aggregate/attention
rollups, flat and connection-wrapped status rollup parsing, PR-head deployment
API aggregation, PR-head commit-status API aggregation and de-duplication, GTK
PR summary feedback, GTK command palette command mapping, and terminal
transcript cursor handling. GTK terminal transcript rendering now restores the
normal screen after common alternate-screen TUI sequences and handles
additional CSI cursor-position variants used by shells/TUIs.
`linux-archductor workspace source-preflight` reports GitHub ready and Linear
blocked by missing `LINEAR_API_KEY`. Live smoke proved authenticated GitHub
issue workspace creation from temporary issue #11, authenticated GitHub PR
workspace creation from PR #10, and `linux-archductor pr summary` on that
generated PR workspace, including status-check aggregation.
Live smoke also proved review-thread summary, resolve, and reopen against
temporary PR #12 with a real GitHub review thread, and PR-head deployment
aggregation against temporary PR #15 with a real GitHub deployment status and a
pending GitHub Actions check. Phase 7 CLI matrix proof created temporary PR #20
and exercised success, failure, pending, and error commit statuses plus success,
failure, pending, and inactive deployments; cleanup closed the PR and deleted
the temporary branch/deployments. A follow-up temporary PR #21 proved the
rollup/head-status de-duplication path and was also cleaned up. Temporary PR
#28 proved real provider rollup handling for cancelled and skipped check runs
through `linux-archductor pr summary issue19-proof`; cancelled checks were
promoted to attention and the skipped check stayed visible in the raw check
list. GTK launched and stayed running on the available display, but
source-creation click-through is not yet automated, though the GTK source form
now has focused request-mapping and lifecycle feedback coverage. Linear live
proof is still blocked until `LINEAR_API_KEY` is set. Broader CLI/GTK suites
should be run before release artifacts.

Phase 8 was completed on 2026-06-21 for the current PTY-backed harness model:
selected Shell/Codex/Claude/Cursor sessions render a structured session header
with kind, status, runtime state, attachment state, event counts, PID,
start/end/exit metadata, harness options, command, and sanitized transcript
events. Composer input, staged review prompts, harness/system notices, and
agent output are labeled as separate app-native transcript events instead of
raw log text, including live appends while a selected session is running.
Multi-line composer input and fenced multi-line review prompts are preserved as
single labeled transcript events. Focused verification:
`cargo test -p linux-archductor-gtk session_surface::tests -- --nocapture`;
workspace verification: `cargo test --workspace`.

Phase 9 has a substantial customization/settings slice in progress:
repository settings now load, merge, save, and round-trip first-class TOML
sections for additional prompts, naming/git style, repository automation, agent
profiles, merge rules, workspace defaults, and view preferences. The GTK
Projects settings page edits all active prompt fields and exposes an advanced
customization TOML block so saving settings from the app preserves these
sections instead of dropping them. Workspace creation now consumes
`customization.workspace_defaults.base_branch`, `branch_prefix`, and
`port_block_size` when the create request does not provide explicit base or
branch values. Focused verification:
`cargo test -p linux-archductor-core settings::tests -- --nocapture`,
`cargo test -p linux-archductor-core workspace::tests::create_workspace_uses_configured -- --nocapture`,
`cargo test -p linux-archductor-core workspace::tests::create_from_ -- --nocapture`,
and `cargo test -p linux-archductor-gtk projects::tests -- --nocapture`.

Phase 9 also has the first monorepo working-directory slice: repository
settings accept `customization.workspace_defaults.working_directory` as a safe
relative path inside the worktree. Runtime setup/run/archive scripts, one-shot
terminal commands, editor/session launches, and Shell/Codex/Claude/Cursor
session cwd selection use that subdirectory when configured. The environment
keeps `ARCHDUCTOR_WORKSPACE_PATH` as the worktree root and adds
`ARCHDUCTOR_WORKING_DIRECTORY` for the selected process cwd. Focused
verification: `cargo test -p linux-archductor-core monorepo -- --nocapture` and
`cargo test -p linux-archductor-core settings::tests::rejects_unsafe_workspace_working_directory_settings -- --nocapture`.

Phase 9 now consumes merge customization instead of only preserving it: PR merge
uses `customization.naming.default_merge_method` when no explicit method is
provided, preserves the existing open-todo/open-local-comment blockers by
default, lets repositories disable those local blockers, and can block merges
on failed or pending PR checks through `customization.merge_rules`. Focused
verification: `cargo test -p linux-archductor-core merge_pull_request -- --nocapture`
and `cargo test -p linux-archductor-core settings::tests -- --nocapture`.

Phase 9 now has unified local chat history for new Linux sessions: saved
PTY-backed Shell/Codex/Claude/Cursor session process logs are exposed through a
core history API, the GTK History page merges those local sessions with
imported macOS Conductor chats, workspace chat panels show local saved
sessions for the selected worktree, and the CLI has `linux-archductor history
list [--workspace <name>]` plus `linux-archductor history show <process-id>`.
Focused verification: `cargo test -p linux-archductor-core local_chat_history -- --nocapture`,
`cargo test -p linux-archductor-gtk history -- --nocapture`, and
`cargo test -p linux-archductor history -- --nocapture`.

Phase 9 now has linked-directory workflows for multi-repository or
multi-workspace agent tasks: a source workspace can link another active
workspace, the link is persisted in SQLite, materialized as
`.context/linked-directories/<target-workspace>`, and exposed to scripts,
terminal commands, editor launches, and Shell/Codex/Claude/Cursor sessions via
`ARCHDUCTOR_LINKED_DIRECTORIES` and `ARCHDUCTOR_LINKED_DIRECTORY_<NAME>`.
The CLI has `workspace link-dir`, `workspace linked-dirs`, and
`workspace unlink-dir`; the GTK workspace chat column has a Linked Directories
panel for the same workflow. Focused verification:
`cargo test -p linux-archductor-core linked_directory -- --nocapture`,
`cargo test -p linux-archductor linked -- --nocapture`, and
`cargo test -p linux-archductor-gtk workspace_command_center::tests -- --nocapture`.

Phase 9 now has first-class GTK launch targets and deep links: startup accepts
`--workspace <name> --tab <tab>`, workspace deep links like
`linux-archductor://workspace/berlin?tab=checks`, and page deep links like
`linux-archductor://history`. Review is now a distinct workspace tab target, so
command palette navigation and deep links can land on Changes, Checks, or
Review instead of collapsing all three to the outer work stack. Focused
verification: `cargo test -p linux-archductor-gtk launch_target -- --nocapture`,
`cargo test -p linux-archductor-gtk command_palette::tests -- --nocapture`, and
`cargo test -p linux-archductor-gtk workspace_command_center::tests::workspace_tab_stack_name_maps_palette_targets_to_tabs -- --nocapture`.

Phase 9 now consumes the configured default workspace tab: repository
customization exposes view defaults through the workspace store, validates
`customization.workspace_defaults.default_visible_tab`, and GTK startup/sidebar
selection use that tab when opening a workspace unless the launch target passes
an explicit tab. Supported values include Changes, Checks, Review,
Chat/Terminal, Big Terminal, Todos, Processes, and Checkpoints plus common
aliases. Focused verification:
`cargo test -p linux-archductor-core default_visible_tab -- --nocapture`,
`cargo test -p linux-archductor-core workspace_view_defaults -- --nocapture`,
and `cargo test -p linux-archductor-gtk state::tests -- --nocapture`.

Phase 9 now consumes the first GTK visual view preferences too: workspace view
defaults normalize `customization.view.theme`, `accent_color`, and `density`
into stable window CSS classes, startup applies the selected workspace's
preferences, and sidebar workspace selection refreshes those classes. The CSS
currently covers common light/dark surfaces, blue/green/amber/rose accents,
and compact/comfortable spacing; this is not full visual parity or a bespoke
settings UI. Focused verification:
`cargo test -p linux-archductor-gtk view_preferences -- --nocapture`.

Phase 9 now has configurable global GTK keybindings for the shortcuts that were
previously hard-coded: refresh, sidebar toggle, and command palette. Repository
view defaults expose `customization.view.keybindings`; GTK resolves that setting
on startup and workspace selection, supports the default/native preset, a `vim`
preset, and custom mappings such as
`palette=ctrl+p,refresh=ctrl+shift+r,sidebar=ctrl+alt+b`. The command palette
shows the active refresh/sidebar shortcuts and filters by those labels. Focused
verification: `cargo test -p linux-archductor-gtk command_palette -- --nocapture`
and `cargo test -p linux-archductor-core workspace_view_defaults -- --nocapture`.

Phase 9 now has CLI settings bundle import/export for registered repositories:
`linux-archductor repo settings <name> export [--local] [--output <path>]`
exports the exact shared or local TOML file, and
`linux-archductor repo settings <name> import <path> [--local]` parses,
validates, and writes shared or local repository settings. Focused
verification:
`cargo test -p linux-archductor-core settings::tests::repository_toml_helpers_parse_validate_and_serialize_settings -- --nocapture`
and `cargo test -p linux-archductor -- --nocapture`.

Phase 9 GTK polish pass completed on 2026-06-21: seven rough/missing
features were finished. (1) Big Terminal tab removes header chrome in
full-mode and expands scroll area height. (2) Tab navigation keybindings
added `changes`, `checks`, `review`, `chat`, `terminal`, `todos`,
`processes`, and `checkpoints` as eight new optional slots in
`customization.view.keybindings`; shortcuts shown in command palette.
(3) Prompt preview dialog: agents panel checks `prompts.general` before
launching a session and shows a modal with the assembled prompt text and
Cancel/Launch buttons. (4) Prompt profiles: agents panel exposes a
`ComboBoxText` profile selector populated from `customization.agent_profiles`
keys; selected profile passed as `--profile <name>` to session launch.
(5) Custom palette commands: `customization.view.command_palette_presets`
entries surface as `RunCommand` palette entries via known aliases
(`test`, `lint`, `build`, `ci`, etc.) or `Label=command` syntax.
(6) Session-stop notifications: a background 5-second timer fires an
`adw::Toast` when a session transitions from Running to stopped, gated on
`notification_rules` containing any `on_session_stop` variant.
(7) Projects settings polish: prompt text views now each have an individual
labeled section header instead of a single combined label. Full suite:
`cargo test --workspace` — 251 tests pass.

## Implemented

### Core And CLI

- Rust workspace with core, CLI, and GTK crates.
- SQLite-backed repository, workspace, process, PR, todo, review, and checkpoint
  state.
- Repository add/list/update/doctor.
- Import from the macOS Conductor database.
- Workspace create/list/archive/restore/discard/rename.
- Real Git worktree creation with `.context` initialization.
- Workspace creation from branch/base, prompt, GitHub issue, GitHub PR, and
  Linear issue.
- GitHub PR workspace creation fetches the PR head ref before creating the
  worktree.
- Linear creation uses `LINEAR_API_KEY` and fails clearly when it is missing.
- Setup/run/archive script plumbing from `.archductor/settings.toml`.
- Shared/local repository settings load/save, including scripts, run mode,
  Spotlight testing, Files to copy, environment variables, durable prompts,
  provider executable/provider fields, Git behavior flags, and advanced
  customization sections for naming, automation, agent profiles, merge rules,
  workspace defaults, and view preferences.
- Repository action prompts are part of the editable settings model; the app
  should keep them first-class because prompt iteration is core to agent work.
- `.worktreeinclude` precedence over `file_include_globs`.
- Stable per-workspace port allocation with repository-configurable port block
  size.
- Monorepo working-directory defaults for scripts, terminal commands, editor
  launches, and agent sessions, with unsafe relative path validation.
- Linked-directory records and `.context/linked-directories` symlinks so one
  workspace can expose related active workspace checkouts to agents and
  commands.
- Background setup/run/session process rows, logs, exit codes, stop handling,
  and stale-process reconciliation.
- Workspace-scoped one-shot terminal commands.
- PTY-backed shell primitive with input, output, resize, process records, and
  transcript logs.
- Shell/Codex/Claude/Cursor session launch primitives.
- Codex and Claude launches honor configured executable paths.
- First Spotlight testing slice: checkpoint/apply/sync/switch/restore tracked
  workspace patches against a clean repository root, with dirty-root refusal and
  explicit repair.
- Git status/diff/log helpers.
- Todo, review comment, checkpoint, conflict, and checks-summary commands.
- GitHub PR create/view/checks/comments/readiness-summary/merge through local
  `gh` auth.
- PR merge blocks open todos and open local review comments.
- PR merge honors repository merge customization for default merge method,
  local todo/comment blockers, and failed/pending check blockers.
- PR merge can archive the workspace when `git.archive_on_merge = true`.
- PR checks, comments, review decisions, review threads, status rollups, and
  deployment rollups can be converted into agent prompts.
- PR readiness summaries now include compact review/thread/check/deployment
  rollup counts and promote requested changes, unresolved review threads,
  failing/pending checks, and failing/pending deployments into an attention
  section before the raw detail lists.
- PR readiness parsing handles simple and nested GitHub deployment rollup
  shapes, including object-valued environment/status fields, deployment status
  URLs, and flat or connection-wrapped status check rollup entries.
- PR readiness summaries also fetch deployments attached to the PR head commit
  through the GitHub deployments API, which covers deployments that `gh pr view
  --json statusCheckRollup` does not return.
- PR readiness summaries also fetch legacy commit statuses attached to the PR
  head commit through the GitHub commit status API, then de-duplicate matching
  rollup/head status entries by normalized name, status, and URL.
- PR state commands use the stored PR number when a workspace was created from
  a PR, so summaries/checks/reviews do not depend on GitHub inferring the PR
  from a renamed local worktree branch.
- PR review thread summaries include GitHub thread node IDs, and the CLI can
  resolve or reopen those threads through GraphQL.
- GitHub and Linear workspace source creation now has explicit source preflight
  status for `gh`, `gh auth`, and `LINEAR_API_KEY`.
- Packaging scaffolding for AppImage, deb, rpm, AUR, and Flatpak.

### GTK App

- Native GTK/libadwaita app with Dashboard, Projects, History, and Workspace
  pages.
- Sidebar workspace search/grouping.
- Command palette is available from the header or `Ctrl+K`, with commands for
  Dashboard, Projects, History, Workspace, Changes, Chat/Terminal, Big
  Terminal, Todos, Processes, Checkpoints, refresh, sidebar toggle, and any
  custom `command_palette_presets` entries from repository settings. The
  palette filters by command label, shortcut, and workflow aliases such as
  `ci`, `diff`, `chat`, and `terminal`.
- `Ctrl+R` refreshes the app and `Ctrl+B` toggles the sidebar. Tab navigation
  supports eight optional keybindings (`changes`, `checks`, `review`, `chat`,
  `terminal`, `todos`, `processes`, `checkpoints`) via
  `customization.view.keybindings`; active shortcuts shown in palette.
- Projects page can add local repos, clone Git URLs, list projects, edit
  shared/local settings, and create workspaces from branch/base, GitHub issue,
  GitHub PR, Linear issue, or prompt.
- Projects settings can edit all active repository prompts (each with its own
  labeled section: General, Code review, Create PR, Fix errors, Resolve merge
  conflicts, Rename branch, Commit message, Test fixing, Refactor style) plus
  an advanced customization TOML block for file-editable workflow defaults.
- Projects source creation uses a test-covered form mapping before dispatching
  to branch, GitHub issue, GitHub PR, Linear, or prompt workspace creation.
- Projects source creation reports test-covered success/failure feedback for
  the selected source type.
- Workspace command center with status header, agents panel, runtime panel,
  changes/checks/review tabs, chat/terminal split, todos, processes, and
  lifecycle controls.
- Agent panel starts PTY-backed Shell, Codex, Claude, and Cursor sessions,
  persists transcripts, sends input, stops selected sessions, creates
  checkpoints, shows harness metadata, surfaces provider/auth/MCP status, sends
  staged review prompts, persists new composer input as user transcript events,
  and renders the selected session through a structured app-native header plus
  labeled transcript body.
- Terminal panels support one-shot commands, PTY shells, multiple shell tabs,
  transcript persistence/search/history/reload, basic ANSI/control redraw
  handling, alternate-screen restore for TUI transcripts, CSI cursor-position
  variants, resize propagation, configured terminal fonts, and configured
  scrollback limits. Repository command presets expand into terminal preset
  buttons from known aliases or `Label=command` entries. Full-mode Big Terminal
  removes header chrome and expands scroll height.
- Agents panel exposes a profile selector (`ComboBoxText`) from
  `customization.agent_profiles` keys; non-default profile passed as
  `--profile <name>` to session launch. Before launching, if `prompts.general`
  is set, a modal prompt-preview dialog shows the assembled text with Cancel
  and Launch buttons.
- In-app session-stop notifications: a background timer fires an `adw::Toast`
  when a session transitions from Running to stopped, gated on
  `notification_rules` containing an `on_session_stop` variant.
- Runtime panel runs setup/run scripts, stops run scripts, shows log tails, and
  controls the current Spotlight slice.
- Changes tab has changed-file tree, per-file unified diff preview, full-diff
  fallback, recent commits, branch push state, git status, file-scoped comments,
  and safe tracked-file revert.
- Review tab can add/resolve local file comments and stage open comments for the
  selected agent session.
- Checks tab can create/refresh PR state, inspect raw PR checks and PR
  comments/reviews, summarize review/check/review-thread/deployment readiness,
  resolve/reopen review threads by GitHub thread ID, stage
  failures/comments/readiness for the selected agent session, show merge
  blockers, merge PRs, and archive after merge through repository settings.
- Conflict panel detects sibling workspaces that changed the same files and can
  preview or copy sibling file changes.
- History page can read saved Linux session history and old chats from the
  macOS Conductor database when available.

## Known Gaps

- Terminal rendering handles common ANSI/control redraws but is not a full
  terminal emulator.
- Later backlog: repository-native knowledge graph and RAG grounding are not
  built yet. Each repository should get its own managed knowledge graph so
  workspace/repository chats can retrieve local context natively. Knowledge
  points should be tagged by repository plus workspace, then promoted from
  workspace scope to the main repository graph when the underlying work is
  pulled in. Claude harness/tooling should also expose the installed tools
  needed to read/write that graph and use the retrieval flow.
- GitHub review-thread sync now has structured read plus CLI resolve/reopen
  mutation paths that were live-proven against temporary PR #12 with a real
  review thread. Check/deployment aggregation now has live-proven CLI coverage
  for success/failure/pending/error commit statuses, success/failure/pending/
  inactive deployment statuses, PR-head deployment API aggregation,
  rollup/head-status de-duplication, and real provider cancelled/skipped check
  rollups from temporary PR #28.
- Project onboarding/settings need more polish and fuller user/managed settings
  visibility.
- Deeper layout/theme coverage is not complete. Not every advanced visual or
  layout option needs a bespoke GUI control.
- Broader customization should cover prompt packs, naming templates, commit/PR
  style, setup automation, hooks, deeper prompt-pack import/export, and richer
  notification options beyond the single session-stop toast.
- Platform direction is Linux-first. Keep the core portable where it does not
  compromise Linux quality; consider WSL before native Windows, and treat macOS
  as lower priority while the original Conductor app covers that platform.
- Visual parity with Archductor is not complete.
- Release packaging still needs full manual validation on target distros.

## Release Readiness Phase

Release readiness must include distribution and website launch work, not just
local artifact creation:

- Local release readiness is now captured in
  [`docs/release-readiness.md`](docs/release-readiness.md) and automated by
  `scripts/release-readiness.sh`, which runs formatting, clippy, tests, release
  build, doctor, `cargo deny check` when available, and optional local package
  artifact/checksum generation.
- Manual publish dispatch now requires an explicit semantic version so dry runs
  do not accidentally package branch names such as `main` as release versions.
- CI pipelines publish to every supported Linux package channel: GitHub release
  artifacts, AppImage, `.deb`/APT, `.rpm`/DNF or zypper, AUR, and Flatpak.
- Package-manager publishing is repeatable from a tag, records checksums and
  provenance where the channel supports it, and has rollback or yanking steps
  documented.
- Each package channel has an install/upgrade/smoke-test path in
  [`docs/manual-testing-checklist.md`](docs/manual-testing-checklist.md).
- The product website builds as a subset of `perceo.ai`, with release downloads,
  install instructions, supported Linux targets, known limitations, and links
  back to the GitHub release artifacts.
- Website build/publish is part of the release pipeline or a required release
  gate, so a public release cannot ship without matching website content.

## Documentation

- Public overview: [`README.md`](README.md)
- End-to-end validation: [`docs/manual-testing-checklist.md`](docs/manual-testing-checklist.md)
- Local deploy/test guide: [`docs/deploy-and-local-test.md`](docs/deploy-and-local-test.md)
- Archductor parity references: [`docs/archductor-docs-parity-map.md`](docs/archductor-docs-parity-map.md)

Keep docs grounded in verified app/core behavior. When a feature exists only in
core, CLI, or GTK, say which layer was verified.
