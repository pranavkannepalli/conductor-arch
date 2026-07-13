use archductor_core::provider_events::{ProviderEventRecord, ProviderEventStore};
use archductor_core::provider_projection::{
    provider_projection_from_records, provider_projection_item_text, ProviderProjectionStatus,
};
use archductor_core::redaction::redact_sensitive_text;
use archductor_core::session_event::{SessionEvent, SessionEventPayload};
use archductor_core::workspace::{ProcessRecord, ProcessStatus, PtyChunkRecord, WorkspaceStore};
use gtk::prelude::*;
use gtk::{
    Box as GBox, Button, CheckButton, Label, ListBox, ListBoxRow, Orientation, PolicyType,
    ScrolledWindow, SelectionMode, Stack,
};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Debug, Clone)]
struct InspectorSessionInput {
    id: i64,
    workspace: String,
    command: String,
    log_path: PathBuf,
    pid: Option<u32>,
    status: ProcessStatus,
    started_at: String,
    ended_at: Option<String>,
    exit_code: Option<i32>,
    raw_output: String,
    raw_chunks: Vec<RawChunk>,
    events: Vec<SessionEvent>,
    provider_events: Vec<ProviderEventRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PtyInspectorModel {
    sessions: Vec<InspectorSessionRow>,
    details: Vec<InspectorSessionDetail>,
    selected: InspectorSessionDetail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InspectorSessionRow {
    session_id: i64,
    workspace: String,
    process_status: String,
    parser_state: String,
    last_activity: String,
    pid: String,
    chunk_count: usize,
    output_rate_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InspectorSessionDetail {
    session_id: Option<i64>,
    log_path: String,
    raw_chunks: Vec<RawChunk>,
    normalized_text: String,
    events: Vec<InspectorEventRow>,
    diagnostics: InspectorDiagnostics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawChunk {
    index: usize,
    text: String,
    normalized_text: String,
    raw_normalized_text: String,
    duplicate: bool,
    delayed: bool,
    partial: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventFilter {
    Prompts,
    Assistant,
    ToolOutput,
    Errors,
    StateTransitions,
    Metadata,
}

impl EventFilter {
    fn label(self) -> &'static str {
        match self {
            Self::Prompts => "Prompts",
            Self::Assistant => "Assistant",
            Self::ToolOutput => "Tools",
            Self::Errors => "Errors",
            Self::StateTransitions => "State",
            Self::Metadata => "Metadata",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InspectorEventRow {
    sequence: String,
    timestamp: String,
    source_chunk: String,
    filter: EventFilter,
    status: String,
    rendered_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct InspectorDiagnostics {
    pid: String,
    command: String,
    cwd_or_workspace: String,
    start_time: String,
    exit_code: String,
    signal: String,
    restart_count: String,
    session_state: String,
    last_lifecycle_action: String,
    lifecycle: Vec<String>,
}

pub(crate) fn build_pty_inspector_page(db_path: PathBuf) -> GBox {
    let sessions = load_session_inputs(db_path)
        .map(build_inspector_model)
        .unwrap_or_else(|_| build_inspector_model(Vec::new()));
    render_inspector_model(sessions)
}

fn load_session_inputs(db_path: PathBuf) -> anyhow::Result<Vec<InspectorSessionInput>> {
    let store = WorkspaceStore::open(&db_path)?;
    let mut sessions = Vec::new();
    for status in store.list_status()? {
        for process in store.list_sessions(&status.workspace.name)? {
            let raw_output = std::fs::read_to_string(&process.log_path).unwrap_or_default();
            let chunk_rows = store.list_pty_chunks(process.id).unwrap_or_default();
            let events = store.list_session_events(process.id).unwrap_or_default();
            let provider_events = ProviderEventStore::new(&db_path)
                .list_for_process(process.id)
                .unwrap_or_default();
            sessions.push(session_input_from_process(
                &status.workspace.name,
                process,
                raw_output,
                chunk_rows,
                events,
                provider_events,
            ));
        }
    }
    sessions.sort_by(|a, b| {
        b.started_at
            .cmp(&a.started_at)
            .then_with(|| b.id.cmp(&a.id))
    });
    Ok(sessions)
}

fn session_input_from_process(
    workspace: &str,
    process: ProcessRecord,
    raw_output: String,
    chunk_rows: Vec<PtyChunkRecord>,
    events: Vec<SessionEvent>,
    provider_events: Vec<ProviderEventRecord>,
) -> InspectorSessionInput {
    let raw_chunks = if chunk_rows.is_empty() {
        raw_chunks_from_log(&raw_output)
    } else {
        raw_chunks_from_records(&chunk_rows)
    };
    InspectorSessionInput {
        id: process.id,
        workspace: workspace.to_owned(),
        command: process.command,
        log_path: process.log_path,
        pid: (process.pid > 0).then_some(process.pid),
        status: process.status,
        started_at: process.started_at,
        ended_at: process.ended_at,
        exit_code: process.exit_code,
        raw_output,
        raw_chunks,
        events,
        provider_events,
    }
}

fn build_inspector_model(sessions: Vec<InspectorSessionInput>) -> PtyInspectorModel {
    let selected = sessions
        .first()
        .map(session_detail)
        .unwrap_or_else(empty_session_detail);
    let details = sessions.iter().map(session_detail).collect();
    let rows = sessions.iter().map(session_row).collect();
    PtyInspectorModel {
        sessions: rows,
        details,
        selected,
    }
}

fn session_row(session: &InspectorSessionInput) -> InspectorSessionRow {
    InspectorSessionRow {
        session_id: session.id,
        workspace: session.workspace.clone(),
        process_status: session.status.as_str().to_owned(),
        parser_state: if session.events.is_empty() && session.provider_events.is_empty() {
            "no parsed events".to_owned()
        } else {
            let event_count = session.events.len()
                + provider_projection_from_records(&session.provider_events)
                    .items
                    .len();
            pluralize(event_count, "event")
        },
        last_activity: session
            .ended_at
            .clone()
            .unwrap_or_else(|| session.started_at.clone()),
        pid: session
            .pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "n/a".to_owned()),
        chunk_count: session.raw_chunks.len(),
        output_rate_label: pluralize(session.raw_chunks.len(), "chunk"),
    }
}

fn session_detail(session: &InspectorSessionInput) -> InspectorSessionDetail {
    let raw_chunks = session.raw_chunks.clone();
    let normalized_text = if raw_chunks.is_empty() {
        normalize_pty_text(&redact_sensitive_text(&session.raw_output))
    } else {
        raw_chunks
            .iter()
            .map(|chunk| chunk.normalized_text.as_str())
            .collect::<String>()
    };
    let events = session
        .events
        .iter()
        .map(|event| event_row(event, &raw_chunks))
        .chain(provider_event_rows(&session.provider_events))
        .collect::<Vec<_>>();
    InspectorSessionDetail {
        session_id: Some(session.id),
        log_path: session.log_path.display().to_string(),
        raw_chunks,
        normalized_text,
        events,
        diagnostics: diagnostics(session),
    }
}

fn empty_session_detail() -> InspectorSessionDetail {
    InspectorSessionDetail {
        session_id: None,
        log_path: "n/a".to_owned(),
        raw_chunks: Vec::new(),
        normalized_text: String::new(),
        events: Vec::new(),
        diagnostics: InspectorDiagnostics {
            pid: "n/a".to_owned(),
            command: "n/a".to_owned(),
            cwd_or_workspace: "n/a".to_owned(),
            start_time: "n/a".to_owned(),
            exit_code: "n/a".to_owned(),
            signal: "n/a".to_owned(),
            restart_count: "0".to_owned(),
            session_state: "empty".to_owned(),
            last_lifecycle_action: "none".to_owned(),
            lifecycle: vec!["No sessions recorded.".to_owned()],
        },
    }
}

fn raw_chunks_from_log(raw: &str) -> Vec<RawChunk> {
    if raw.is_empty() {
        return Vec::new();
    }
    let chunks = raw
        .split_inclusive('\n')
        .chain((!raw.ends_with('\n')).then_some(""))
        .filter(|chunk| !chunk.is_empty())
        .enumerate()
        .map(|(index, chunk)| (index + 1, chunk))
        .collect::<Vec<_>>();
    raw_chunks_from_slices(raw, &chunks)
}

fn raw_chunks_from_records(records: &[PtyChunkRecord]) -> Vec<RawChunk> {
    let raw = records
        .iter()
        .map(|record| record.text.as_str())
        .collect::<String>();
    let mut offset = 0;
    let chunks = records
        .iter()
        .map(|record| {
            let start = offset;
            offset += record.text.len();
            (record.sequence as usize, &raw[start..offset])
        })
        .collect::<Vec<_>>();
    raw_chunks_from_slices(&raw, &chunks)
}

fn raw_chunks_from_slices(raw: &str, chunks: &[(usize, &str)]) -> Vec<RawChunk> {
    if raw.is_empty() {
        return Vec::new();
    }
    let redacted = redact_sensitive_text(raw);
    let redacted_chunks = split_redacted_stream_for_chunks(raw, &redacted, chunks);
    let mut previous = String::new();
    chunks
        .iter()
        .enumerate()
        .map(|(position, (index, chunk))| {
            let text = redacted_chunks.get(position).cloned().unwrap_or_default();
            let normalized_text = normalize_pty_text(&text);
            let raw_normalized_text = normalize_pty_text(chunk);
            let duplicate = !normalized_text.trim().is_empty() && normalized_text == previous;
            previous = normalized_text.clone();
            RawChunk {
                index: *index,
                text,
                normalized_text,
                raw_normalized_text,
                duplicate,
                delayed: false,
                partial: !chunk.ends_with('\n'),
            }
        })
        .collect()
}

fn split_redacted_stream_for_chunks(
    raw: &str,
    redacted: &str,
    chunks: &[(usize, &str)],
) -> Vec<String> {
    let mut output = Vec::with_capacity(chunks.len());
    let mut raw_cursor = 0;
    let mut cursor = 0;
    let raw_boundaries = chunks
        .iter()
        .take(chunks.len().saturating_sub(1))
        .map(|(_, chunk)| {
            raw_cursor += chunk.len();
            raw_cursor
        })
        .collect::<Vec<_>>();
    let redacted_boundaries = redacted_offsets_for_raw_boundaries(raw, redacted, &raw_boundaries);
    for (position, _) in chunks.iter().enumerate() {
        let is_last = position + 1 == chunks.len();
        let end = if is_last {
            redacted.len()
        } else {
            redacted_boundaries[position].max(cursor)
        };
        output.push(redacted[cursor..end].to_owned());
        cursor = end;
    }
    output
}

fn redacted_offsets_for_raw_boundaries(
    raw: &str,
    redacted: &str,
    raw_boundaries: &[usize],
) -> Vec<usize> {
    const MARKER: &str = "[redacted]";
    let mut offsets = Vec::with_capacity(raw_boundaries.len());
    let mut raw_cursor = 0;
    let mut redacted_cursor = 0;

    for &boundary in raw_boundaries {
        while raw_cursor < boundary && raw_cursor < raw.len() && redacted_cursor < redacted.len() {
            let raw_char = raw[raw_cursor..].chars().next();
            let redacted_char = redacted[redacted_cursor..].chars().next();
            if redacted[redacted_cursor..].starts_with(MARKER)
                && !raw_redaction_marker_is_literal(raw, raw_cursor, redacted, redacted_cursor)
            {
                redacted_cursor += MARKER.len();
                raw_cursor =
                    find_redaction_resume_offset(raw, raw_cursor, redacted, redacted_cursor);
            } else if raw_char == redacted_char {
                advance_char(raw, &mut raw_cursor);
                advance_char(redacted, &mut redacted_cursor);
            } else if redacted[redacted_cursor..].starts_with(MARKER) {
                redacted_cursor += MARKER.len();
                raw_cursor =
                    find_redaction_resume_offset(raw, raw_cursor, redacted, redacted_cursor);
            } else {
                advance_char(raw, &mut raw_cursor);
                advance_char(redacted, &mut redacted_cursor);
            }
        }
        offsets.push(redacted_cursor);
    }
    offsets
}

fn raw_redaction_marker_is_literal(
    raw: &str,
    raw_cursor: usize,
    redacted: &str,
    redacted_cursor: usize,
) -> bool {
    const MARKER: &str = "[redacted]";
    if !raw[raw_cursor..].starts_with(MARKER) {
        return false;
    }
    let raw_after_marker = raw_cursor + MARKER.len();
    let redacted_after_marker = redacted_cursor + MARKER.len();
    redacted_after_marker >= redacted.len()
        || raw[raw_after_marker..] == redacted[redacted_after_marker..]
        || common_prefix_chars(&raw[raw_after_marker..], &redacted[redacted_after_marker..]) >= 4
}

fn find_redaction_resume_offset(
    raw: &str,
    raw_start: usize,
    redacted: &str,
    redacted_start: usize,
) -> usize {
    if redacted_start >= redacted.len() {
        return raw.len();
    }
    let mut best = raw.len();
    for (offset, _) in raw[raw_start..].char_indices() {
        let candidate = raw_start + offset;
        if common_prefix_chars(&raw[candidate..], &redacted[redacted_start..]) >= 4 {
            return candidate;
        }
        if best == raw.len()
            && raw[candidate..].chars().next() == redacted[redacted_start..].chars().next()
        {
            best = candidate;
        }
    }
    best
}

fn common_prefix_chars(left: &str, right: &str) -> usize {
    left.chars()
        .zip(right.chars())
        .take_while(|(left, right)| left == right)
        .count()
}

fn advance_char(value: &str, cursor: &mut usize) {
    if let Some(ch) = value[*cursor..].chars().next() {
        *cursor += ch.len_utf8();
    } else {
        *cursor = value.len();
    }
}

fn normalize_pty_text(raw: &str) -> String {
    raw.replace("\r\n", "\n").replace('\r', "\n")
}

fn event_row(event: &SessionEvent, chunks: &[RawChunk]) -> InspectorEventRow {
    let rendered_text = redact_sensitive_text(&event.render_text());
    let source_chunk = event
        .raw_text
        .as_deref()
        .and_then(|raw| {
            let raw = normalize_pty_text(raw);
            chunks
                .iter()
                .find(|chunk| chunk.raw_normalized_text.contains(raw.trim()))
                .map(|chunk| format!("chunk {}", chunk.index))
        })
        .unwrap_or_else(|| "unmatched".to_owned());
    InspectorEventRow {
        sequence: event
            .sequence
            .map(|sequence| sequence.to_string())
            .unwrap_or_else(|| "n/a".to_owned()),
        timestamp: event
            .occurred_at_ms
            .map(|timestamp| timestamp.to_string())
            .unwrap_or_else(|| "n/a".to_owned()),
        source_chunk,
        filter: event_filter(event),
        status: event_status_label(event),
        rendered_text,
    }
}

fn provider_event_rows(events: &[ProviderEventRecord]) -> Vec<InspectorEventRow> {
    let projection = provider_projection_from_records(events);
    projection
        .items
        .iter()
        .map(|item| InspectorEventRow {
            sequence: item.sequence.to_string(),
            timestamp: events
                .iter()
                .find(|event| event.received_sequence.max(0) as u64 == item.sequence)
                .map(|event| event.occurred_at_ms.to_string())
                .unwrap_or_else(|| "provider".to_owned()),
            source_chunk: "provider-event".to_owned(),
            filter: if item.status == ProviderProjectionStatus::Failed {
                EventFilter::Errors
            } else {
                provider_event_filter(item.category)
            },
            status: format!("{} / {}", item.status.as_str(), item.stream_state.as_str()),
            rendered_text: redact_sensitive_text(&provider_projection_item_text(item)),
        })
        .collect()
}

fn provider_event_filter(
    category: archductor_core::provider_projection::ProviderProjectionCategory,
) -> EventFilter {
    use archductor_core::provider_projection::ProviderProjectionCategory as Category;
    match category {
        Category::UserMessage | Category::Question | Category::Approval => EventFilter::Prompts,
        Category::AssistantMessage | Category::Plan | Category::Reasoning => EventFilter::Assistant,
        Category::Command
        | Category::Process
        | Category::FileRead
        | Category::FileWrite
        | Category::FilePatch
        | Category::FileDiff
        | Category::McpTool
        | Category::NativeTool
        | Category::Skill
        | Category::Plugin
        | Category::Hook
        | Category::Subagent
        | Category::NestedTranscript => EventFilter::ToolOutput,
        Category::RateLimit | Category::Error => EventFilter::Errors,
        Category::Status => EventFilter::StateTransitions,
        Category::BackgroundTerminal
        | Category::BackgroundTask
        | Category::Web
        | Category::Image
        | Category::Usage
        | Category::Cost
        | Category::Context
        | Category::Unknown => EventFilter::Metadata,
    }
}

fn event_filter(event: &SessionEvent) -> EventFilter {
    match &event.payload {
        SessionEventPayload::Prompt { .. } => EventFilter::Prompts,
        SessionEventPayload::AssistantText { .. } => EventFilter::Assistant,
        SessionEventPayload::CommandOutput { status, .. } => {
            if matches!(
                status,
                archductor_core::session_event::SessionCommandOutputStatus::Failed
            ) {
                EventFilter::Errors
            } else {
                EventFilter::ToolOutput
            }
        }
        SessionEventPayload::Error { .. } => EventFilter::Errors,
        SessionEventPayload::StatusChange { .. } => EventFilter::StateTransitions,
        SessionEventPayload::Metadata { .. } | SessionEventPayload::UserInput { .. } => {
            EventFilter::Metadata
        }
    }
}

fn event_status_label(event: &SessionEvent) -> String {
    match &event.payload {
        SessionEventPayload::CommandOutput { status, .. } => format!("{status:?}").to_lowercase(),
        SessionEventPayload::Error { recoverable, .. } => {
            if *recoverable {
                "recoverable error".to_owned()
            } else {
                "error".to_owned()
            }
        }
        SessionEventPayload::StatusChange { status, .. } => format!("{status:?}").to_lowercase(),
        _ => format!("{:?}", event.source).to_lowercase(),
    }
}

fn diagnostics(session: &InspectorSessionInput) -> InspectorDiagnostics {
    let session_state = session.status.as_str().to_owned();
    let exit_code = session
        .exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "n/a".to_owned());
    InspectorDiagnostics {
        pid: session
            .pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "n/a".to_owned()),
        command: redact_sensitive_text(&session.command),
        cwd_or_workspace: session.workspace.clone(),
        start_time: session.started_at.clone(),
        signal: signal_label(session.exit_code),
        restart_count: "0".to_owned(),
        last_lifecycle_action: session_state.clone(),
        lifecycle: vec![
            format!("{} started", session.started_at),
            session
                .ended_at
                .as_ref()
                .map(|ended| format!("{ended} {}", session.status.as_str()))
                .unwrap_or_else(|| format!("{} still running", session.status.as_str())),
        ],
        session_state,
        exit_code,
    }
}

fn diagnostic_bundle_text(detail: &InspectorSessionDetail) -> String {
    let mut out = String::new();
    out.push_str("Archductor session log diagnostic bundle\n");
    out.push_str(&format!("App version: {}\n", env!("CARGO_PKG_VERSION")));
    out.push_str(&format!(
        "Session: {}\n",
        detail
            .session_id
            .map(|id| format!("#{id}"))
            .unwrap_or_else(|| "n/a".to_owned())
    ));
    out.push_str(&format!("Raw log path: {}\n\n", detail.log_path));

    out.push_str("Process metadata\n");
    out.push_str(&format!("PID: {}\n", detail.diagnostics.pid));
    out.push_str(&format!("Command: {}\n", detail.diagnostics.command));
    out.push_str(&format!(
        "Workspace: {}\n",
        detail.diagnostics.cwd_or_workspace
    ));
    out.push_str(&format!("Started: {}\n", detail.diagnostics.start_time));
    out.push_str(&format!("Exit code: {}\n", detail.diagnostics.exit_code));
    out.push_str(&format!("Signal: {}\n", detail.diagnostics.signal));
    out.push_str(&format!("State: {}\n", detail.diagnostics.session_state));
    out.push_str(&format!(
        "Last action: {}\n\n",
        detail.diagnostics.last_lifecycle_action
    ));

    out.push_str("Lifecycle\n");
    for item in &detail.diagnostics.lifecycle {
        out.push_str(&format!("- {item}\n"));
    }

    out.push_str("\nState transitions\n");
    let state_rows = detail
        .events
        .iter()
        .filter(|event| event.filter == EventFilter::StateTransitions)
        .collect::<Vec<_>>();
    if state_rows.is_empty() {
        out.push_str("- None recorded.\n");
    } else {
        for event in state_rows {
            out.push_str(&format!(
                "- #{} {} {}\n{}\n",
                event.sequence, event.timestamp, event.status, event.rendered_text
            ));
        }
    }

    out.push_str("\nParsed events\n");
    if detail.events.is_empty() {
        out.push_str("- None recorded.\n");
    } else {
        for event in &detail.events {
            out.push_str(&format!(
                "- #{} {} {:?} {} {}\n{}\n",
                event.sequence,
                event.timestamp,
                event.filter,
                event.source_chunk,
                event.status,
                event.rendered_text
            ));
        }
    }

    out.push_str("\nRaw output (redacted)\n");
    if detail.raw_chunks.is_empty() {
        out.push_str("No raw output.\n");
    } else {
        for chunk in &detail.raw_chunks {
            out.push_str(&format!("chunk {}\n{}\n", chunk.index, chunk.text));
        }
    }

    out.push_str("\nNormalized output (redacted)\n");
    if detail.normalized_text.is_empty() {
        out.push_str("No normalized output.\n");
    } else {
        out.push_str(&detail.normalized_text);
        if !detail.normalized_text.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

fn signal_label(exit_code: Option<i32>) -> String {
    match exit_code {
        Some(code) if code >= 128 => format!("signal {}", code - 128),
        _ => "n/a".to_owned(),
    }
}

fn pluralize(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

fn render_inspector_model(model: PtyInspectorModel) -> GBox {
    let root = GBox::new(Orientation::Vertical, 12);
    root.add_css_class("workspace-detail");
    root.set_margin_top(16);
    root.set_margin_bottom(16);
    root.set_margin_start(16);
    root.set_margin_end(16);

    let title = Label::new(Some("Session Logs"));
    title.add_css_class("section-title");
    title.set_xalign(0.0);
    root.append(&title);
    root.append(&small_label(
        "Debug mode only. Raw session logs can contain sensitive data; secret-looking values are redacted before display or copy.",
    ));

    let layout = GBox::new(Orientation::Horizontal, 12);
    layout.set_hexpand(true);
    layout.set_vexpand(true);
    root.append(&layout);

    let session_list = ListBox::new();
    session_list.add_css_class("workspace-list");
    session_list.set_selection_mode(SelectionMode::Single);
    if model.sessions.is_empty() {
        session_list.append(&label_row("No active or recent sessions."));
    } else {
        for session in &model.sessions {
            session_list.append(&session_row_widget(session));
        }
        if let Some(row) = session_list.row_at_index(0) {
            session_list.select_row(Some(&row));
        }
    }
    let session_scroll = scroller(&session_list);
    session_scroll.set_min_content_width(300);
    layout.append(&session_scroll);

    let center_stack = Stack::new();
    center_stack.set_hexpand(true);
    center_stack.set_vexpand(true);
    let raw_container = GBox::new(Orientation::Vertical, 6);
    render_raw_chunks_into(&raw_container, &model.selected);
    let normalized_container = GBox::new(Orientation::Vertical, 6);
    render_normalized_into(&normalized_container, &model.selected);
    let raw_scroll = scroller(&raw_container);
    raw_scroll.set_min_content_height(480);
    let normalized_scroll = scroller(&normalized_container);
    normalized_scroll.set_min_content_height(480);
    center_stack.add_named(&raw_scroll, Some("raw"));
    center_stack.add_named(&normalized_scroll, Some("normalized"));
    center_stack.set_visible_child_name("raw");
    let selected_detail = Rc::new(RefCell::new(model.selected.clone()));
    let center = GBox::new(Orientation::Vertical, 8);
    let toolbar = GBox::new(Orientation::Horizontal, 6);
    let raw_toggle = CheckButton::with_label("Raw stream (redacted)");
    raw_toggle.set_active(true);
    let normalized_toggle = CheckButton::with_label("Normalized text");
    normalized_toggle.set_group(Some(&raw_toggle));
    {
        let center_stack = center_stack.clone();
        raw_toggle.connect_toggled(move |button| {
            if button.is_active() {
                center_stack.set_visible_child_name("raw");
            }
        });
    }
    {
        let center_stack = center_stack.clone();
        normalized_toggle.connect_toggled(move |button| {
            if button.is_active() {
                center_stack.set_visible_child_name("normalized");
            }
        });
    }
    let clear_btn = Button::with_label("Clear local view");
    {
        let raw_container = raw_container.clone();
        let normalized_container = normalized_container.clone();
        clear_btn.connect_clicked(move |_| {
            clear_box(&raw_container);
            raw_container.append(&small_label("Local raw view cleared."));
            clear_box(&normalized_container);
            normalized_container.append(&small_label("Local normalized view cleared."));
        });
    }
    let copy_btn = Button::with_label("Copy redacted output");
    let visible_output = Rc::new(RefCell::new(model.selected.normalized_text.clone()));
    {
        let visible_output = Rc::clone(&visible_output);
        copy_btn.connect_clicked(move |_| {
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&visible_output.borrow());
            }
        });
    }
    let export_btn = Button::with_label("Copy diagnostic bundle");
    {
        let selected_detail = Rc::clone(&selected_detail);
        export_btn.connect_clicked(move |_| {
            let bundle = diagnostic_bundle_text(&selected_detail.borrow());
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&bundle);
            }
        });
    }
    let jump_btn = Button::with_label("Jump to latest");
    let pause_toggle = CheckButton::with_label("Pause auto-scroll");
    toolbar.append(&raw_toggle);
    toolbar.append(&normalized_toggle);
    toolbar.append(&pause_toggle);
    toolbar.append(&clear_btn);
    toolbar.append(&copy_btn);
    toolbar.append(&export_btn);
    toolbar.append(&jump_btn);
    center.append(&toolbar);
    center.append(&center_stack);
    layout.append(&center);

    let right = GBox::new(Orientation::Vertical, 10);
    right.set_size_request(360, -1);
    let events_container = GBox::new(Orientation::Vertical, 6);
    render_events_into(
        &events_container,
        &model.selected.events,
        &all_event_filters(),
    );
    let active_filters = Rc::new(RefCell::new(all_event_filters()));
    right.append(&event_filters_panel(
        &events_container,
        Rc::clone(&selected_detail),
        Rc::clone(&active_filters),
    ));
    let diagnostics_container = GBox::new(Orientation::Vertical, 6);
    render_diagnostics_into(&diagnostics_container, &model.selected.diagnostics);
    right.append(&events_container);
    right.append(&diagnostics_container);
    layout.append(&scroller(&right));

    {
        let details = model.details.clone();
        let raw_container = raw_container.clone();
        let normalized_container = normalized_container.clone();
        let events_container = events_container.clone();
        let diagnostics_container = diagnostics_container.clone();
        let visible_output = Rc::clone(&visible_output);
        let selected_detail = Rc::clone(&selected_detail);
        let active_filters = Rc::clone(&active_filters);
        session_list.connect_row_selected(move |_, row| {
            let Some(detail) = row
                .and_then(|row| usize::try_from(row.index()).ok())
                .and_then(|index| details.get(index))
            else {
                return;
            };
            render_raw_chunks_into(&raw_container, detail);
            render_normalized_into(&normalized_container, detail);
            render_events_into(&events_container, &detail.events, &active_filters.borrow());
            render_diagnostics_into(&diagnostics_container, &detail.diagnostics);
            *visible_output.borrow_mut() = detail.normalized_text.clone();
            *selected_detail.borrow_mut() = detail.clone();
        });
    }

    root
}

fn scroller<W: IsA<gtk::Widget>>(child: &W) -> ScrolledWindow {
    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    scroll.set_vexpand(true);
    scroll.set_hexpand(true);
    scroll.set_child(Some(child));
    scroll
}

fn label_row(text: &str) -> ListBoxRow {
    let label = Label::new(Some(text));
    label.add_css_class("empty-label");
    label.set_xalign(0.0);
    ListBoxRow::builder().child(&label).build()
}

fn session_row_widget(session: &InspectorSessionRow) -> ListBoxRow {
    let body = GBox::new(Orientation::Vertical, 4);
    body.add_css_class("project-row");
    let title = Label::new(Some(&format!(
        "#{} {}",
        session.session_id, session.workspace
    )));
    title.add_css_class("workspace-name");
    title.set_xalign(0.0);
    body.append(&title);
    body.append(&small_label(&format!(
        "{} · {} · PID {}",
        session.process_status, session.parser_state, session.pid
    )));
    body.append(&small_label(&format!(
        "{} · {} · {}",
        session.last_activity, session.output_rate_label, session.chunk_count
    )));
    ListBoxRow::builder().child(&body).build()
}

fn clear_box(panel: &GBox) {
    while let Some(child) = panel.first_child() {
        panel.remove(&child);
    }
}

fn render_raw_chunks_into(panel: &GBox, detail: &InspectorSessionDetail) {
    clear_box(panel);
    panel.append(&section_label("Sensitive raw logs (redacted)"));
    if detail.raw_chunks.is_empty() {
        panel.append(&small_label("No raw output."));
    }
    for chunk in &detail.raw_chunks {
        let flags = [
            chunk.duplicate.then_some("duplicate"),
            chunk.delayed.then_some("delayed"),
            chunk.partial.then_some("partial"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");
        let text = if flags.is_empty() {
            format!("chunk {}\n{}", chunk.index, chunk.text)
        } else {
            format!("chunk {} [{}]\n{}", chunk.index, flags, chunk.text)
        };
        panel.append(&mono_label(&text));
    }
}

fn render_normalized_into(panel: &GBox, detail: &InspectorSessionDetail) {
    clear_box(panel);
    panel.append(&section_label("Normalized output (redacted)"));
    panel.append(&mono_label(if detail.normalized_text.is_empty() {
        "No normalized output."
    } else {
        &detail.normalized_text
    }));
}

fn event_filters_panel(
    events_container: &GBox,
    selected_detail: Rc<RefCell<InspectorSessionDetail>>,
    active_filters: Rc<RefCell<Vec<EventFilter>>>,
) -> GBox {
    let panel = GBox::new(Orientation::Horizontal, 4);
    for filter_kind in all_event_filters() {
        let filter = CheckButton::with_label(filter_kind.label());
        filter.set_active(true);
        {
            let events_container = events_container.clone();
            let selected_detail = Rc::clone(&selected_detail);
            let active_filters = Rc::clone(&active_filters);
            filter.connect_toggled(move |button| {
                let mut filters = active_filters.borrow_mut();
                if button.is_active() {
                    if !filters.contains(&filter_kind) {
                        filters.push(filter_kind);
                    }
                } else {
                    filters.retain(|filter| *filter != filter_kind);
                }
                render_events_into(
                    &events_container,
                    &selected_detail.borrow().events,
                    &filters,
                );
            });
        }
        panel.append(&filter);
    }
    panel
}

fn all_event_filters() -> Vec<EventFilter> {
    vec![
        EventFilter::Prompts,
        EventFilter::Assistant,
        EventFilter::ToolOutput,
        EventFilter::Errors,
        EventFilter::StateTransitions,
        EventFilter::Metadata,
    ]
}

fn filter_event_rows<'a>(
    events: &'a [InspectorEventRow],
    filters: &[EventFilter],
) -> Vec<&'a InspectorEventRow> {
    events
        .iter()
        .filter(|event| filters.contains(&event.filter))
        .collect()
}

fn render_events_into(panel: &GBox, events: &[InspectorEventRow], filters: &[EventFilter]) {
    clear_box(panel);
    panel.append(&section_label("Parsed events"));
    let visible_events = filter_event_rows(events, filters);
    if visible_events.is_empty() {
        panel.append(&small_label("No parsed events."));
    }
    for event in visible_events {
        panel.append(&small_label(&format!(
            "#{} {} {:?} {}",
            event.sequence, event.timestamp, event.filter, event.source_chunk
        )));
        panel.append(&mono_label(&format!(
            "{}\n{}",
            event.status, event.rendered_text
        )));
    }
}

fn render_diagnostics_into(panel: &GBox, diagnostics: &InspectorDiagnostics) {
    clear_box(panel);
    panel.append(&section_label("Process diagnostics"));
    for (key, value) in [
        ("PID", diagnostics.pid.as_str()),
        ("Command", diagnostics.command.as_str()),
        ("Workspace", diagnostics.cwd_or_workspace.as_str()),
        ("Started", diagnostics.start_time.as_str()),
        ("Exit code", diagnostics.exit_code.as_str()),
        ("Signal", diagnostics.signal.as_str()),
        ("Restarts", diagnostics.restart_count.as_str()),
        ("State", diagnostics.session_state.as_str()),
        ("Last action", diagnostics.last_lifecycle_action.as_str()),
    ] {
        panel.append(&small_label(&format!("{key}: {value}")));
    }
    panel.append(&section_label("Lifecycle"));
    for item in &diagnostics.lifecycle {
        panel.append(&small_label(item));
    }
}

fn section_label(text: &str) -> Label {
    let label = Label::new(Some(text));
    label.add_css_class("section-title");
    label.set_xalign(0.0);
    label
}

fn small_label(text: &str) -> Label {
    let label = Label::new(Some(text));
    label.add_css_class("card-meta");
    label.set_xalign(0.0);
    label.set_wrap(true);
    label
}

fn mono_label(text: &str) -> Label {
    let label = small_label(text);
    label.add_css_class("terminal-output");
    label.set_selectable(true);
    label
}

#[cfg(test)]
mod tests {
    use super::*;
    use archductor_core::provider_events::{ProviderEventKind, ProviderEventPhase};
    use archductor_core::session_event::{
        SessionCommandOutputStatus, SessionEvent, SessionEventPayload, SessionEventSource,
        SessionEventStatus,
    };
    use archductor_core::workspace::ProcessStatus;
    use serde_json::json;

    #[test]
    fn inspector_model_summarizes_sessions_output_events_and_diagnostics() {
        let session = InspectorSessionInput {
            id: 7,
            workspace: "berlin".to_owned(),
            command: "ARCHDUCTOR_TOKEN=secret codex --model gpt-5".to_owned(),
            log_path: PathBuf::from("/tmp/session-7.log"),
            pid: Some(4242),
            status: ProcessStatus::Running,
            started_at: "2026-07-05T12:00:00Z".to_owned(),
            ended_at: None,
            exit_code: None,
            raw_output: "hello\r\nhello\r\npartial".to_owned(),
            raw_chunks: raw_chunks_from_log("hello\r\nhello\r\npartial"),
            events: vec![
                SessionEvent::new(
                    SessionEventSource::Assistant,
                    Some("hello".to_owned()),
                    SessionEventPayload::AssistantText {
                        text: "hello".to_owned(),
                    },
                )
                .with_sequence(1)
                .with_occurred_at_ms(100),
                SessionEvent::new(
                    SessionEventSource::Runtime,
                    Some("tool failed".to_owned()),
                    SessionEventPayload::CommandOutput {
                        title: "cargo test".to_owned(),
                        output: "failed".to_owned(),
                        status: SessionCommandOutputStatus::Failed,
                    },
                )
                .with_sequence(2)
                .with_occurred_at_ms(120),
            ],
            provider_events: Vec::new(),
        };

        let model = build_inspector_model(vec![session]);

        assert_eq!(model.sessions[0].session_id, 7);
        assert_eq!(model.sessions[0].workspace, "berlin");
        assert_eq!(model.sessions[0].process_status, "running");
        assert_eq!(model.sessions[0].parser_state, "2 events");
        assert_eq!(model.sessions[0].chunk_count, 3);
        assert_eq!(model.sessions[0].output_rate_label, "3 chunks");
        assert_eq!(model.selected.raw_chunks.len(), 3);
        assert_eq!(model.selected.normalized_text, "hello\nhello\npartial");
        assert_eq!(
            model
                .selected
                .events
                .iter()
                .map(|event| event.filter)
                .collect::<Vec<_>>(),
            vec![EventFilter::Assistant, EventFilter::Errors]
        );
        assert!(model.selected.diagnostics.command.contains("[redacted]"));
        assert!(!model.selected.diagnostics.command.contains("secret"));
        assert_eq!(model.selected.diagnostics.last_lifecycle_action, "running");
    }

    #[test]
    fn pluralize_keeps_single_event_label_singular() {
        assert_eq!(pluralize(1, "event"), "1 event");
        assert_eq!(pluralize(2, "event"), "2 events");
    }

    #[test]
    fn inspector_model_prefers_persisted_pty_chunks_over_formatted_log_text() {
        let session = InspectorSessionInput {
            id: 9,
            workspace: "oslo".to_owned(),
            command: "codex".to_owned(),
            log_path: PathBuf::from("/tmp/session-9.log"),
            pid: Some(2222),
            status: ProcessStatus::Running,
            started_at: "2026-07-06T12:00:00Z".to_owned(),
            ended_at: None,
            exit_code: None,
            raw_output: "[codex raw]\nformatted\n[/codex raw]\n".to_owned(),
            raw_chunks: vec![
                RawChunk {
                    index: 1,
                    text: "\u{1b}[2Jreal".to_owned(),
                    normalized_text: "\u{1b}[2Jreal".to_owned(),
                    raw_normalized_text: "\u{1b}[2Jreal".to_owned(),
                    duplicate: false,
                    delayed: false,
                    partial: true,
                },
                RawChunk {
                    index: 2,
                    text: " chunk\n".to_owned(),
                    normalized_text: " chunk\n".to_owned(),
                    raw_normalized_text: " chunk\n".to_owned(),
                    duplicate: false,
                    delayed: false,
                    partial: false,
                },
            ],
            events: Vec::new(),
            provider_events: Vec::new(),
        };

        let model = build_inspector_model(vec![session]);

        assert_eq!(model.sessions[0].chunk_count, 2);
        assert_eq!(model.selected.normalized_text, "\u{1b}[2Jreal chunk\n");
        assert_eq!(model.selected.raw_chunks[0].text, "\u{1b}[2Jreal");
        assert!(!model.selected.normalized_text.contains("[codex raw]"));
    }

    #[test]
    fn inspector_model_redacts_raw_chunks_normalized_text_and_events() {
        let session = InspectorSessionInput {
            id: 10,
            workspace: "milan".to_owned(),
            command: "codex".to_owned(),
            log_path: PathBuf::from("/tmp/session-10.log"),
            pid: Some(3333),
            status: ProcessStatus::Stopped,
            started_at: "2026-07-06T12:00:00Z".to_owned(),
            ended_at: Some("2026-07-06T12:01:00Z".to_owned()),
            exit_code: Some(0),
            raw_output: "TOKEN=raw-secret\n".to_owned(),
            raw_chunks: raw_chunks_from_log(
                "TOKEN=raw-secret\nAuthorization: Bearer bearer-secret\n",
            ),
            events: vec![SessionEvent::new(
                SessionEventSource::Runtime,
                Some("TOKEN=event-secret".to_owned()),
                SessionEventPayload::CommandOutput {
                    title: "print env".to_owned(),
                    output: "api_key: event-api-secret".to_owned(),
                    status: SessionCommandOutputStatus::Succeeded,
                },
            )
            .with_sequence(1)
            .with_occurred_at_ms(100)],
            provider_events: Vec::new(),
        };

        let model = build_inspector_model(vec![session]);
        let raw_text = model
            .selected
            .raw_chunks
            .iter()
            .map(|chunk| chunk.text.as_str())
            .collect::<String>();
        let rendered_events = model
            .selected
            .events
            .iter()
            .map(|event| event.rendered_text.as_str())
            .collect::<String>();

        for leaked in [
            "raw-secret",
            "bearer-secret",
            "event-secret",
            "event-api-secret",
        ] {
            assert!(
                !raw_text.contains(leaked)
                    && !model.selected.normalized_text.contains(leaked)
                    && !rendered_events.contains(leaked),
                "{leaked} leaked"
            );
        }
        assert!(raw_text.contains("TOKEN=[redacted]"));
        assert!(model.selected.normalized_text.contains("Bearer [redacted]"));
        assert!(rendered_events.contains("api_key: [redacted]"));
    }

    #[test]
    fn inspector_redacts_secrets_split_across_chunk_boundaries() {
        let chunks = vec![
            PtyChunkRecord {
                id: 1,
                process_id: 1,
                sequence: 1,
                occurred_at_ms: 10,
                stream: "stdout".to_owned(),
                text: "TOKEN=".to_owned(),
                created_at: "now".to_owned(),
            },
            PtyChunkRecord {
                id: 2,
                process_id: 1,
                sequence: 2,
                occurred_at_ms: 11,
                stream: "stdout".to_owned(),
                text: "split-secret\n".to_owned(),
                created_at: "now".to_owned(),
            },
        ];

        let rendered = raw_chunks_from_records(&chunks)
            .into_iter()
            .map(|chunk| chunk.text)
            .collect::<String>();

        assert_eq!(rendered, "TOKEN=[redacted]\n");
        assert!(!rendered.contains("split-secret"));
    }

    #[test]
    fn inspector_keeps_non_newline_raw_chunks_non_empty() {
        let chunks = vec![
            PtyChunkRecord {
                id: 1,
                process_id: 1,
                sequence: 1,
                occurred_at_ms: 10,
                stream: "stdout".to_owned(),
                text: "abc".to_owned(),
                created_at: "now".to_owned(),
            },
            PtyChunkRecord {
                id: 2,
                process_id: 1,
                sequence: 2,
                occurred_at_ms: 11,
                stream: "stdout".to_owned(),
                text: "def\n".to_owned(),
                created_at: "now".to_owned(),
            },
        ];

        let rendered = raw_chunks_from_records(&chunks);

        assert_eq!(rendered[0].text, "abc");
        assert_eq!(
            rendered
                .iter()
                .map(|chunk| chunk.text.as_str())
                .collect::<String>(),
            "abcdef\n"
        );
    }

    #[test]
    fn inspector_redacted_value_does_not_consume_later_visible_chunks() {
        let chunks = vec![
            PtyChunkRecord {
                id: 1,
                process_id: 1,
                sequence: 1,
                occurred_at_ms: 10,
                stream: "stdout".to_owned(),
                text: "TOKEN=very-long-secret-value".to_owned(),
                created_at: "now".to_owned(),
            },
            PtyChunkRecord {
                id: 2,
                process_id: 1,
                sequence: 2,
                occurred_at_ms: 11,
                stream: "stdout".to_owned(),
                text: "\nvisible\n".to_owned(),
                created_at: "now".to_owned(),
            },
        ];

        let rendered = raw_chunks_from_records(&chunks);

        assert_eq!(rendered[0].text, "TOKEN=[redacted]");
        assert_eq!(rendered[1].text, "\nvisible\n");
    }

    #[test]
    fn inspector_redacted_bracket_value_does_not_consume_later_visible_chunks() {
        let chunks = vec![
            PtyChunkRecord {
                id: 1,
                process_id: 1,
                sequence: 1,
                occurred_at_ms: 10,
                stream: "stdout".to_owned(),
                text: "TOKEN=[secret-value]".to_owned(),
                created_at: "now".to_owned(),
            },
            PtyChunkRecord {
                id: 2,
                process_id: 1,
                sequence: 2,
                occurred_at_ms: 11,
                stream: "stdout".to_owned(),
                text: "\nvisible\n".to_owned(),
                created_at: "now".to_owned(),
            },
        ];

        let rendered = raw_chunks_from_records(&chunks);

        assert_eq!(rendered[0].text, "TOKEN=[redacted]");
        assert_eq!(rendered[1].text, "\nvisible\n");
    }

    #[test]
    fn inspector_redacted_literal_prefix_value_does_not_consume_later_visible_chunks() {
        let chunks = vec![
            PtyChunkRecord {
                id: 1,
                process_id: 1,
                sequence: 1,
                occurred_at_ms: 10,
                stream: "stdout".to_owned(),
                text: "TOKEN=[redacted]suffix".to_owned(),
                created_at: "now".to_owned(),
            },
            PtyChunkRecord {
                id: 2,
                process_id: 1,
                sequence: 2,
                occurred_at_ms: 11,
                stream: "stdout".to_owned(),
                text: "\nvisible\n".to_owned(),
                created_at: "now".to_owned(),
            },
        ];

        let rendered = raw_chunks_from_records(&chunks);

        assert_eq!(rendered[0].text, "TOKEN=[redacted]");
        assert_eq!(rendered[1].text, "\nvisible\n");
    }

    #[test]
    fn inspector_preserves_literal_redacted_marker_split_across_chunks() {
        let chunks = vec![
            PtyChunkRecord {
                id: 1,
                process_id: 1,
                sequence: 1,
                occurred_at_ms: 10,
                stream: "stdout".to_owned(),
                text: "notice [red".to_owned(),
                created_at: "now".to_owned(),
            },
            PtyChunkRecord {
                id: 2,
                process_id: 1,
                sequence: 2,
                occurred_at_ms: 11,
                stream: "stdout".to_owned(),
                text: "acted]\n".to_owned(),
                created_at: "now".to_owned(),
            },
        ];

        let rendered = raw_chunks_from_records(&chunks);

        assert_eq!(rendered[0].text, "notice [red");
        assert_eq!(rendered[1].text, "acted]\n");
    }

    #[test]
    fn event_source_chunk_uses_raw_text_before_redacted_matching() {
        let chunks = raw_chunks_from_log("TOKEN=first-secret\nTOKEN=second-secret\n");
        let event = SessionEvent::new(
            SessionEventSource::Runtime,
            Some("TOKEN=second-secret".to_owned()),
            SessionEventPayload::CommandOutput {
                title: "env".to_owned(),
                output: "TOKEN=second-secret".to_owned(),
                status: SessionCommandOutputStatus::Succeeded,
            },
        );

        let row = event_row(&event, &chunks);

        assert_eq!(row.source_chunk, "chunk 2");
        assert!(!row.rendered_text.contains("second-secret"));
    }

    #[test]
    fn diagnostic_bundle_includes_redacted_debug_data() {
        let session = InspectorSessionInput {
            id: 11,
            workspace: "zurich".to_owned(),
            command: "TOKEN=command-secret codex".to_owned(),
            log_path: PathBuf::from("/tmp/session-11.log"),
            pid: Some(4444),
            status: ProcessStatus::Running,
            started_at: "2026-07-06T12:00:00Z".to_owned(),
            ended_at: None,
            exit_code: None,
            raw_output: String::new(),
            raw_chunks: raw_chunks_from_log("TOKEN=raw-secret\n"),
            events: vec![
                SessionEvent::new(
                    SessionEventSource::Runtime,
                    Some("running".to_owned()),
                    SessionEventPayload::StatusChange {
                        status: SessionEventStatus::Running,
                        message: Some("session running".to_owned()),
                    },
                )
                .with_sequence(1)
                .with_occurred_at_ms(100),
                SessionEvent::new(
                    SessionEventSource::Runtime,
                    Some("api_key=event-secret".to_owned()),
                    SessionEventPayload::CommandOutput {
                        title: "env".to_owned(),
                        output: "api_key=event-secret".to_owned(),
                        status: SessionCommandOutputStatus::Succeeded,
                    },
                )
                .with_sequence(2)
                .with_occurred_at_ms(120),
            ],
            provider_events: Vec::new(),
        };
        let model = build_inspector_model(vec![session]);

        let bundle = diagnostic_bundle_text(&model.selected);

        assert!(bundle.contains("Archductor session log diagnostic bundle"));
        assert!(bundle.contains("App version:"));
        assert!(bundle.contains("Session: #11"));
        assert!(bundle.contains("Raw log path: /tmp/session-11.log"));
        assert!(bundle.contains("Process metadata"));
        assert!(bundle.contains("State transitions"));
        assert!(bundle.contains("Parsed events"));
        assert!(bundle.contains("Raw output (redacted)"));
        assert!(bundle.contains("TOKEN=[redacted]"));
        for leaked in ["command-secret", "raw-secret", "event-secret"] {
            assert!(!bundle.contains(leaked), "{leaked} leaked in bundle");
        }
    }

    #[test]
    fn provider_event_rows_use_projected_count_timestamp_and_failed_filter() {
        let completed = provider_record(
            1,
            ProviderEventKind::Tool,
            ProviderEventPhase::Completed,
            "tool_call",
            "Tool",
            "ok",
        );
        let mut failed = provider_record(
            2,
            ProviderEventKind::Tool,
            ProviderEventPhase::Failed,
            "tool_call",
            "Tool",
            "boom",
        );
        failed.provider_item_id = Some("item-2".to_owned());
        failed.identity_key = "codex:thread-1:item-2:failed".to_owned();
        let session = InspectorSessionInput {
            id: 12,
            workspace: "rome".to_owned(),
            command: "codex".to_owned(),
            log_path: PathBuf::from("/tmp/session-12.log"),
            pid: Some(5555),
            status: ProcessStatus::Running,
            started_at: "2026-07-06T12:00:00Z".to_owned(),
            ended_at: None,
            exit_code: None,
            raw_output: String::new(),
            raw_chunks: Vec::new(),
            events: Vec::new(),
            provider_events: vec![completed, failed],
        };

        let model = build_inspector_model(vec![session]);

        assert_eq!(model.sessions[0].parser_state, "2 events");
        assert_eq!(
            model
                .selected
                .events
                .iter()
                .map(|event| {
                    (
                        event.sequence.as_str(),
                        event.timestamp.as_str(),
                        event.filter,
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                ("1", "1001", EventFilter::ToolOutput),
                ("2", "1002", EventFilter::Errors),
            ]
        );
    }

    #[test]
    fn event_rows_filter_by_selected_categories() {
        let events = vec![
            InspectorEventRow {
                sequence: "1".to_owned(),
                timestamp: "100".to_owned(),
                source_chunk: "chunk 1".to_owned(),
                filter: EventFilter::Assistant,
                status: "assistant".to_owned(),
                rendered_text: "hello".to_owned(),
            },
            InspectorEventRow {
                sequence: "2".to_owned(),
                timestamp: "120".to_owned(),
                source_chunk: "chunk 2".to_owned(),
                filter: EventFilter::Errors,
                status: "error".to_owned(),
                rendered_text: "failed".to_owned(),
            },
        ];

        let visible = filter_event_rows(&events, &[EventFilter::Errors]);

        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].rendered_text, "failed");
    }

    #[test]
    fn command_redaction_handles_env_bearer_and_flag_style_secrets() {
        let redacted = redact_sensitive_text(
            "OPENAI_API_KEY=sk-openai ANTHROPIC_API_KEY=sk-ant \
             TOKEN=abc API_KEY=def password=hunter2 secret=sauce \
             curl -H 'Authorization: Bearer bearer-secret' \
             codex --token cli-token --password=cli-pass --api-key cli-key",
        );

        for leaked in [
            "sk-openai",
            "sk-ant",
            "abc",
            "def",
            "hunter2",
            "sauce",
            "bearer-secret",
            "cli-token",
            "cli-pass",
            "cli-key",
        ] {
            assert!(!redacted.contains(leaked), "{leaked} leaked in {redacted}");
        }
        assert!(redacted.contains("OPENAI_API_KEY=[redacted]"));
        assert!(redacted.contains("ANTHROPIC_API_KEY=[redacted]"));
        assert!(redacted.contains("--token [redacted]"));
        assert!(redacted.contains("--password=[redacted]"));
    }

    fn provider_record(
        sequence: i64,
        kind: ProviderEventKind,
        phase: ProviderEventPhase,
        subtype: &str,
        title: &str,
        body: &str,
    ) -> ProviderEventRecord {
        ProviderEventRecord {
            id: sequence,
            identity_key: format!("codex:thread-1:item-{sequence}:{}", phase.as_str()),
            provider: "codex".to_owned(),
            provider_event_id: Some(format!("evt-{sequence}")),
            provider_item_id: Some(format!("item-{sequence}")),
            provider_thread_id: Some("thread-1".to_owned()),
            provider_turn_id: Some("turn-1".to_owned()),
            parent_provider_item_id: None,
            parent_provider_thread_id: None,
            workspace_id: Some(1),
            chat_thread_id: Some(7),
            process_id: Some(12),
            phase,
            kind,
            provider_subtype: Some(subtype.to_owned()),
            provider_sequence: Some(sequence),
            received_sequence: sequence,
            occurred_at_ms: 1000 + sequence as u64,
            normalized_payload: json!({"title": title, "body": body}),
            raw_json: json!({"sequence": sequence}),
            schema_version: 1,
            adapter_version: "test".to_owned(),
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
        }
    }
}
