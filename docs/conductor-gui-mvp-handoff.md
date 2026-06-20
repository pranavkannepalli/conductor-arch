# Conductor GUI MVP Handoff

This document is the source of truth for the corrected MVP direction.

The previous implementation work over-indexed on a CLI/backend workflow and
underbuilt the actual Conductor product experience. The target MVP is a
GUI-first Conductor-style desktop app, not a CLI tool with a dashboard.

Phase 0 must keep this document aligned with the official Conductor docs at
<https://www.conductor.build/docs>. Match Conductor first. Better-than-Conductor
features should be explicit product decisions, not accidental scope drift.
For the local crawl map, see
[`docs/conductor-docs-parity-map.md`](conductor-docs-parity-map.md).

## Product Definition

Conductor is a local desktop control plane for parallel coding agents.

The app lets a user add a repository, create isolated Git worktree workspaces,
run Claude Code, Codex, and Cursor inside those workspaces, inspect the work,
review changes, manage GitHub PRs/checks, and archive completed work without
juggling many terminal windows.

The key product value is that agent coordination happens inside the Conductor
GUI.

## Conductor Parity Baseline

The MVP should preserve the documented Conductor loop:

1. Add or clone a repository.
2. Configure shared project setup, run, archive, files-to-copy, environment,
   provider, and durable prompt settings.
3. Create one workspace per shippable unit, or multiple agent chats in one
   workspace when they share one branch and code state.
4. Run Claude Code, Codex, or Cursor inside the selected workspace.
5. Run setup, app/dev scripts, terminal commands, and tests from the workspace
   unless Spotlight testing is configured for repository-root execution.
6. Review the diff, leave comments, send comments back to an agent, and inspect
   checks/todos/comments/conflicts before merge.
7. Create/update/merge the pull request from the GUI.
8. Archive the workspace and keep chat/history restorable.

Important documented behaviors to match:

- Workspace isolation is development isolation, not a sandbox.
- A workspace maps to one branch and one Git worktree.
- A branch can be checked out in only one worktree at a time.
- Settings are layered: managed, local project override, repository shared,
  user shared, built-in defaults.
- Repository shared settings live in `.conductor/settings.toml`; local
  overrides live in `.conductor/settings.local.toml`.
- `.worktreeinclude` and Files to copy copy gitignored local files only.
- Setup/run/archive scripts and terminals receive `CONDUCTOR_*` variables.
- `CONDUCTOR_PORT` is the base of a workspace-specific port range.
- Normal run scripts execute from the workspace; Spotlight testing exists for
  projects that must run from the repository root.
- MCP, provider settings, model/agent modes, checkpoints, slash commands,
  keyboard shortcuts, command palette, deep links, and tool approvals are part
  of the product surface, even if some are post-MVP polish.

## Core Model

| Term | Meaning | Relationship |
| --- | --- | --- |
| Project | App-level entry for one codebase. Holds repo settings, scripts, instructions, and workspaces. | 1 project contains 1 repository |
| Repository | Local Git codebase behind the project. Can be an existing folder or cloned source. | 1 repository contains many workspaces |
| Workspace | Isolated copy of a project/repo for one task, issue, experiment, or PR. | 1 workspace maps to 1 branch |
| Branch | Git branch checked out inside a workspace. This is the review/PR unit. | 1 branch has 1 worktree |
| Working tree | Files on disk for one workspace, created by Git worktree. | 1 working tree belongs to 1 workspace |
| Running environment | App/server/watchers/tests/terminals running inside a workspace. | 1 workspace can run many processes |
| Agent session | Claude Code, Codex, or Cursor process attached to a workspace. | 1 workspace can have many sessions |

Workspace isolation is development isolation, not a security boundary. Agents
and commands still run on the user's machine with the user's permissions.

## MVP User Flow

1. User opens the Conductor-style GUI.
2. User adds an existing local repo or clones a Git repo.
3. App creates a project entry and shows repo settings/workspaces.
4. User creates a workspace for one task/issue/branch/PR.
5. App creates a Git worktree and branch.
6. User clicks into the workspace.
7. User starts Claude Code, Codex, or Cursor from the workspace page.
8. Agent output appears in a Conductor-native chat UI, not as raw terminal
   output in an unrelated window.
