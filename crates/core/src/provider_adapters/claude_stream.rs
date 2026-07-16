use std::collections::BTreeMap;
use std::collections::{HashSet, VecDeque};

use crate::archcar::harness::ClaudeHarnessController;
use crate::archcar::harness_contract::{
    HarnessAdapterContext, HarnessCapability, HarnessControl, HarnessControlPlan,
    HarnessDescriptor, HarnessEffect, HarnessInput, HarnessPreflightSpec, HarnessRecoveryCause,
    HarnessRecoveryPlan, HarnessSignal, HarnessTurnStatus, ManagedHarness, ManagedHarnessAdapter,
    NativeRecord, NativeWrite, SupportMode, MANAGED_HARNESS_CONTRACT_VERSION,
    REQUIRED_HARNESS_FEATURES,
};
use crate::provider_events::{
    ProviderEventContext, ProviderEventDraft, ProviderEventKind, ProviderEventPhase,
};
use crate::workspace::SessionKind;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const CLAUDE_PROVIDER_NAME: &str = "claude";

const CLAUDE_OPTIONAL_CAPABILITIES: &[(HarnessCapability, SupportMode)] = &[
    (
        HarnessCapability::Goals,
        SupportMode::Unsupported {
            reason: "Claude stream-json does not expose native goals",
        },
    ),
    (
        HarnessCapability::NativeSlashCommands,
        SupportMode::Unsupported {
            reason: "Claude stream-json does not expose interactive slash commands",
        },
    ),
];

pub static CLAUDE_MANAGED_HARNESS_DESCRIPTOR: HarnessDescriptor = HarnessDescriptor {
    contract_version: MANAGED_HARNESS_CONTRACT_VERSION,
    kind: SessionKind::Claude,
    provider_key: CLAUDE_PROVIDER_NAME,
    display_name: "Claude Code",
    default_executable: "claude",
    preflight: HarnessPreflightSpec {
        command: &["claude", "auth", "status"],
        auth_guidance: "Run `claude auth login`.",
    },
    required_features: REQUIRED_HARNESS_FEATURES,
    optional_capabilities: CLAUDE_OPTIONAL_CAPABILITIES,
};

impl ManagedHarness for ClaudeHarnessController {
    fn descriptor(&self) -> &'static HarnessDescriptor {
        &CLAUDE_MANAGED_HARNESS_DESCRIPTOR
    }

    fn create_adapter(
        &self,
        context: HarnessAdapterContext,
    ) -> Result<Box<dyn ManagedHarnessAdapter>> {
        Ok(Box::new(ClaudeManagedAdapter::new(context)))
    }
}

pub(crate) struct ClaudeManagedAdapter {
    context: HarnessAdapterContext,
    parser: ClaudeStreamParser,
    pending_inputs: VecDeque<String>,
    active_input_id: Option<String>,
    completed_inputs: HashSet<String>,
}

impl ClaudeManagedAdapter {
    pub(crate) fn new(context: HarnessAdapterContext) -> Self {
        Self {
            context,
            parser: ClaudeStreamParser::default(),
            pending_inputs: VecDeque::new(),
            active_input_id: None,
            completed_inputs: HashSet::new(),
        }
    }

    fn provider_context(&self) -> ProviderEventContext {
        ProviderEventContext::runtime(
            None,
            Some(self.context.thread_id),
            Some(self.context.session_id),
            "claude-stream-json",
        )
    }

    pub(crate) fn settle_failed_input_write(&mut self, local_input_id: &str) {
        self.pending_inputs
            .retain(|pending_input_id| pending_input_id != local_input_id);
        if self.active_input_id.as_deref() == Some(local_input_id) {
            self.active_input_id = None;
        }
    }
}

impl ManagedHarnessAdapter for ClaudeManagedAdapter {
    fn encode_input(&mut self, input: HarnessInput) -> Result<NativeWrite> {
        let payload = encode_claude_user_message(&input.content);
        let mut native_payload =
            serde_json::to_vec(&payload).context("serialize Claude stream-json input")?;
        native_payload.push(b'\n');
        self.pending_inputs.push_back(input.local_input_id.clone());

        Ok(NativeWrite {
            provider_key: CLAUDE_PROVIDER_NAME,
            local_input_id: Some(input.local_input_id),
            payload: native_payload,
        })
    }

