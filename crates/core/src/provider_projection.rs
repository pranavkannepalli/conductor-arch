use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use serde_json::Value;

use crate::provider_events::{ProviderEventKind, ProviderEventPhase, ProviderEventRecord};
use crate::redaction::redact_sensitive_text;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderProjectionCategory {
    UserMessage,
    AssistantMessage,
    Plan,
    Reasoning,
    Command,
    // Forward-compatible categories kept so new provider subtypes can map to
    // stable render classes without reshaping the public projection model.
    Process,
    FileRead,
    FileWrite,
    FilePatch,
    FileDiff,
    McpTool,
    NativeTool,
    Skill,
    Plugin,
    Hook,
    Subagent,
    NestedTranscript,
    BackgroundTerminal,
    BackgroundTask,
    Approval,
    Question,
    Web,
    Image,
    Usage,
    Cost,
    Context,
    RateLimit,
    Error,
    Status,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderProjectionStatus {
    Pending,
    Running,
    Complete,
    Failed,
    Canceled,
}

impl ProviderProjectionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Complete => "complete",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderProjectionStreamState {
    Snapshot,
    Streaming,
    Complete,
}

impl ProviderProjectionStreamState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Snapshot => "snapshot",
            Self::Streaming => "streaming",
            Self::Complete => "complete",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectionRenderClass {
    UserChat,
    AssistantChat,
    PlanCard,
    ReasoningCard,
    CommandCard,
    ProcessCard,
    FileCard,
    DiffCard,
    ToolCard,
    SkillCard,
    PluginCard,
    HookCard,
    SubagentCard,
    NestedTranscriptCard,
    BackgroundCard,
    PromptCard,
    WebCard,
    ImageCard,
    UsageCard,
    WarningCard,
    ErrorCard,
    StatusCard,
    FallbackCard,
}

impl ProjectionRenderClass {
    pub fn role_label(self) -> &'static str {
        match self {
            Self::UserChat => "user",
            Self::AssistantChat => "assistant",
            Self::PlanCard => "plan",
            Self::ReasoningCard => "reasoning",
            Self::CommandCard => "command",
            Self::ProcessCard => "process",
            Self::FileCard => "file",
            Self::DiffCard => "diff",
            Self::ToolCard => "tool",
            Self::SkillCard => "skill",
            Self::PluginCard => "plugin",
            Self::HookCard => "hook",
            Self::SubagentCard => "subagent",
            Self::NestedTranscriptCard => "nested",
            Self::BackgroundCard => "background",
            Self::PromptCard => "prompt",
            Self::WebCard => "web",
            Self::ImageCard => "image",
            Self::UsageCard => "usage",
            Self::WarningCard => "warning",
            Self::ErrorCard => "error",
            Self::StatusCard => "status",
            Self::FallbackCard => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderProjectionEvent {
    pub canonical_id: String,
    pub sequence: u64,
    pub category: ProviderProjectionCategory,
    pub title: String,
    pub body: String,
    pub status: ProviderProjectionStatus,
    pub stream_state: ProviderProjectionStreamState,
    pub parent_id: Option<String>,
    pub nested_thread_id: Option<String>,
    pub raw_payload: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderProjectionItem {
    pub id: String,
    pub sequence: u64,
    pub category: ProviderProjectionCategory,
    pub render_class: ProjectionRenderClass,
    pub title: String,
    pub body: String,
    pub status: ProviderProjectionStatus,
    pub stream_state: ProviderProjectionStreamState,
    pub parent_id: Option<String>,
    pub nested_thread_id: Option<String>,
    pub raw_payload: Option<String>,
    pub inspectable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderProjection {
    pub items: Vec<ProviderProjectionItem>,
    pub signature: ProviderProjectionSignature,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderProjectionSignature {
    pub item_ids: Vec<String>,
    pub items: Vec<ProviderProjectionItemSignature>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderProjectionItemSignature {
    pub id: String,
    pub category: ProviderProjectionCategory,
    pub render_class: ProjectionRenderClass,
    pub status: ProviderProjectionStatus,
    pub stream_state: ProviderProjectionStreamState,
    pub parent_id: Option<String>,
    pub nested_thread_id: Option<String>,
    pub content_hash: u64,
}

pub fn provider_projection_from_records(records: &[ProviderEventRecord]) -> ProviderProjection {
    render_provider_event_projection(
        records
            .iter()
            .map(provider_projection_event_from_record)
            .collect(),
    )
}

pub fn render_provider_event_projection(
    events: Vec<ProviderProjectionEvent>,
) -> ProviderProjection {
    let mut events = events;
    events.sort_by(|left, right| {
        left.sequence
            .cmp(&right.sequence)
            .then_with(|| left.canonical_id.cmp(&right.canonical_id))
    });

    let mut items = Vec::<ProviderProjectionItem>::new();
    let mut positions = HashMap::<String, usize>::new();

    for event in events {
        let id = canonical_projection_id(&event);
        let mut item = projection_item_from_event(id.clone(), event);
        if let Some(index) = positions.get(&id).copied() {
            if item.body.trim().is_empty() && !items[index].body.trim().is_empty() {
                item.body = items[index].body.clone();
            }
            items[index] = ProviderProjectionItem {
                sequence: items[index].sequence,
                ..item
            };
        } else {
            positions.insert(id, items.len());
            items.push(item);
        }
    }

    items.sort_by(|left, right| {
        left.sequence
            .cmp(&right.sequence)
            .then_with(|| left.id.cmp(&right.id))
    });
    let signature = projection_signature(&items);

    ProviderProjection { items, signature }
}

pub fn provider_projection_item_is_relevant_chat_event(item: &ProviderProjectionItem) -> bool {
    match item.render_class {
        ProjectionRenderClass::FallbackCard => false,
        ProjectionRenderClass::StatusCard => matches!(
            item.status,
            ProviderProjectionStatus::Failed | ProviderProjectionStatus::Canceled
        ),
        _ => true,
    }
}

pub fn provider_projection_item_is_pending_interaction_event(
    item: &ProviderProjectionItem,
) -> bool {
    matches!(
        item.category,
        ProviderProjectionCategory::Approval | ProviderProjectionCategory::Question
    ) && matches!(
        item.status,
        ProviderProjectionStatus::Pending | ProviderProjectionStatus::Running
    )
}

pub fn provider_projection_item_text(item: &ProviderProjectionItem) -> String {
    match item.render_class {
        ProjectionRenderClass::UserChat | ProjectionRenderClass::AssistantChat => item.body.clone(),
        _ => {
            let mut text = item.title.clone();
            if !item.body.trim().is_empty() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&item.body);
            }
            text
        }
    }
}

fn provider_projection_event_from_record(record: &ProviderEventRecord) -> ProviderProjectionEvent {
    let category = provider_projection_category(record.kind, record.provider_subtype.as_deref());
    let raw_payload =
        provider_projection_category_uses_raw_payload(category).then(|| record.raw_json.clone());
    ProviderProjectionEvent {
        canonical_id: provider_projection_canonical_id(record),
        sequence: record.received_sequence.max(0) as u64,
        category,
        title: record
            .normalized_payload
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        body: record
            .normalized_payload
            .get("body")
            .or_else(|| record.normalized_payload.get("text"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        status: provider_projection_status(record.phase),
        stream_state: provider_projection_stream_state(record.phase),
        parent_id: provider_projection_parent_id(record),
        nested_thread_id: record.parent_provider_thread_id.clone(),
        raw_payload,
    }
}

fn provider_projection_canonical_id(record: &ProviderEventRecord) -> String {
    let thread_id = record
        .provider_thread_id
        .as_deref()
        .or(record.parent_provider_thread_id.as_deref())
        .unwrap_or("-");
    if let Some(item_id) = record.provider_item_id.as_deref() {
        return format!("{}:{thread_id}:{item_id}", record.provider);
    }
    if let Some(event_id) = record.provider_event_id.as_deref() {
        return format!("{}:{thread_id}:event:{event_id}", record.provider);
    }
    record.identity_key.clone()
}

fn provider_projection_parent_id(record: &ProviderEventRecord) -> Option<String> {
    record.parent_provider_item_id.as_ref().map(|parent| {
        let thread_id = record
            .parent_provider_thread_id
            .as_deref()
            .or(record.provider_thread_id.as_deref())
            .unwrap_or("-");
        format!("{}:{thread_id}:{parent}", record.provider)
    })
}

fn provider_projection_category(
    kind: ProviderEventKind,
    provider_subtype: Option<&str>,
) -> ProviderProjectionCategory {
    let subtype = provider_subtype
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match kind {
        ProviderEventKind::UserInput => ProviderProjectionCategory::UserMessage,
        ProviderEventKind::AssistantOutput => ProviderProjectionCategory::AssistantMessage,
        ProviderEventKind::PlanningReasoning if subtype.contains("plan") => {
            ProviderProjectionCategory::Plan
        }
        ProviderEventKind::PlanningReasoning => ProviderProjectionCategory::Reasoning,
        ProviderEventKind::CommandProcess => ProviderProjectionCategory::Command,
        ProviderEventKind::TerminalRuntime => ProviderProjectionCategory::BackgroundTerminal,
        ProviderEventKind::FileSystem if subtype_contains_any(&subtype, &["write", "create"]) => {
            ProviderProjectionCategory::FileWrite
        }
        ProviderEventKind::FileSystem if subtype_contains_any(&subtype, &["patch", "edit"]) => {
            ProviderProjectionCategory::FilePatch
        }
        ProviderEventKind::FileSystem => ProviderProjectionCategory::FileRead,
        ProviderEventKind::DiffFileChange => ProviderProjectionCategory::FileDiff,
        ProviderEventKind::Tool => ProviderProjectionCategory::NativeTool,
        ProviderEventKind::Mcp => ProviderProjectionCategory::McpTool,
        ProviderEventKind::SkillPluginHook if subtype.contains("plugin") => {
            ProviderProjectionCategory::Plugin
        }
        ProviderEventKind::SkillPluginHook if subtype.contains("hook") => {
            ProviderProjectionCategory::Hook
        }
        ProviderEventKind::SkillPluginHook => ProviderProjectionCategory::Skill,
        ProviderEventKind::ApprovalPermission => ProviderProjectionCategory::Approval,
        ProviderEventKind::SubagentCollaboration => ProviderProjectionCategory::Subagent,
        ProviderEventKind::WebBrowserMedia
            if subtype_contains_any(&subtype, &["image", "media"]) =>
        {
            ProviderProjectionCategory::Image
        }
        ProviderEventKind::WebBrowserMedia => ProviderProjectionCategory::Web,
        ProviderEventKind::LimitFailure if subtype_contains_any(&subtype, &["rate", "limit"]) => {
            ProviderProjectionCategory::RateLimit
        }
        ProviderEventKind::LimitFailure if subtype.contains("usage") => {
            ProviderProjectionCategory::Usage
        }
        ProviderEventKind::LimitFailure => ProviderProjectionCategory::Error,
        ProviderEventKind::ThreadSession
        | ProviderEventKind::GoalTask
        | ProviderEventKind::Turn
        | ProviderEventKind::AccountAuth
        | ProviderEventKind::EnvironmentConfigModel => ProviderProjectionCategory::Status,
        ProviderEventKind::Unknown => ProviderProjectionCategory::Unknown,
    }
}

fn subtype_contains_any(subtype: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| subtype.contains(needle))
}

fn provider_projection_category_uses_raw_payload(category: ProviderProjectionCategory) -> bool {
    matches!(
        category,
        ProviderProjectionCategory::Command
            | ProviderProjectionCategory::Process
            | ProviderProjectionCategory::FileRead
            | ProviderProjectionCategory::FileWrite
            | ProviderProjectionCategory::FilePatch
            | ProviderProjectionCategory::FileDiff
            | ProviderProjectionCategory::McpTool
            | ProviderProjectionCategory::NativeTool
            | ProviderProjectionCategory::Skill
            | ProviderProjectionCategory::Plugin
            | ProviderProjectionCategory::Hook
            | ProviderProjectionCategory::Subagent
            | ProviderProjectionCategory::NestedTranscript
            | ProviderProjectionCategory::BackgroundTerminal
            | ProviderProjectionCategory::BackgroundTask
            | ProviderProjectionCategory::Approval
            | ProviderProjectionCategory::Question
            | ProviderProjectionCategory::Web
            | ProviderProjectionCategory::Image
            | ProviderProjectionCategory::RateLimit
            | ProviderProjectionCategory::Error
            | ProviderProjectionCategory::Unknown
    )
}

fn provider_projection_status(phase: ProviderEventPhase) -> ProviderProjectionStatus {
    match phase {
        ProviderEventPhase::Started | ProviderEventPhase::Progress | ProviderEventPhase::Delta => {
            ProviderProjectionStatus::Running
        }
        ProviderEventPhase::Completed => ProviderProjectionStatus::Complete,
        ProviderEventPhase::Failed => ProviderProjectionStatus::Failed,
        ProviderEventPhase::Declined | ProviderEventPhase::Interrupted => {
            ProviderProjectionStatus::Canceled
        }
        ProviderEventPhase::Unknown => ProviderProjectionStatus::Pending,
    }
}

fn provider_projection_stream_state(phase: ProviderEventPhase) -> ProviderProjectionStreamState {
    match phase {
        ProviderEventPhase::Started | ProviderEventPhase::Progress | ProviderEventPhase::Delta => {
            ProviderProjectionStreamState::Streaming
        }
        ProviderEventPhase::Completed
        | ProviderEventPhase::Failed
        | ProviderEventPhase::Declined
        | ProviderEventPhase::Interrupted => ProviderProjectionStreamState::Complete,
        ProviderEventPhase::Unknown => ProviderProjectionStreamState::Snapshot,
    }
}

fn canonical_projection_id(event: &ProviderProjectionEvent) -> String {
    let id = event.canonical_id.trim();
    if id.is_empty() {
        format!("missing-canonical-id-{}", event.sequence)
    } else {
        id.to_owned()
    }
}

fn projection_item_from_event(
    id: String,
    event: ProviderProjectionEvent,
) -> ProviderProjectionItem {
    let render_class = render_class_for_category(event.category);
    let title = projection_title(event.category, &event.title);
    let raw_payload = event
        .raw_payload
        .as_ref()
        .map(redacted_payload_display)
        .filter(|payload| !payload.trim().is_empty());
    let inspectable = raw_payload.is_some()
        || matches!(
            render_class,
            ProjectionRenderClass::FallbackCard
                | ProjectionRenderClass::ToolCard
                | ProjectionRenderClass::CommandCard
                | ProjectionRenderClass::FileCard
                | ProjectionRenderClass::DiffCard
                | ProjectionRenderClass::SubagentCard
                | ProjectionRenderClass::NestedTranscriptCard
        );

    ProviderProjectionItem {
        id,
        sequence: event.sequence,
        category: event.category,
        render_class,
        title,
        body: event.body,
        status: event.status,
        stream_state: event.stream_state,
        parent_id: event.parent_id,
        nested_thread_id: event.nested_thread_id,
        raw_payload,
        inspectable,
    }
}

fn render_class_for_category(category: ProviderProjectionCategory) -> ProjectionRenderClass {
    match category {
        ProviderProjectionCategory::UserMessage => ProjectionRenderClass::UserChat,
        ProviderProjectionCategory::AssistantMessage => ProjectionRenderClass::AssistantChat,
        ProviderProjectionCategory::Plan => ProjectionRenderClass::PlanCard,
        ProviderProjectionCategory::Reasoning => ProjectionRenderClass::ReasoningCard,
        ProviderProjectionCategory::Command => ProjectionRenderClass::CommandCard,
        ProviderProjectionCategory::Process => ProjectionRenderClass::ProcessCard,
        ProviderProjectionCategory::FileRead
        | ProviderProjectionCategory::FileWrite
        | ProviderProjectionCategory::FilePatch => ProjectionRenderClass::FileCard,
        ProviderProjectionCategory::FileDiff => ProjectionRenderClass::DiffCard,
        ProviderProjectionCategory::McpTool | ProviderProjectionCategory::NativeTool => {
            ProjectionRenderClass::ToolCard
        }
        ProviderProjectionCategory::Skill => ProjectionRenderClass::SkillCard,
        ProviderProjectionCategory::Plugin => ProjectionRenderClass::PluginCard,
        ProviderProjectionCategory::Hook => ProjectionRenderClass::HookCard,
        ProviderProjectionCategory::Subagent => ProjectionRenderClass::SubagentCard,
        ProviderProjectionCategory::NestedTranscript => ProjectionRenderClass::NestedTranscriptCard,
        ProviderProjectionCategory::BackgroundTerminal
        | ProviderProjectionCategory::BackgroundTask => ProjectionRenderClass::BackgroundCard,
        ProviderProjectionCategory::Approval | ProviderProjectionCategory::Question => {
            ProjectionRenderClass::PromptCard
        }
        ProviderProjectionCategory::Web => ProjectionRenderClass::WebCard,
        ProviderProjectionCategory::Image => ProjectionRenderClass::ImageCard,
        ProviderProjectionCategory::Usage
        | ProviderProjectionCategory::Cost
        | ProviderProjectionCategory::Context => ProjectionRenderClass::UsageCard,
        ProviderProjectionCategory::RateLimit => ProjectionRenderClass::WarningCard,
        ProviderProjectionCategory::Error => ProjectionRenderClass::ErrorCard,
        ProviderProjectionCategory::Status => ProjectionRenderClass::StatusCard,
        ProviderProjectionCategory::Unknown => ProjectionRenderClass::FallbackCard,
    }
}

fn projection_title(category: ProviderProjectionCategory, title: &str) -> String {
    let title = title.trim();
    if !title.is_empty() {
        return title.to_owned();
    }

    match category {
        ProviderProjectionCategory::UserMessage => "User".to_owned(),
        ProviderProjectionCategory::AssistantMessage => "Assistant".to_owned(),
        ProviderProjectionCategory::Plan => "Plan".to_owned(),
        ProviderProjectionCategory::Reasoning => "Reasoning".to_owned(),
        ProviderProjectionCategory::Command => "Command".to_owned(),
        ProviderProjectionCategory::Process => "Process".to_owned(),
        ProviderProjectionCategory::FileRead => "File read".to_owned(),
        ProviderProjectionCategory::FileWrite => "File write".to_owned(),
        ProviderProjectionCategory::FilePatch => "Patch".to_owned(),
        ProviderProjectionCategory::FileDiff => "Diff".to_owned(),
        ProviderProjectionCategory::McpTool => "MCP tool".to_owned(),
        ProviderProjectionCategory::NativeTool => "Native tool".to_owned(),
        ProviderProjectionCategory::Skill => "Skill".to_owned(),
        ProviderProjectionCategory::Plugin => "Plugin".to_owned(),
        ProviderProjectionCategory::Hook => "Hook".to_owned(),
        ProviderProjectionCategory::Subagent => "Subagent".to_owned(),
        ProviderProjectionCategory::NestedTranscript => "Nested transcript".to_owned(),
        ProviderProjectionCategory::BackgroundTerminal => "Background terminal".to_owned(),
        ProviderProjectionCategory::BackgroundTask => "Background task".to_owned(),
        ProviderProjectionCategory::Approval => "Approval".to_owned(),
        ProviderProjectionCategory::Question => "Question".to_owned(),
        ProviderProjectionCategory::Web => "Web".to_owned(),
        ProviderProjectionCategory::Image => "Image".to_owned(),
        ProviderProjectionCategory::Usage => "Usage".to_owned(),
        ProviderProjectionCategory::Cost => "Cost".to_owned(),
        ProviderProjectionCategory::Context => "Context".to_owned(),
        ProviderProjectionCategory::RateLimit => "Rate limit".to_owned(),
        ProviderProjectionCategory::Error => "Error".to_owned(),
        ProviderProjectionCategory::Status => "Status".to_owned(),
        ProviderProjectionCategory::Unknown => "Unknown provider event".to_owned(),
    }
}

fn projection_signature(items: &[ProviderProjectionItem]) -> ProviderProjectionSignature {
    ProviderProjectionSignature {
        item_ids: items.iter().map(|item| item.id.clone()).collect(),
        items: items
            .iter()
            .map(|item| ProviderProjectionItemSignature {
                id: item.id.clone(),
                category: item.category,
                render_class: item.render_class,
                status: item.status,
                stream_state: item.stream_state,
                parent_id: item.parent_id.clone(),
                nested_thread_id: item.nested_thread_id.clone(),
                content_hash: projection_item_content_hash(item),
            })
            .collect(),
    }
}

fn projection_item_content_hash(item: &ProviderProjectionItem) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    item.title.hash(&mut hasher);
    item.body.hash(&mut hasher);
    item.raw_payload.hash(&mut hasher);
    hasher.finish()
}

fn redacted_payload_display(payload: &Value) -> String {
    serde_json::to_string_pretty(&redact_json_value(payload)).unwrap_or_default()
}

fn redact_json_value(value: &Value) -> Value {
    match value {
        Value::Object(entries) => Value::Object(
            entries
                .iter()
                .map(|(key, value)| {
                    let redacted = if secret_like_key(key) {
                        Value::String("[redacted]".to_owned())
                    } else {
                        redact_json_value(value)
                    };
                    (key.clone(), redacted)
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(redact_json_value).collect()),
        Value::String(value) => Value::String(redact_sensitive_text(value)),
        other => other.clone(),
    }
}

fn secret_like_key(key: &str) -> bool {
    let key = key.trim().replace(['-', '.'], "_").to_ascii_lowercase();
    key.contains("token")
        || key.contains("secret")
        || key.contains("password")
        || key.contains("api_key")
        || key.contains("apikey")
        || key.contains("access_key")
        || key.contains("private_key")
        || key.contains("credential")
        || key == "auth"
        || key.ends_with("_auth")
        || matches!(
            key.as_str(),
            "authorization"
                | "proxy_authorization"
                | "www_authorization"
                | "authorization_header"
                | "auth_header"
                | "bearer"
        )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::provider_adapters::claude_stream::parse_claude_stream_json_lines;
    use crate::provider_events::{ProviderEventKind, ProviderEventPhase};
    use serde_json::json;

    fn record(
        kind: ProviderEventKind,
        phase: ProviderEventPhase,
        subtype: &str,
    ) -> ProviderEventRecord {
        ProviderEventRecord {
            id: 1,
            identity_key: "codex:thread-1:item-1".to_owned(),
            provider: "codex".to_owned(),
            provider_event_id: None,
            provider_item_id: Some("item-1".to_owned()),
            provider_thread_id: Some("thread-1".to_owned()),
            provider_turn_id: None,
            parent_provider_item_id: None,
            parent_provider_thread_id: None,
            workspace_id: None,
            chat_thread_id: Some(7),
            process_id: Some(9),
            phase,
            kind,
            provider_subtype: Some(subtype.to_owned()),
            provider_sequence: Some(1),
            received_sequence: 1,
            occurred_at_ms: 1,
            normalized_payload: json!({
                "title": "Event",
                "body": "body"
            }),
            raw_json: json!({ "id": 1 }),
            schema_version: 1,
            adapter_version: "test".to_owned(),
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
        }
    }

    #[test]
    fn completed_empty_update_preserves_previous_streaming_body() {
        let mut streaming = record(
            ProviderEventKind::AssistantOutput,
            ProviderEventPhase::Delta,
            "assistant",
        );
        streaming.normalized_payload = json!({
            "title": "Assistant",
            "body": "streamed answer"
        });
        let mut completed = streaming.clone();
        completed.identity_key = "codex:thread-1:item-1:complete".to_owned();
        completed.phase = ProviderEventPhase::Completed;
        completed.received_sequence = 2;
        completed.normalized_payload = json!({
            "title": "Assistant",
            "body": ""
        });

        let projection = provider_projection_from_records(&[streaming, completed]);

        assert_eq!(projection.items.len(), 1);
        assert_eq!(projection.items[0].body, "streamed answer");
        assert_eq!(
            projection.items[0].status,
            ProviderProjectionStatus::Complete
        );
    }

    #[test]
    fn pending_interaction_events_are_detectable_for_dedicated_cards() {
        let pending = record(
            ProviderEventKind::ApprovalPermission,
            ProviderEventPhase::Started,
            "permission_request",
        );
        let mut completed = record(
            ProviderEventKind::ApprovalPermission,
            ProviderEventPhase::Completed,
            "permission_result",
        );
        completed.identity_key = "codex:thread-1:item-2".to_owned();
        completed.provider_item_id = Some("item-2".to_owned());

        let projection = provider_projection_from_records(&[pending, completed]);
        let pending_item = projection
            .items
            .iter()
            .find(|item| item.status == ProviderProjectionStatus::Running)
            .unwrap();
        let completed_item = projection
            .items
            .iter()
            .find(|item| item.status == ProviderProjectionStatus::Complete)
            .unwrap();

        assert!(provider_projection_item_is_pending_interaction_event(
            pending_item
        ));
        assert!(!provider_projection_item_is_pending_interaction_event(
            completed_item
        ));
    }

    #[test]
    fn claude_projection_combines_streaming_deltas_and_final_assistant_body() {
        let native = r#"{"type":"stream_event","session_id":"claude-session","event":{"type":"message_start","message":{"id":"claude-message","role":"assistant","content":[]}}}
{"type":"stream_event","session_id":"claude-session","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}
{"type":"stream_event","session_id":"claude-session","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Complete "}}}
{"type":"stream_event","session_id":"claude-session","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"answer"}}}
{"type":"assistant","session_id":"claude-session","message":{"id":"claude-message","role":"assistant","content":[{"type":"text","text":"Complete answer"}]}}"#;
        let temp = tempfile::tempdir().unwrap();
        let store = crate::provider_events::ProviderEventStore::new(temp.path().join("state.db"));
        let mut records = Vec::new();

        for (sequence, event) in parse_claude_stream_json_lines(native)
            .unwrap()
            .into_iter()
            .enumerate()
        {
            let mut draft =
                event.into_provider_event_draft(crate::provider_events::ProviderEventContext {
                    workspace_id: None,
                    chat_thread_id: None,
                    process_id: None,
                    occurred_at_ms: sequence as u64,
                    schema_version: 1,
                    adapter_version: "claude-projection-test".to_owned(),
                });
            draft.provider_sequence = Some(sequence as i64);
            records.push(store.upsert_event(&draft).unwrap());
        }

        let projection = provider_projection_from_records(&records);
        let assistant = projection
            .items
            .iter()
            .filter(|item| item.category == ProviderProjectionCategory::AssistantMessage)
            .collect::<Vec<_>>();

        assert_eq!(assistant.len(), 1);
        assert_eq!(assistant[0].body, "Complete answer");
        assert_eq!(assistant[0].status, ProviderProjectionStatus::Complete);
    }

    #[test]
    fn claude_projection_full_fixture_keeps_non_message_events_out_of_assistant_identity() {
        let native = include_str!("../tests/fixtures/claude_stream/basic_turn.jsonl");
        let temp = tempfile::tempdir().unwrap();
        let store = crate::provider_events::ProviderEventStore::new(temp.path().join("state.db"));
        let mut latest = BTreeMap::new();

        for (sequence, event) in parse_claude_stream_json_lines(native)
            .unwrap()
            .into_iter()
            .enumerate()
        {
            let mut draft =
                event.into_provider_event_draft(crate::provider_events::ProviderEventContext {
                    workspace_id: None,
                    chat_thread_id: None,
                    process_id: None,
                    occurred_at_ms: sequence as u64,
                    schema_version: 1,
                    adapter_version: "claude-projection-test".to_owned(),
                });
            draft.provider_sequence = Some(sequence as i64);
            let record = store.upsert_event(&draft).unwrap();
            latest.insert(record.identity_key.clone(), record);
        }

        let projection = provider_projection_from_records(
            &latest.into_values().collect::<Vec<ProviderEventRecord>>(),
        );
        let assistant = projection
            .items
            .iter()
            .find(|item| item.category == ProviderProjectionCategory::AssistantMessage)
            .unwrap();

        assert_eq!(assistant.body, "Fixture complete.");
        assert_eq!(assistant.status, ProviderProjectionStatus::Complete);
        assert!(projection
            .items
            .iter()
            .any(|item| item.category == ProviderProjectionCategory::Status
                && item.id != assistant.id));
        assert!(projection
            .items
            .iter()
            .any(|item| item.category == ProviderProjectionCategory::Unknown
                && item.id != assistant.id));
    }

    #[test]
    fn claude_projection_authoritative_final_repairs_partial_delta_before_empty_stops() {
        let native = r#"{"type":"stream_event","session_id":"claude-session","event":{"type":"message_start","message":{"id":"claude-message","role":"assistant","content":[]}}}
{"type":"stream_event","session_id":"claude-session","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}
{"type":"stream_event","session_id":"claude-session","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Complete "}}}
{"type":"assistant","session_id":"claude-session","message":{"id":"claude-message","role":"assistant","content":[{"type":"text","text":"Complete answer"}]}}
{"type":"stream_event","session_id":"claude-session","event":{"type":"content_block_stop","index":0}}
{"type":"stream_event","session_id":"claude-session","event":{"type":"message_stop"}}"#;
        let temp = tempfile::tempdir().unwrap();
        let store = crate::provider_events::ProviderEventStore::new(temp.path().join("state.db"));
        let mut latest = BTreeMap::new();

        for (sequence, event) in parse_claude_stream_json_lines(native)
            .unwrap()
            .into_iter()
            .enumerate()
        {
            let draft =
                event.into_provider_event_draft(crate::provider_events::ProviderEventContext {
                    workspace_id: None,
                    chat_thread_id: None,
                    process_id: None,
                    occurred_at_ms: sequence as u64,
                    schema_version: 1,
                    adapter_version: "claude-projection-test".to_owned(),
                });
            let record = store.upsert_event(&draft).unwrap();
            latest.insert(record.identity_key.clone(), record);
        }

        let projection = provider_projection_from_records(
            &latest.into_values().collect::<Vec<ProviderEventRecord>>(),
        );
        let assistant = projection
            .items
            .iter()
            .filter(|item| item.category == ProviderProjectionCategory::AssistantMessage)
            .collect::<Vec<_>>();

        assert_eq!(assistant.len(), 1);
        assert_eq!(assistant[0].body, "Complete answer");
        assert_eq!(assistant[0].status, ProviderProjectionStatus::Complete);
    }

    #[test]
    fn claude_projection_mixed_final_snapshot_keeps_assistant_and_reasoning_rows_distinct() {
        let native = r#"{"type":"assistant","session_id":"claude-session","uuid":"mixed-final-1","message":{"id":"claude-message","role":"assistant","content":[{"type":"thinking","thinking":"Private reasoning"},{"type":"text","text":"Public answer"}]}}"#;
        let temp = tempfile::tempdir().unwrap();
        let store = crate::provider_events::ProviderEventStore::new(temp.path().join("state.db"));
        let event = parse_claude_stream_json_lines(native)
            .unwrap()
            .pop()
            .unwrap();
        let records = event
            .into_provider_event_drafts(crate::provider_events::ProviderEventContext {
                workspace_id: None,
                chat_thread_id: None,
                process_id: None,
                occurred_at_ms: 1,
                schema_version: 1,
                adapter_version: "claude-projection-test".to_owned(),
            })
            .iter()
            .map(|draft| store.upsert_event(draft).unwrap())
            .collect::<Vec<_>>();

        let projection = provider_projection_from_records(&records);
        let assistant = projection
            .items
            .iter()
            .find(|item| item.category == ProviderProjectionCategory::AssistantMessage)
            .unwrap();
        let reasoning = projection
            .items
            .iter()
            .find(|item| item.category == ProviderProjectionCategory::Reasoning)
            .unwrap();

        assert_eq!(assistant.body, "Public answer");
        assert_eq!(reasoning.body, "Private reasoning");
        assert_ne!(assistant.id, reasoning.id);
    }

    #[test]
    fn projection_dedupes_in_sequence_order_before_latest_update_wins() {
        let mut streaming = record(
            ProviderEventKind::AssistantOutput,
            ProviderEventPhase::Delta,
            "assistant",
        );
        streaming.normalized_payload = json!({
            "title": "Assistant",
            "body": "streamed answer"
        });
        let mut completed = streaming.clone();
        completed.identity_key = "codex:thread-1:item-1:complete".to_owned();
        completed.phase = ProviderEventPhase::Completed;
        completed.received_sequence = 2;
        completed.normalized_payload = json!({
            "title": "Assistant",
            "body": ""
        });

        let projection = provider_projection_from_records(&[completed, streaming]);

        assert_eq!(projection.items.len(), 1);
        assert_eq!(projection.items[0].body, "streamed answer");
        assert_eq!(
            projection.items[0].status,
            ProviderProjectionStatus::Complete
        );
    }

    #[test]
    fn chat_projection_skips_unused_raw_payload_materialization() {
        let raw = record(
            ProviderEventKind::AssistantOutput,
            ProviderEventPhase::Completed,
            "assistant",
        );

        let projection = provider_projection_from_records(&[raw]);

        assert_eq!(
            projection.items[0].render_class,
            ProjectionRenderClass::AssistantChat
        );
        assert_eq!(projection.items[0].raw_payload, None);
        assert!(!projection.items[0].inspectable);
    }

    #[test]
    fn projection_canonicalizes_parent_item_ids() {
        let mut child = record(
            ProviderEventKind::Tool,
            ProviderEventPhase::Completed,
            "tool_call",
        );
        child.provider_item_id = Some("tool-1".to_owned());
        child.parent_provider_item_id = Some("msg-1".to_owned());
        child.parent_provider_thread_id = Some("thread-parent".to_owned());

        let projection = provider_projection_from_records(&[child]);

        assert_eq!(
            projection.items[0].parent_id.as_deref(),
            Some("codex:thread-parent:msg-1")
        );
    }

    #[test]
    fn projection_classifies_plan_and_usage_before_generic_fallbacks() {
        let plan = record(
            ProviderEventKind::PlanningReasoning,
            ProviderEventPhase::Completed,
            "plan_update",
        );
        let usage = record(
            ProviderEventKind::LimitFailure,
            ProviderEventPhase::Completed,
            "usage",
        );
        let mut usage = usage;
        usage.identity_key = "codex:thread-1:item-2".to_owned();
        usage.provider_item_id = Some("item-2".to_owned());

        let projection = provider_projection_from_records(&[plan, usage]);

        assert!(projection
            .items
            .iter()
            .any(|item| item.category == ProviderProjectionCategory::Plan));
        assert!(projection
            .items
            .iter()
            .any(|item| item.category == ProviderProjectionCategory::Usage));
    }

    #[test]
    fn raw_payload_redacts_hyphenated_secret_keys() {
        let mut raw = record(
            ProviderEventKind::Unknown,
            ProviderEventPhase::Completed,
            "future",
        );
        raw.provider_item_id = Some("raw-1".to_owned());
        raw.raw_json = json!({
            "api-key": "abc123",
            "access-key": "def456",
            "private-key": "ghi789",
            "Proxy-Authorization": "Bearer proxy-secret",
            "authorization-url": "https://example.test/auth"
        });

        let projection = provider_projection_from_records(&[raw]);
        let payload = projection.items[0].raw_payload.as_deref().unwrap();

        assert!(!payload.contains("abc123"));
        assert!(!payload.contains("def456"));
        assert!(!payload.contains("ghi789"));
        assert!(!payload.contains("proxy-secret"));
        assert!(payload.contains("authorization-url"));
    }
}
