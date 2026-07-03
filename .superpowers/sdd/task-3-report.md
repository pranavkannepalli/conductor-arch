# Task 3 Report

- Status: complete
- Commit: `workspace: persist codex parser cursor and chat events`
- Files changed:
  - `crates/core/src/workspace.rs`
  - `.superpowers/sdd/task-3-report.md`
- Summary:
  - Added persistent Codex parse cursors keyed by `process_id`.
  - Added persistent `chat_events` storage with JSON payloads and idempotent duplicate detection.
  - Added shared monotonic `chat_timeline_seq` allocation and `timeline_seq` on chat messages/events for cross-table ordering.
  - Added persistence tests for cursor/event separation, shared timeline ordering, and idempotent event insertion.
- Tests run:
  - `cargo test -p linux-archductor-core codex_parser_cursor_and_events_persist_separately_from_messages -- --nocapture` -> passed
  - `cargo test -p linux-archductor-core chat_messages -- --nocapture` -> passed
  - `cargo test -p linux-archductor-core workspace::tests::chat -- --nocapture` -> passed
- Concerns:
  - Existing `chat_messages` rows are not backfilled with `timeline_seq`; they continue to sort via `COALESCE(timeline_seq, id)` until rewritten.
