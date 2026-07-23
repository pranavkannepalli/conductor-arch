# Background Chat Update Map

Inspected on 2026-07-23.

This document maps how chat state updates while the user is looking at another
workspace, another chat thread, or another page.

## Short Version

Background chat updates use four coordinated mechanisms:

- a two-second GTK sampler reads lightweight running-chat markers from SQLite
- Archcar owns the durable chat input queue and drains it from session
  ready/turn-complete events, independent of the focused GTK chat
- a GTK-level background chat runner owns Archcar events, queue wakeups, and UI
  cache refresh for chats that are not currently visible in the chat surface
- the Archcar async bridge wakes the selected chat surface when the sidecar
  emits session events, responses, or bridge errors

These mechanisms avoid loading every hidden timeline. Routine background work emits
typed refresh events, updates summaries and tab state, and loads a full
timeline only when the affected thread needs to render.

## Primary Files

| File                                             | Role                                                                                                               |
| ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------ |
| `crates/gtk-app/src/main.rs`                     | Installs the two-second background sync timer and forwards diffed events through `AppState::request_refresh`.      |
| `crates/core/src/archcar/server.rs`              | Owns Archcar queue requests, queue events, and FIFO drain from ready managed sessions.                             |
| `crates/core/src/workspace.rs`                   | Provides durable chat queue storage plus running chat summary queries.                                             |
| `crates/gtk-app/src/background_chat.rs`          | Runs the app-lifetime hidden chat event reducer, queue wake planner, queue-cache sync, and PR refresh trigger.     |
| `crates/gtk-app/src/background_sync.rs`          | Loads lightweight running-thread snapshots, diffs them, coalesces refresh events, and builds chat tab nav items.   |
| `crates/gtk-app/src/refresh.rs`                  | Routes `WorkspaceChatMessagesChanged` and `WorkspaceChatLifecycleChanged` to granular chat handlers.               |
| `crates/gtk-app/src/workspace_command_center.rs` | Owns workspace chat tabs, running/unread/draft visual state, and selected-workspace filtering.                     |
| `crates/gtk-app/src/session_surface.rs`          | Owns selected chat rendering, Archcar event draining, timeline cache, message refresh dispatch, and wake debounce. |
| `crates/gtk-app/src/archcar_async.rs`            | Runs Archcar request and event bridges on background threads and calls the installed GTK wake callback.            |

## Data Sampled In The Background

`WorkspaceStore::list_running_chat_thread_summaries` samples only threads with
a running session process. Each row includes:

- workspace name
- chat thread id
- title
- provider
- thread status
- latest persisted chat message id
- latest provider event sequence
- running session id
- thread updated timestamp

The query does not load chat message bodies, provider event payloads,
transcripts, diffs, PR state, or terminal logs.

## Background Sync Timer

`main.rs` installs a two-second `glib::timeout_add_seconds_local` loop.

Flow:

1. skip if a previous background sync job is still in flight
2. run `load_background_sync_snapshot` in `spawn_background_job`
3. compare the new snapshot with the previous snapshot
4. coalesce duplicate events
5. call `AppState::request_refresh(event)` for each event

The previous snapshot is stored in GTK memory. If both previous and next
snapshots have no running threads, the timer returns without emitting events.

## Background Chat Runner

`background_chat.rs` installs an app-lifetime `AsyncArchcarBridge` separate
from the selected chat surface bridge.

Archcar owns queued input durability and delivery. Queue rows live in
`chat_queued_inputs`; both CLI and GTK write to Archcar queue requests and read
the same SQLite-backed queue. `claim_next_queued_chat_input()` removes the FIFO
row before provider delivery; if delivery fails, Archcar requeues the claimed
input at the front or surfaces the restore failure so recovery can preserve the
queued input instead of silently dropping it. `ChatQueueUpdated` is emitted
whenever the queue changes, and GTK refreshes its local composer cache from that
event. The selected chat surface also reconciles queue add/remove/list responses
so optimistic GTK rows are quickly replaced by durable Archcar queue ids.

