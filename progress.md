# Progress

## Current State

Linux Conductor now has a usable app-first loop for one repository:

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
9. Restore archived workspaces and inspect older imported Conductor chats.

The GTK app is usable but still rough. Agent sessions are PTY/transcript based,
terminal rendering is not a full emulator, and several app controls remain
unfinished.

Latest verification on 2026-06-20: `cargo fmt --all -- --check`,
`cargo test -p linux-conductor-core --lib` (96 tests), and
`cargo test -p linux-conductor-gtk` (92 tests) passed for PTY session input,
structured PR readiness summaries, recorded-PR-number fallback,
review-thread resolve/reopen mutations, PR readiness aggregate/attention
rollups, flat and connection-wrapped status rollup parsing, PR-head deployment
API aggregation, PR-head commit-status API aggregation and de-duplication, GTK
PR summary feedback, GTK command palette command mapping, and terminal
transcript cursor handling. GTK terminal transcript rendering now restores the
normal screen after common alternate-screen TUI sequences and handles
additional CSI cursor-position variants used by shells/TUIs.
`linux-conductor workspace source-preflight` reports GitHub ready and Linear
blocked by missing `LINEAR_API_KEY`. Live smoke proved authenticated GitHub
issue workspace creation from temporary issue #11, authenticated GitHub PR
workspace creation from PR #10, and `linux-conductor pr summary` on that
generated PR workspace, including status-check aggregation.
Live smoke also proved review-thread summary, resolve, and reopen against
temporary PR #12 with a real GitHub review thread, and PR-head deployment
aggregation against temporary PR #15 with a real GitHub deployment status and a
pending GitHub Actions check. Phase 7 CLI matrix proof created temporary PR #20
and exercised success, failure, pending, and error commit statuses plus success,
failure, pending, and inactive deployments; cleanup closed the PR and deleted
the temporary branch/deployments. A follow-up temporary PR #21 proved the
rollup/head-status de-duplication path and was also cleaned up. GitHub commit
statuses can be synthesized as success/failure/error/pending through the CLI;
cancelled/skipped remain covered by parser/classifier tests and by real
provider rollups when GitHub returns them. GTK launched and stayed running on
the available display, but source-creation click-through is not yet automated,
though the GTK source form now has focused request-mapping and lifecycle
feedback coverage. Linear live proof is still blocked until `LINEAR_API_KEY` is
set. Broader CLI/GTK suites should be run before release artifacts.

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
- Setup/run/archive script plumbing from `.conductor/settings.toml`.
- Shared/local repository settings load/save, including scripts, run mode,
  Spotlight testing, Files to copy, environment variables, durable prompts,
  provider executable/provider fields, and Git behavior flags.
- Repository action prompts are part of the editable settings model; the app
  should keep them first-class because prompt iteration is core to agent work.
- `.worktreeinclude` precedence over `file_include_globs`.
- Stable per-workspace port allocation.
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
  Terminal, Todos, Processes, Checkpoints, refresh, and sidebar toggle.
- `Ctrl+R` refreshes the app and `Ctrl+B` toggles the sidebar.
- Projects page can add local repos, clone Git URLs, list projects, edit
  shared/local settings, and create workspaces from branch/base, GitHub issue,
  GitHub PR, Linear issue, or prompt.
- Projects source creation uses a test-covered form mapping before dispatching
  to branch, GitHub issue, GitHub PR, Linear, or prompt workspace creation.
- Projects source creation reports test-covered success/failure feedback for
  the selected source type.
- Workspace command center with status header, agents panel, runtime panel,
  changes/checks/review tabs, chat/terminal split, todos, processes, and
  lifecycle controls.
- Agent panel starts PTY-backed Shell, Codex, Claude, and Cursor sessions,
  persists transcripts, sends input, stops selected sessions, creates
  checkpoints, shows harness metadata, surfaces provider/auth/MCP status, and
  sends staged review prompts.
- Terminal panels support one-shot commands, PTY shells, multiple shell tabs,
  transcript persistence/search/history/reload, basic ANSI/control redraw
  handling, alternate-screen restore for TUI transcripts, CSI cursor-position
  variants, and resize propagation.
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
- History page can read old chats from the macOS Conductor database when
  available.

## Known Gaps

- Agent chat is transcript-oriented, not a polished structured message UI with
  attachments.
- Terminal rendering handles common ANSI/control redraws but is not a full
  terminal emulator.
- Command palette search/filtering, broad customizable keyboard shortcuts, deep
  links, and polished Big Terminal Mode are not complete.
- Monorepo directory selection and linked-directory workflows are not complete.
- GitHub review-thread sync now has structured read plus CLI resolve/reopen
  mutation paths that were live-proven against temporary PR #12 with a real
  review thread. Check/deployment aggregation now has live-proven CLI coverage
  for success/failure/pending/error commit statuses, success/failure/pending/
  inactive deployment statuses, PR-head deployment API aggregation, and
  rollup/head-status de-duplication. Cancelled/skipped remain limited to
  parser/classifier coverage or real provider rollups because GitHub's commit
  status API does not synthesize those states.
- Unified local history for all new chats is not complete.
- Project onboarding/settings need more polish and fuller user/managed settings
  visibility.
- Theme/view customization needs a documented config-file model. Not every
  advanced visual or layout option needs a bespoke GUI control.
- Broader customization should cover prompt packs, naming templates, commit/PR
  style, setup automation, hooks, agent profiles, merge blockers, workspace
  defaults, notification rules, keybindings, command presets, and settings
  import/export.
- Platform direction is Linux-first. Keep the core portable where it does not
  compromise Linux quality; consider WSL before native Windows, and treat macOS
  as lower priority while the original Conductor app covers that platform.
- Visual parity with Conductor is not complete.
- Release packaging still needs full manual validation on target distros.

## Release Readiness Phase

Release readiness must include distribution and website launch work, not just
local artifact creation:

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
- Conductor parity references: [`docs/conductor-docs-parity-map.md`](docs/conductor-docs-parity-map.md)

Keep docs grounded in verified app/core behavior. When a feature exists only in
core, CLI, or GTK, say which layer was verified.
