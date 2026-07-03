Status: DONE

Commits:
- `Implement task 1 codex tui delta parsing`

Files changed:
- `crates/core/src/codex_tui.rs`
- `.superpowers/sdd/task-1-report.md`

Tests run:
- `cargo test -p linux-archductor-core screen_delta_starts_after_latest_known_user_message -- --nocapture` — passed
- `cargo test -p linux-archductor-core codex_tui -- --nocapture` — passed

Concerns:
- `CodexParsedItem::Event` and `CodexTranscriptEvent` are defined for the requested API, but this task only exercised message delta parsing.
- `parse_codex_screen_delta` currently fingerprints the trimmed screen text to suppress duplicate deltas when the cursor matches.

Follow-up after Task 2 review:
- Fixed `parse_codex_screen_delta` to emit `CodexParsedItem::Event` when a post-anchor message is actually a tool, skill, or file-change block.
- Replaced the lossy cursor summary with the full normalized screen text, so duplicate suppression now keys off the exact screen contents.
- Added regression tests for event emission after the latest known user message and exact-screen duplicate suppression.
- Verified with `cargo test -p linux-archductor-core codex_tui -- --nocapture` — passed (`28` tests).

Task 1 parser review follow-up:
- Added failing regressions for event delta anchoring after the latest agent message in boxed and live TUI layouts.
- Extended `delta_event_start_index` to honor `last_agent_message` with the same boundary handling used for user anchors.
- Kept the change scoped to `crates/core/src/codex_tui.rs`.
- Verified with `cargo test -p linux-archductor-core codex_tui -- --nocapture` — passed (`30` tests).

Task 1 multiline anchor fix:
- Added a regression test for multiline `last_user_message` anchoring so older tool events are not replayed.
- Reworked `delta_event_start_index` to anchor from parsed message spans and assembled message bodies instead of single physical lines.
- Kept `parse_codex_screen_messages` behavior intact and left the change scoped to `crates/core/src/codex_tui.rs`.
- Verified with `cargo test -p linux-archductor-core codex_tui -- --nocapture` — passed (`31` tests).
