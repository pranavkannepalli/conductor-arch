# AppState And Refresh Map

Inspected on 2026-07-23.

This document maps the GTK `AppState` manager, every direct consumer, how updates
flow through the app, and the speed choices in the current harness.

## Short Version

`AppState` is GTK-only hot UI state. It is not the source of truth for
projects, workspaces, sessions, chat history, terminal output, PR state,
checks, or chat input queues. Durable state lives in `crates/core` stores and
SQLite, with Archcar owning managed session runtime and queued chat delivery.
`AppState` holds only the selected workspace/page/tab/chat/session, pending
target drafts, a composer queue cache for rendering, optimistic
workspace/chat phases, and navigation history.

The speed model is:

- keep hot UI state tiny and cheap to clone with `Rc<RefCell<...>>`
- emit narrow `AppStateEvent`s for local UI changes
- route durable changes through typed `RefreshEvent`s
- reload durable data off the GTK thread where possible
- avoid `RefreshScope::All` for routine updates
- update chat messages, chat tabs, runtime, review, and workspace nav rows
  independently instead of rebuilding the whole workspace page

## Primary Files

| File                                             | Role                                                                                                                                                                                                          |
| ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/gtk-app/src/state.rs`                    | Owns `AppState`, snapshots, events, queue APIs, optimistic phases, and navigation history.                                                                                                                    |
| `crates/gtk-app/src/refresh.rs`                  | Owns `RefreshHub`, typed `RefreshEvent`s, refresh scopes, granular workspace refresh slots, and refresh metrics.                                                                                              |
| `crates/gtk-app/src/main.rs`                     | Creates the singleton `AppState`, bridges `AppStateEvent::RefreshRequested` into `RefreshHub`, installs global refresh/timer paths.                                                                           |
| `crates/gtk-app/src/background_chat.rs`          | Owns app-lifetime hidden chat Archcar events, queued input auto-drain, and scoped chat/review refresh events.                                                                                                 |
| `crates/gtk-app/src/sidebar.rs`                  | Reads/mutates navigation state for page/workspace selection, stale workspace cleanup, rename/delete/archive navigation, and nav-row updates.                                                                  |
| `crates/gtk-app/src/projects.rs`                 | Marks optimistic workspace creation phases and navigates to the inserted workspace while creation continues.                                                                                                  |
| `crates/gtk-app/src/workspace_command_center.rs` | Reads selected workspace/tab/thread, wires workspace tabs, watches workspace phases, creates pending chat targets, queues prompts, and routes workspace action refreshes.                                     |
| `crates/gtk-app/src/session_surface.rs`          | Owns the hot chat/session UI around `AppState`: selected thread/session, pending targets, composer queues, staged prompts, Archcar readiness, chat message refreshes, and deleted/renamed workspace recovery. |

No `AppState` dependency was found in `crates/core` or `crates/cli`. Those
layers stay independent of GTK UI state.

## What AppState Stores

`AppStateSnapshot` stores these slices:

- navigation: `selected_workspace`, `active_page`, `active_workspace_tab`,
  `active_workspace_right_panel_tab`, back/forward stacks
- chat selection: `selected_chat_thread`, `selected_chat_target`
- session selection: `selected_agent_session`
- staged prompt text: `staged_review_prompt`, `pending_chat_prompt`
- composer queues: per-thread `queued_chat_inputs` as a cache of Archcar queue
  rows, pending-target `queued_pending_chat_inputs` before a real thread exists,
  and `editing_queued_chat_inputs`
- optimistic phases: per-workspace `workspace_phases`, per-target
  `chat_phases`
- pending target id allocation: `next_pending_chat_id`
- immutable app paths: `paths`

The state object is intentionally single-process and single-UI-thread:
`AppState` is a cheap clone handle around `Rc<RefCell<AppStateSnapshot>>` plus
watchers.

## AppState Events

`AppStateEvent` covers local UI state only:

- `WorkspaceSelectionChanged`
- `WorkspaceTabChanged`
- `ChatThreadSelectionChanged`
- `ComposerQueueChanged`
- `ComposerTargetQueueChanged`
- `WorkspacePhaseChanged`
- `ChatPhaseChanged`
- `AgentSessionSelectionChanged`
- `StagedReviewPromptChanged`
- `RefreshRequested(RefreshEvent)`

`emit()` snapshots after mutation and clones watcher callbacks before invoking
them. That avoids keeping a `RefCell` borrow alive while subscribers run.
Subscriptions are RAII handles; dropping the handle removes the watcher.

## Refresh Events

`RefreshHub` is the durable-state fanout layer. It does not own data. It calls
registered page/surface callbacks.

Typed events:

- `Manual`
- `ProjectInventoryChanged`
- `SettingsChanged`
- `WorkspaceSelectionChanged`
- `WorkspaceInventoryChanged`
- `WorkspaceMetadataChanged`
- `WorkspaceRuntimeChanged`
- `WorkspaceReviewChanged`
- `WorkspaceChatLifecycleChanged`
- `WorkspaceChatMessagesChanged`
- `TerminalChanged`

Granular workspace targets:

- shell/full workspace
- chat surface
- chat tabs
- runtime
- review
- workspace nav row

Important routing:

- chat message changes update only chat surface and chat tabs
- chat lifecycle changes update sidebar, dashboard, history, chat tabs, and
  chat surface
- runtime/terminal changes update sidebar, dashboard, history, and runtime
- review changes update dashboard, history, and review
- metadata changes update only the workspace nav row
- `Manual` maps to `RefreshScope::All`

Tests in `refresh.rs` lock in the intended fanout and assert routine sources do
not call `RefreshScope::All`.

## Update Flows

### Startup

`main.rs` creates `AppState` from launch target workspace/page/tab. If a
workspace launch does not explicitly specify a tab, startup loads the
workspace's default visible tab from `WorkspaceStore`.

Immediately after, `main.rs` subscribes to `AppStateEvent::RefreshRequested`
and forwards the contained `RefreshEvent` to `RefreshHub`.

### Page And Workspace Navigation

Sidebar, command palette, dashboard callbacks, and history callbacks navigate
through `AppState`:

- page buttons call `navigate_to_page`
- workspace rows call `navigate_to_workspace_with_default_tab`
- palette workspace tabs call `navigate_to_workspace_tab`
- back/forward buttons call `navigate_back` and `navigate_forward`

Workspace changes clear selected chat thread, selected chat target, selected
session, staged review prompt, and pending prompt. That prevents stale chat or
session state leaking across workspaces.

### Workspace Creation

`projects.rs` uses `spawn_workspace_create_with_navigation`.

Flow:

1. create runs in a background job with progress
2. once the workspace row exists, `mark_workspace_phase(Creating)` is written
3. UI navigates to the new workspace immediately
4. on success, phase becomes `StartingAgent`
5. on failure, phase becomes `Failed`

`workspace_command_center.rs` subscribes to `WorkspacePhaseChanged` for the
current workspace and updates the status label without rebuilding the page.

### Workspace Metadata And Lifecycle

Renames call `rename_workspace_in_navigation` and emit
`WorkspaceMetadataChanged`. The sidebar has a `workspace_nav_row` handler that
updates the visible row label/meta in place.

Archive/delete/discard/remove paths call `remove_workspace_from_navigation` so
selected workspace and history stacks cannot point at removed workspace names.
Then they emit `WorkspaceInventoryChanged` for sidebar/dashboard/history and
the workspace shell.

### Chat Thread Creation

There are two creation paths:

- workspace tab bar creation in `workspace_command_center.rs`
- session surface creation in `session_surface.rs`

Both allocate a `ChatUiTarget::Pending`, mark it `Creating`, switch to Chats,
and start a background create. On success they call
`resolve_pending_chat_target`, which:

- moves queued pending inputs to the real thread id
- converts a `Creating` phase to `Ready`
- selects the real thread
- emits chat thread and queue events

If queued input exists after resolution, the thread phase becomes
`StartingAgent`.

### Prompt Staging

Workspace actions queue prompt drafts through `queue_pending_chat_prompt` and
switch the active workspace tab to Chats. Examples include create PR, commit
and push, merge source branch, fix blockers, latest check output, PR summary,
PR review, failing checks, continue after merge, and open review comments.

Prompt-stage actions that need immediate chat UI refresh publish
`WorkspaceChatLifecycleChanged`. This updates the chat surface and chat tabs
through chat routing instead of rebuilding the full workspace shell.

`session_surface.rs` consumes `take_pending_chat_prompt` when the chat surface
is built/refreshed and inserts the text into the composer on the GTK idle loop.

### Composer Input

Composer send logic uses `selected_chat_target_for_submit`:

- if the selected target is pending, input is queued on
  `queued_pending_chat_inputs` until the real thread exists
- if a real thread is selected and the agent is busy/startup-blocked, input is
  queued through Archcar and mirrored in `queued_chat_inputs` for immediate UI
  rendering
- if the thread/session is ready, input is sent through Archcar
- Ctrl+Enter uses immediate delivery for managed harnesses

Archcar queue mutations emit `ChatQueueUpdated`; GTK reloads the cache and then
emits `ComposerQueueChanged`. Pending-target mutations still emit
`ComposerTargetQueueChanged` because no durable thread id exists yet. The
overlay renders queued items, supports delete/edit from the cache, and avoids
rebuilding when its signature has not changed.

### Queue Drain And Archcar Readiness

`session_surface.rs` tracks visible-chat Archcar readiness outside `AppState` in
small local maps (`archcar_ready_cache`, `inflight_archcar_actions`,
`pending_archcar_inputs`, `working_threads`). `background_chat.rs` tracks the
same class of readiness/in-flight/held queue state for hidden chats. `AppState`
only owns durable UI queue intent and optimistic chat phases.

When Archcar events/responses arrive:

- `SessionReady` marks the thread ready and clears `StartingAgent`
- `TurnCompleted` clears in-flight sends and may allow queue drain
- failed/interrupted turns hold queue drain
- send rejection requeues the input and retries ensure/startup

Queued input is popped one turn at a time. If send fails, it is requeued at the
front. The background runner skips the currently visible selected chat thread,
so the composer and instant-send override remain owned by the visible surface.

### Chat Message Refresh

External chat message refreshes use `RefreshEvent::WorkspaceChatMessagesChanged`.
The workspace command center only invokes the selected workspace's chat surface.
`session_surface.rs` then loads the selected thread timeline in a background
job, caches it by thread id, and renders only if that thread is still selected.

Nonselected thread message refreshes warm the cache and skip rendering.

### Background Sync

`background_sync.rs` samples persisted running chat thread markers:

- workspace
- thread id
- title/provider/status
- running session id
- latest message id
- latest provider sequence

The diff emits only chat message or chat lifecycle refresh events. Events are
coalesced by workspace/thread before being sent through
`AppState::request_refresh`.

This keeps sidebar/dashboard/history/chat tabs current for off-focus work
without loading hidden full chat timelines.

### Background Chat Runner

`background_chat.rs` has its own Archcar bridge and wakes on Archcar events with
the same 150 ms debounce used by the selected chat surface. It drains events for
hidden chats even when no chat surface is focused.

The runner emits narrow events only:

- `WorkspaceChatMessagesChanged` for message/provider-interaction updates
- `WorkspaceChatLifecycleChanged` for session starts/completions/errors,
  title/status/count/queue-visible changes, and hidden send state
- `WorkspaceReviewChanged` after turn completion triggers a background
  pull-request state refresh

It scans only queued thread ids from `AppState`, loads queue candidates in a
background job, and sends one queued input when the hidden thread is managed,
idle, and ready.

CLI and GTK use the same Archcar delivery contract. Default sends use
`ArchcarInputDelivery::Auto`, which Archcar rejects for not-ready managed
Codex/Claude sessions before provider enqueue. Ctrl+Enter and CLI
`--immediate` use `ArchcarInputDelivery::Immediate`, which is the only path that
may steer an active turn.

### Runtime And Terminal

Runtime reconciliation runs on startup, focus, close, file-watch events, and a
five-second fallback timer. If changes are detected, selected workspace context
drives a typed `WorkspaceRuntimeChanged`; otherwise inventory refresh is used.

Terminal surfaces emit `TerminalChanged`, which routes like runtime changes.

### Notifications

The notification sampler reads `selected_workspace` from `AppState`. It samples
session/check status every five seconds and shows toasts for enabled rules.
This path does not mutate `AppState`.

## Speed Optimizations In The Harness

- `AppState` stores only hot UI state, so snapshots are cheap.
- Watchers receive a cloned snapshot after mutation, avoiding long borrows and
  reentrant `RefCell` panics.
- `RefreshHub` separates local UI events from durable reloads.
- Typed refresh events prevent broad page rebuilds.
- `RefreshScope::All` is reserved for explicit manual refresh; tests guard
  routine code against using it.
- Workspace refresh is split into shell, chat surface, chat tabs, runtime,
  review, and nav row.
- Chat message refresh never falls back to full workspace shell rebuild.
- Chat lifecycle refresh updates summaries/tabs/surface without touching
  projects.
- Chat session callbacks and PR prompt staging publish
  `WorkspaceChatLifecycleChanged` instead of manually refreshing
  sidebar/dashboard/history or the full workspace shell.
- Metadata refresh updates only the visible nav row.
- Background sync samples ids and sequence markers instead of full chat
  timelines.
- Background chat scans only queued thread ids and skips the visible selected
  chat thread to avoid duplicate sends.
- Chat timeline loads run in `spawn_background_job`.
- Message refreshes use per-thread generations so stale background results are
  dropped.
- Nonselected message refreshes warm cache but skip rendering.
- Archcar wake uses a one-shot 150 ms debounce to collapse event bursts.
- Queue overlay render is signature-based to avoid churn during streaming.
- Sidebar workspace selection uses a generation counter so stale async lookup
  results are ignored.
- Runtime, notification, and background sync timers use in-flight flags to avoid
  overlapping work.
- Spotlight file watching and refresh setup are done off the GTK thread.
- Refresh metrics can be enabled with `ARCHDUCTOR_GTK_REFRESH_METRICS`.

## Current Boundaries And Risks

- `AppState` is process-local. Restart recovery must come from SQLite/core
  stores, not from `AppState`.
- Right-panel tab and some page-only navigation helpers still mutate without a
  dedicated event. Callers manually refresh or update local chrome where needed.
- `set_selected_workspace_with_default_tab` and
  `navigate_to_workspace_with_default_tab` now emit selection events when the
  selected workspace changes.
- `workspace_phases` and `chat_phases` are optimistic UI hints. They should not
  be treated as durable lifecycle truth.
- `pending_chat_prompt` is a single slot. Multiple prompt-stage actions before
  consumption overwrite earlier text.
- `RefreshHub` intentionally has no shared error channel; pages own load/store
  errors and render failures locally.
- Some non-chat workspace content actions still use the full workspace shell
  refresh because there is no narrower target yet. Current examples are file
  save, non-metadata branch actions, checkpoint create/restore, linked
  directory changes, and sibling conflict copy.
- Some fallback timers remain because not every producer emits reliable typed
  refresh events yet.

## Tests Covering This Contract

Relevant written tests live in:

- `crates/gtk-app/src/state.rs`
- `crates/gtk-app/src/refresh.rs`
- `crates/gtk-app/src/session_surface.rs`
- `crates/gtk-app/src/workspace_command_center.rs`
- `crates/gtk-app/src/projects.rs`
- `crates/gtk-app/src/sidebar.rs`
- `crates/gtk-app/src/background_sync.rs`

Notable protections:

- subscribers are notified after mutation and dropped cleanly
- refresh requests bridge through `AppStateEvent::RefreshRequested`
- pending chat queues migrate to real thread ids
- workspace create progress marks optimistic phases
- typed refresh fanout counts are asserted
- routine sources are forbidden from using `RefreshScope::All`
- chat session callbacks and PR prompt staging are guarded against direct
  sidebar/dashboard/history triples or workspace-shell rebuilds
- chat message refresh loads timelines off the GTK thread
- message refreshes warm nonselected thread caches without rendering them
- Archcar wake burst debounce is asserted

## Branch Focus Recommendation

Dedicate this branch to the `AppState` and refresh harness only:

- make transient state/event semantics explicit
- remove or narrow broad refreshes
- keep fallback timers only where a producer still lacks typed events
- add tests before changing refresh routing
- keep core/CLI independent of `AppState`
- preserve GTK responsiveness as the main customer value