    fn observe_native(&mut self, record: NativeRecord) -> Result<Vec<HarnessEffect>> {
        anyhow::ensure!(
            record.provider_key == CLAUDE_PROVIDER_NAME,
            "Claude adapter received native record for {}",
            record.provider_key,
        );
        let input = std::str::from_utf8(&record.payload).context("decode Claude stream-json")?;
        let mut effects = Vec::new();

        for line in input.lines().filter(|line| !line.trim().is_empty()) {
            let Some(event) = self.parser.parse_line(line)? else {
                continue;
            };
            let event_kind = event.kind;
            let native_session_id = event.session_id.clone();

            if let Some(native_session_id) = native_session_id {
                if self.context.native_session_id.as_deref() != Some(native_session_id.as_str()) {
                    self.context.native_session_id = Some(native_session_id.clone());
                    effects.push(HarnessEffect::Initialized {
                        native_session_id,
                        model: self.context.controls.model.clone(),
                    });
                    effects.push(HarnessEffect::Ready);
                }
            }

            if matches!(
                event_kind,
                ClaudeProviderEventKind::UserMessage | ClaudeProviderEventKind::MessageStart
            ) && self.active_input_id.is_none()
            {
                self.active_input_id = self.pending_inputs.pop_front();
                if let Some(local_input_id) = self.active_input_id.clone() {
                    effects.push(HarnessEffect::TurnStarted { local_input_id });
                }
            }

            let turn_status = managed_claude_turn_status(event_kind, &event.raw_json);
            effects.push(HarnessEffect::ProviderEvent(
                event.into_provider_event_draft(self.provider_context()),
            ));

            if event_kind == ClaudeProviderEventKind::Result {
                if let Some(local_input_id) = self.active_input_id.take() {
                    if self.completed_inputs.insert(local_input_id.clone()) {
                        effects.push(HarnessEffect::TurnCompleted {
                            local_input_id,
                            status: turn_status,
                        });
                    }
                }
                effects.push(HarnessEffect::Ready);
            }
        }

        Ok(effects)
    }

    fn plan_control(&mut self, control: HarnessControl) -> HarnessControlPlan {
        match control {
            HarnessControl::Kill => {
                HarnessControlPlan::Signal(HarnessSignal::TerminateProcessGroup)
            }
            HarnessControl::Interrupt => HarnessControlPlan::Unsupported {
                reason: "Claude stream-json interrupt is not implemented".to_owned(),
            },
            HarnessControl::SetModel(model) => {
                self.context.controls.model = model;
                HarnessControlPlan::RestartRequired(self.context.controls.clone())
            }
            HarnessControl::SetEffort(effort) => {
                self.context.controls.effort = effort;
                HarnessControlPlan::RestartRequired(self.context.controls.clone())
            }
            HarnessControl::SetPermissionMode(permission_mode) => {
                self.context.controls.permission_mode = permission_mode;
                HarnessControlPlan::RestartRequired(self.context.controls.clone())
            }
            HarnessControl::ResolveInteraction(_) => HarnessControlPlan::Unsupported {
                reason: "Claude interaction resolution is not implemented".to_owned(),
            },
        }
    }

    fn recovery_plan(&self, _cause: HarnessRecoveryCause) -> HarnessRecoveryPlan {
        match self.context.native_session_id.clone() {
            Some(native_session_id) => HarnessRecoveryPlan::RestartAndResume {
                native_session_id,
                controls: self.context.controls.clone(),
            },
            None => HarnessRecoveryPlan::Fail {
                message: "Claude session has no native session id to resume".to_owned(),
            },
        }
    }
}

