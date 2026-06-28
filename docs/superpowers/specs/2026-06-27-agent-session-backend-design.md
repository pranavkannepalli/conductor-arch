# Agent Session Backend Redesign

## Goal

Replace PTY-driven chat sessions in the GTK app with supported agent transports for both Codex and Claude.

This redesign applies only to agent chat sessions. Shell terminals stay PTY-based.

## Why

The current GTK chat path treats Codex and Claude like terminal apps:

- launch a CLI in a PTY
- inject prompt text as keystrokes
- scrape ANSI output
- infer transcript structure after the fact

That is brittle and already failing for Codex. It also puts the GTK app at the wrong abstraction boundary. The agent runtimes should own agent execution semantics. The GTK app should own UI, persistence, workspace context, and rendering.

## Supported Runtime Targets

### Codex

Use `codex app-server` over `stdio://` JSONL transport as the primary backend for Codex sessions.

GTK should communicate with the Codex app server using structured requests and events instead of terminal input emulation.

### Claude

Use the Claude Agent SDK streaming session path as the primary backend for Claude sessions.

GTK should communicate with Claude through the supported SDK session interface instead of PTY keystroke injection.

## Non-Goals

- Do not replace shell terminal sessions.
- Do not migrate run/setup/background process infrastructure in this change.
- Do not redesign the full visual language of the GTK UI.
- Do not normalize Codex and Claude into a fake lowest-common-denominator model API.

## Current State

Current agent sessions are built around:

- `PtySession` in `crates/core/src/pty.rs`
- `SessionConnection` in `crates/gtk-app/src/session_surface.rs`
- transcript logging to process log files
- event recovery by parsing rendered terminal output

The GTK layer currently assumes:

- every agent session is a local terminal process
- user input is transmitted by writing bytes to a PTY
- assistant output is received by polling PTY output
- session resume means terminal reattachment when possible

Those assumptions must be removed for agent chat sessions.

## Recommended Architecture

### 1. Introduce a backend abstraction

Add a new internal abstraction for agent chat sessions:

- `AgentSessionBackend`
- one implementation for Codex
- one implementation for Claude

This abstraction should own:

- backend process or SDK lifecycle
- session start
- session resume
- message submission
- streaming event delivery
- stop/shutdown

Suggested interface shape:

- `start(config) -> AgentSessionHandle`
- `resume(config, native_session_id) -> AgentSessionHandle`
- `send_user_message(session_id, text)`
- `poll_events(session_id) -> Vec<AgentEvent>` or callback/event-channel equivalent
- `stop(session_id)`

The final implementation can use channels instead of polling if that fits GTK integration better. The important part is that GTK consumes structured events, not terminal bytes.

### 2. Normalize backend events

Define a shared event model consumed by GTK and persisted by the app.

Suggested event set:

- `SessionStarted`
- `SessionResumed`
- `Status`
- `UserMessage`
- `AgentMessageDelta`
- `AgentMessageFinal`
- `ToolCallStarted`
- `ToolCallFinished`
- `ApprovalRequested`
- `SystemNotice`
- `SessionFinished`
- `SessionError`

This event model should be narrow and UI-focused. It should not mirror every provider-specific internal detail.

### 3. Persist structured agent transcripts

Persist structured session events instead of only appending raw terminal text.

Required changes:

- keep process/session rows for workspace-level visibility
- add durable storage for agent session events
- store backend-specific native session IDs for resume

Suggested storage additions:

- new table for agent session events keyed by local process/session row
- new metadata fields on session records for backend type and native resume/session ID

Raw backend logs can still exist for debugging, but transcript rendering should be driven by structured events first.

### 4. Split agent sessions from shell terminals

Keep PTY logic for:

- shell sessions
- embedded terminal

Remove PTY assumptions from:

- Codex chat sessions
- Claude chat sessions

This means `SessionConnection` should no longer be the shared mechanism for all agent types.

### 5. Backend-specific implementations

#### Codex backend

Codex backend responsibilities:

- launch `codex app-server`
- communicate over stdio JSONL transport
- create or resume native Codex session/thread state
- map app-server streamed events into `AgentEvent`
- send user messages through the supported protocol
- capture backend errors and surface them as session events

