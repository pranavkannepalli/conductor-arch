# GTK Refresh And Background Chat Risk Status

Date: 2026-07-24

## Scope

This note captures current evidence for three related slices:

- `app-state-refresh`
- `background-chat-update`
- `codex-claude-archductor-event` mapping

Use the durable docs for architecture details:

- `docs/app-state-refresh-map.md`
- `docs/background-chat-update-map.md`
- `docs/codex-claude-archductor-event-map.md`

## Bottom Line

`codex-claude-archductor-event` is done for Bash/Read/Edit/Write classification.

`app-state-refresh` is mostly done, but not risk-free. The main GTK-thread sync
work was moved out of routine chat refresh paths, but some broad refresh paths
and fallback timers remain.

`background-chat-update` is useful and tested, but not done. It still uses
polling for some off-focus work and does not cover every non-running thread
change without explicit producer events.

## App-State-Refresh

Done:

- `AppState` owns hot process-local UI state.
- Typed refresh events cover routine runtime, review, workspace inventory,
  terminal, chat lifecycle, and chat message paths.
- `RefreshScope::All` is reserved for manual refresh and startup
  reconciliation.
- Chat timeline loads run in background jobs.
- Message refreshes use per-thread generations.
- Nonselected message refreshes warm cache without rendering.
- Sidebar workspace selection uses a generation counter.
- Runtime, notification, and background sync timers use in-flight guards.
- Spotlight file watching and refresh setup are off the GTK thread.

Still risky:

- `AppState` is not durable. Restart recovery must come from SQLite/core stores.
- Some page-local navigation and right-panel state still mutate without a
  dedicated event.
- `workspace_phases` and `chat_phases` are optimistic hints, not durable truth.
- `pending_chat_prompt` is still a single slot.
- Some non-chat workspace actions still use full workspace shell refreshes.
- Some fallback timers remain because not every producer emits reliable typed
  events.

Status: keep, but do not call the whole slice complete until remaining broad
refreshes and fallback timers are either removed or accepted as product policy.

## Background-Chat-Update

Done:

- Background sync samples lightweight running chat ids and sequence markers.
- Hidden full chat timelines are not loaded for off-focus work.
- Lifecycle refreshes are coalesced by workspace.
- Background chat queue scans use Archcar durable queue ids.
- Hidden queued sends skip the selected visible chat thread.
- Managed Codex/Claude automatic sends are rejected by Archcar until the session
  is ready.
- Immediate sends can still steer an active turn.
- Message-vs-lifecycle refresh intent is tested.
- Archcar wake bursts are debounced.

Still risky:

- Background sync still polls because producer events are not complete.
- The sampler tracks running chat threads only.
- Non-running thread changes depend on explicit lifecycle events from the action
  path.
- Some selected-workspace chat refresh paths still do synchronous store reads.
- Previous snapshot state is in memory, so restart begins from an empty
  comparison.
- Selected visible chat readiness/render cache remains local to
  `session_surface.rs`.

Status: not complete. Next work should replace polling with reliable typed
producer events and remove remaining selected-workspace synchronous store reads.

## Codex-Claude-Archductor-Event

Done in commit `e29490c`:

- Claude `Bash` maps to `CommandProcess` with subtype `command`.
- Claude `Read` maps to `FileSystem` with subtype `read`.
- Claude `Edit`, `MultiEdit`, `NotebookEdit`, `apply_patch`, and `patch` map
  to `FileSystem` with subtype `edit`.
- Claude `Write` and `Create` map to `FileSystem` with subtype `write`.
- Codex `dynamicToolCall` thread items inspect `params.item.tool`.
- Codex `item/tool/call` requests inspect `params.tool`.
- Codex `item/tool/call` preserves `params.callId` as a provider item id.
- Unknown dynamic tools remain generic `Tool`.
- MCP tools remain MCP/generic, not local file/command actions.

Verification from the mapping fix:

- `cargo test -p archductor-core --lib`
- `cargo test -p archductor-core claude_stream --lib -- --nocapture`
- `cargo test -p archductor-core codex_app_server --lib -- --nocapture`
- `cargo test -p archductor-core provider_projection --lib -- --nocapture`
- `cargo test -p archductor --test cli_sessions cli_archcar_messages_renders_claude_projected_provider_events -- --nocapture`
- `cargo test -p archductor-gtk provider_projection -- --nocapture`
- `cargo fmt --all -- --check`
- `cargo clippy -p archductor-core -p archductor -p archductor-gtk --all-targets -- -D warnings`

Not verified:

- Live authenticated Codex session.
- Live authenticated Claude session.
- Manual GTK session rendering with real provider events.

Status: adapter/projection mapping is complete for local Bash/Read/Edit/Write
events, with live-provider smoke still outstanding.