fn managed_claude_turn_status(kind: ClaudeProviderEventKind, payload: &Value) -> HarnessTurnStatus {
    if kind != ClaudeProviderEventKind::Result {
        return HarnessTurnStatus::Deferred;
    }
    if payload
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || matches!(
            payload.get("subtype").and_then(Value::as_str),
            Some("error" | "failed")
        )
    {
        HarnessTurnStatus::Failed
    } else {
        HarnessTurnStatus::Success
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaudeStreamLaunchConfig {
    pub persistent_input: bool,
    pub replay_user_messages: bool,
    pub resume: Option<String>,
    pub permission_mode: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub append_system_prompt: Option<String>,
    pub settings_json: Option<String>,
}

pub fn encode_claude_user_message(input: &str) -> Value {
    json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": [{"type": "text", "text": input}],
        },
        "parent_tool_use_id": null,
    })
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
        if config.replay_user_messages {
            args.push("--replay-user-messages".to_owned());
        }
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
    push_optional_arg(&mut args, "--settings", config.settings_json.as_deref());

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
        let body = if self.kind == ClaudeProviderEventKind::Hook {
            claude_hook_body(&self.raw_json).unwrap_or_default()
        } else {
            self.content_delta
                .clone()
                .or_else(|| string_at(&self.raw_json, &["result"]))
                .unwrap_or_default()
        };
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
                "title": claude_event_title(self.kind, self.tool_name.as_deref(), &self.raw_json),
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
        if claude_system_hook_event(value) {
            return ClaudeProviderEventKind::Hook;
        }
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