The GTK runner owns hidden-chat UI state that must continue while the user is looking
elsewhere:

- Archcar session readiness cache
- session id to chat thread mapping
- chat thread to workspace mapping
- in-flight ensures and status probes
- held queue wakeups after failed/interrupted/deferred turns
- working markers for hidden turns

The runner wakes on Archcar events with a 32 ms debounce and also ticks once per
second. The 32 ms path is event wake debounce, not polling; it keeps UI state
nearly instant while still collapsing same-burst updates. Each tick:

1. drains pending Archcar events/responses from the runner bridge
2. emits scoped typed refresh events for affected chat threads/workspaces
3. scans only durable queued-thread ids from `chat_queued_inputs`
4. loads lightweight queue candidates in a background job
5. ensures or probes the managed session so Archcar can drain the queue when
   the hidden thread is idle and ready

The runner skips direct delivery for the currently visible selected chat
thread too. The mounted chat surface owns visible optimistic UI, interrupt
controls, and immediate sends, but queued auto delivery still goes through
Archcar.

## Diff Rules

`background_sync::diff_background_sync` compares threads by
`(workspace, thread_id)`.

Message refresh:

- latest chat message id changed
- latest provider event sequence changed

Lifecycle refresh:

- thread appeared
- thread disappeared
- title changed
- provider changed
- status changed
- running session id changed

Ignored:

- timestamp-only changes

Coalescing:

- duplicate message events collapse per `(workspace, thread_id)`
- duplicate lifecycle events collapse per workspace
- distinct threads keep distinct message events

## Refresh Routing

`AppState::request_refresh` emits `AppStateEvent::RefreshRequested`. `main.rs`
subscribes once and forwards the contained `RefreshEvent` to `RefreshHub`.

`RefreshHub` routes chat events narrowly:

- `WorkspaceChatMessagesChanged` updates chat surface and chat tabs only
- `WorkspaceChatLifecycleChanged` updates sidebar, dashboard, history, chat
  tabs, and chat surface

Neither route rebuilds the Projects page. Message changes do not rebuild the
full workspace shell. Chat session lifecycle callbacks and PR prompt staging
also publish `WorkspaceChatLifecycleChanged`, so chat-related navigation
summaries update through the chat route instead of hand-refreshing
sidebar/dashboard/history or rebuilding the workspace shell.

## Chat Tabs

`workspace_command_center.rs` registers `set_workspace_chat_tabs`.

On matching chat events:

1. ignore events for non-current workspaces
2. increment a tab snapshot generation
3. load `WorkspaceChatTabSnapshot` in a background job
4. drop stale results if a newer generation exists
5. load thread records plus `WorkspaceChatNavItem`s
6. reconcile finished/unread state
7. rebuild visible chat tabs from the snapshot

Tab state is derived from:

- selected thread
- provider work running
- finished-unread marker
- dirty composer draft marker

Visual outcomes are selected, running, selected-running, finished-unread,
editing, or read.

## Chat Nav Items

`background_sync::load_workspace_chat_nav` loads all chat threads for the
workspace and checks provider events for active work.

A thread is considered running when provider events show active nonterminal
work, either directly from turn events or through the provider projection.

Unread means:

- active work is running
- the thread is not selected

If provider events do not show active work, an idle running process alone does
not mark the tab as generating.

## Chat Surface

`workspace_command_center.rs` registers `set_workspace_chat_surface` when the
session surface reports that its refresh callback is ready.

It filters by selected workspace:

- message events refresh only when the event workspace matches selected
  workspace
- lifecycle events refresh only when the event workspace matches selected
  workspace

It converts refresh events to `ChatRefreshKind`:

- `WorkspaceChatMessagesChanged` -> `Messages { thread_id }`
- `WorkspaceChatLifecycleChanged` -> `ThreadNav`
- anything else -> `Full`

Prompt staging that needs the composer to update immediately uses the lifecycle
route. The staged prompt still lives in `AppState`; the durable stores remain
the source of truth for threads, messages, sessions, and provider events.

