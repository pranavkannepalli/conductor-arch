# Archcar Harness Architecture

**Goal:** Replace the current GTK-driven PTY harness path with a reusable sidecar-style controller service that both GTK and CLI can call, with Codex fully implemented and Claude Code left as stubs on the same interface.

## Why

The current harness path lets GTK callbacks spawn and manage PTYs directly. That makes workspace selection and chat actions vulnerable to UI stalls, and it tightly couples session lifecycle logic to frontend code.

We want the Conductor-style shape instead:

- frontend sends commands to a controller
- controller manages sessions asynchronously
- frontend renders state from controller snapshots and events
- adding a new harness is a small, well-bounded implementation task

## Product Requirements

- Selecting a workspace should auto-spawn the default Codex session in the background.
- Auto-spawn must be asynchronous and debounced per workspace.
- GTK must never block on PTY spawn or terminal boot.
- CLI and GTK must use the same reusable core service.
- The first transport should be Unix socket RPC.
- The controller should support both request/response RPC and pushed event subscriptions.
- Codex must work end to end on the new architecture.
- Claude Code should exist only as a stub harness on the same interface.
- The old direct frontend harness path should be removed or explicitly deprecated so session control goes through the new controller.

## Architecture

### High-Level Shape

Add a new sidecar binary named `archcar`.

`archcar` will:

- bind a Unix domain socket in the app state directory
- own PTY-backed session workers
- keep cached screen, message, and lifecycle state per session
- expose command RPC for spawn/send/status/screen/messages/kill
- expose a subscription stream for state-change events
- debounce background auto-spawn per workspace

GTK and CLI will:

- connect to `archcar` through a shared client in `crates/core`
- send RPC commands
- subscribe to events where live updates matter
- stop spawning PTYs or parsing live terminal state directly

### Layering

- `crates/core`
  - shared protocol
  - shared Unix socket client
  - shared harness abstractions
  - server/session implementation used by `archcar`

- `crates/archcar`
  - binary entrypoint
  - socket lifecycle
  - process bootstrap and shutdown

- `crates/gtk-app`
  - `ArchcarClient` consumer
  - event-driven UI refresh

- `crates/cli`
  - `ArchcarClient` consumer
  - session inspection/debug commands first

## Core Components

### 1. Protocol

Create shared request/response/event types in `crates/core/src/archcar/protocol.rs`.

Requests:

- `EnsureWorkspaceDefaultSession`
- `SpawnSession`
- `SendInput`
- `GetSessionStatus`
- `GetSessionScreen`
- `GetSessionMessages`
- `KillSession`
- `Subscribe`

Responses:

- immediate ack or snapshot payload
- typed error payloads for unsupported harnesses, missing sessions, invalid state, and internal failures

Events:

- `SessionSpawnQueued`
- `SessionStarted`
- `SessionReady`
- `SessionScreenUpdated`
- `SessionMessagesUpdated`
- `SessionStatusChanged`
- `SessionExited`
- `SessionError`

All protocol payloads should be `serde`-serializable and stable enough for both GTK and CLI use.

### 2. Harness Interface

Create a shared harness abstraction in `crates/core/src/archcar/harness.rs`.

The goal is to make a new harness easy to add. Each harness implementation should define:

- how to build spawn command, args, cwd, and env
- how to detect readiness from screen/output
- how to handle startup prompts like Codex trust
- how to convert visible screen/output into structured chat messages
- how to send input lines or control commands
- whether workspace auto-spawn is supported

Codex implementation:

- real spawn config
- real readiness detection
- real trust prompt handling
- real screen parsing into structured messages

Claude implementation:

- same interface
- returns structured `NotImplemented` results for unsupported operations

### 3. Session Worker

Create `crates/core/src/archcar/session.rs`.

Each session worker owns:

- one PTY
- the spawned child process
- a `vt100` parser
- cached visible screen text
- cached structured messages
- cached lifecycle status
- command input channel
- event output channel

Session workers process commands asynchronously and push events back to the server fanout.

### 4. Server

Create `crates/core/src/archcar/server.rs`.

The server owns:

- Unix listener accept loop
- session registry
- workspace debounce registry
- subscriber registry
- command dispatch
- snapshot read APIs

Behavior:

- `EnsureWorkspaceDefaultSession` returns immediately
- if a workspace already has a running or queued default Codex session, no duplicate spawn occurs
- otherwise a background spawn task is queued and events are emitted as lifecycle state changes

### 5. Client

Create `crates/core/src/archcar/client.rs`.

Client responsibilities:

- connect to the Unix socket
- send typed requests
- parse typed responses
- expose subscription handling for live events
- provide a small, clean API for GTK and CLI

## Data Flow

### Workspace Selection

1. User selects a workspace in GTK.
2. GTK calls `EnsureWorkspaceDefaultSession { workspace, harness: Codex }`.
3. `archcar` immediately returns an ack.
4. If the workspace is already running or queued, `archcar` does nothing else.
5. Otherwise `archcar` queues a background spawn task.
6. Session worker starts the PTY asynchronously.
7. Worker emits `SessionStarted`, then `SessionReady` once the harness says the session is usable.
8. GTK updates local state from subscription events and uses snapshot calls as fallback.

### Sending Input

1. GTK or CLI calls `SendInput`.
2. `archcar` writes to the worker input channel.
3. The worker sends input to the harness-managed PTY.
4. New terminal output updates screen/message caches.
5. The worker emits `SessionScreenUpdated` and `SessionMessagesUpdated`.

### Session Shutdown

1. GTK or CLI calls `KillSession`.
2. `archcar` stops the worker asynchronously.
3. Worker updates status and emits `SessionExited`.

## Unix Socket Transport

The first transport is Unix domain sockets, not TCP ports.

Why:

- local-only communication
- no port reservation/config complexity
- matches sidecar expectations better than local HTTP
- simpler security story for a desktop app

Socket path should live under the app state directory owned by Archductor.

## Migration Plan

### Phase 1: Core Controller

- add `crates/archcar`
- add shared protocol/client/server/session/harness modules
- implement Codex harness
- implement Claude stub harness
- add tests for protocol, debounce, readiness, trust handling, and event fanout

### Phase 2: GTK Migration

- replace direct PTY/session spawn paths in `session_surface.rs`
- switch workspace selection to `EnsureWorkspaceDefaultSession`
- render chat/session state from `archcar` snapshots and events
- remove or deprecate old frontend-owned harness code

### Phase 3: CLI Migration

- add CLI commands that prove controller reuse
- support status, screen, messages, send, and kill on the new path

## Testing

Required automated coverage:

- request/response protocol round trips
- event subscription delivery
- workspace auto-spawn debounce
- spawn returns immediately while session boot continues later
- Codex readiness detection
- Codex trust prompt auto-resolution
- screen-to-message parsing
- unsupported Claude stub behavior
- regression guard that GTK workspace selection no longer directly spawns PTYs

## Non-Goals For This Pass

- full Claude Code implementation
- remote/distributed controller deployment
- generalized multi-host message bus
- elaborate auth/security model beyond local Unix socket access
- replacing existing repository/workspace database structures unless the controller needs small integration points

## Implementation Constraints

- prefer small diffs where possible, but do not preserve the current direct GTK harness path just for compatibility
- keep the controller API typed and explicit
- keep harness responsibilities isolated so new providers are easy to add
- keep frontend code thin; PTY/session state belongs in `archcar`
