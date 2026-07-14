# Archductor Docs Parity Map

Use this map when updating the Archductor MVP docs or implementing the GUI.
The product target is Archductor parity first. Better-than-Archductor features
should be explicit product decisions after parity is understood.

## Canonical Structure

Use this structure consistently across docs, code comments, UI copy, and agent
instructions:

- `Project`: the Archductor entry for one codebase. It owns repository-level
  settings, scripts, instructions, and the list of workspaces.
- `Repository`: the Git codebase behind a project.
- `Workspace`: one isolated copy of a project/repository for one task or PR.
- `Branch`: the Git branch checked out inside a workspace.
- `Working tree`: the files on disk for that workspace.
- `Running environment`: terminals, agents, setup/run scripts, servers, tests,
  and watchers inside the workspace.
- `Turn`: the actions a coding agent takes after one user message and before
  the next user message in the same chat thread. One tool call or file write is
  not a turn.

Relationship model:

- `1 project contains 1 repository`
- `1 repository contains many workspaces`
- `1 workspace maps to 1 branch`
- `1 branch has 1 working tree`
- `1 working tree belongs to 1 workspace`
- `1 workspace can run many processes in its running environment`
- `1 turn can contain many tool calls and file writes`

## Core Product Loop

- Introduction: <https://www.conductor.build/docs>
- Install: <https://www.conductor.build/docs/installation>
- Your first workspace: <https://www.conductor.build/docs/first-workspace>
- Configure your project: <https://www.conductor.build/docs/configure-your-project>

Key expectation: the app guides the user from project and repository onboarding
to workspace creation, agent work, review, pull request, merge, archive, and
history.

## Workspace Model

- Isolated workspaces:
  <https://www.conductor.build/docs/concepts/workspaces-and-branches>
- Workflow: <https://www.conductor.build/docs/concepts/workflow>
- Parallel agents: <https://www.conductor.build/docs/concepts/parallel-agents>
- Testing: <https://www.conductor.build/docs/concepts/testing>
- Git worktrees: <https://www.conductor.build/docs/concepts/git-worktrees>

Key expectation: one workspace is one branch and one Git worktree for one
stream of work. Multiple workspaces are for independently reviewable work;
multiple sessions in one workspace are shared running-environment state inside
that workspace, not separate workspaces.

## Project Setup

- Project scripts: <https://www.conductor.build/docs/reference/scripts>
- Files to copy: <https://www.conductor.build/docs/reference/files-to-copy>
- `.worktreeinclude`: <https://www.conductor.build/docs/reference/worktreeinclude>
- Environment variables:
  <https://www.conductor.build/docs/reference/environment-variables>
- Shell configuration: <https://www.conductor.build/docs/reference/shells>
- Spotlight testing:
  <https://www.conductor.build/docs/reference/scripts/spotlight-testing>
- Settings: <https://www.conductor.build/docs/reference/settings>
- Settings reference:
  <https://www.conductor.build/docs/reference/settings/reference>

Key expectation: setup/run/archive scripts, run mode, Spotlight testing, copied
gitignored files, environment variables, prompts, providers, and Git behavior
are project/repository-level controls visible and editable from the app, not
only through CLI flags.

Linux-specific expectation: customization should go deeper than the default app
surface. Prompts should be GUI-editable because they are part of daily agent
work. Advanced theme, view, layout, and power-user preferences may be
file-editable instead of having dedicated controls for every option.

Additional Linux-first customization targets: branch/workspace naming templates,
commit style, PR title/body templates, setup automation, pre/post hooks, default
agent profiles, approval/reasoning defaults, merge blockers, definition of done,
checkpoint timing, notification rules, keybindings, command palette presets,
terminal presets, dashboard columns, and import/export for team settings.

Platform expectation: keep Linux quality primary while maintaining the native
Windows port. Core process, path, PTY, IPC, shell, and packaging boundaries must
compile on both platforms. CI covers glibc, musl, representative Linux distro
families, and native Windows; real package/runtime smoke is still platform
specific.

