# Linux Archductor UI Sketches

These sketches define a fast MVP interface for a Linux-native Archductor-like
workflow. They are inspired by the public Conductor product layout and the
official Conductor docs: dark app chrome, a narrow workspace sidebar, central
agent session surface, bottom composer, embedded terminal mode, project
settings, and review/checks affordances.

They are implementation wireframes, not a pixel clone of the macOS app. The
product target is still Archductor parity first; better-than-Archductor ideas
should be chosen deliberately after parity is specified.

## Sketches

- [Main workspace shell](./linux-archductor-main-workspace.svg)
- [New workspace flow](./linux-archductor-new-workspace.svg)
- [Review and checks panel](./linux-archductor-review-checks.svg)

## UI Principles

- Keep the first screen operational: repositories, workspaces, active sessions, diffs, checks, and run state should be visible without a landing page.
- Use a dark, dense layout suited to repeated developer work.
- Keep cards shallow and compact; reserve panels for functional surfaces like session output, diffs, checks, and forms.
- Use city/workspace names as stable anchors.
- Make branch, PR, and run state visible in the workspace row.
- Keep the bottom prompt composer attached to the active workspace and agent.
- Keep agent controls near the composer/session surface: Plan Mode, Fast Mode,
  reasoning/effort, approvals, checkpoints, provider status, and MCP status
  where supported by the selected harness.
- Make project settings feel first-class: scripts, run mode, Spotlight testing,
  Files to copy, `.worktreeinclude`, environment variables, prompts, providers,
  and Git behavior should not feel like hidden CLI configuration.
- Include command palette and shortcut affordances for repeated developer work.
- Make review state visible before PR creation so the user does not need to leave the app to know whether work is ready.

## MVP Screen Set

### Main Workspace

The main screen has three regions:

- left sidebar for repositories, active workspaces, and archived workspaces
- center session area for agent chat, terminal output, and prompt composer
- right review area for changed files, checks, run status, todos, and PR state

This should be the default screen after opening a repository.

The shell should support:

- command palette
- keyboard shortcuts
- deep-link entry points for prompt, repository path, issue, and async-plan
  flows
- Big Terminal Mode or equivalent full-center terminal state

### New Workspace

The new workspace dialog creates a branch and worktree from a selected base ref. It should show the setup preview before creation so the user can catch bad branch names, wrong repos, or missing file-copy behavior early.

Minimum fields:

- repository
- base ref
- source type: new task, branch, PR, GitHub issue, Linear issue, prompt
- issue/task link
- workspace name
- branch name
- starting agent
- setup preview
- Files to copy / `.worktreeinclude` preview
- Spotlight testing indicator
- visible directories for monorepos where relevant

### Review And Checks

The review screen focuses on merge readiness:

- changed files
- unified diff
- local comments
- GitHub review comments
- send comments to agent
- PR state
- CI state
- todos
- conflicts
- merge readiness

The MVP can start with a basic unified diff, but it must be good enough to
review real work, leave comments, send comments back to agents, and understand
merge blockers. Side-by-side diff and full GitHub review-thread syncing can
follow after the first usable workflow.

## Visual References

- Official Conductor product page screenshot: https://www.conductor.build/
- Conductor docs workflow model: https://www.conductor.build/docs/concepts/workflow
- Conductor checks model: https://www.conductor.build/docs/reference/checks
- Conductor diff viewer model: https://www.conductor.build/docs/reference/diff-viewer
- Conductor settings model: https://www.conductor.build/docs/reference/settings
- Conductor agent modes: https://www.conductor.build/docs/reference/agent-modes
- Conductor keyboard shortcuts: https://www.conductor.build/docs/reference/keyboard-shortcuts
