# Task 6 Report

## Summary

- Added `LiveChatSource` with the `StructuredStore` variant and the `live_chat_source()` helper in `crates/gtk-app/src/session_surface.rs`.
- Wired the live `agent_session_panel` refresh path through that helper so the chat list is sourced from the structured store (`list_chat_messages` + `list_chat_events`) rather than transcript reparsing.
- Added the guard test `live_chat_uses_structured_store_not_session_log_reparse`.

## Audit

- Ran:
  `rg -n "parse_session_transcript_events|render_session_transcript_events|live_session_append_text" crates/gtk-app/src/session_surface.rs`
- Result:
  - `parse_session_transcript_events` remains in the explicit selected-session transcript renderer and the transcript parsing helpers/tests.
  - `render_session_transcript_events` and `live_session_append_text` remain in the legacy session transcript surface.
  - No `parse_session_transcript_events` call site appears inside the live `agent_session_panel` refresh branch.

## Tests

- `cargo test -p linux-archductor-gtk live_chat_uses_structured_store_not_session_log_reparse -- --nocapture`
- `cargo test -p linux-archductor-gtk session_surface -- --nocapture`

## Notes

- The transcript parser was intentionally left in place for explicit history/session transcript rendering.
