Task 4 Report
=============

Status
- Implemented benchmark-based Codex screen delta persistence in `WorkspaceStore::persist_codex_screen_delta`.
- Routed live Codex session screen updates through the new persistence path in `crates/core/src/archcar/session.rs`.
- Left raw and screen audit log writes in place via `append_session_process_output`.
- Removed log-replay-based visible chat persistence from `append_session_process_output`.

Tests
- Added replay regression coverage for old-message replay after new user input.
- Updated structured screen persistence coverage to use `persist_codex_screen_delta`.
- Verified with:
  - `cargo test -p linux-archductor-core codex_screen_delta_does_not_replay_old_messages_after_new_user_input -- --nocapture`
  - `cargo test -p linux-archductor-core codex_tui -- --nocapture`
  - `cargo test -p linux-archductor-core chat_messages -- --nocapture`

Commit
- Recorded in git as `Use benchmarks for Codex screen persistence`.

Concerns
- None from the requested verification scope.
