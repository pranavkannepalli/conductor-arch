# Immediate Codex Delivery And Stable Queue Design

## Goal

Make Ctrl+Enter an intentful "deliver now" action. Plain Enter continues to
queue a follow-up while Codex is generating. Keep queued-message controls
stable while the chat transcript refreshes so provider events do not interrupt
hover interactions.

## Current Failure

GTK labels Ctrl+Enter as a steer, but sends the same generic Archcar input
request used by normal messages. Archcar infers `turn/steer` versus
`turn/start` from mutable readiness state, so the immediate intent is not
represented across the GTK, CLI, Archcar, and Codex app-server boundary.

The queue overlay is rebuilt inside the general composer-state updater. Chat
and provider refreshes invoke that updater, destroy the queued row widgets, and
reset their hover state even when the queue did not change.

## Delivery Semantics

- Plain Enter during active generation adds a local queued input. It is sent
  after the current turn completes.
- Ctrl+Enter delivers immediately.
- The queued-row "Send immediately" action uses the same immediate-delivery
  path as Ctrl+Enter.
- Immediate delivery uses Codex app-server `turn/steer` with `threadId`, input,
  and the required `expectedTurnId` when a matching active turn exists.
- If no active turn exists when Archcar handles the request, immediate delivery
  uses `turn/start`.
- If the active turn completes or changes before app-server accepts the steer,
  Archcar retries the same input once with `turn/start`. This expected race does
  not produce a user-facing error.
- Other provider or transport failures remain visible and do not silently lose
  input.
- The input is persisted once and displayed immediately as a user message. It
  defines a local turn boundary even when the provider continues work under
  the same in-flight provider turn.

## Components

### Shared Archcar Protocol

Add a backward-compatible input delivery value to `ArchcarRequest::SendInput`
and the internal session command. Existing serialized requests default to the
current automatic behavior. GTK Ctrl+Enter and the CLI immediate flag select
immediate delivery explicitly.

### Codex App-Server Session

Track the app-server request associated with an immediate input. Choose
`turn/steer` when the active provider turn ID is available and `turn/start`
otherwise. Inspect the steer response: if its expected active turn has already
ended or changed, issue one `turn/start` request for the same input without
persisting a duplicate message.

The local user-input provider event is the authoritative Archductor boundary.
Provider events received after it belong to the continued local turn segment
for transcript ordering and turn-scoped change summaries.

### GTK Composer And Queue

Rename the GTK submit intent from steer-specific wording to immediate-delivery
wording. Render an immediate input optimistically as a sent user message rather
than a queued row, then deduplicate it against the persisted message.

Split queue rendering from general composer and transcript rendering:

- Composer state updates only placeholder, send-button, and readiness state.
- A dedicated queue renderer owns the queue overlay and a queue signature.
- It rebuilds rows only after enqueue, edit, delete, immediate-send, automatic
  dequeue, or selected-thread changes.
- Chat/provider refreshes do not invoke queue rendering.
- Queue mutations update the queue overlay directly and request transcript
  refresh only when the transcript itself changed.

### CLI Parity

Add an immediate-delivery option to both the user-facing session send command
and the lower-level Archcar send command. The output distinguishes immediate
delivery from ordinary send while keeping existing commands compatible.

## Error Handling

- A stale expected turn ID falls back to `turn/start` once.
- A failed fallback or unrelated provider failure is surfaced normally.
- A closed Archcar channel keeps or restores the unsent input.
- Immediate optimistic display is reconciled with persistence so retries and
  refreshes do not duplicate the user message.

## Verification

Written tests will cover:

- protocol serialization and backward-compatible defaults;
- active immediate input emits `turn/steer` with `expectedTurnId`;
- idle immediate input emits `turn/start`;
- a stale steer response retries once with `turn/start` and persists once;
- Ctrl+Enter selects immediate delivery while Enter queues;
- queued-row immediate send uses the same path;
- chat refresh does not rebuild unchanged queue rows;
- queue mutations and thread selection do refresh the queue;
- CLI parsing and request construction for immediate delivery.

CLI smoke will exercise immediate session-send argument handling. GTK smoke
will run the focused session-surface tests and a GTK build/test path; a live
authenticated Codex smoke will be reported separately if the environment
cannot safely provide a deterministic in-flight turn.
