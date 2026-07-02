# Premium Project Setup Flow Design

## Goal

Make Linux Archductor feel like a premium desktop developer product by improving the typography package, replacing the generic add-repository flow with a compact project creation menu plus focused modals, and blocking startup when required command-line dependencies are missing.

## Scope

In scope:

- Premium black/grey visual language with bundled or packaged fonts.
- No Inter, Geist, or other overused SaaS defaults.
- Sidebar plus button opens a compact chooser popover.
- Each project creation path gets its own focused modal.
- Startup readiness check blocks app use when `gh` is missing or when none of `codex`, `claude`, or `opencode` is installed.
- Setup modal gives direct install guidance and a `Recheck` action.

Out of scope:

- Authentication checks for `gh auth`.
- Auto-installing third-party tools.
- Redesigning workspace/session internals.
- New cloud account setup.

## Typography

Use a less generic premium developer-product pairing:

- UI font: `Mona Sans`
- Mono font: `Commit Mono`
- Fallback UI stack: `"Mona Sans", "Adwaita Sans", "SF Pro Text", "Segoe UI", "Cantarell", "Noto Sans", sans-serif`
- Fallback mono stack: `"Commit Mono", "JetBrains Mono", "SF Mono", "Cascadia Mono", "Menlo", monospace`

Implementation should package font files where the target bundle format supports it, then reference those font names in GTK CSS. If a packaging target cannot bundle fonts cleanly yet, it should keep the stack in CSS and include install guidance in packaging docs.

## Project Creation UX

The sidebar plus button should not open a large multi-mode form. It should open a compact dark popover anchored to the plus button.

Popover rows:

- `Open project`
- `Open GitHub project`
- `Quick start`

Each row should have a small vector icon, a clear label, and a stable hover/selected state. The popover should look like a command surface: dark graphite background, subtle border, tight row height, 8px radius or less, no decorative card nesting.

## Open Project Modal

Purpose: register an existing checked-out repository.

Layout:

- Focused title and one-line description.
- Large folder picker surface.
- Selected folder path preview.
- Optional inferred project name field.
- Primary action: `Open project`.

Behavior:

- Browse button opens folder picker.
- If no explicit name is entered, infer from selected folder.
- Existing `add_repository_from_path` behavior remains the backing action.

## Open GitHub Project Modal

Purpose: clone a repository from GitHub.

Layout:

- Top control row with `Load GitHub repositories`.
- Scrollable repository selector.
- Repo rows/cards should show owner/name and URL.
- Optional manual Git URL field remains available.
- Optional project name override.
- Primary action: `Clone project`.

Behavior:

- `Load GitHub repositories` uses the existing `gh repo list` path.
- Selecting a repo fills the URL and inferred project name.
- Manual URL entry still works if the list fails or the user wants another repo.
- Existing `clone_repository_into_default_parent` behavior remains the backing action.

## Quick Start Modal

Purpose: scaffold and register a new local project.

Layout:

- Large selectable template cards.
- Templates:
  - Empty Git Repo
  - Rust CLI
  - Rust Library
- Parent folder picker.
- Project name field.
- Primary action: `Create project`.

Behavior:

- Template card selection replaces the current dropdown.
- Existing `create_repository_from_template` behavior remains the backing action.

## Blocking Setup Modal

On app startup, after the main window and theme are available, run a readiness check.

The modal is blocking if either condition is true:

- `gh` is missing.
- none of `codex`, `claude`, or `opencode` is installed.

The user cannot dismiss or continue while required readiness is unmet. This is intentional because core features fast-fail without these tools.

The modal should show:

- Product title and short setup message.
- Status rows for:
  - GitHub CLI (`gh`)
  - Codex
  - Claude
  - OpenCode
- A clear required/optional distinction:
  - `gh` required.
  - at least one agent required.
