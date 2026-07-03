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
