# Workspace Open Chat Loading And Archcar Recovery Design

## Goal

Opening a workspace should feel immediate even when Codex startup is slow or fails.
Archcar restarts should also self-heal stale managed session state before the UI touches any workspace.

The workspace view should render right away. The chat panel should show existing content if available, plus a lightweight inline loading card while Codex is starting. If startup fails, that failure must be surfaced inline in the same area.

If archcar was destroyed or restarted, previously managed archcar session records must be marked killed during archcar startup so the system never treats stale sessions as still running.

## Problem

Today the workspace open flow triggers Codex startup in the background, but the chat experience does not clearly represent that state. When startup is slow or fails, the panel can look stuck or empty. The user cannot tell whether Codex is still booting, ready, or broken.

Separately, archcar restart behavior can leave stale process records behind. That allows later workspace opens to encounter invalid "running" session state and produce confusing spawn failures instead of recovering cleanly.

## Chosen Approach

Use a chat-panel-local startup state model plus an eager archcar startup sweep.

The workspace navigation path remains immediate and unchanged. The chat panel owns a small Codex startup state and renders a compact inline status card in the transcript column.

When archcar boots, it must reconcile persisted managed sessions before serving requests. Any archcar-managed session record that is still marked `running` but is no longer valid must be marked killed/exited immediately.

This is the smallest useful diff because:

- It does not block workspace open.
- It does not require new global app state.
- It reuses the existing archcar event/response flow.
- It surfaces errors where the user is already looking.
- It removes stale managed-session state before it can poison later workspace opens.

## UX

### Initial open

When a workspace opens:

- Render the workspace shell immediately.
- Render the chat panel immediately.
- If existing transcript/history exists, show it right away.
- If Codex is not yet known ready, show a small inline loading card above the transcript.
- If archcar is recovering from a restart, the UI should still open immediately and reflect the new startup attempt rather than stale session state.

### Loading state

The loading card should:

- Be lightweight and inline with the chat panel content.
- Show a small spinner/loading indicator.
- Use short copy such as `Starting Codex...`.

### Ready state

When archcar reports the Codex session is ready:

- Remove the loading card.
- Continue rendering transcript/live updates normally.

### Error state

If archcar reports session startup failure:

- Replace the loading card with an inline error card.
- Show short failure copy plus the surfaced sidecar error message.
- Keep any existing transcript/history visible below the error card.

### No transcript case

If the workspace has no transcript yet:

- The chat panel still opens immediately.
- Show the loading card and the normal empty chat shell.

## State Model

Add a local UI model inside `crates/gtk-app/src/session_surface.rs`.

```rust
enum CodexStartupState {
    Idle,
    Loading { message: String },
    Error { message: String },
    Ready,
}
```

Expected use:

- `Loading` is the default when opening a workspace without a known ready Codex session.
- `Ready` clears the status card.
- `Error` shows the inline failure card.
- `Idle` is available for non-Codex contexts or fallback cases.

This state is separate from transcript data and separate from global app state.

## Archcar Startup Recovery

Add a startup reconciliation step in archcar server initialization.

On archcar startup:

- Open the workspace database before the server begins serving requests.
- Find persisted archcar-managed session records still marked `running`.
- Treat them as orphaned unless they can be proven valid under the new sidecar lifecycle.
- Mark them killed/exited immediately.

Behavioral rule:

- A fresh archcar instance must not inherit stale `running` managed sessions from an older destroyed sidecar.
- The system should prefer starting a new clean managed session over trying to reuse invalid state.

This is specifically to avoid later workspace-open flows encountering stale records and producing bad launch behavior or confusing errors.

## Event Flow

Use the existing archcar poll loop already running in `session_surface.rs`.

Transitions:

- Workspace opens without ready Codex session -> `Loading`
- `ArchcarEvent::SessionSpawnQueued` -> `Loading`
- `ArchcarEvent::SessionStarted` -> `Loading`
- `ArchcarEvent::SessionReady` -> `Ready`
- `ArchcarEvent::SessionError` -> `Error`
- Status probe showing `ready = true` -> `Ready`
- Failed ensure/send/status response for the active startup path -> `Error` when appropriate

Important rule:

- Error state must be surfaced, not only logged.
- Stale persisted managed sessions from a destroyed sidecar must be cleaned before they can affect these transitions.

## Rendering

Render the card inside the chat message column, above transcript content.

Add a helper that returns an optional widget for the current startup state:

- `Loading` -> spinner + short text
- `Error` -> error icon/visual treatment + message
- `Ready` / `Idle` -> no card

The card should follow the existing visual language in the GTK app rather than introducing a new global component system.

Error copy should prefer user-facing clarity over raw internals, but may include the sidecar message body for now so failures are visible immediately.

## Scope

In scope:

- Immediate workspace open
- Inline loading indicator in chat panel
- Inline startup error surfacing in chat panel
- Preserving existing transcript/history while loading
- Archcar boot-time cleanup of stale managed session records

Out of scope:

- Global banners
- Toast notifications
- Retry button UX
- Reworking sidebar navigation ownership
- Moving startup orchestration out of current archcar event flow
- Full retry UX redesign

## Testing

Add focused regression tests around the new local state logic.

Minimum coverage:

- Workspace-open-derived state enters `Loading` when no ready Codex session is known.
- `SessionReady` clears inline startup status.
- `SessionError` produces surfaced error state.
- Existing transcript rendering is not blocked by loading state.
- Archcar startup sweep marks stale managed sessions as killed/exited before serving requests.
- Clean startup after sweep allows a fresh managed session spawn path.

Prefer small state-transition helpers that can be tested without full GTK integration.

## Risks

Main risk:

- Mixing startup UI state with existing ready-cache logic could create conflicting transitions.
- Over-aggressive session cleanup could mark a valid session dead if ownership rules are unclear.

Mitigation:

- Keep a single small state helper for transitions.
- Drive state from the same archcar events/responses already used for Codex readiness.
- Restrict the sweep to archcar-managed persisted sessions that should never survive sidecar destruction under this model.

## Implementation Notes

- Keep the change local to `session_surface.rs` unless a tiny shared helper clearly reduces complexity.
- Add archcar-side startup reconciliation near server boot, not lazily on workspace open.
- Do not add global app state for this.
- Do not block workspace navigation on sidecar readiness.
- Preserve current logging; add UI surfacing instead of replacing logs.
- The target behavior is that stale-session-derived spawn errors should disappear because invalid state is cleared before requests arrive.