- Install guidance:
  - GitHub CLI: https://cli.github.com/
  - Codex: https://developers.openai.com/codex/cli
  - Claude: https://docs.anthropic.com/en/docs/claude-code
  - OpenCode: https://opencode.ai/
- A `Recheck` primary action.
- No `Continue` action until requirements pass.

When requirements pass after `Recheck`, the modal closes automatically. There is no separate `Continue` button.

## Toast Manager

Add a general GTK toast manager so important app events use consistent copy, timing, and severity instead of direct `ToastOverlay` calls.

Variants:

- `Info`: neutral operational updates.
- `Success`: completed actions such as chat/session finished, project created, settings saved.
- `Warning`: recoverable problems or attention-needed states.
- `Error`: failed actions, failed setup checks after recheck, runtime errors.

Behavior:

- The manager wraps `adw::ToastOverlay`.
- Call sites send a `ToastMessage` with variant and text.
- Variant mapping controls timeout and optional prefix/accessibility copy.
- Errors stay visible longer than normal informational toasts.
- Existing raw toast calls should migrate to the manager.

Initial call sites:

- Runtime/action failures in the workspace command center use `Error`.
- Generic action feedback uses `Info` or `Success` depending on the caller.
- Session stopped/finished notification uses `Success` or `Info`.
- Future chat-finished events can call the same manager without new UI plumbing.

## Readiness Model

Add a small shared model in `crates/core/src/doctor.rs` so CLI and GTK can share dependency semantics:

```rust
struct SetupReadiness {
    gh_installed: bool,
    codex_installed: bool,
    claude_installed: bool,
    opencode_installed: bool,
}
```

Derived behavior:

```rust
fn setup_blockers(readiness: &SetupReadiness) -> Vec<SetupBlocker>
```

Rules:

- missing `gh` creates a blocker.
- no installed agent among `codex`, `claude`, `opencode` creates a blocker.
- installed `codex`, `claude`, or `opencode` satisfies the agent requirement.

This model should be unit tested without GTK.

## Styling

Add focused CSS classes rather than relying only on generic modal classes:

- setup modal shell
- setup status rows
- setup status icon/text states
- project creation popover
- project creation popover row
- template cards
- repository selector rows
- folder picker surface

Visual requirements:

- graphite shell, not blue/slate.
- high contrast text with restrained green CTA.
- no nested card clutter.
- subtle borders and row hover states.
- typography uses Mona Sans and Commit Mono stacks.

## Testing

Core/unit tests:

- `setup_blockers` blocks when `gh` is missing.
- `setup_blockers` blocks when all agents are missing.
- `setup_blockers` passes when `gh` and any one agent are present.
- `opencode` is included in dependency/readiness detection.
- Project mode copy/metadata covers the three plus-menu actions.

GTK-focused tests:

- Theme CSS exposes new setup/project modal classes.
- Theme CSS references the premium font stack and does not reintroduce old blue/slate tokens.
- Toast manager maps variants to stable timeout/copy behavior.
- Existing raw `ToastOverlay` usages are replaced by the manager.

Manual verification:

- Launch with missing `gh` in `PATH`: blocking setup modal appears.
- Launch with `gh` present and no agents: blocking setup modal appears.
- Launch with `gh` and one agent: setup modal does not block.
- Sidebar plus opens compact chooser.
- Each chooser item opens the correct focused modal.
- Trigger an action failure and verify an error toast appears.

## Risks

- GTK popover anchoring can be fiddly depending on the exact sidebar button type.
- Bundling fonts may vary by packaging target.
- External install URLs can drift; keep them centralized so copy is easy to update.
- A blocking modal must avoid trapping users after they install tools; `Recheck` must be reliable.

## Success Criteria

- The plus-button flow resembles the referenced Conductor interaction: compact chooser first, focused modal second.
- The add project experience no longer has a generic mode dropdown.
- Startup missing-tool failures are caught before users reach broken features.
- The app visually reads as a premium graphite developer tool.
- Font styling no longer depends on Inter or other overused defaults.