Codex bootstrap settings such as plan mode, fast mode, goals, and similar options should be expressed through supported request/config/session parameters where possible, not through fake terminal boot text.

If some existing harness settings do not map cleanly yet, preserve them as app metadata and only transmit the subset Codex officially supports.

#### Claude backend

Claude backend responsibilities:

- launch or attach to a Claude Agent SDK session
- use streaming mode for interactive GTK chat
- map SDK stream events into `AgentEvent`
- persist SDK-native session identity for resume
- submit user messages through the supported SDK interface

Claude-specific controls should remain available only where they map cleanly to the SDK.

### 6. GTK session surface redesign

The GTK session surface should render from structured events instead of parsed ANSI output.

Required behavior:

- send button submits a backend message event
- streaming assistant output updates the visible transcript incrementally
- tool activity is shown from structured tool events
- session status badges derive from backend status events
- resume displays a continuous transcript across native resumed sessions

The current transcript parser should remain only as compatibility logic for old saved PTY transcripts during migration.

### 7. Resume model

Resume must become backend-native:

- Codex resumes by stored native app-server session/thread identity
- Claude resumes by stored SDK-native session identity

Do not treat “reattach to terminal device” as the primary resume strategy for agent chats anymore.

### 8. Launch model

Session launch should branch by backend type:

- shell -> PTY
- Codex -> Codex backend
- Claude -> Claude backend

Existing session settings UI can stay mostly intact at first, but launch code should route into backend-specific startup paths.

## Migration Strategy

### Phase 1

Introduce the abstraction and Codex backend behind the existing UI.

Target outcome:

- Codex sessions work with structured transport
- shell sessions remain unchanged
- old saved Codex PTY transcripts remain readable

### Phase 2

Migrate Claude sessions to the new backend abstraction.

Target outcome:

- both agent types use supported transports
- PTY chat transport is no longer used for agent sessions

### Phase 3

Clean up legacy parsing and transport code that only existed for PTY-based agent chats.

Target outcome:

- `SessionConnection` is no longer the agent chat transport
- transcript parsing becomes migration-only or can be removed if unused

## Testing Strategy

### Unit tests

Add tests for:

- event normalization for Codex backend messages
- event normalization for Claude backend messages
- session persistence and resume ID storage
- transcript rendering from structured events

### Integration tests

Add tests for:

- launching a Codex backend session and receiving structured events
- sending a user message and observing assistant output events
- launching/resuming a Claude backend session and receiving structured events
- migration path for old saved PTY transcripts

Where live external agent processes are too heavy for CI, add backend fakes that emit realistic event streams and keep one smaller smoke path for real-process coverage if practical.

### Manual verification

Manual checks must include:

- start Codex session
- send prompt
- observe streamed reply in GTK
- close and resume session
- confirm transcript continuity
- repeat the same flow for Claude

## Risks

### Codex and Claude protocol mismatch

Codex app-server and Claude SDK will not expose identical event shapes.

Mitigation:

- normalize into a narrow app event model
- keep backend-specific metadata separate from shared rendering state

### Existing persistence assumptions

Current history and transcript views assume flat text logs.

Mitigation:

- add structured event storage
- provide compatibility rendering for old sessions
- migrate history reads incrementally

### UI update complexity

GTK currently refreshes by reading log files and polling active sessions.

Mitigation:

- move agent sessions onto explicit event channels
- keep polling only where still needed for shell/PTy flows

## Open Decisions Resolved

- Replace current Codex session path completely: yes
- Apply the same transport redesign goal to Claude: yes
- Keep shell sessions on PTY: yes
- Prefer native supported agent transports over simulated terminal interaction: yes

## Recommended Delivery Order

1. Add shared agent backend abstraction and event model.
2. Add Codex app-server backend.
3. Wire GTK session surface to structured events for Codex.
4. Add persistent backend-native resume/session metadata.
5. Add Claude Agent SDK backend.
6. Remove PTY-based agent submission and transcript dependence.

## Success Criteria

- GTK no longer submits Codex prompts by PTY keystroke injection.
- GTK no longer submits Claude prompts by PTY keystroke injection.
- Codex replies stream into the GTK UI through supported app-server events.
- Claude replies stream into the GTK UI through supported SDK events.
- Agent session resume uses stored backend-native session identity.
- Shell terminal behavior remains intact.