9. User can run setup scripts, run app scripts, open a workspace terminal, view
   logs, and inspect active processes from the workspace page.
10. User reviews changes, checks, todos, comments, and GitHub PR state in the
    same workspace view.
11. User creates/updates/merges a PR and archives the workspace when complete.
12. Archived workspaces and old chats remain available from History.

## Required MVP Features

### 1. GUI Shell

- Native desktop window.
- Left sidebar with projects, active workspaces, archived/history entry, search,
  and status badges.
- Main area with navigable pages: Dashboard, Project, Workspace, Diff/Review,
  Settings, History.
- Command palette and keyboard-shortcut model for core actions:
  - add repository
  - create workspace
  - open workspace
  - start/stop run script
  - show diff
  - show checks
  - create PR
  - merge PR
  - archive workspace
  - open settings
- Basic navigation must also be clickable.
- Deep-link architecture should be planned for prompts, repository paths,
  issues, and async plans even if not all links ship in the first MVP.
- App should not require normal workflow coordination through CLI commands.

### 2. Project/Repository Onboarding

- Add existing local repository from GUI.
- Clone repository from Git URL from GUI.
- Detect default/base branch.
- Fetch latest remote before workspace creation.
- Show repository path, remote, default branch, workspace parent.
- Project settings screen for:
  - setup script
  - run script
  - archive script
  - run mode
  - Spotlight testing
  - environment variables
  - files to copy / include globs
  - `.worktreeinclude` preview/precedence
  - durable instructions/prompts
  - provider executable/API settings
  - Git behavior such as archive-after-merge and branch naming
- Settings UI must reflect Conductor's layered model:
  - managed settings
  - local project override
  - repository shared settings
  - user shared settings
  - built-in defaults
- Do not commit secrets. Shared settings can hold patterns/prompts/scripts;
  local overrides hold machine-specific secrets.

### 3. Workspace Management

- Create a workspace from GUI.
- Workspace creation creates a real Git worktree.
- Workspace maps to one branch.
- Workspace creation can start from a new task, branch, pull request, GitHub
  issue, or Linear issue.
- Workspace names should have durable friendly city-style names while the
  branch/issue/PR remains the primary work identifier.
- Workspace page shows:
  - project/repo
  - branch
  - path
  - run state
  - active agent sessions
  - PR/check status
  - changed file count
  - todos/comments/conflicts
- Workspace page can select visible monorepo directories where sparse checkout
  is needed.
- Workspace page can link sibling workspaces/directories for multi-repository
  work where an agent must read or edit related code.
- Archive, restore, discard, and rename from GUI.
- Destructive actions require confirmations.

### 4. Agent Coordination

- Supported agents: Claude Code, Codex, Cursor.
- Agents use the user's existing local auth.
- Agent sessions are launched from the workspace page.
- Sessions are visible in the workspace page.
- Multiple agents can run in one workspace.
- Multiple workspaces can run agents in parallel.
- Agent output should be formatted in a Conductor-native chat surface.
- Session controls should include the Conductor controls that apply by harness:
  - Plan Mode
  - Fast Mode
  - reasoning/effort level
  - Codex personality
  - Codex goals where supported
  - checkpoints where supported
  - skills where supported
  - tool approvals
- Repository instructions and action prompts should be surfaced:
  - general preferences
  - code review preferences
  - create PR preferences
  - fix errors preferences
  - resolve conflict preferences
  - branch rename preferences
- MCP status should be visible for Claude Code and Codex sessions; Cursor MCP
  is managed by Cursor but should be documented in the UI.
- MVP should avoid forcing users into many external terminals.
- If the first implementation uses PTY-backed processes, the UI should still
  render a readable app-native transcript and session state.

### 5. Terminal And Runtime

- Workspace page includes terminal access scoped to that workspace.
- Big-terminal style mode should be part of the runtime direction: a terminal
  can become the center panel and run common presets or arbitrary commands.