## Message Timeline Refresh

For `ChatRefreshKind::Messages`, `session_surface.rs` runs a per-thread
background timeline load:

1. capture current scroll position
2. increment a message-refresh generation for that thread
3. load chat messages, chat events, provider events, and transcript display
   preferences in `spawn_background_job`
4. drop stale results if the generation no longer matches
5. cache the loaded `ChatTimelineSnapshot` by thread id
6. if the thread is not selected, stop after warming the cache
7. if the thread is selected, render the timeline snapshot and restore scroll

This is the key optimization for off-focus chats: nonselected threads update
their cache and tab state without rebuilding visible message widgets.

## Full Surface Refresh

The full chat refresh path is still used for startup, explicit surface refresh,
thread navigation, and some Archcar event outcomes.

It reloads:

- workspace existence
- sessions for the workspace
- chat threads for the workspace
- selected thread metadata
- selected thread timeline when needed
- pending Archcar messages/responses

It can recover renamed workspaces by selected thread id and clears the surface
if the selected workspace was deleted.

## Archcar Event Wake Path

`archcar_async.rs` runs two background bridges:

- request bridge: sends GTK requests to Archcar and posts responses
- event bridge: subscribes to Archcar sidecar events and reconnects after
  disconnects or subscription failures

Both post `AsyncArchcarMessage`s into a channel and call the installed wake
callback.

`session_surface.rs` installs that wake callback with `install_archcar_wake`:

- each chat surface gets a wake id in `CHAT_WAKE_REGISTRY`
- wake calls are debounced with `CHAT_REFRESH_WAKE_DELAY_MS = 32`
- the scheduled GTK callback drops itself if the owning surface was destroyed
- otherwise it runs the registered chat surface refresh

During refresh, `archcar_bridge.try_recv()` drains all available messages.

`background_chat.rs` installs the same style of debounced wake for its own
bridge. That wake drains hidden-chat events even when no chat surface is
mounted or the selected workspace is not the affected workspace.

## Archcar Event Effects

Archcar messages are reduced into `ArchcarRefreshIntent`.

These events request chat surface, workspace nav, and global summary refresh:

- session spawn queued
- session started
- turn completed
- session exited
- session error

These events request chat surface only:

- session messages updated
- session ready
- capabilities changed
- screen updated
- provider interaction requested/resolved
- responses
- bridge errors

Readiness and queue side effects are split by visibility:

- the selected visible chat surface owns visible send state and instant sends
- the background chat runner owns the same readiness, turn completion, failed
  turn hold, requeue, ensure, and status-probe behavior for hidden chats

## Composer And Queue Background Behavior

Queued composer input is stored durably by Archcar. `AppState` keeps a
per-thread composer cache for immediate GTK rendering and reconciles it from
Archcar queue responses/events.

When a background event makes a managed session ready, the selected surface may:

- flush pending Archcar inputs
- stage ready background queued chat inputs
- pop one queued input for auto drain
- requeue the input at the front if send fails

The queue overlay is refreshed separately from full timeline rendering.

For hidden chats, `background_chat.rs` performs the corresponding drain:

- reads only durable queued thread ids from `chat_queued_inputs`
- skips the visible selected chat thread to avoid duplicate sends
- loads the queued thread record, branch prefix, process markers, provider
  active-work marker, and message list in a background job
- preserves first-message metadata injection by reusing the same send
  preparation function as the focused surface
- sends one queued input with automatic delivery
- relies on Archcar's shared automatic-send readiness gate, so GTK background
  queue drain and CLI default sends cannot both write into an active managed
  turn
- records a turn checkpoint before send and discards it if Archcar rejects the
  input
- keeps the send in-flight after Ack until a message/update/turn boundary
  arrives
- clears staged review prompt state after a review prompt is submitted
- emits `WorkspaceChatLifecycleChanged` for queue/count/working state changes

Turn completion also schedules a background pull-request state refresh and then
emits `WorkspaceReviewChanged` for the affected workspace. That keeps PR/GitHub
status surfaces current without repainting unrelated chat or shell UI.