fn claude_system_hook_event(value: &Value) -> bool {
    string_at(value, &["subtype"]).is_some_and(|subtype| subtype.starts_with("hook_"))
        || string_at(value, &["hook_event"]).is_some()
        || string_at(value, &["hook_name"]).is_some()
        || string_at(value, &["hook_event_name"]).is_some()
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

fn claude_event_title(
    kind: ClaudeProviderEventKind,
    tool_name: Option<&str>,
    raw_json: &Value,
) -> String {
    if kind == ClaudeProviderEventKind::Hook {
        return string_at(raw_json, &["hook_event_name"])
            .or_else(|| string_at(raw_json, &["hook_name"]))
            .or_else(|| string_at(raw_json, &["hook_event"]))
            .or_else(|| string_at(raw_json, &["hook", "name"]))
            .or_else(|| string_at(raw_json, &["event", "hook_event_name"]))
            .unwrap_or_else(|| claude_title_for(kind, tool_name));
    }

    claude_title_for(kind, tool_name)
}

fn claude_hook_body(raw_json: &Value) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(tool_name) = string_at(raw_json, &["tool_name"]) {
        if !tool_name.trim().is_empty() {
            parts.push(tool_name);
        }
    }
    if let Some(message) = string_at(raw_json, &["message"])
        .or_else(|| string_at(raw_json, &["event", "message"]))
        .or_else(|| string_at(raw_json, &["output"]))
        .or_else(|| string_at(raw_json, &["result"]))
    {
        if !message.trim().is_empty() {
            parts.push(message);
        }
    }
    if let Some(input) = raw_json
        .get("tool_input")
        .or_else(|| raw_json.pointer("/event/tool_input"))
        .or_else(|| raw_json.pointer("/hook/tool_input"))
    {
        if let Ok(display) = serde_json::to_string_pretty(input) {
            if !display.trim().is_empty() {
                parts.push(display);
            }
        }
    }

    (!parts.is_empty()).then(|| parts.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASIC_TURN_FIXTURE: &str =
        include_str!("../../tests/fixtures/claude_stream/basic_turn.jsonl");
    const TOOLS_HOOKS_LIMITS_FIXTURE: &str =
        include_str!("../../tests/fixtures/claude_stream/tools_hooks_limits.jsonl");
    const DEFERRED_AND_FAILED_RESULTS_FIXTURE: &str =
        include_str!("../../tests/fixtures/claude_stream/deferred_and_failed_results.jsonl");

    fn fixture_values(name: &str, input: &str) -> Result<Vec<Value>> {
        input
            .lines()
            .enumerate()
            .map(|(index, line)| {
                serde_json::from_str(line)
                    .with_context(|| format!("parse {name} fixture line {}", index + 1))
            })
            .collect()
    }

    fn validate_fixture_contract(
        basic: &[Value],
        tools: &[Value],
        results: &[Value],
    ) -> Result<()> {
        let init_index = basic
            .iter()
            .position(|record| record["type"] == "system" && record["subtype"] == "init")
            .context("basic fixture missing system/init")?;
        anyhow::ensure!(
            init_index > 0,
            "basic fixture needs startup records before init"
        );
        anyhow::ensure!(
            basic[..init_index]
                .iter()
                .all(|record| record["type"].is_string() && record["subtype"].is_string()),
            "startup records need string type and subtype"
        );
        let init = &basic[init_index];
        anyhow::ensure!(
            init["session_id"].is_string(),
            "init session_id must be string"
        );
        anyhow::ensure!(init["model"].is_string(), "init model must be string");
        anyhow::ensure!(
            init["tools"]
                .as_array()
                .is_some_and(|values| values.iter().all(Value::is_string)),
            "init tools must be an array of strings"
        );
        anyhow::ensure!(
            init["mcp_servers"].as_array().is_some_and(|servers| {
                servers
                    .iter()
                    .all(|server| server["name"].is_string() && server["status"].is_string())
            }),
            "init mcp_servers must contain string name/status fields"
        );
        anyhow::ensure!(
            init["plugins"].as_array().is_some_and(|plugins| {
                plugins
                    .iter()
                    .all(|plugin| plugin["name"].is_string() && plugin["path"].is_string())
            }),
            "init plugins must contain string name/path fields"
        );
        if let Some(capabilities) = init.get("capabilities") {
            anyhow::ensure!(
                capabilities
                    .as_array()
                    .is_some_and(|values| values.iter().all(Value::is_string)),
                "optional init capabilities must be an array of strings"
            );
        }

        let replay = basic
            .iter()
            .find(|record| record["type"] == "user" && record["isReplay"] == true)
            .context("basic fixture missing replayed user input")?;
        anyhow::ensure!(
            replay["session_id"].is_string(),
            "replay session_id must be string"
        );
        anyhow::ensure!(replay["parent_tool_use_id"].is_null());
        anyhow::ensure!(replay["message"]["role"] == "user");
        anyhow::ensure!(
            replay["message"]["content"]
                .as_array()
                .is_some_and(|content| content
                    .iter()
                    .any(|block| { block["type"] == "text" && block["text"].is_string() })),
            "replayed user content needs a text block"
        );
        anyhow::ensure!(basic.iter().any(|record| {
            record["type"] == "stream_event"
                && record["event"]["type"] == "content_block_delta"
                && record["event"]["delta"]["type"] == "text_delta"
                && record["event"]["delta"]["text"].is_string()
        }));
        anyhow::ensure!(basic.iter().any(|record| {
            record["type"] == "assistant"
                && record["message"]["role"] == "assistant"
                && record["message"]["content"].is_array()
        }));
        anyhow::ensure!(basic.iter().any(|record| {
            record["type"] == "result"
                && record["subtype"] == "success"
                && record["is_error"].is_boolean()
                && record["result"].is_string()
                && record["duration_ms"].is_u64()
                && record["usage"]["input_tokens"].is_u64()
                && record["usage"]["output_tokens"].is_u64()
        }));
        anyhow::ensure!(basic.iter().any(|record| {
            record["type"].is_string() && record["type"] == "future_fixture_record"
        }));

        anyhow::ensure!(tools.iter().any(|record| {
            record["event"]["delta"]["type"] == "input_json_delta"
                && record["event"]["delta"]["partial_json"]
                    .as_str()
                    .is_some_and(|partial| serde_json::from_str::<Value>(partial).is_ok())
        }));
        anyhow::ensure!(tools.iter().any(|record| {
            record["type"] == "user"
                && record["message"]["content"]
                    .as_array()
                    .is_some_and(|content| {
                        content.iter().any(|block| {
                            block["type"] == "tool_result"
                                && block["tool_use_id"].is_string()
                                && block["is_error"].is_boolean()
                        })
                    })
        }));
        for subtype in ["hook_started", "hook_response"] {
            let hook = tools
                .iter()
                .find(|record| record["type"] == "system" && record["subtype"] == subtype)
                .with_context(|| format!("tools fixture missing {subtype}"))?;
            anyhow::ensure!(hook["hook_id"].is_string());
            anyhow::ensure!(hook["hook_name"].is_string());
            anyhow::ensure!(hook["hook_event"].is_string());
        }
        let retry = tools
            .iter()
            .find(|record| record["type"] == "system" && record["subtype"] == "api_retry")
            .context("tools fixture missing api_retry")?;
        anyhow::ensure!(retry["attempt"].is_u64());
        anyhow::ensure!(retry["max_retries"].is_u64());
        anyhow::ensure!(retry["retry_delay_ms"].is_u64());
        anyhow::ensure!(retry["error"].is_string());
        let rate_limit = tools
            .iter()
            .find(|record| record["type"] == "rate_limit_event")
            .context("tools fixture missing rate_limit_event")?;
        anyhow::ensure!(rate_limit["rate_limit_info"]["status"].is_string());
        anyhow::ensure!(rate_limit["rate_limit_info"]["resetsAt"].is_u64());
        anyhow::ensure!(rate_limit["rate_limit_info"]["rateLimitType"].is_string());
        anyhow::ensure!(rate_limit["rate_limit_info"]["isUsingOverage"].is_boolean());

        anyhow::ensure!(results.iter().any(|record| {
            record["type"] == "user"
                && record["tool_use_result"]["type"] == "tool_deferred"
                && record["message"]["content"]
                    .as_array()
                    .is_some_and(|content| {
                        content.iter().any(|block| {
                            block["type"] == "tool_result" && block["tool_use_id"].is_string()
                        })
                    })
        }));
        for subtype in ["error_during_execution", "interrupted"] {
            let result = results
                .iter()
                .find(|record| record["type"] == "result" && record["subtype"] == subtype)
                .with_context(|| format!("results fixture missing {subtype}"))?;
            anyhow::ensure!(result["is_error"].is_boolean());
            anyhow::ensure!(result["result"].is_string());
            anyhow::ensure!(result["duration_ms"].is_u64());
        }
        Ok(())
    }

    #[test]
    fn claude_stream_contract_launches_persistent_native_stream_with_settings() {
        let args = build_claude_stream_args(&ClaudeStreamLaunchConfig {
            persistent_input: true,
            replay_user_messages: true,
            settings_json: Some(r#"{"hooks":{}}"#.to_owned()),
            ..ClaudeStreamLaunchConfig::default()
        });

        assert!(args
            .windows(2)
            .any(|v| v == ["--input-format", "stream-json"]));
        assert!(args.iter().any(|v| v == "--replay-user-messages"));
        assert!(args.windows(2).any(|v| v[0] == "--settings"));
        assert!(!args.iter().any(|v| v == "--bare"));
    }

    #[test]
    fn claude_stream_contract_omits_replay_without_persistent_input() {
        let args = build_claude_stream_args(&ClaudeStreamLaunchConfig {
            persistent_input: false,
            replay_user_messages: true,
            ..ClaudeStreamLaunchConfig::default()
        });

        assert!(!args.iter().any(|v| v == "--replay-user-messages"));
    }

    #[test]
    fn claude_stream_contract_encodes_native_user_input_through_adapter() {
        assert_eq!(encode_claude_user_message("hello")["type"], "user");
        assert_eq!(
            encode_claude_user_message("hello")["parent_tool_use_id"],
            Value::Null
        );

        let mut adapter = ClaudeManagedAdapter::new(HarnessAdapterContext {
            session_id: 7,
            thread_id: 11,
            workspace: "fixture-workspace".to_owned(),
            native_session_id: None,
            controls: Default::default(),
        });
        let input = HarnessInput {
            local_input_id: "local-input-fixture".to_owned(),
            content: "hello".to_owned(),
            visible_content: None,
            kind: crate::archcar::protocol::ArchcarInputKind::User,
            delivery: crate::archcar::protocol::ArchcarInputDelivery::Auto,
        };
        let write = adapter.encode_input(input).unwrap();

        assert_eq!(write.provider_key, "claude");
        assert_eq!(write.local_input_id.as_deref(), Some("local-input-fixture"));
        assert_eq!(
            serde_json::from_slice::<Value>(&write.payload).unwrap(),
            encode_claude_user_message("hello")
        );
        assert_eq!(write.payload.last(), Some(&b'\n'));
    }

    #[test]
    fn claude_failed_input_write_settles_pending_and_active_correlation() {
        let mut pending_adapter = ClaudeManagedAdapter::new(HarnessAdapterContext {
            session_id: 7,
            thread_id: 11,
            workspace: "fixture-workspace".to_owned(),
            native_session_id: None,
            controls: Default::default(),
        });
        pending_adapter
            .encode_input(HarnessInput {
                local_input_id: "pending-input".to_owned(),
                content: "pending".to_owned(),
                visible_content: None,
                kind: crate::archcar::protocol::ArchcarInputKind::User,
                delivery: crate::archcar::protocol::ArchcarInputDelivery::Auto,
            })
            .unwrap();

        pending_adapter.settle_failed_input_write("pending-input");

        assert!(pending_adapter.pending_inputs.is_empty());
        assert!(pending_adapter.active_input_id.is_none());

        let mut active_adapter = ClaudeManagedAdapter::new(HarnessAdapterContext {
            session_id: 7,
            thread_id: 11,
            workspace: "fixture-workspace".to_owned(),
            native_session_id: None,
            controls: Default::default(),
        });
        active_adapter
            .encode_input(HarnessInput {
                local_input_id: "active-input".to_owned(),
                content: "active".to_owned(),
                visible_content: None,
                kind: crate::archcar::protocol::ArchcarInputKind::User,
                delivery: crate::archcar::protocol::ArchcarInputDelivery::Auto,
            })
            .unwrap();
        active_adapter
            .observe_native(NativeRecord {
                provider_key: "claude",
                payload: br#"{"type":"stream_event","session_id":"fixture-session","event":{"type":"message_start","message":{"id":"fixture-message"}}}
"#
                .to_vec(),
            })
            .unwrap();

        active_adapter.settle_failed_input_write("active-input");

        assert!(active_adapter.pending_inputs.is_empty());
        assert!(active_adapter.active_input_id.is_none());
    }

    #[test]
    fn claude_stream_contract_fixtures_cover_native_record_families_losslessly() {
        let basic = parse_claude_stream_json_lines(BASIC_TURN_FIXTURE).unwrap();
        let tools = parse_claude_stream_json_lines(TOOLS_HOOKS_LIMITS_FIXTURE).unwrap();
        let results = parse_claude_stream_json_lines(DEFERRED_AND_FAILED_RESULTS_FIXTURE).unwrap();

        assert_eq!(basic.len(), BASIC_TURN_FIXTURE.lines().count());
        assert_eq!(tools.len(), TOOLS_HOOKS_LIMITS_FIXTURE.lines().count());
        assert_eq!(
            results.len(),
            DEFERRED_AND_FAILED_RESULTS_FIXTURE.lines().count()
        );
        assert_eq!(basic[0].raw_json["subtype"], "hook_started");
        assert_eq!(basic[1].raw_json["subtype"], "init");
        assert_eq!(basic[1].raw_json["mcp_servers"][0]["name"], "fixture-mcp");
        assert!(basic.iter().any(|event| event.raw_json["isReplay"] == true));
        assert!(basic.iter().any(|event| {
            event.raw_json["type"] == "assistant"
                && event.raw_json["message"]["content"][0]["text"] == "Fixture complete."
        }));
        assert!(basic
            .iter()
            .any(|event| event.kind == ClaudeProviderEventKind::Unknown));
        assert!(tools
            .iter()
            .any(|event| { event.raw_json["message"]["content"][0]["type"] == "tool_result" }));
        assert!(tools
            .iter()
            .any(|event| event.raw_json["subtype"] == "hook_response"));
        assert!(tools
            .iter()
            .any(|event| event.raw_json["subtype"] == "api_retry"));
        assert!(tools
            .iter()
            .any(|event| event.raw_json["type"] == "rate_limit_event"));
        assert!(results
            .iter()
            .any(|event| event.raw_json["tool_use_result"]["type"] == "tool_deferred"));
        assert!(results.iter().any(|event| {
            event.raw_json["type"] == "result" && event.raw_json["is_error"] == true
        }));
        assert!(results
            .iter()
            .any(|event| event.raw_json["subtype"] == "interrupted"));
    }

    #[test]
    fn claude_stream_contract_fixtures_have_required_native_field_shapes() {
        let basic = fixture_values("basic", BASIC_TURN_FIXTURE).unwrap();
        let tools = fixture_values("tools", TOOLS_HOOKS_LIMITS_FIXTURE).unwrap();
        let results = fixture_values("results", DEFERRED_AND_FAILED_RESULTS_FIXTURE).unwrap();

        validate_fixture_contract(&basic, &tools, &results).unwrap();

        let mut malformed_basic = basic.clone();
        let init = malformed_basic
            .iter_mut()
            .find(|record| record["subtype"] == "init")
            .unwrap();
        init["tools"] = Value::String("not-an-array".to_owned());
        assert!(validate_fixture_contract(&malformed_basic, &tools, &results).is_err());
    }

    #[test]
    fn claude_stream_contract_fixtures_drive_common_adapter_effects() {
        let mut adapter = ClaudeManagedAdapter::new(HarnessAdapterContext {
            session_id: 7,
            thread_id: 11,
            workspace: "fixture-workspace".to_owned(),
            native_session_id: None,
            controls: Default::default(),
        });
        adapter
            .encode_input(HarnessInput {
                local_input_id: "fixture-input".to_owned(),
                content: "fixture".to_owned(),
                visible_content: None,
                kind: crate::archcar::protocol::ArchcarInputKind::User,
                delivery: crate::archcar::protocol::ArchcarInputDelivery::Auto,
            })
            .unwrap();

        let effects = adapter
            .observe_native(NativeRecord {
                provider_key: "claude",
                payload: BASIC_TURN_FIXTURE.as_bytes().to_vec(),
            })
            .unwrap();

        assert!(effects.iter().any(|effect| matches!(
            effect,
            HarnessEffect::Initialized { native_session_id, .. }
                if native_session_id == "session_fixture_basic"
        )));
        assert!(effects.iter().any(|effect| matches!(
            effect,
            HarnessEffect::TurnStarted { local_input_id } if local_input_id == "fixture-input"
        )));
        assert!(effects.iter().any(|effect| matches!(
            effect,
            HarnessEffect::TurnCompleted { local_input_id, status: HarnessTurnStatus::Success }
                if local_input_id == "fixture-input"
        )));
        assert!(effects.iter().any(|effect| matches!(
            effect,
            HarnessEffect::ProviderEvent(event)
                if event.kind == ProviderEventKind::Unknown
                    && event.raw_json["type"] == "future_fixture_record"
        )));
        assert!(effects
            .iter()
            .any(|effect| matches!(effect, HarnessEffect::Ready)));

        let edge_effects = adapter
            .observe_native(NativeRecord {
                provider_key: "claude",
                payload: TOOLS_HOOKS_LIMITS_FIXTURE.as_bytes().to_vec(),
            })
            .unwrap();
        assert!(edge_effects.iter().any(|effect| matches!(
            effect,
            HarnessEffect::ProviderEvent(event)
                if event.raw_json["subtype"] == "hook_response"
        )));
        assert!(edge_effects.iter().any(|effect| matches!(
            effect,
            HarnessEffect::ProviderEvent(event)
                if event.raw_json["subtype"] == "api_retry"
        )));
        assert!(edge_effects.iter().any(|effect| matches!(
            effect,
            HarnessEffect::ProviderEvent(event)
                if event.raw_json["type"] == "rate_limit_event"
        )));
    }

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
            replay_user_messages: false,
            settings_json: None,
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
    fn canonical_conversion_renders_hook_event_details() {
        let events = parse_claude_stream_json_lines(
            r#"{"type":"hook_event","session_id":"s1","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"cargo test"},"message":"approved by hook"}"#,
        )
        .unwrap();

        let canonical = events[0].clone().into_provider_event_draft(
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
            crate::provider_events::ProviderEventKind::SkillPluginHook
        );
        assert_eq!(canonical.provider_subtype.as_deref(), Some("hook_event"));
        assert_eq!(canonical.normalized_payload["title"], "PreToolUse");
        assert_eq!(
            canonical.normalized_payload["body"],
            "Bash\napproved by hook\n{\n  \"command\": \"cargo test\"\n}"
        );
    }

    #[test]
    fn system_hook_events_render_real_sdk_details() {
        let events = parse_claude_stream_json_lines(
            r#"{"type":"system","subtype":"hook_event","session_id":"s1","hook_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"cargo test"},"output":"approved by hook"}"#,
        )
        .unwrap();

        let canonical = events[0].clone().into_provider_event_draft(
            crate::provider_events::ProviderEventContext {
                workspace_id: Some(1),
                chat_thread_id: Some(7),
                process_id: Some(9),
                occurred_at_ms: 42,
                schema_version: 1,
                adapter_version: "claude-stream-json-test".to_owned(),
            },
        );

        assert_eq!(events[0].kind, ClaudeProviderEventKind::Hook);
        assert_eq!(
            canonical.kind,
            crate::provider_events::ProviderEventKind::SkillPluginHook
        );
        assert_eq!(canonical.provider_subtype.as_deref(), Some("hook_event"));
        assert_eq!(canonical.normalized_payload["title"], "PreToolUse");
        assert_eq!(
            canonical.normalized_payload["body"],
            "Bash\napproved by hook\n{\n  \"command\": \"cargo test\"\n}"
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