- Run setup script from GUI.
- Run app/dev script from GUI.
- Stop run script from GUI.
- Show process list.
- Show logs.
- Show ports and `CONDUCTOR_*` environment context.
- Support `scripts.run_mode = "concurrent"` and `"nonconcurrent"`.
- Support Spotlight testing for repositories that need root-checkout execution
  or a single shared local stack.

### 6. Git And Review

- Show changed files with file-level additions/deletions summaries.
- Show git status.
- Show recent commits.
- Show unified diff.
- Diff viewer should evolve into:
  - file tree
  - side-by-side or inline diff
  - inline comments
  - send selected comments back to agent
- Show GitHub review comments and local review comments in the same review
  flow where possible.
- Resolve comments/threads from the GUI when the issue is handled.
- Revert selected files from the diff viewer where safe.
- Detect conflicting active workspaces that changed the same files.

### 7. GitHub Integration

Use local `gh` auth for MVP.

Required GUI workflows:

- Push branch / detect upstream state.
- Create PR.
- View PR title, number, URL, state.
- Show CI/check status.
- Surface failing checks.
- Show basic PR review/comment state.
- Let the user send failing checks, unresolved comments, or todos to an agent.
- Merge PR.
- Archive workspace after merge.

The backend can call `gh`; the user should not need to run `gh` manually for
normal Conductor flow.

### 8. Checks/Todos/Attention

Workspace page should answer:

- What changed?
- Is an agent running?
- Is the app/dev server running?
- Are there failing checks?
- Are there open todos?
- Are there unresolved comments?
- Are there conflicts?
- Is this ready to PR/merge/archive?

Todos should be editable from GUI and syncable from `.context`.
Open todos, unresolved comments, failing checks, and conflicts should be
treated as merge blockers unless the user explicitly clears or overrides them.

### 9. History

- History page shows archived workspaces.
- History page shows old chats.
- User can inspect old chats and restore archived workspaces.
- Existing macOS Conductor DB import/read support is useful for migration, but
  the app needs its own unified local history model.

### 10. App Controls, Integrations, And Safety

- Command palette for all major app actions.
- Keyboard shortcuts for navigation, workspace actions, chat, terminal, review,
  Git, and settings.
- `conductor://`-style deep links for:
  - prompt
  - prompt plus repository path
  - issue id plus optional prompt
  - async plan
- Provider settings for Claude Code, Codex, and Cursor.
- Cursor sessions use API-key configuration and can open the workspace in
  Cursor/VS Code without replacing Conductor's workspace ownership.
- Security/privacy model is explicit:
  - agents run locally with the user's permissions
  - approval prompts can gate risky actions
  - model traffic goes to the selected provider
  - enterprise data privacy disables features that require external AI
    providers/custom MCP where applicable

## What Exists Now

### Strong Foundation

- Rust workspace with core, CLI, and GTK crates.
- SQLite-backed repository/workspace/process state.
- Repository add/list/update/doctor.
- Import from macOS Conductor DB.
- Workspace create/list/archive/restore/discard/rename.
- Git worktree creation.
- `.context` initialization.
- setup/run/archive script plumbing.
- per-workspace port allocation.
- run/stop/logs for background scripts.
- Shell/Codex/Claude/Cursor session launch primitives.
- diff/status/log helpers.
- todos, review comments, checkpoints.
- PR create/view/checks/merge through `gh`.
- conflict detection and checks summaries.
- Packaging scaffolding for AppImage, deb, rpm, AUR, Flatpak.

### Rough GTK Prototype

- GTK window with sidebar and pages.
- Dashboard columns.
- Projects page can add local repo, clone repo, list projects, and create a
  workspace.
- Projects page can edit shared/local repository settings and preview
  `.worktreeinclude` precedence.
- Workspace page has basic metadata/actions/tabs.
- History page reads old macOS Conductor chats directly from the Conductor DB.
- Some workspace actions are wired to core APIs.

## What Is Partial Or Misleading

