use std::collections::BTreeMap;

use crate::provider_events::{
    ProviderEventContext, ProviderEventDraft, ProviderEventKind, ProviderEventPhase,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const CLAUDE_PROVIDER_NAME: &str = "claude";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaudeStreamLaunchConfig {
    pub persistent_input: bool,
    pub resume: Option<String>,
    pub permission_mode: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub append_system_prompt: Option<String>,
}

pub fn build_claude_stream_args(config: &ClaudeStreamLaunchConfig) -> Vec<String> {
    let mut args = vec![
        "-p".to_owned(),
        "--output-format".to_owned(),
        "stream-json".to_owned(),
        "--verbose".to_owned(),
        "--include-partial-messages".to_owned(),
    ];

    if config.persistent_input {
        args.push("--input-format".to_owned());
        args.push("stream-json".to_owned());
    }
    push_optional_arg(&mut args, "--resume", config.resume.as_deref());
    push_optional_arg(
        &mut args,
        "--permission-mode",
        config.permission_mode.as_deref(),
    );
    push_optional_arg(&mut args, "--model", config.model.as_deref());
    push_optional_arg(&mut args, "--effort", config.effort.as_deref());
    push_optional_arg(
        &mut args,
        "--append-system-prompt",
        config.append_system_prompt.as_deref(),
    );

    args
}

fn push_optional_arg(args: &mut Vec<String>, flag: &str, value: Option<&str>) {
    if let Some(value) = value.and_then(non_empty) {
        args.push(flag.to_owned());
        args.push(value.to_owned());
    }
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeActionCapability {
    AuthSession,
    Messages,
    ContentBlockDeltasAndFinals,
    ToolUse,
    FileEdits,
    ShellCommands,
    Mcp,
    SkillsPluginsHooks,
    PermissionsUserQuestions,
    SubagentsTasks,
    UsageCostContextRateLimits,
    Errors,
    UnknownEvents,
}

pub const CLAUDE_ACTION_CAPABILITIES: &[ClaudeActionCapability] = &[
    ClaudeActionCapability::AuthSession,
    ClaudeActionCapability::Messages,
    ClaudeActionCapability::ContentBlockDeltasAndFinals,
    ClaudeActionCapability::ToolUse,
    ClaudeActionCapability::FileEdits,
    ClaudeActionCapability::ShellCommands,
    ClaudeActionCapability::Mcp,
    ClaudeActionCapability::SkillsPluginsHooks,
    ClaudeActionCapability::PermissionsUserQuestions,
    ClaudeActionCapability::SubagentsTasks,
    ClaudeActionCapability::UsageCostContextRateLimits,
    ClaudeActionCapability::Errors,
    ClaudeActionCapability::UnknownEvents,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeProviderEventKind {
    Session,
    UserMessage,
    AssistantMessage,
    MessageStart,
    MessageDelta,
    MessageStop,
    ContentBlockStart,
    ContentBlockDelta,
    ContentBlockStop,
    ToolUse,
    ToolInputDelta,
    ToolResult,
    Permission,
    Hook,
    Subagent,
    Usage,
    Result,
    Error,
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeUsageDraft {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u64>,
}

impl ClaudeUsageDraft {
    fn is_empty(&self) -> bool {
        self.input_tokens.is_none()
            && self.output_tokens.is_none()
            && self.cache_creation_input_tokens.is_none()
            && self.cache_read_input_tokens.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClaudeProviderEventDraft {
    pub provider: String,
    pub kind: ClaudeProviderEventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_block_index: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_delta: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtype: Option<String>,
    #[serde(default, skip_serializing_if = "ClaudeUsageDraft::is_empty")]
    pub usage: ClaudeUsageDraft,
    pub raw_json: Value,
}

impl ClaudeProviderEventDraft {
    pub fn into_provider_event_draft(self, context: ProviderEventContext) -> ProviderEventDraft {
        let kind = claude_kind_to_provider_kind(self.kind);
        let phase = claude_phase_for(self.kind);
        let provider_event_id = self
            .provider_event_id
            .clone()
            .or_else(|| self.provider_tool_use_id.clone())
            .or_else(|| self.provider_message_id.clone());
        let provider_item_id = self
            .provider_tool_use_id
            .clone()
            .or_else(|| self.provider_message_id.clone());
        let body = self
            .content_delta
            .clone()
            .or_else(|| string_at(&self.raw_json, &["result"]))
            .unwrap_or_default();
        let provider_subtype = self
            .subtype
            .clone()
            .or_else(|| Some(format!("{:?}", self.kind).to_ascii_lowercase()));

        ProviderEventDraft {
            provider: self.provider,
            provider_event_id,
            provider_item_id,
            provider_thread_id: self.session_id,
            provider_turn_id: None,
            parent_provider_item_id: self.parent_tool_use_id,
            parent_provider_thread_id: None,
            workspace_id: context.workspace_id,
            chat_thread_id: context.chat_thread_id,
            process_id: context.process_id,
            phase,
            kind,
            provider_subtype,
            provider_sequence: self
                .content_block_index
                .and_then(|value| i64::try_from(value).ok()),
            occurred_at_ms: context.occurred_at_ms,
            normalized_payload: json!({
                "title": claude_title_for(self.kind, self.tool_name.as_deref()),
                "body": body,
                "tool_name": self.tool_name,
                "cost_usd": self.cost_usd,
                "duration_ms": self.duration_ms,
                "usage": self.usage,
            }),
            raw_json: self.raw_json,
            schema_version: context.schema_version,
            adapter_version: context.adapter_version,
        }
    }
}

#[derive(Debug, Default)]
pub struct ClaudeStreamParser {
    current_message_id: Option<String>,
    tool_use_by_block: BTreeMap<u64, ToolBlockState>,
}

#[derive(Debug, Clone)]
struct ToolBlockState {
    id: Option<String>,
    name: Option<String>,
}

impl ClaudeStreamParser {
    pub fn parse_line(&mut self, line: &str) -> Result<Option<ClaudeProviderEventDraft>> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        let raw_json: Value =
            serde_json::from_str(trimmed).context("parse claude stream-json line")?;
        Ok(Some(self.map_value(raw_json)))
    }

    fn map_value(&mut self, raw_json: Value) -> ClaudeProviderEventDraft {
        let top_type = string_at(&raw_json, &["type"]);
        let mut draft = ClaudeProviderEventDraft {
            provider: CLAUDE_PROVIDER_NAME.to_owned(),
            kind: self.kind_for(&raw_json),
            provider_event_id: string_at(&raw_json, &["uuid"])
                .or_else(|| string_at(&raw_json, &["event", "uuid"]))
                .or_else(|| string_at(&raw_json, &["event", "id"])),
            session_id: string_at(&raw_json, &["session_id"]),
            parent_tool_use_id: string_at(&raw_json, &["parent_tool_use_id"]),
            provider_message_id: None,
            provider_tool_use_id: None,
            content_block_index: number_at(&raw_json, &["event", "index"]),
            tool_name: None,
            content_delta: None,
            cost_usd: raw_json.get("total_cost_usd").and_then(Value::as_f64),
            duration_ms: number_at(&raw_json, &["duration_ms"]),
            subtype: string_at(&raw_json, &["subtype"])
                .or_else(|| string_at(&raw_json, &["event", "delta", "type"]))
                .or_else(|| string_at(&raw_json, &["event", "content_block", "type"]))
                .or(top_type),
            usage: usage_from(&raw_json),
            raw_json,
        };

        self.apply_identity_state(&mut draft);
        draft
    }

    fn kind_for(&self, value: &Value) -> ClaudeProviderEventKind {
        match string_at(value, &["type"]).as_deref() {
            Some("system") => ClaudeProviderEventKind::Session,
            Some("user") => ClaudeProviderEventKind::UserMessage,
            Some("assistant") => ClaudeProviderEventKind::AssistantMessage,
            Some("result") => ClaudeProviderEventKind::Result,
            Some("error") => ClaudeProviderEventKind::Error,
            Some("hook_event") | Some("hook") => ClaudeProviderEventKind::Hook,
            Some("permission_request") | Some("permission") | Some("user_input_request") => {
                ClaudeProviderEventKind::Permission
            }
            Some("stream_event") => match string_at(value, &["event", "type"]).as_deref() {
                Some("message_start") => ClaudeProviderEventKind::MessageStart,
                Some("message_delta") => ClaudeProviderEventKind::MessageDelta,
                Some("message_stop") => ClaudeProviderEventKind::MessageStop,
                Some("content_block_start") => {
                    match string_at(value, &["event", "content_block", "type"]).as_deref() {
                        Some("tool_use") => ClaudeProviderEventKind::ToolUse,
                        _ => ClaudeProviderEventKind::ContentBlockStart,
                    }
                }
                Some("content_block_delta") => {
                    match string_at(value, &["event", "delta", "type"]).as_deref() {
                        Some("input_json_delta") => ClaudeProviderEventKind::ToolInputDelta,
                        _ => ClaudeProviderEventKind::ContentBlockDelta,
                    }
                }
                Some("content_block_stop") => ClaudeProviderEventKind::ContentBlockStop,
                Some("error") => ClaudeProviderEventKind::Error,
                _ => ClaudeProviderEventKind::Unknown,
            },
            Some("tool_result") => ClaudeProviderEventKind::ToolResult,
            Some("subagent") | Some("task") => ClaudeProviderEventKind::Subagent,
            Some("usage") | Some("rate_limit") | Some("context_limit") => {
                ClaudeProviderEventKind::Usage
            }
            _ => ClaudeProviderEventKind::Unknown,
        }
    }

    fn apply_identity_state(&mut self, draft: &mut ClaudeProviderEventDraft) {
        match draft.kind {
            ClaudeProviderEventKind::MessageStart => {
                draft.provider_message_id = string_at(&draft.raw_json, &["event", "message", "id"]);
                self.current_message_id = draft.provider_message_id.clone();
            }
            ClaudeProviderEventKind::AssistantMessage | ClaudeProviderEventKind::UserMessage => {
                draft.provider_message_id = string_at(&draft.raw_json, &["message", "id"]);
            }
            _ => {
                draft.provider_message_id = string_at(&draft.raw_json, &["message", "id"])
                    .or_else(|| self.current_message_id.clone());
            }
        }

        if let Some(index) = draft.content_block_index {
            if draft.kind == ClaudeProviderEventKind::ToolUse {
                let state = ToolBlockState {
                    id: string_at(&draft.raw_json, &["event", "content_block", "id"]),
                    name: string_at(&draft.raw_json, &["event", "content_block", "name"]),
                };
                draft.provider_tool_use_id = state.id.clone();
                draft.tool_name = state.name.clone();
                self.tool_use_by_block.insert(index, state);
            } else if let Some(state) = self.tool_use_by_block.get(&index) {
                draft.provider_tool_use_id = state.id.clone();
                draft.tool_name = state.name.clone();
            }
        }

        if draft.kind == ClaudeProviderEventKind::ToolInputDelta {
            draft.content_delta = string_at(&draft.raw_json, &["event", "delta", "partial_json"]);
        } else if draft.kind == ClaudeProviderEventKind::ContentBlockDelta {
            draft.content_delta = string_at(&draft.raw_json, &["event", "delta", "text"]);
        }
    }
}

pub fn parse_claude_stream_json_lines(input: &str) -> Result<Vec<ClaudeProviderEventDraft>> {
    let mut parser = ClaudeStreamParser::default();
    let mut events = Vec::new();
    for line in input.lines() {
        if let Some(event) = parser.parse_line(line)? {
            events.push(event);
        }
    }
    Ok(events)
}

fn usage_from(value: &Value) -> ClaudeUsageDraft {
    let usage = value
        .get("usage")
        .or_else(|| value.pointer("/message/usage"))
        .or_else(|| value.pointer("/event/message/usage"))
        .or_else(|| value.pointer("/event/usage"));

    let Some(usage) = usage else {
        return ClaudeUsageDraft::default();
    };

    ClaudeUsageDraft {
        input_tokens: number_at(usage, &["input_tokens"]),
        output_tokens: number_at(usage, &["output_tokens"]),
        cache_creation_input_tokens: number_at(usage, &["cache_creation_input_tokens"]),
        cache_read_input_tokens: number_at(usage, &["cache_read_input_tokens"]),
    }
}

fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn number_at(value: &Value, path: &[&str]) -> Option<u64> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))
        .and_then(Value::as_u64)
}

fn claude_kind_to_provider_kind(kind: ClaudeProviderEventKind) -> ProviderEventKind {
    match kind {
        ClaudeProviderEventKind::Session => ProviderEventKind::ThreadSession,
        ClaudeProviderEventKind::UserMessage => ProviderEventKind::UserInput,
        ClaudeProviderEventKind::AssistantMessage
        | ClaudeProviderEventKind::MessageStart
        | ClaudeProviderEventKind::MessageDelta
        | ClaudeProviderEventKind::MessageStop
        | ClaudeProviderEventKind::ContentBlockStart
        | ClaudeProviderEventKind::ContentBlockDelta
        | ClaudeProviderEventKind::ContentBlockStop => ProviderEventKind::AssistantOutput,
        ClaudeProviderEventKind::ToolUse
        | ClaudeProviderEventKind::ToolInputDelta
        | ClaudeProviderEventKind::ToolResult => ProviderEventKind::Tool,
        ClaudeProviderEventKind::Permission => ProviderEventKind::ApprovalPermission,
        ClaudeProviderEventKind::Hook => ProviderEventKind::SkillPluginHook,
        ClaudeProviderEventKind::Subagent => ProviderEventKind::SubagentCollaboration,
        ClaudeProviderEventKind::Usage | ClaudeProviderEventKind::Error => {
            ProviderEventKind::LimitFailure
        }
        ClaudeProviderEventKind::Result => ProviderEventKind::Turn,
        ClaudeProviderEventKind::Unknown => ProviderEventKind::Unknown,
    }
}

fn claude_phase_for(kind: ClaudeProviderEventKind) -> ProviderEventPhase {
    match kind {
        ClaudeProviderEventKind::MessageStart
        | ClaudeProviderEventKind::ContentBlockStart
        | ClaudeProviderEventKind::ToolUse
        | ClaudeProviderEventKind::Session
        | ClaudeProviderEventKind::UserMessage => ProviderEventPhase::Started,
        ClaudeProviderEventKind::MessageDelta
        | ClaudeProviderEventKind::ContentBlockDelta
        | ClaudeProviderEventKind::ToolInputDelta => ProviderEventPhase::Delta,
        ClaudeProviderEventKind::Permission
        | ClaudeProviderEventKind::Hook
        | ClaudeProviderEventKind::Subagent
        | ClaudeProviderEventKind::Usage => ProviderEventPhase::Progress,
        ClaudeProviderEventKind::MessageStop
        | ClaudeProviderEventKind::ContentBlockStop
        | ClaudeProviderEventKind::ToolResult
        | ClaudeProviderEventKind::Result
        | ClaudeProviderEventKind::AssistantMessage => ProviderEventPhase::Completed,
        ClaudeProviderEventKind::Error => ProviderEventPhase::Failed,
        ClaudeProviderEventKind::Unknown => ProviderEventPhase::Unknown,
    }
}

fn claude_title_for(kind: ClaudeProviderEventKind, tool_name: Option<&str>) -> String {
    match kind {
        ClaudeProviderEventKind::ToolUse
        | ClaudeProviderEventKind::ToolInputDelta
        | ClaudeProviderEventKind::ToolResult => tool_name.unwrap_or("Tool").to_owned(),
        ClaudeProviderEventKind::UserMessage => "User input".to_owned(),
        ClaudeProviderEventKind::AssistantMessage
        | ClaudeProviderEventKind::MessageStart
        | ClaudeProviderEventKind::MessageDelta
        | ClaudeProviderEventKind::MessageStop
        | ClaudeProviderEventKind::ContentBlockStart
        | ClaudeProviderEventKind::ContentBlockDelta
        | ClaudeProviderEventKind::ContentBlockStop => "Assistant output".to_owned(),
        ClaudeProviderEventKind::Permission => "Permission request".to_owned(),
        ClaudeProviderEventKind::Hook => "Hook".to_owned(),
        ClaudeProviderEventKind::Subagent => "Subagent".to_owned(),
        ClaudeProviderEventKind::Usage => "Usage".to_owned(),
        ClaudeProviderEventKind::Result => "Turn result".to_owned(),
        ClaudeProviderEventKind::Session => "Session".to_owned(),
        ClaudeProviderEventKind::Error => "Provider error".to_owned(),
        ClaudeProviderEventKind::Unknown => "Unknown provider event".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_args_use_structured_stream_json_without_bare_mode() {
        let args = build_claude_stream_args(&ClaudeStreamLaunchConfig {
            persistent_input: false,
            ..ClaudeStreamLaunchConfig::default()
        });

        assert_eq!(
            args,
            vec![
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
            ]
        );
        assert!(!args.iter().any(|arg| arg == "--bare"));
        assert!(!args.iter().any(|arg| arg == "--input-format"));
    }

    #[test]
    fn persistent_streamed_input_adds_stream_json_input_format() {
        let args = build_claude_stream_args(&ClaudeStreamLaunchConfig {
            persistent_input: true,
            resume: Some("session-123".to_owned()),
            permission_mode: Some("plan".to_owned()),
            model: None,
            effort: Some("low".to_owned()),
            append_system_prompt: Some("Archductor context".to_owned()),
        });

        assert_eq!(
            args,
            vec![
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--input-format",
                "stream-json",
                "--resume",
                "session-123",
                "--permission-mode",
                "plan",
                "--effort",
                "low",
                "--append-system-prompt",
                "Archductor context",
            ]
        );
        assert!(!args.iter().any(|arg| arg == "--bare"));
    }

    #[test]
    fn capability_list_covers_required_claude_event_categories() {
        assert_eq!(
            CLAUDE_ACTION_CAPABILITIES,
            &[
                ClaudeActionCapability::AuthSession,
                ClaudeActionCapability::Messages,
                ClaudeActionCapability::ContentBlockDeltasAndFinals,
                ClaudeActionCapability::ToolUse,
                ClaudeActionCapability::FileEdits,
                ClaudeActionCapability::ShellCommands,
                ClaudeActionCapability::Mcp,
                ClaudeActionCapability::SkillsPluginsHooks,
                ClaudeActionCapability::PermissionsUserQuestions,
                ClaudeActionCapability::SubagentsTasks,
                ClaudeActionCapability::UsageCostContextRateLimits,
                ClaudeActionCapability::Errors,
                ClaudeActionCapability::UnknownEvents,
            ]
        );
    }

    #[test]
    fn parses_stream_json_lines_into_lossless_provider_event_drafts() {
        let input = r#"{"type":"system","subtype":"init","session_id":"s1","cwd":"/repo","tools":["Read","Bash"]}
{"type":"stream_event","uuid":"u1","session_id":"s1","event":{"type":"message_start","message":{"id":"msg_1","role":"assistant","usage":{"input_tokens":7}}}}
{"type":"stream_event","uuid":"u2","session_id":"s1","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}
{"type":"stream_event","uuid":"u3","session_id":"s1","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}}
{"type":"stream_event","uuid":"u4","session_id":"s1","event":{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tool_1","name":"Bash","input":{}}}}
{"type":"stream_event","uuid":"u5","session_id":"s1","event":{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"command\":\"cargo test\"}"}}}
{"type":"stream_event","uuid":"u6","session_id":"s1","event":{"type":"content_block_stop","index":1}}
{"type":"assistant","session_id":"s1","parent_tool_use_id":null,"message":{"id":"msg_1","role":"assistant","content":[{"type":"text","text":"Hello"},{"type":"tool_use","id":"tool_1","name":"Bash","input":{"command":"cargo test"}}],"usage":{"output_tokens":3}}}
{"type":"result","subtype":"success","session_id":"s1","result":"done","total_cost_usd":0.01,"duration_ms":2500,"usage":{"input_tokens":7,"output_tokens":3}}
{"type":"new_future_type","session_id":"s1","value":42}"#;

        let events = parse_claude_stream_json_lines(input).unwrap();

        assert_eq!(events.len(), 10);
        assert_eq!(events[0].kind, ClaudeProviderEventKind::Session);
        assert_eq!(events[1].kind, ClaudeProviderEventKind::MessageStart);
        assert_eq!(events[3].kind, ClaudeProviderEventKind::ContentBlockDelta);
        assert_eq!(events[4].kind, ClaudeProviderEventKind::ToolUse);
        assert_eq!(events[5].kind, ClaudeProviderEventKind::ToolInputDelta);
        assert_eq!(events[7].kind, ClaudeProviderEventKind::AssistantMessage);
        assert_eq!(events[8].kind, ClaudeProviderEventKind::Result);
        assert_eq!(events[9].kind, ClaudeProviderEventKind::Unknown);
        assert_eq!(events[7].provider_message_id.as_deref(), Some("msg_1"));
        assert_eq!(events[4].provider_tool_use_id.as_deref(), Some("tool_1"));
        assert_eq!(events[5].provider_tool_use_id.as_deref(), Some("tool_1"));
        assert_eq!(
            events[5].content_delta.as_deref(),
            Some(r#"{"command":"cargo test"}"#)
        );
        assert_eq!(events[8].usage.input_tokens, Some(7));
        assert_eq!(events[8].usage.output_tokens, Some(3));
        assert_eq!(events[8].cost_usd, Some(0.01));
        assert_eq!(events[9].raw_json["value"], 42);
    }

    #[test]
    fn canonical_conversion_keeps_tool_events_out_of_assistant_output() {
        let input = r#"{"type":"stream_event","uuid":"u4","session_id":"s1","event":{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tool_1","name":"Bash","input":{}}}}
{"type":"stream_event","uuid":"u5","session_id":"s1","event":{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"command\":\"cargo test\"}"}}}"#;
        let events = parse_claude_stream_json_lines(input).unwrap();

        let canonical = events[1].clone().into_provider_event_draft(
            crate::provider_events::ProviderEventContext {
                workspace_id: Some(1),
                chat_thread_id: Some(7),
                process_id: Some(9),
                occurred_at_ms: 42,
                schema_version: 1,
                adapter_version: "claude-stream-json-test".to_owned(),
            },
        );

        assert_eq!(
            canonical.kind,
            crate::provider_events::ProviderEventKind::Tool
        );
        assert_eq!(
            canonical.phase,
            crate::provider_events::ProviderEventPhase::Delta
        );
        assert_eq!(canonical.provider_item_id.as_deref(), Some("tool_1"));
        assert_eq!(canonical.normalized_payload["tool_name"], "Bash");
        assert_eq!(
            canonical.normalized_payload["body"],
            r#"{"command":"cargo test"}"#
        );
    }

    #[test]
    fn canonical_conversion_preserves_unknown_future_raw_json() {
        let events = parse_claude_stream_json_lines(
            r#"{"type":"new_future_type","session_id":"s1","value":42}"#,
        )
        .unwrap();

        let canonical = events[0].clone().into_provider_event_draft(
            crate::provider_events::ProviderEventContext {
                workspace_id: None,
                chat_thread_id: Some(7),
                process_id: None,
                occurred_at_ms: 42,
                schema_version: 1,
                adapter_version: "claude-stream-json-test".to_owned(),
            },
        );

        assert_eq!(
            canonical.kind,
            crate::provider_events::ProviderEventKind::Unknown
        );
        assert_eq!(canonical.raw_json["value"], 42);
    }
}
