# Chat Threads And Codex Resume Design

## Goal

Add first-class multi-chat support per workspace, persist real provider-native resume identities for Codex and Claude, remove mocked chat controls, and make visible chat controls map to real provider behavior.

## Scope

This design applies to the existing Archductor chat/session system in core, CLI, and GTK.

It covers:

- Separate named chat threads per workspace per provider
- Exact Codex resume by stored native id instead of `resume --last` for new threads
- Structured persisted chat messages
- Real control-command behavior for model/reasoning and related toggles
- GTK and CLI session flows using the same persisted thread model

It does not cover:

- Replacing PTY transport for Codex chats
- Replacing shell terminals with thread-based chats
- New networked backend services

## Current Problems

The current system treats chat sessions mostly as process records plus append-only transcript logs.

That creates four problems:

1. A workspace does not have first-class multiple chat threads. It mostly has multiple process rows with implicit grouping.
2. Codex does not persist a real native resume id. New resume logic falls back to `resume --last`.
3. Messages are reconstructed from transcripts for rendering, but they are not stored as first-class chat message records.
4. Several chat controls in GTK behave like UI state or launch-only metadata rather than real provider commands.

## Requirements

### Functional

- A workspace can have many chat threads.
- Each chat thread belongs to exactly one provider (`codex`, `claude`, later others if added).
- A chat thread can be named and selected independently of the running PTY/process.
- User messages are stored immediately as structured message rows.
- Agent messages parsed from rendered Codex screens are stored as structured message rows.
- New Codex threads persist their real native resume identity when it becomes available.
- Resume for new Codex threads uses the stored native id when present.
- Legacy Codex sessions may still fall back to `resume --last` until migrated.
- Visible chat controls must either:
  - change launch settings for a new thread, or
  - enqueue and send real provider commands before the next user message.
- Controls with no real provider behavior must be removed from the chat surface.

### Non-Functional

- Keep PTY-accurate Codex input behavior:
  - write text bytes
  - flush
  - wait about 20ms
  - write `\r`
- Keep rendered-screen-based Codex parsing.
- Keep transcript/log compatibility where practical during migration.
- Keep shell terminals separate from agent chat threads.

## Recommended Approach

Create first-class `chat_threads` and `chat_messages` tables and link running `processes` rows to threads instead of overloading `processes` as the thread model.

This is preferred over continuing to stretch `processes` because:

- provider-native resume identities belong to a conversation, not a PTY lifetime
- multiple chats per workspace become explicit instead of inferred
- structured message persistence stops forcing the frontend to rebuild all meaning from raw logs
- control-command history can be represented clearly

## Alternatives Considered

### Approach 1: Extend `processes` only

Add more metadata columns to `processes`, keep parsing logs, and treat each process row as the chat thread.

Pros:

- Smaller short-term diff
- Reuses current history code

Cons:

- Mixes PTY lifecycle with conversation identity
- Makes exact resume and multi-thread UI harder
- Keeps structured messages as a derived view instead of real data

### Approach 2: First-class `chat_threads` and `chat_messages` tables

Pros:

- Clean model for many chats per workspace
- Exact native resume id lives where it belongs
- Frontend can render stored messages directly
- Easier to represent control-command events and thread metadata

Cons:

- Requires schema migration and broader refactor

Recommendation: Approach 2.

## Data Model

### `chat_threads`

Add a new table:

- `id INTEGER PRIMARY KEY`
- `workspace_id INTEGER NOT NULL REFERENCES workspaces(id)`
- `provider TEXT NOT NULL`
- `title TEXT NOT NULL`
- `status TEXT NOT NULL`
- `native_thread_id TEXT`
- `harness_metadata TEXT`
- `created_at TEXT NOT NULL`
- `updated_at TEXT NOT NULL`
- `archived_at TEXT`

Rules:

- `provider` is immutable after thread creation.
- `native_thread_id` is nullable at creation, then filled once known.
- `status` is thread state (`active`, `archived`, possibly `errored` later if needed).

### `chat_messages`

Add a new table:

- `id INTEGER PRIMARY KEY`
- `thread_id INTEGER NOT NULL REFERENCES chat_threads(id)`
- `role TEXT NOT NULL`
- `content TEXT NOT NULL`
- `source TEXT NOT NULL`
- `created_at TEXT NOT NULL`
- `updated_at TEXT NOT NULL`

Sources:

- `user_send`
- `agent_screen_parse`
- `system`
- `control_command`
- `review_prompt`

### `processes`

Keep `processes` for running PTY lifecycles, but add a nullable link:

- `chat_thread_id INTEGER REFERENCES chat_threads(id)`

Meaning:

- a thread can have many historical process rows over time
- at most one running process should normally be attached to a thread at once
- shell terminal processes remain outside the chat-thread model

## Runtime Model

### Thread Creation

Creating a new agent chat thread:

1. Create `chat_threads` row with workspace, provider, title, and requested harness metadata.
2. Create a PTY-backed provider process for that thread.
3. Record the `processes` row linked to `chat_thread_id`.

### Sending Messages

When the user sends a message on a thread:

1. Persist a `chat_messages` row with role `user` and source `user_send`.
2. Flush any pending control commands for the thread.
3. Send the user message through the PTY using terminal-accurate enter behavior.
4. Append compatibility transcript markers if legacy transcript consumers still depend on them.

### Receiving Codex Output

For Codex threads:

1. Read raw PTY bytes.
2. Feed them into the `vt100` screen parser.
3. Persist `[codex raw]` blocks only for debug/log compatibility.
4. Persist `[codex screen]` snapshots for replay/debug compatibility.
5. Parse the rendered screen into agent messages.
6. Merge repaint/stream updates against the latest stored agent messages.
7. Upsert resulting agent messages into `chat_messages`.

### Resume

For Codex:

- New threads resume by exact stored `native_thread_id`.
- Legacy threads without a stored id may use `resume --last` only as a compatibility fallback.

For Claude:

- Continue using stored `session_resume_id` behavior, but move the conversation identity onto the thread record model.

### Multi-Chat Selection

Selecting a thread in GTK or CLI loads:

- thread metadata
- structured messages from `chat_messages`
- latest process state if a live process is attached

The thread remains viewable even when no PTY is attached.

## Capturing The Real Codex Resume Identity

The system must stop guessing and stop relying on `--last` for new threads.

Design requirement:

- detect the real Codex-native session/thread id from a supported source
- persist it to `chat_threads.native_thread_id`
- only mark a thread as exact-resumable after this value is captured

Supported-source constraint:

- use a deterministic source from Codex behavior or files that are actually produced in the environment
- do not invent ids
- do not infer from unrelated local state

Implementation note:

- the exact extraction point is an implementation detail, but the persistence contract is fixed: new Codex threads must capture and store the real id as part of thread startup or early session lifecycle.

## Chat Control Model

### Principle

Every visible control must be real.

Each control falls into one of two buckets:

1. `new-thread-only`
2. `next-message command`

### New-Thread-Only Controls

These affect launch config and are shown when creating a new thread:

- provider
- initial model, if provider requires it at launch
- launch-only harness settings that cannot be changed live

If a control is launch-only, the UI must say so.

### Next-Message Command Controls

These enqueue provider commands and flush them before the next user message:

- model selection if supported live
- reasoning / thinking level if supported live
- similar slash-command-backed controls

Behavior:

1. User changes a control.
2. UI/backend records a pending command for the active thread.
3. On next send, pending commands are emitted in order.
4. Each emitted command is also stored in `chat_messages` with source `control_command`.
5. Then the real user message is sent and stored.

### Unsupported Controls

If a control cannot be mapped to a supported provider behavior:

- remove it from the surface, or
- move it to “new thread settings” if it is launch-only

It must not remain as a decorative toggle.

## Frontend Behavior

### GTK

GTK should render structured thread messages from `chat_messages` as the primary source.

Live PTY state still matters for:

- sending messages
- tracking active process status
- parsing Codex screen updates into stored agent messages

GTK should stop depending on raw transcript parsing as the primary model for active chat rendering once structured messages exist.

### CLI

CLI history and logs can keep transcript compatibility output, but thread/message queries should become first-class commands over the new tables.

## Migration

### Existing Data

- Existing `processes` rows remain valid.
- Existing transcript logs remain readable.
- Existing history views should continue to work during transition.

### Legacy Chat Conversion

Legacy sessions do not need perfect retroactive normalization immediately.

Minimum migration behavior:

- new threads use `chat_threads` and `chat_messages`
- old sessions remain visible in legacy history
- optional backfill can later create thread/message rows from legacy transcripts

## Testing

Required coverage:

- thread creation creates distinct threads per workspace/provider
- many threads can exist in one workspace
- user messages persist immediately
- screen-parsed agent messages persist and merge correctly across repaints
- Codex native resume id is captured and stored
- Codex resume uses stored native id when present
- legacy Codex fallback still uses `resume --last` when no stored id exists
- control changes enqueue and send real provider commands before the next message
- unsupported controls are removed or marked launch-only
- GTK thread selection renders persisted messages for inactive threads

## Risks

### Resume Id Extraction Risk

The hardest part is capturing the real Codex-native id from a supported deterministic source. This must be solved explicitly in implementation and tested end to end.

### Transitional Complexity

There will be a period where:

- legacy transcript parsing still exists
- structured message persistence is the new primary path

That transition must be kept deliberate to avoid double-rendering or duplicate history.

## Success Criteria

This work is complete when:

- a workspace can create and keep multiple named Codex or Claude chat threads
- each new Codex thread stores a real native resume id
- resume uses the stored id instead of `--last` for new threads
- GTK chat controls only show real functional behaviors
- model/reasoning changes emit real provider commands before the next message
- parsed agent messages are stored structurally and rendered correctly