- GUI is functional prototype quality, not MVP-complete.
- Current UI does not match Conductor closely enough.
- Agent sessions still mostly behave like launched external tools.
- There is no true embedded Conductor chat UI.
- There is a basic PTY-backed workspace shell with process records, but not a
  polished terminal emulator with cursor/session management yet. Stale terminal
  process rows are reconciled on app startup and periodic refresh, active PTY
  shells are resized from the GTK terminal allocation, each terminal process
  record has a distinct log path, and PTY command/output chunks are appended to
  that raw transcript log. The visible transcript strips common ANSI/OSC escape
  sequences, applies carriage-return, backspace, cursor-up, cursor-left/right
  overwrite, saved-cursor restore, and erase-line plus clear-screen/home
  redraws, and caps on-screen scrollback while keeping persisted logs raw. The
  terminal panel can search persisted transcript logs with one-line before/after
  context, load a selected past transcript, restore the latest transcript after
  app restart, and list recorded terminal sessions/logs with status counts,
  line/byte counts, and last-output previews newest first. The terminal history
  selector uses the same newest-first order. The terminal panel has clickable
  tabs for recent shells (including stopped sessions) and auto-selects another
  running tab after a stop; broader cursor/session emulation and a more polished
  terminal history/scrollback browser has a clickable session list, but richer
  scrollback browsing beyond basic session loading is still missing.
- Spotlight testing has a first manual checkpoint/apply/restore/switch/sync
  slice with app-wide polling sync, dirty-root refusal before patch reversal,
  review-style root-only affected-path dirty-root guidance in the Runtime panel,
  passive Runtime status for clean vs extra-root-edit state, explicit
  destructive root repair, and app-open recursive file watching for active
  Spotlight workspace trees, but it is not full Conductor-level Spotlight parity.
- Changes/checks/todos/process panels are basic, not review-grade. Local review
  comments can be added and marked resolved from the Review tab.
- GitHub integration is still backend/CLI-heavy; GTK can create, refresh,
  inspect raw checks and PR comments/reviews, stage failing checks or PR
  comments/reviews to an agent, show merge blockers, merge a PR, and archive
  after merge through the Checks tab, but structured GitHub review-thread sync
  and richer checks/deployment aggregation are not complete.
- History reads old Conductor DB but is not a first-class app history model.
- Project settings editor is functional but still needs Conductor-level polish,
  validation, and full managed/user layer visibility.
- Command palette, keyboard shortcut coverage, and deep-link handling are
  missing or incomplete.
- MCP/agent-control surfaces are incomplete.
- Open review comments can be staged into the agent prompt surface, but there is
  still no live selected-agent send/response loop.
- Monorepo directory selection and linked-directory flows are missing.
- Error handling, progress, confirmations, refresh, and toasts are incomplete;
  Runtime and lifecycle button failures now have first-slice app toasts.

## What Is Not Built Yet

- Full GUI-first agent coordination.
- Native formatted Claude/Codex/Cursor chat.
- Resumable in-app agent sessions.
- Polished PTY-backed embedded terminal panes.
- Real Conductor-like visual design.
- Live-verified and polished workspace creation from GitHub issue, branch, PR,
  or Linear issue.
- Full Conductor-level project settings editor, including managed/user layer
  visibility and user-only model defaults.
- Diff viewer with inline comments.
- GUI PR/check/review/merge flow.
- Agent status model comparable to Conductor.
- Command palette, keyboard shortcuts, and deep links.
- MCP/provider settings and status UI.
- Full Spotlight testing parity.
- Monorepo sparse-checkout controls.
- Linked-directory/multi-repository workspace context.
- Safe, polished destructive action flows.
- Release-ready packaging validation.

## Evaluation Of `progress.md`

`progress.md` currently overstates completion.

Incorrect or misleading claims:

- "MVP complete" is not accurate for the GUI-first product.
- "GUI MVP complete" is not accurate.
- "Remaining nice-to-haves" lists features that are actually MVP-critical:
  embedded terminal, real agent UI, GUI GitHub workflow, settings, review
  surfaces.
- Packaging exists, but release readiness should wait until the GUI workflow is
  real.
- Several historical entries describe an older richer GTK layout that was later
  replaced or changed; current docs should describe current behavior only.

Correct interpretation:

- CLI/backend foundation is substantially implemented.
- GTK app is an early prototype.
- The true MVP remains unfinished because the product is GUI-first.

## Implementation Plan

### Phase 0: Documentation Reset

