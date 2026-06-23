# Agent Harnesses Design

> Goal: build first-class Claude Code and Codex harnesses that Conductor can launch, control, checkpoint, and render with structured transcripts.

## Summary

Conductor already owns workspaces, PTY sessions, checkpoints, and transcript storage. The missing piece is a shared harness layer that turns those primitives into real agent-specific sessions instead of generic terminals with a few option flags.

This design keeps the current PTY model and adds a small, explicit harness abstraction for `Codex` and `Claude`. The harness layer is responsible for launch composition, startup bootstrapping, option translation, and transcript markers. GTK stays the control surface, but the harness logic moves into `core` so CLI and GUI both use the same behavior.

## What This Ships

- Codex and Claude sessions launched from the workspace view.
- Plan mode, fast mode, reasoning/effort, personality, goals, and skills controls for supported harnesses.
- Session checkpoints tied to the active agent turn.
- Transcript prettification for user input, review prompts, harness notices, tool/skill output, and checkpoint events.
- Stable session metadata so the GUI can switch between running sessions and show what each session was started with.

## Architecture

The core of the change is a new harness module in `crates/core` that builds a `SessionLaunch` for each agent kind. It owns the decision of which CLI flags to set, which startup prompt or slash command to inject, and which metadata string to persist in the process table.

`WorkspaceStore::session_launch_with_options` and `start_session_with_options` stay as the public entry points, but they delegate the agent-specific parts to the harness module. That keeps existing callers stable while making Codex and Claude behavior easy to test in one place.

GTK keeps the session panel and transcript view, but it stops treating every session as generic text. Instead, it renders harness metadata and parses the session log into labeled events. Existing PTY-backed live attachment stays in place.

## Harness Behavior

### Codex

- Launch in the repository worktree or configured working directory.
- Use the interactive Codex CLI, not the non-interactive `exec` path, so the session can stay open and keep editing.
- Apply plan mode and fast mode through the Codex slash-command surface when the session starts.
- Apply goals through Codex goal mode when a goal is present.
- Keep Codex-specific personality, goals, and skills in the launch metadata and initial prompt payload so the GUI can show exactly what was requested.
- Prefer local `codex` config overrides and documented CLI flags where they exist.

### Claude Code

- Launch in the repository worktree or configured working directory.
- Use documented Claude Code flags for permission mode and effort when possible.
- Keep skills enabled through Claude’s native skill discovery.
- Use a startup prompt and session metadata for personality and goals where Claude does not expose a direct flag.
- Preserve plan mode as an explicit session state.

## Transcript Rendering

The session transcript renderer keeps the current labeled-event model and extends it to recognize:

- harness notices
- checkpoint notices
- tool-use blocks
- skill references
- slash-command style control events

The goal is not a perfect emulation of each vendor UI. The goal is a readable, stable Conductor transcript that makes the important parts obvious: what the user sent, what the agent said, what tools or skills were invoked, and when the workspace was checkpointed.

## Non-Goals

- Cursor integration.
- A new remote agent service.
- Replacing the PTY model with a browser or websocket transport.
- Building a generic abstraction for every possible agent CLI.
- Exact parity with the vendor UIs.

## Testing

The implementation should be covered by:

- unit tests for launch construction and metadata generation
- unit tests for transcript parsing and rendering
- CLI tests with fake `codex` and `claude` wrappers
- a narrow GTK smoke test if any UI wiring changes

The tests should prove that launch arguments, harness metadata, and transcript labels stay stable without depending on live vendor services.