## CLI And GTK Parallel Use

CLI and GTK both write through Archcar and read the same SQLite sources:

- session state comes from Archcar `GetSessionStatus` and persisted process rows
- chat messages come from persisted chat messages plus provider events
- provider interactions come from the shared interaction store
- GTK queued background sends and CLI default sends both use
  `ArchcarInputDelivery::Auto`
- Ctrl+Enter and CLI `--immediate` both use `ArchcarInputDelivery::Immediate`

Archcar is the concurrency boundary. It rejects automatic sends for managed
Codex/Claude sessions whose snapshot is not ready, before enqueueing the input
to the provider loop. Only immediate delivery may enter a not-ready session to
steer the active turn. This keeps GTK background queue drain, visible composer
sends, and CLI sends aligned even when they run at the same time.

## Safety Guards

- background sync uses an in-flight flag to avoid overlapping sampler jobs
- background chat queue scans use an in-flight flag to avoid overlapping queue
  candidate loads
- hidden queue drain skips the visible selected chat thread
- GTK queue caches carry durable Archcar queue ids whenever the row has been
  persisted, so delete/edit actions can target the shared queue
- tab snapshots use generations to drop stale async results
- message timeline refreshes use per-thread generations
- message timeline DB loads do not run synchronously in GTK callbacks
- nonselected message refreshes warm cache before skipping render
- Archcar wake is debounced at 32 ms
- wake registry entries are removed when the chat surface is destroyed
- background chat Archcar wake is debounced at 32 ms and lives for the app
  lifetime
- selected workspace filtering prevents off-workspace events from repainting
  the visible chat surface
- deleted workspace refresh clears navigation and visible chat widgets instead
  of showing stale session errors

## Tests Covering This Contract

Relevant tests live in:

- `crates/gtk-app/src/background_sync.rs`
- `crates/gtk-app/src/refresh.rs`
- `crates/gtk-app/src/session_surface.rs`
- `crates/gtk-app/src/workspace_command_center.rs`
- `crates/core/src/workspace.rs`

Notable assertions:

- new running thread produces lifecycle refresh
- message marker changes produce message refresh only
- title/status/session changes produce lifecycle refresh
- timestamp-only changes are ignored
- duplicate message/lifecycle events are coalesced
- selected running thread is not unread
- hidden queued chats are ensured before first send
- hidden queued chats probe running sessions until ready
- hidden queued chats send only when ready, idle, and not selected-visible
- failed hidden turns hold queue drain; successful turns release it
- background send Ack stays in-flight until an event boundary
- Archcar rejects automatic sends to not-ready managed sessions before enqueue
- Archcar still accepts immediate sends to not-ready managed sessions for
  explicit steering
- idle running process is not enough to mark active provider work
- active provider turn marks nav item running
- chat message refresh maps to `ChatRefreshKind::Messages`
- lifecycle refresh maps to `ChatRefreshKind::ThreadNav`
- message refresh loads timeline in a background job
- nonselected thread message refresh warms cache but skips rendering
- Archcar wake burst debounce is enforced

## Current Gaps

- background sync is still polling because not every producer emits reliable
  typed events for off-focus work
- the sampler only tracks running chat threads, so non-running thread changes
  rely on explicit lifecycle events from the action path
- chat surface full refresh still does several synchronous store reads in some
  selected-workspace paths
- background sync keeps only in-memory previous snapshot state, so app restart
  starts from an empty comparison
- the selected visible chat still has local readiness/render cache state in
  `session_surface.rs`; the background runner intentionally skips delivery for
  it to avoid duplicate sends

## Branch Focus Recommendation

For this branch, keep changes in the background chat harness focused on:

- replacing timer fallbacks with reliable typed producer events
- preserving the message-vs-lifecycle split
- keeping hidden timeline bodies out of background sync
- adding tests before changing refresh fanout or queue-drain behavior
- measuring with `ARCHDUCTOR_GTK_REFRESH_METRICS` when narrowing refreshes