- Rewrite README around the corrected GUI-first MVP.
- Rewrite `progress.md` current-state section.
- Keep this file as the full handoff spec.
- Clearly mark current implementation as foundation/prototype.
- Align all docs with the official Conductor docs crawl:
  - workspace model
  - workflow
  - project settings
  - Files to copy and `.worktreeinclude`
  - run scripts and Spotlight testing
  - agent controls and harness differences
  - diff/checks/todos/review flow
  - command palette, shortcuts, and deep links
  - security/privacy model

### Phase 1: App Architecture Cleanup

- Split `crates/gtk-app/src/main.rs` into modules:
  - `app_shell`
  - `dashboard`
  - `projects`
  - `workspaces`
  - `agents`
  - `terminal`
  - `diff`
  - `checks`
  - `settings`
  - `history`
- Define app state:
  - selected project
  - selected workspace
  - active page
  - active workspace tab
  - running sessions/processes
  - selected agent session
  - review/checks attention state
  - settings layer being edited
- Replace ad hoc refresh closures with an explicit refresh/event model.

### Phase 2: Project Onboarding And Settings

- Build polished add/clone repo flow.
- Add project settings page.
- Persist settings edits to `.conductor/settings.toml` or local settings as
  appropriate.
- Validate setup/run/archive scripts.
- Show file-copy settings and environment variables.
- Show `.worktreeinclude` precedence and read-only state when present.
- Add provider settings and durable action prompts.
- Add Spotlight testing setting.

### Phase 3: Workspace Command Center

- Build real workspace page layout:
  - header/status
  - agents panel
  - runtime panel
  - changes/checks/review tabs
  - chat/terminal split
- Create workspace from GUI with branch/base options.
- Create workspace from branch, PR, GitHub issue, Linear issue, and prompt.
- Archive/restore/discard/rename with confirmations and visible progress.

### Phase 4: Embedded Runtime

- Add embedded terminal support.
- Add Big Terminal Mode direction/preset support.
- Run setup/run scripts from GUI and stream logs.
- Show processes and ports.
- Stop processes reliably.
- Surface errors and exit codes.
- Add Spotlight testing support for root-checkout execution.

### Phase 5: Agent Sessions

- Create PTY/process layer for Claude Code, Codex, and Cursor.
- Persist session metadata and transcript.
- Render app-native messages.
- Track status: idle, working, waiting, errored, done.
- Support Plan Mode, Fast Mode, reasoning/effort controls, Codex personality,
  Codex goals where supported, checkpoints, skills, and tool approvals.
- Surface provider/auth/MCP status.
- Support multiple sessions per workspace.
- Support history/resume where possible.

### Phase 6: Git/Diff/Review

- Build changed-file tree.
- Build usable diff viewer.
- Add inline comments.
- Send comments back to selected agent.
- Show conflicts with sibling workspaces.
- Show commits and branch state.

### Phase 7: GitHub Workflow

- GUI PR create/view/raw-checks/merge first slice.
- Show CI failures.
- Show PR comments/review state.
- Let user ask agents to fix failing checks or review comments.
- Archive after merge.

### Phase 7b: App Controls

- Add command palette.
- Add core keyboard shortcuts.
- Add deep-link handling for prompt, repo path, issue, and async plan flows.

### Phase 8: History And Restore

- Create unified local history model.
- Import existing Conductor sessions/workspaces into that model where possible.
- Browse archived workspaces.
- Restore workspace with chats/context.

### Phase 9: Release Readiness

- Manual smoke tests for the full GUI workflow.
- Visual QA against Conductor screenshots.
- Packaging validation on target Linux distros.
- Remove stale docs/claims.
- Cut release only after GUI workflow is usable without CLI coordination.

## Next Recommended Task

Phase 0 documentation reset is the baseline. Start Phase 1 next:

1. Refactor `crates/gtk-app/src/main.rs` into focused page/component modules.
2. Define explicit app state for selected project, selected workspace, active
   page, active tab, selected agent session, running processes, review/checks
   attention state, and settings layer.
3. Replace ad hoc refresh closures with a clear refresh/event model.
4. Keep each change oriented toward the GUI-first Conductor parity baseline in
   this document and `docs/conductor-docs-parity-map.md`.