## Agents And Tools

- Agent modes: <https://www.conductor.build/docs/concepts/agent-modes>
- Agent behavior: <https://www.conductor.build/docs/reference/agent-behavior>
- Big Terminal Mode:
  <https://www.conductor.build/docs/reference/big-terminal-mode>
- Slash commands: <https://www.conductor.build/docs/reference/slash-commands>
- MCP: <https://www.conductor.build/docs/reference/mcp>
- Harness overview: <https://www.conductor.build/docs/reference/harnesses>
- Claude Code: <https://www.conductor.build/docs/reference/harnesses/claude-code>
- Codex: <https://www.conductor.build/docs/reference/harnesses/codex>
- Cursor: <https://www.conductor.build/docs/reference/harnesses/cursor>

Key expectation: Claude Code, Codex, and Cursor are harnesses inside the
workspace running environment. Archductor owns the project, repository,
workspace, branch, terminal, diff, checks, PR state, and archive/history flow
around those agents.

## Review And Merge

- Diff viewer: <https://www.conductor.build/docs/reference/diff-viewer>
- Checks: <https://www.conductor.build/docs/reference/checks>
- Checkpoints: <https://www.conductor.build/docs/reference/checkpoints>
- Todos: <https://www.conductor.build/docs/reference/todos>
- From issue to PR: <https://www.conductor.build/docs/guides/issue-to-pr>
- Review and merge:
  <https://www.conductor.build/docs/guides/review-and-merge>

Key expectation: review happens in the GUI. Changed files, inline comments,
GitHub comments, failing checks, todos, conflicts, PR state, merge, and archive
are part of one workspace readiness flow.

## App Controls

- Keyboard shortcuts:
  <https://www.conductor.build/docs/reference/keyboard-shortcuts>
- Deep links: <https://www.conductor.build/docs/reference/deep-links>

Key expectation: repeated developer actions are reachable through app controls:
command palette, shortcuts, clickable controls, and deep links.

## Repository Layouts And Parallel Work

- Configure settings:
  <https://www.conductor.build/docs/guides/configure-settings>
- Use Files to copy:
  <https://www.conductor.build/docs/guides/use-files-to-copy>
- Configure providers: <https://www.conductor.build/docs/guides/providers>
- Set up MCP servers:
  <https://www.conductor.build/docs/guides/configure-mcp-servers>
- Work with Cursor:
  <https://www.conductor.build/docs/guides/migrate-from-cursor>
- Work in monorepos:
  <https://www.conductor.build/docs/guides/repositories/monorepos>
- Linking multiple directories:
  <https://www.conductor.build/docs/guides/repositories/linking-multiple-directories>
- Run multiple Claude Code sessions:
  <https://www.conductor.build/docs/guides/parallel-agents/run-multiple-claude-code-sessions>
- Run multiple Codex sessions:
  <https://www.conductor.build/docs/guides/parallel-agents/run-multiple-codex-sessions>
- Run multiple Cursor sessions:
  <https://www.conductor.build/docs/guides/parallel-agents/run-multiple-cursor-sessions>

Key expectation: the GUI should make it natural to fan out work safely, keep
shared context durable, and bring each workspace back through review/merge.

## Safety And Troubleshooting

- Security and permissions:
  <https://www.conductor.build/docs/reference/security-and-permissions>
- Privacy: <https://www.conductor.build/docs/reference/privacy>
- FAQ: <https://www.conductor.build/docs/faq>
- Troubleshooting:
  <https://www.conductor.build/docs/troubleshooting/issues>

Key expectation: the app is explicit that agents run locally with user
permissions, approvals can gate risky actions, model traffic goes to configured
providers, enterprise data privacy changes feature availability, and common
auth/script/workspace/review blockers are surfaced in the UI.
