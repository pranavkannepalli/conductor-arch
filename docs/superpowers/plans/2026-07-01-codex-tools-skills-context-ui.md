# Codex Tools Skills Context UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Parse Codex-native tool calls, skill invocations, and context-window usage, then render them as structured GTK chat UI instead of raw transcript text.

**Architecture:** Extend the existing Codex TUI parser in `crates/core/src/codex_tui.rs` with small, tested data models for structured events and usage. Keep archcar messages backward-compatible by adding optional metadata only where useful. Render the richer events in `crates/gtk-app/src/session_surface.rs` with expandable inline rows, loading states, and a compact context usage ring beside the send control.

**Tech Stack:** Rust, GTK4/libadwaita, existing archcar protocol, existing chat message persistence, existing CSS in `crates/gtk-app/src/theme.rs`.

---

### Task 1: Core Codex Event Parser

**Files:**
- Modify: `crates/core/src/codex_tui.rs`
- Modify: `crates/core/src/archcar/protocol.rs`

- [ ] **Step 1: Add failing parser tests**

Add tests that cover:
- tool markers from Codex transcript text such as `functions.exec_command`, `web.run`, `multi_agent_v1.spawn_agent`, and `tool_search.tool_search_tool`
- skill announcements such as `Using superpowers:brainstorming to shape the implementation`
- expandable file references from skill/tool output paths
- context usage text such as `Context window: 42%` and `128k / 200k tokens`

Run: `cargo test -p linux-archductor-core codex_tui -- --nocapture`
Expected: FAIL until the parser types and functions exist.

- [ ] **Step 2: Implement the minimal parser**

Add public types:
- `CodexInlineEventKind::{Tool, Skill}`
- `CodexInlineEventStatus::{Loading, Complete, Failed}`
- `CodexInlineEvent { kind, title, subtitle, body, path, status }`
- `CodexContextUsage { used_tokens, max_tokens, percent }`
- `CodexParsedLine::{Message, InlineEvent, ContextUsage}`

Add functions:
- `parse_codex_inline_events(text: &str) -> Vec<CodexInlineEvent>`
- `parse_codex_context_usage(text: &str) -> Option<CodexContextUsage>`
- `parse_codex_structured_lines(text: &str) -> Vec<CodexParsedLine>`

Keep parsing conservative: prefer no event over a false positive. Do not change `parse_codex_screen_messages` behavior except to reuse helpers where it is clearly safe.

- [ ] **Step 3: Extend archcar message metadata compatibly**

Add optional fields to `ArchcarMessage`:
- `inline_event: Option<CodexInlineEvent>`
- `context_usage: Option<CodexContextUsage>`

Use `#[serde(default, skip_serializing_if = "Option::is_none")]` so older snapshots remain valid.

- [ ] **Step 4: Verify**

Run: `cargo test -p linux-archductor-core codex_tui archcar::protocol -- --nocapture`
Expected: PASS.

### Task 2: GTK Inline Event Rendering

**Files:**
- Modify: `crates/gtk-app/src/session_surface.rs`
- Modify: `crates/gtk-app/src/theme.rs`

- [ ] **Step 1: Add focused widget tests where possible**

Add pure helper tests for:
- mapping `CodexInlineEventKind` and status to display labels/classes
- deciding whether a path can be loaded for inline preview
- truncating long event bodies without losing expandability

Run: `cargo test -p linux-archductor-gtk session_surface -- --nocapture`
Expected: FAIL until helpers exist.

- [ ] **Step 2: Render expandable inline tool/skill rows**

Update `chat_message_widget` and supporting helpers so messages with tool or skill markers render as compact rows with:
- icon/glyph-style label from the event kind
- title and subtitle
- status indicator for loading/complete/failed
- expandable body using a `ToggleButton`
- inline file preview when `path` points to an existing local text file

Keep normal user/agent messages unchanged.

- [ ] **Step 3: Style the rows**

Add CSS classes:
- `.chat-inline-event`
- `.chat-inline-event-header`
- `.chat-inline-event-title`
- `.chat-inline-event-meta`
- `.chat-inline-event-body`
- `.chat-inline-event-loading`
- `.chat-inline-event-failed`

Match the existing dark chat surface, keep radius at or under the local style, and avoid card nesting.

- [ ] **Step 4: Verify**

Run: `cargo test -p linux-archductor-gtk session_surface -- --nocapture`
Expected: PASS.

### Task 3: Context Usage Ring Beside Send

**Files:**
- Modify: `crates/gtk-app/src/session_surface.rs`
- Modify: `crates/gtk-app/src/theme.rs`

- [ ] **Step 1: Add usage-state helper tests**

Add tests for:
- unknown usage displays neutral state
- percent under 70 is normal
- 70-89 is warning
- 90+ is danger
- tooltip text includes exact token counts when available

Run: `cargo test -p linux-archductor-gtk session_surface -- --nocapture`
Expected: FAIL until helpers exist.

- [ ] **Step 2: Add send-adjacent usage widget**

Create a compact `DrawingArea` or label-backed ring in the toolbar `right_group`, placed before the send button where the pie chart is supposed to live. It should:
- show current percent from parsed Codex context usage
- expose a tooltip with used/max tokens
- update when the selected thread refreshes
- stay visible as a neutral placeholder when usage is unknown

- [ ] **Step 3: Style usage states**

Add CSS classes:
- `.chat-context-usage`
- `.chat-context-usage-normal`
- `.chat-context-usage-warning`
- `.chat-context-usage-danger`
- `.chat-context-usage-empty`

Ensure the widget has stable dimensions and does not shift the send button.

- [ ] **Step 4: Verify**

Run: `cargo test -p linux-archductor-gtk session_surface -- --nocapture`
Expected: PASS.

### Task 4: Integration And Regression Pass

**Files:**
- Modify as needed only in files touched by Tasks 1-3.

- [ ] **Step 1: Parse persisted chat messages for events**

Make `chat_message_widget` parse existing `ChatMessageRecord.content` so already-persisted transcripts containing tool/skill text render structurally without a database migration.

- [ ] **Step 2: Preserve send flow behavior**

Confirm user messages, staged review sends, archcar Codex sends, and non-Codex sessions still use the existing paths.

- [ ] **Step 3: Run focused tests**

Run:
- `cargo test -p linux-archductor-core codex_tui archcar::protocol -- --nocapture`
- `cargo test -p linux-archductor-gtk session_surface -- --nocapture`

Expected: PASS.

- [ ] **Step 4: Run workspace check if platform dependencies allow**

Run: `cargo check --workspace`
Expected on macOS may fail if GTK/libadwaita system libraries are missing. If so, report the exact dependency failure and rely on the focused pure-Rust tests that can run.
