# Task 5 Report

## Summary

- Added `ChatTimelineItem` and `merge_chat_timeline_for_render` in `crates/gtk-app/src/session_surface.rs`.
- Switched GTK chat refresh to load both persisted chat messages and persisted chat events, then render one unified timeline ordered by `timeline_seq` with stable kind/id tie-breaks.
- Added `chat_event_widget` that reconstructs stored `payload_json` into `CodexTranscriptEvent` shapes and renders the existing inline event cards without reparsing ordinary chat messages.
- Kept a narrow legacy fallback: inline-event parsing from chat message text only runs when a thread has no persisted chat events.

## Tests

- `cargo test -p linux-archductor-gtk chat_timeline_keeps_messages_and_events_in_persisted_order -- --nocapture`
- `cargo test -p linux-archductor-gtk session_surface -- --nocapture`

## Notes

- `CodexTranscriptEvent` is not deserializable across the GTK crate boundary as-is, so GTK reconstructs it from the stored tagged JSON payload locally rather than changing core APIs.
