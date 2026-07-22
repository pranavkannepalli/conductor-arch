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
use tracing::{debug, info};

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
    pub(crate) tracker: ClaudeTurnTracker,
}

#[derive(Debug, Clone)]
struct TrackedClaudeInput {
    local_input_id: String,
    content: String,
    visible_content: Option<String>,
    turn_id: u64,
    acknowledged: bool,
    started: bool,
}

#[derive(Debug, Default)]
pub(crate) struct ClaudeTurnTracker {
    initialized: bool,
    next_local_turn: u64,
    written_inputs: VecDeque<TrackedClaudeInput>,
    active_turn: Option<TrackedClaudeInput>,
    completed_result_ids: HashSet<(u64, String)>,
}

impl ClaudeTurnTracker {
    pub(crate) fn ready(&self) -> bool {
        self.initialized && self.written_inputs.is_empty() && self.active_turn.is_none()
    }

    fn note_initialized(&mut self) {
        self.initialized = true;
    }

    fn note_input_written(
        &mut self,
        local_input_id: String,
        content: String,
        visible_content: Option<String>,
    ) {
        self.next_local_turn = self.next_local_turn.saturating_add(1);
        self.written_inputs.push_back(TrackedClaudeInput {
            local_input_id,
            content,
            visible_content,
            turn_id: self.next_local_turn,
            acknowledged: false,
            started: false,
        });
    }

    fn settle_failed_input_write(&mut self, local_input_id: &str) {
        self.written_inputs
            .retain(|pending| pending.local_input_id != local_input_id);
        if self
            .active_turn
            .as_ref()
            .is_some_and(|active| active.local_input_id == local_input_id)
        {
            self.active_turn = None;
        }
    }

    fn note_replayed_user(&mut self, text: &str, effects: &mut Vec<HarnessEffect>) {
        if self.active_turn.is_none() && self.initialized {
            let Some(front) = self.written_inputs.front() else {
                return;
            };
            if front.content != text {
                return;
            }
            self.active_turn = self.written_inputs.pop_front();
        }

        let Some(active) = self.active_turn.as_mut() else {
            return;
        };
        if active.content == text && !active.acknowledged {
            active.acknowledged = true;
            effects.push(HarnessEffect::InputAcknowledged {
                local_input_id: active.local_input_id.clone(),
            });
        }
        if active.content == text && !active.started {
            active.started = true;
            effects.push(HarnessEffect::TurnStarted {
                local_input_id: active.local_input_id.clone(),
            });
        }
    }

    fn visible_content_for_replayed_user(&self, text: &str) -> Option<&str> {
        self.active_turn
            .as_ref()
            .filter(|active| active.content == text)
            .and_then(|active| active.visible_content.as_deref())
            .or_else(|| {
                self.written_inputs
                    .front()
                    .filter(|pending| pending.content == text)
                    .and_then(|pending| pending.visible_content.as_deref())
            })
    }

    fn note_provider_turn_started(&mut self, effects: &mut Vec<HarnessEffect>) {
        if self.active_turn.is_none() {
            self.active_turn = self.written_inputs.pop_front();
        }
        if let Some(active) = self.active_turn.as_mut().filter(|active| !active.started) {
            active.started = true;
            effects.push(HarnessEffect::TurnStarted {
                local_input_id: active.local_input_id.clone(),
            });
        }
    }

    fn result_completed(&self, result_id: &str) -> bool {
        if let Some(turn_id) = self
            .active_turn
            .as_ref()
            .map(|active| active.turn_id)
            .or_else(|| self.written_inputs.front().map(|pending| pending.turn_id))
        {
            return self
                .completed_result_ids
                .contains(&(turn_id, result_id.to_owned()));
        }

        self.completed_result_ids
            .iter()
            .any(|(_, completed_id)| completed_id == result_id)
    }

    fn note_terminal_result(
        &mut self,
        result_id: String,
        status: ClaudeResultStatus,
        effects: &mut Vec<HarnessEffect>,
    ) {
        if self.active_turn.is_none() {
            self.active_turn = self.written_inputs.pop_front();
            if let Some(active) = self.active_turn.as_mut().filter(|active| !active.started) {
                active.started = true;
                effects.push(HarnessEffect::TurnStarted {
                    local_input_id: active.local_input_id.clone(),
                });
            }
        }
        let Some(active) = self.active_turn.take() else {
            return;
        };
        if !self
            .completed_result_ids
            .insert((active.turn_id, result_id))
        {
            return;
        }
        effects.push(HarnessEffect::TurnCompleted {
            local_input_id: active.local_input_id,
            status: claude_result_to_harness_status(status),
        });
    }
}

impl ClaudeManagedAdapter {
    pub(crate) fn new(context: HarnessAdapterContext) -> Self {
        Self {
            context,
            parser: ClaudeStreamParser::default(),
            tracker: ClaudeTurnTracker::default(),
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
        self.tracker.settle_failed_input_write(local_input_id);
    }
}

fn claude_result_to_harness_status(status: ClaudeResultStatus) -> HarnessTurnStatus {
    match status {
        ClaudeResultStatus::Success => HarnessTurnStatus::Success,
        ClaudeResultStatus::Failed | ClaudeResultStatus::Declined => HarnessTurnStatus::Failed,
        ClaudeResultStatus::Interrupted => HarnessTurnStatus::Interrupted,
        ClaudeResultStatus::Deferred => HarnessTurnStatus::Deferred,
    }
}

impl ManagedHarnessAdapter for ClaudeManagedAdapter {
    fn encode_input(&mut self, input: HarnessInput) -> Result<NativeWrite> {
        let payload = encode_claude_user_message(&input.content);
        let mut native_payload =
            serde_json::to_vec(&payload).context("serialize Claude stream-json input")?;
        native_payload.push(b'\n');
        self.tracker.note_input_written(
            input.local_input_id.clone(),
            input.content,
            input.visible_content,
        );

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
            let lifecycle_signal = event.lifecycle_signal();
            log_claude_stream_event(&event, lifecycle_signal.as_ref());
            let terminal_result_id = matches!(event_kind, ClaudeProviderEventKind::Result)
                .then(|| claude_terminal_result_id(&event));
            if terminal_result_id
                .as_deref()
                .is_some_and(|result_id| self.tracker.result_completed(result_id))
            {
                continue;
            }
            let replayed_input = matches!(
                &lifecycle_signal,
                Some(ClaudeLifecycleSignal::UserInputReplayed { .. })
            );
            let visible_replayed_input = if let Some(ClaudeLifecycleSignal::UserInputReplayed {
                text,
            }) = &lifecycle_signal
            {
                self.tracker.visible_content_for_replayed_user(text)
            } else {
                None
            }
            .map(str::to_owned);
            let retry_message = string_at(&event.raw_json, &["error"]);
            let retry_delay_ms = number_at(&event.raw_json, &["retry_delay_ms"]);
            let limit_message = rate_limit_message(&event.raw_json);
            let limit_retry_after_ms = rate_limit_retry_after_ms(&event.raw_json);
            let limit_is_allowed = rate_limit_is_allowed(&event.raw_json);
            let limit_is_terminal = rate_limit_is_terminal(&event.raw_json);
            let mut provider_event_drafts = event
                .clone()
                .into_provider_event_drafts(self.provider_context());
            if let Some(visible_input) = visible_replayed_input.as_deref() {
                for draft in provider_event_drafts.iter_mut() {
                    if draft.kind == ProviderEventKind::UserInput {
                        draft.normalized_payload["body"] = json!(visible_input);
                    }
                }
            }
            if !(event_kind == ClaudeProviderEventKind::RateLimit && limit_is_allowed) {
                effects.extend(
                    provider_event_drafts
                        .into_iter()
                        .map(HarnessEffect::ProviderEvent),
                );
            }

            match lifecycle_signal {
                Some(ClaudeLifecycleSignal::Initialized(init)) => {
                    self.tracker.note_initialized();
                    self.context.native_session_id = Some(init.session_id.clone());
                    effects.push(HarnessEffect::Initialized {
                        native_session_id: init.session_id,
                        model: init.model,
                    });
                    effects.push(HarnessEffect::CapabilitiesObserved(init.capabilities));
                    if self.tracker.ready() {
                        effects.push(HarnessEffect::Ready);
                    }
                }
                Some(ClaudeLifecycleSignal::UserInputReplayed { text }) => {
                    self.tracker.note_replayed_user(&text, &mut effects);
                }
                Some(ClaudeLifecycleSignal::TurnFinished {
                    status,
                    stop_reason,
                }) => {
                    let _ = stop_reason;
                    if let Some(message) = claude_terminal_failure_message(&event) {
                        effects.push(HarnessEffect::Fatal(message));
                    }
                    self.tracker.note_terminal_result(
                        terminal_result_id
                            .unwrap_or_else(|| claude_fallback_event_id(&event.raw_json)),
                        status,
                        &mut effects,
                    );
                    if self.tracker.ready() {
                        effects.push(HarnessEffect::Ready);
                    }
                }
                Some(ClaudeLifecycleSignal::DeferredTool { tool_use }) => {
                    let _ = tool_use;
                }
                None => {}
            }

            if matches!(
                event_kind,
                ClaudeProviderEventKind::UserMessage | ClaudeProviderEventKind::MessageStart
            ) && !replayed_input
            {
                self.tracker.note_provider_turn_started(&mut effects);
            }

            if event_kind == ClaudeProviderEventKind::ApiRetry {
                effects.push(HarnessEffect::Retry {
                    message: retry_message.unwrap_or_else(|| "Claude API retry".to_owned()),
                    delay_ms: retry_delay_ms,
                });
            } else if event_kind == ClaudeProviderEventKind::RateLimit && !limit_is_allowed {
                effects.push(HarnessEffect::RateLimited {
                    message: limit_message,
                    retry_after_ms: limit_retry_after_ms,
                });
                if limit_is_terminal {
                    self.tracker.note_terminal_result(
                        claude_fallback_event_id(&event.raw_json),
                        ClaudeResultStatus::Failed,
                        &mut effects,
                    );
                    if self.tracker.ready() {
                        effects.push(HarnessEffect::Ready);
                    }
                }
            }
        }

        Ok(effects)
    }

    fn plan_control(&mut self, control: HarnessControl) -> HarnessControlPlan {
        match control {
            HarnessControl::Kill => {
                HarnessControlPlan::Signal(HarnessSignal::TerminateProcessGroup)
            }
            HarnessControl::Interrupt => {
                HarnessControlPlan::Signal(HarnessSignal::InterruptProcessGroup)
            }
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
            HarnessControl::ResolveInteraction(_) => {
                HarnessControlPlan::RestartRequired(self.context.controls.clone())
            }
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

fn claude_terminal_result_id(event: &ClaudeProviderEventDraft) -> String {
    event
        .provider_event_id
        .clone()
        .unwrap_or_else(|| claude_fallback_event_id(&event.raw_json))
}

fn log_claude_stream_event(
    event: &ClaudeProviderEventDraft,
    lifecycle_signal: Option<&ClaudeLifecycleSignal>,
) {
    match lifecycle_signal {
        Some(ClaudeLifecycleSignal::Initialized(init)) => {
            info!(
                native_session_id = %init.session_id,
                model = init.model.as_deref().unwrap_or("unknown"),
                capabilities = ?init.capabilities,
                "claude stream initialized"
            );
        }
        Some(ClaudeLifecycleSignal::UserInputReplayed { text }) => {
            info!(
                provider_event_id = event.provider_event_id.as_deref().unwrap_or("unknown"),
                chars = text.chars().count(),
                "claude stream replayed user input"
            );
        }
        Some(ClaudeLifecycleSignal::TurnFinished {
            status,
            stop_reason,
        }) => {
            info!(
                provider_event_id = event.provider_event_id.as_deref().unwrap_or("unknown"),
                status = ?status,
                stop_reason = stop_reason.as_deref().unwrap_or("none"),
                result_chars = event
                    .content_delta
                    .as_ref()
                    .map(|text| text.chars().count())
                    .unwrap_or(0),
                "claude stream turn finished"
            );
        }
        Some(ClaudeLifecycleSignal::DeferredTool { .. }) => {
            info!(
                provider_event_id = event.provider_event_id.as_deref().unwrap_or("unknown"),
                "claude stream deferred tool result"
            );
        }
        None => {}
    }

    if event.kind == ClaudeProviderEventKind::Reasoning {
        info!(
            provider_event_id = event.provider_event_id.as_deref().unwrap_or("unknown"),
            provider_message_id = event.provider_message_id.as_deref().unwrap_or("unknown"),
            content_block_index = ?event.content_block_index,
            subtype = event.subtype.as_deref().unwrap_or("unknown"),
            chars = event
                .content_delta
                .as_ref()
                .map(|text| text.chars().count())
                .unwrap_or(0),
            "claude stream thinking trace observed"
        );
    } else if let Some(thinking_tokens) = event.usage.thinking_tokens {
        info!(
            provider_event_id = event.provider_event_id.as_deref().unwrap_or("unknown"),
            kind = ?event.kind,
            thinking_tokens,
            "claude stream thinking token usage"
        );
    } else if event.kind != ClaudeProviderEventKind::Hook {
        debug!(
            provider_event_id = event.provider_event_id.as_deref().unwrap_or("unknown"),
            kind = ?event.kind,
            subtype = event.subtype.as_deref().unwrap_or("unknown"),
            content_chars = event
                .content_delta
                .as_ref()
                .map(|text| text.chars().count())
                .unwrap_or(0),
            "claude stream event"
        );
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
pub enum ClaudeProviderEventKind {
    Initialization,
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
    ApiRetry,
    RateLimit,
    PromptSuggestion,
    Reasoning,
    DeferredResult,
    Result,
    Error,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeInitMetadata {
    pub session_id: String,
    pub model: Option<String>,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudeResultStatus {
    Success,
    Failed,
    Interrupted,
    Declined,
    Deferred,
}

#[derive(Debug, Clone, PartialEq)]
enum ClaudeLifecycleSignal {
    Initialized(ClaudeInitMetadata),
    UserInputReplayed {
        text: String,
    },
    TurnFinished {
        status: ClaudeResultStatus,
        stop_reason: Option<String>,
    },
    DeferredTool {
        tool_use: Value,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeUsageDraft {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u64>,
}

impl ClaudeUsageDraft {
    fn is_empty(&self) -> bool {
        self.input_tokens.is_none()
            && self.output_tokens.is_none()
            && self.thinking_tokens.is_none()
            && self.cache_creation_input_tokens.is_none()
            && self.cache_read_input_tokens.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ClaudeReasoningBlockDraft {
    content_block_index: u64,
    thinking: String,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    reasoning_blocks: Vec<ClaudeReasoningBlockDraft>,
    pub raw_json: Value,
}

impl ClaudeProviderEventDraft {
    fn lifecycle_signal(&self) -> Option<ClaudeLifecycleSignal> {
        match self.kind {
            ClaudeProviderEventKind::Initialization => {
                Some(ClaudeLifecycleSignal::Initialized(ClaudeInitMetadata {
                    session_id: self.session_id.clone()?,
                    model: string_at(&self.raw_json, &["model"]),
                    capabilities: string_array_at(&self.raw_json, &["capabilities"]),
                }))
            }
            ClaudeProviderEventKind::UserMessage
                if self
                    .raw_json
                    .get("isReplay")
                    .and_then(Value::as_bool)
                    .unwrap_or(false) =>
            {
                Some(ClaudeLifecycleSignal::UserInputReplayed {
                    text: message_content_text(&self.raw_json, "text", "text")?,
                })
            }
            ClaudeProviderEventKind::Result => Some(ClaudeLifecycleSignal::TurnFinished {
                status: self.result_status()?,
                stop_reason: string_at(&self.raw_json, &["stop_reason"]),
            }),
            ClaudeProviderEventKind::DeferredResult => Some(ClaudeLifecycleSignal::DeferredTool {
                tool_use: self.raw_json.get("tool_use_result")?.clone(),
            }),
            _ => None,
        }
    }

    pub fn result_status(&self) -> Option<ClaudeResultStatus> {
        if self.kind == ClaudeProviderEventKind::DeferredResult {
            return Some(ClaudeResultStatus::Deferred);
        }
        if self.kind != ClaudeProviderEventKind::Result {
            return None;
        }
        Some(claude_result_status_from_json(&self.raw_json))
    }

    pub fn into_provider_event_drafts(
        mut self,
        context: ProviderEventContext,
    ) -> Vec<ProviderEventDraft> {
        let reasoning_blocks = std::mem::take(&mut self.reasoning_blocks);
        let reasoning_base = self.clone();
        let mut drafts = vec![self.into_provider_event_draft(context.clone())];
        drafts.extend(reasoning_blocks.into_iter().map(|block| {
            ClaudeProviderEventDraft {
                provider: reasoning_base.provider.clone(),
                kind: ClaudeProviderEventKind::Reasoning,
                provider_event_id: reasoning_base
                    .provider_event_id
                    .as_ref()
                    .map(|event_id| format!("{event_id}:reasoning:{}", block.content_block_index)),
                session_id: reasoning_base.session_id.clone(),
                parent_tool_use_id: reasoning_base.parent_tool_use_id.clone(),
                provider_message_id: reasoning_base.provider_message_id.clone(),
                provider_tool_use_id: None,
                content_block_index: Some(block.content_block_index),
                tool_name: None,
                content_delta: Some(block.thinking),
                cost_usd: reasoning_base.cost_usd,
                duration_ms: reasoning_base.duration_ms,
                subtype: Some("thinking".to_owned()),
                usage: ClaudeUsageDraft::default(),
                reasoning_blocks: Vec::new(),
                raw_json: reasoning_base.raw_json.clone(),
            }
            .into_provider_event_draft(context.clone())
        }));
        drafts
    }

    pub fn into_provider_event_draft(self, context: ProviderEventContext) -> ProviderEventDraft {
        let kind = claude_kind_to_provider_kind(self.kind);
        let phase = claude_phase_for(self.kind, &self.raw_json);
        let provider_event_id = self.provider_event_id.clone();
        let provider_item_id = match self.kind {
            ClaudeProviderEventKind::ToolUse
            | ClaudeProviderEventKind::ToolInputDelta
            | ClaudeProviderEventKind::ToolResult
            | ClaudeProviderEventKind::DeferredResult => self.provider_tool_use_id.clone(),
            ClaudeProviderEventKind::Reasoning => {
                self.provider_message_id.as_ref().map(|message_id| {
                    format!(
                        "{message_id}:reasoning:{}",
                        self.content_block_index.unwrap_or_default()
                    )
                })
            }
            ClaudeProviderEventKind::AssistantMessage
            | ClaudeProviderEventKind::MessageStart
            | ClaudeProviderEventKind::MessageDelta
            | ClaudeProviderEventKind::MessageStop
            | ClaudeProviderEventKind::ContentBlockStart
            | ClaudeProviderEventKind::ContentBlockDelta
            | ClaudeProviderEventKind::ContentBlockStop
            | ClaudeProviderEventKind::UserMessage => self.provider_message_id.clone(),
            _ => None,
        };
        let body = if self.kind == ClaudeProviderEventKind::Hook {
            claude_hook_body(&self.raw_json).unwrap_or_default()
        } else {
            self.content_delta
                .clone()
                .or_else(|| string_at(&self.raw_json, &["result"]))
                .unwrap_or_default()
        };
        let stream_delta = (phase == ProviderEventPhase::Delta)
            .then(|| self.content_delta.clone())
            .flatten();
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
                "stream_delta": stream_delta,
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
    reasoning_blocks: HashSet<u64>,
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
        let kind = self.kind_for(&raw_json);
        let mut draft = ClaudeProviderEventDraft {
            provider: CLAUDE_PROVIDER_NAME.to_owned(),
            kind,
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
            reasoning_blocks: if kind == ClaudeProviderEventKind::AssistantMessage {
                reasoning_blocks_from(&raw_json)
            } else {
                Vec::new()
            },
            raw_json,
        };

        self.apply_identity_state(&mut draft);
        if draft.provider_event_id.is_none() {
            draft.provider_event_id = Some(claude_fallback_event_id(&draft.raw_json));
        }
        draft
    }

    fn kind_for(&self, value: &Value) -> ClaudeProviderEventKind {
        if claude_system_hook_event(value) {
            return ClaudeProviderEventKind::Hook;
        }
        let top_type = string_at(value, &["type"]);
        let subtype = string_at(value, &["subtype"]);
        if top_type.as_deref() == Some("system") && subtype.as_deref() == Some("init") {
            return ClaudeProviderEventKind::Initialization;
        }
        if top_type.as_deref() == Some("system") && subtype.as_deref() == Some("api_retry") {
            return ClaudeProviderEventKind::ApiRetry;
        }
        if matches!(
            (top_type.as_deref(), subtype.as_deref()),
            (Some("prompt_suggestion"), _) | (Some("system"), Some("prompt_suggestion"))
        ) {
            return ClaudeProviderEventKind::PromptSuggestion;
        }
        if matches!(
            top_type.as_deref(),
            Some("rate_limit_event" | "rate_limit" | "context_limit")
        ) {
            return ClaudeProviderEventKind::RateLimit;
        }
        if top_type.as_deref() == Some("user") && deferred_tool_result(value).is_some() {
            return ClaudeProviderEventKind::DeferredResult;
        }
        if top_type.as_deref() == Some("user") && message_has_block_type(value, "tool_result") {
            return ClaudeProviderEventKind::ToolResult;
        }
        if top_type.as_deref() == Some("assistant")
            && message_has_block_type(value, "thinking")
            && !message_has_block_type(value, "text")
        {
            return ClaudeProviderEventKind::Reasoning;
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
                        Some("thinking") => ClaudeProviderEventKind::Reasoning,
                        _ => ClaudeProviderEventKind::ContentBlockStart,
                    }
                }
                Some("content_block_delta") => {
                    match string_at(value, &["event", "delta", "type"]).as_deref() {
                        Some("input_json_delta") => ClaudeProviderEventKind::ToolInputDelta,
                        Some("thinking_delta") => ClaudeProviderEventKind::Reasoning,
                        _ => ClaudeProviderEventKind::ContentBlockDelta,
                    }
                }
                Some("content_block_stop") => {
                    let index = number_at(value, &["event", "index"]);
                    if index.is_some_and(|index| self.tool_use_by_block.contains_key(&index)) {
                        ClaudeProviderEventKind::ToolResult
                    } else if index.is_some_and(|index| self.reasoning_blocks.contains(&index)) {
                        ClaudeProviderEventKind::Reasoning
                    } else {
                        ClaudeProviderEventKind::ContentBlockStop
                    }
                }
                Some("error") => ClaudeProviderEventKind::Error,
                _ => ClaudeProviderEventKind::Unknown,
            },
            Some("tool_result") => ClaudeProviderEventKind::ToolResult,
            Some("subagent") | Some("task") => ClaudeProviderEventKind::Subagent,
            Some("usage") => ClaudeProviderEventKind::Usage,
            _ => ClaudeProviderEventKind::Unknown,
        }
    }

    fn apply_identity_state(&mut self, draft: &mut ClaudeProviderEventDraft) {
        match draft.kind {
            ClaudeProviderEventKind::Initialization => {
                self.current_message_id = None;
                self.tool_use_by_block.clear();
                self.reasoning_blocks.clear();
            }
            ClaudeProviderEventKind::MessageStart => {
                draft.provider_message_id = string_at(&draft.raw_json, &["event", "message", "id"]);
                self.current_message_id = draft.provider_message_id.clone();
                self.tool_use_by_block.clear();
                self.reasoning_blocks.clear();
            }
            ClaudeProviderEventKind::AssistantMessage
            | ClaudeProviderEventKind::UserMessage
            | ClaudeProviderEventKind::ToolResult
            | ClaudeProviderEventKind::DeferredResult => {
                draft.provider_message_id = string_at(&draft.raw_json, &["message", "id"]);
            }
            ClaudeProviderEventKind::MessageDelta
            | ClaudeProviderEventKind::MessageStop
            | ClaudeProviderEventKind::ContentBlockStart
            | ClaudeProviderEventKind::ContentBlockDelta
            | ClaudeProviderEventKind::ContentBlockStop
            | ClaudeProviderEventKind::Reasoning => {
                draft.provider_message_id = self.current_message_id.clone();
            }
            _ => {}
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
            } else if draft.kind == ClaudeProviderEventKind::Reasoning
                && string_at(&draft.raw_json, &["event", "type"]).as_deref()
                    == Some("content_block_start")
            {
                self.reasoning_blocks.insert(index);
            } else if let Some(state) = self.tool_use_by_block.get(&index) {
                draft.provider_tool_use_id = state.id.clone();
                draft.tool_name = state.name.clone();
            }
        }

        if matches!(
            draft.kind,
            ClaudeProviderEventKind::ToolResult | ClaudeProviderEventKind::DeferredResult
        ) {
            let message_tool_use_id = draft
                .raw_json
                .pointer("/message/content")
                .and_then(Value::as_array)
                .and_then(|content| {
                    content.iter().find_map(|block| {
                        (block.get("type").and_then(Value::as_str) == Some("tool_result"))
                            .then(|| {
                                block
                                    .get("tool_use_id")
                                    .and_then(Value::as_str)
                                    .map(ToOwned::to_owned)
                            })
                            .flatten()
                    })
                });
            if message_tool_use_id.is_some() {
                draft.provider_tool_use_id = message_tool_use_id;
            }
        }

        draft.content_delta = match draft.kind {
            ClaudeProviderEventKind::ToolInputDelta => {
                string_at(&draft.raw_json, &["event", "delta", "partial_json"])
            }
            ClaudeProviderEventKind::ContentBlockDelta => {
                string_at(&draft.raw_json, &["event", "delta", "text"])
            }
            ClaudeProviderEventKind::Reasoning => {
                string_at(&draft.raw_json, &["event", "delta", "thinking"])
                    .or_else(|| message_content_text(&draft.raw_json, "thinking", "thinking"))
            }
            ClaudeProviderEventKind::AssistantMessage => {
                message_content_text(&draft.raw_json, "text", "text")
            }
            ClaudeProviderEventKind::UserMessage => {
                message_content_text(&draft.raw_json, "text", "text")
            }
            ClaudeProviderEventKind::ToolResult | ClaudeProviderEventKind::DeferredResult => {
                message_content_text(&draft.raw_json, "tool_result", "content")
            }
            ClaudeProviderEventKind::Result => string_at(&draft.raw_json, &["result"]),
            _ => None,
        };

        if string_at(&draft.raw_json, &["event", "type"]).as_deref() == Some("content_block_stop") {
            if let Some(index) = draft.content_block_index {
                self.tool_use_by_block.remove(&index);
                self.reasoning_blocks.remove(&index);
            }
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
        thinking_tokens: number_at(usage, &["output_tokens_details", "thinking_tokens"])
            .or_else(|| number_at(usage, &["thinking_tokens"])),
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
        .and_then(|value| {
            value.as_u64().or_else(|| {
                value.as_f64().and_then(|number| {
                    (number.is_finite() && number >= 0.0 && number <= u64::MAX as f64)
                        .then_some(number as u64)
                })
            })
        })
}

fn claude_fallback_event_id(value: &Value) -> String {
    let hash = value
        .to_string()
        .bytes()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
    let native_type = string_at(value, &["type"]).unwrap_or_else(|| "unknown".to_owned());
    let subtype = string_at(value, &["subtype"]);
    match subtype {
        Some(subtype) => format!("{native_type}:{subtype}:{hash:016x}"),
        None => format!("{native_type}:{hash:016x}"),
    }
}

fn string_array_at(value: &Value, path: &[&str]) -> Vec<String> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn message_content_text(value: &Value, block_type: &str, field: &str) -> Option<String> {
    let text = value
        .pointer("/message/content")?
        .as_array()?
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some(block_type))
        .filter_map(|block| block.get(field).and_then(Value::as_str))
        .collect::<String>();
    (!text.is_empty()).then_some(text)
}

fn reasoning_blocks_from(value: &Value) -> Vec<ClaudeReasoningBlockDraft> {
    value
        .pointer("/message/content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, block)| {
            (block.get("type").and_then(Value::as_str) == Some("thinking"))
                .then(|| {
                    block
                        .get("thinking")
                        .and_then(Value::as_str)
                        .filter(|thinking| !thinking.is_empty())
                        .map(|thinking| ClaudeReasoningBlockDraft {
                            content_block_index: index as u64,
                            thinking: thinking.to_owned(),
                        })
                })
                .flatten()
        })
        .collect()
}

fn message_has_block_type(value: &Value, block_type: &str) -> bool {
    value
        .pointer("/message/content")
        .and_then(Value::as_array)
        .is_some_and(|content| {
            content
                .iter()
                .any(|block| block.get("type").and_then(Value::as_str) == Some(block_type))
        })
}

fn deferred_tool_result(value: &Value) -> Option<&Value> {
    value
        .get("tool_use_result")
        .filter(|result| result.get("type").and_then(Value::as_str) == Some("tool_deferred"))
}

fn rate_limit_message(value: &Value) -> String {
    let status = string_at(value, &["rate_limit_info", "status"])
        .unwrap_or_else(|| "rate limited".to_owned());
    let limit_type = string_at(value, &["rate_limit_info", "rateLimitType"]);
    match limit_type {
        Some(limit_type) => format!("Claude {limit_type} limit: {status}"),
        None => format!("Claude rate limit: {status}"),
    }
}

fn rate_limit_retry_after_ms(value: &Value) -> Option<u64> {
    number_at(value, &["rate_limit_info", "retry_after_ms"])
        .or_else(|| number_at(value, &["retry_after_ms"]))
}

fn rate_limit_is_allowed(value: &Value) -> bool {
    string_at(value, &["rate_limit_info", "status"])
        .is_some_and(|status| status.eq_ignore_ascii_case("allowed"))
}

fn rate_limit_is_terminal(value: &Value) -> bool {
    !rate_limit_is_allowed(value)
}

fn claude_result_status_from_json(value: &Value) -> ClaudeResultStatus {
    let subtype = value
        .get("subtype")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if matches!(subtype, "interrupted" | "cancelled" | "canceled") {
        ClaudeResultStatus::Interrupted
    } else if matches!(subtype, "declined" | "denied" | "rejected") {
        ClaudeResultStatus::Declined
    } else if matches!(subtype, "deferred" | "tool_deferred") {
        ClaudeResultStatus::Deferred
    } else if value
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || matches!(subtype, "error" | "failed" | "error_during_execution")
    {
        ClaudeResultStatus::Failed
    } else {
        ClaudeResultStatus::Success
    }
}

fn claude_terminal_failure_message(event: &ClaudeProviderEventDraft) -> Option<String> {
    if event.kind != ClaudeProviderEventKind::Result {
        return None;
    }
    if event.result_status()? != ClaudeResultStatus::Failed {
        return None;
    }
    let message = string_at(&event.raw_json, &["result"])
        .or_else(|| string_at(&event.raw_json, &["error"]))
        .unwrap_or_else(|| "Claude Code returned a failed result.".to_owned());
    if claude_auth_failure(&event.raw_json) {
        return Some(format!(
            "Claude is not logged in. Run `claude auth login`, then try again. {message}"
        ));
    }
    Some(message)
}

fn claude_auth_failure(value: &Value) -> bool {
    number_at(value, &["api_error_status"]) == Some(401)
        || number_at(value, &["error_status"]) == Some(401)
        || string_at(value, &["error"]).is_some_and(|error| {
            let error = error.to_ascii_lowercase();
            error.contains("authentication")
                || error.contains("unauthorized")
                || error.contains("auth")
        })
}

fn claude_system_hook_event(value: &Value) -> bool {
    string_at(value, &["subtype"]).is_some_and(|subtype| subtype.starts_with("hook_"))
        || string_at(value, &["hook_event"]).is_some()
        || string_at(value, &["hook_name"]).is_some()
        || string_at(value, &["hook_event_name"]).is_some()
}

fn claude_kind_to_provider_kind(kind: ClaudeProviderEventKind) -> ProviderEventKind {
    match kind {
        ClaudeProviderEventKind::Initialization | ClaudeProviderEventKind::Session => {
            ProviderEventKind::ThreadSession
        }
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
        ClaudeProviderEventKind::Usage
        | ClaudeProviderEventKind::ApiRetry
        | ClaudeProviderEventKind::RateLimit
        | ClaudeProviderEventKind::Error => ProviderEventKind::LimitFailure,
        ClaudeProviderEventKind::PromptSuggestion => ProviderEventKind::GoalTask,
        ClaudeProviderEventKind::Reasoning => ProviderEventKind::PlanningReasoning,
        ClaudeProviderEventKind::DeferredResult => ProviderEventKind::Tool,
        ClaudeProviderEventKind::Result => ProviderEventKind::Turn,
        ClaudeProviderEventKind::Unknown => ProviderEventKind::Unknown,
    }
}

fn claude_phase_for(kind: ClaudeProviderEventKind, raw_json: &Value) -> ProviderEventPhase {
    match kind {
        ClaudeProviderEventKind::MessageStart
        | ClaudeProviderEventKind::ContentBlockStart
        | ClaudeProviderEventKind::ToolUse
        | ClaudeProviderEventKind::Initialization
        | ClaudeProviderEventKind::Session
        | ClaudeProviderEventKind::UserMessage => ProviderEventPhase::Started,
        ClaudeProviderEventKind::MessageDelta
        | ClaudeProviderEventKind::ContentBlockDelta
        | ClaudeProviderEventKind::ToolInputDelta => ProviderEventPhase::Delta,
        ClaudeProviderEventKind::Reasoning
            if string_at(raw_json, &["event", "type"]).as_deref()
                == Some("content_block_delta") =>
        {
            ProviderEventPhase::Delta
        }
        ClaudeProviderEventKind::Reasoning
            if string_at(raw_json, &["event", "type"]).as_deref() == Some("content_block_stop") =>
        {
            ProviderEventPhase::Completed
        }
        ClaudeProviderEventKind::Reasoning
            if string_at(raw_json, &["type"]).as_deref() == Some("assistant") =>
        {
            ProviderEventPhase::Completed
        }
        ClaudeProviderEventKind::Reasoning => ProviderEventPhase::Started,
        ClaudeProviderEventKind::Permission
        | ClaudeProviderEventKind::Hook
        | ClaudeProviderEventKind::Subagent
        | ClaudeProviderEventKind::Usage
        | ClaudeProviderEventKind::ApiRetry
        | ClaudeProviderEventKind::PromptSuggestion => ProviderEventPhase::Progress,
        ClaudeProviderEventKind::RateLimit
            if string_at(raw_json, &["rate_limit_info", "status"])
                .is_some_and(|status| status.contains("allowed")) =>
        {
            ProviderEventPhase::Progress
        }
        ClaudeProviderEventKind::RateLimit => ProviderEventPhase::Failed,
        ClaudeProviderEventKind::MessageStop
        | ClaudeProviderEventKind::ContentBlockStop
        | ClaudeProviderEventKind::ToolResult
        | ClaudeProviderEventKind::AssistantMessage => ProviderEventPhase::Completed,
        ClaudeProviderEventKind::DeferredResult => ProviderEventPhase::Declined,
        ClaudeProviderEventKind::Result => match claude_result_status_from_json(raw_json) {
            ClaudeResultStatus::Success => ProviderEventPhase::Completed,
            ClaudeResultStatus::Failed => ProviderEventPhase::Failed,
            ClaudeResultStatus::Interrupted => ProviderEventPhase::Interrupted,
            ClaudeResultStatus::Declined | ClaudeResultStatus::Deferred => {
                ProviderEventPhase::Declined
            }
        },
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
        ClaudeProviderEventKind::ApiRetry => "API retry".to_owned(),
        ClaudeProviderEventKind::RateLimit => "Rate limit".to_owned(),
        ClaudeProviderEventKind::PromptSuggestion => "Prompt suggestion".to_owned(),
        ClaudeProviderEventKind::Reasoning => "Reasoning".to_owned(),
        ClaudeProviderEventKind::DeferredResult => "Deferred tool".to_owned(),
        ClaudeProviderEventKind::Result => "Turn result".to_owned(),
        ClaudeProviderEventKind::Initialization => "Session initialized".to_owned(),
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

        assert!(pending_adapter.tracker.written_inputs.is_empty());
        assert!(pending_adapter.tracker.active_turn.is_none());

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

        assert!(active_adapter.tracker.written_inputs.is_empty());
        assert!(active_adapter.tracker.active_turn.is_none());
    }

    #[test]
    fn terminal_rate_limit_waits_for_all_inputs_before_ready() {
        let mut adapter = ClaudeManagedAdapter::new(HarnessAdapterContext {
            session_id: 7,
            thread_id: 11,
            workspace: "fixture-workspace".to_owned(),
            native_session_id: Some("fixture-session".to_owned()),
            controls: Default::default(),
        });
        adapter.tracker.initialized = true;
        adapter
            .encode_input(HarnessInput {
                local_input_id: "active-input".to_owned(),
                content: "active".to_owned(),
                visible_content: None,
                kind: crate::archcar::protocol::ArchcarInputKind::User,
                delivery: crate::archcar::protocol::ArchcarInputDelivery::Auto,
            })
            .unwrap();
        adapter
            .encode_input(HarnessInput {
                local_input_id: "queued-input".to_owned(),
                content: "queued".to_owned(),
                visible_content: None,
                kind: crate::archcar::protocol::ArchcarInputKind::User,
                delivery: crate::archcar::protocol::ArchcarInputDelivery::Auto,
            })
            .unwrap();

        let effects = adapter
            .observe_native(NativeRecord {
                provider_key: "claude",
                payload: br#"{"type":"rate_limit_event","session_id":"fixture-session","uuid":"rate_limit_terminal","rate_limit_info":{"status":"blocked","rateLimitType":"five_hour"}}
"#
                .to_vec(),
            })
            .unwrap();

        assert!(effects
            .iter()
            .any(|effect| matches!(effect, HarnessEffect::RateLimited { .. })));
        assert!(effects.iter().any(|effect| matches!(
            effect,
            HarnessEffect::TurnCompleted {
                local_input_id,
                status: HarnessTurnStatus::Failed,
            } if local_input_id == "active-input"
        )));
        assert!(!effects
            .iter()
            .any(|effect| matches!(effect, HarnessEffect::Ready)));
        assert_eq!(adapter.tracker.written_inputs.len(), 1);
    }

    #[test]
    fn claude_auth_failure_result_emits_fatal_session_error() {
        let mut adapter = ClaudeManagedAdapter::new(HarnessAdapterContext {
            session_id: 7,
            thread_id: 11,
            workspace: "fixture-workspace".to_owned(),
            native_session_id: Some("fixture-session".to_owned()),
            controls: Default::default(),
        });
        adapter.tracker.initialized = true;
        adapter
            .encode_input(HarnessInput {
                local_input_id: "auth-input".to_owned(),
                content: "hello".to_owned(),
                visible_content: None,
                kind: crate::archcar::protocol::ArchcarInputKind::User,
                delivery: crate::archcar::protocol::ArchcarInputDelivery::Auto,
            })
            .unwrap();

        let effects = adapter
            .observe_native(NativeRecord {
                provider_key: "claude",
                payload: br#"{"type":"result","subtype":"success","is_error":true,"api_error_status":401,"error":"authentication_failed","result":"Failed to authenticate. API Error: 401 OAuth access token has expired. Re-authenticate to continue.","session_id":"fixture-session","uuid":"auth-result"}
"#
                .to_vec(),
            })
            .unwrap();

        assert!(effects.iter().any(|effect| matches!(
            effect,
            HarnessEffect::Fatal(message)
                if message.contains("Claude is not logged in")
                    && message.contains("claude auth login")
                    && message.contains("Failed to authenticate")
                    && message.contains("Re-authenticate")
        )));
        assert!(effects.iter().any(|effect| matches!(
            effect,
            HarnessEffect::TurnCompleted {
                local_input_id,
                status: HarnessTurnStatus::Failed,
            } if local_input_id == "auth-input"
        )));
    }

    #[test]
    fn claude_api_retry_accepts_fractional_retry_delay_ms() {
        let mut adapter = ClaudeManagedAdapter::new(HarnessAdapterContext {
            session_id: 7,
            thread_id: 11,
            workspace: "fixture-workspace".to_owned(),
            native_session_id: Some("fixture-session".to_owned()),
            controls: Default::default(),
        });

        let effects = adapter
            .observe_native(NativeRecord {
                provider_key: "claude",
                payload: br#"{"type":"system","subtype":"api_retry","attempt":1,"max_retries":10,"retry_delay_ms":562.276368885656,"error_status":401,"error":"authentication_failed","session_id":"fixture-session","uuid":"retry-1"}
"#
                .to_vec(),
            })
            .unwrap();

        assert!(effects.iter().any(|effect| matches!(
            effect,
            HarnessEffect::Retry {
                message,
                delay_ms: Some(562),
            } if message == "authentication_failed"
        )));
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
    fn claude_fixture_classifies_native_lifecycle_and_output_records() {
        let basic = parse_claude_stream_json_lines(BASIC_TURN_FIXTURE).unwrap();
        let tools = parse_claude_stream_json_lines(TOOLS_HOOKS_LIMITS_FIXTURE).unwrap();
        let results = parse_claude_stream_json_lines(DEFERRED_AND_FAILED_RESULTS_FIXTURE).unwrap();

        let init = basic
            .iter()
            .find(|event| event.raw_json["subtype"] == "init")
            .unwrap();
        assert!(matches!(
            init.lifecycle_signal(),
            Some(ClaudeLifecycleSignal::Initialized(ClaudeInitMetadata {
                ref session_id,
                ref model,
                ref capabilities,
            })) if session_id == "session_fixture_basic"
                && model.as_deref() == Some("claude-sonnet-fixture")
                && capabilities == &["streaming"]
        ));

        let rate_limit = tools
            .iter()
            .find(|event| event.raw_json["type"] == "rate_limit_event")
            .unwrap();
        let api_retry = tools
            .iter()
            .find(|event| event.raw_json["subtype"] == "api_retry")
            .unwrap();
        let deferred = results
            .iter()
            .find(|event| event.raw_json["tool_use_result"]["type"] == "tool_deferred")
            .unwrap();
        let final_assistant = basic
            .iter()
            .find(|event| event.kind == ClaudeProviderEventKind::AssistantMessage)
            .unwrap();
        let unknown = parse_claude_stream_json_lines(
            r#"{"type":"future_record","session_id":"future-session","future_field":42}"#,
        )
        .unwrap()
        .pop()
        .unwrap();

        assert_eq!(rate_limit.kind, ClaudeProviderEventKind::RateLimit);
        assert_eq!(api_retry.kind, ClaudeProviderEventKind::ApiRetry);
        assert_eq!(deferred.result_status(), Some(ClaudeResultStatus::Deferred));
        assert_eq!(
            final_assistant.content_delta.as_deref(),
            Some("Fixture complete.")
        );
        assert_eq!(unknown.raw_json["future_field"], 42);
    }

    #[test]
    fn claude_fixture_non_item_records_use_stable_event_identity_only() {
        let input = format!("{BASIC_TURN_FIXTURE}\n{TOOLS_HOOKS_LIMITS_FIXTURE}");
        let events = parse_claude_stream_json_lines(&input).unwrap();

        for event in events.iter().filter(|event| {
            matches!(
                event.kind,
                ClaudeProviderEventKind::Result
                    | ClaudeProviderEventKind::ApiRetry
                    | ClaudeProviderEventKind::RateLimit
                    | ClaudeProviderEventKind::Hook
                    | ClaudeProviderEventKind::Unknown
            )
        }) {
            let canonical = event
                .clone()
                .into_provider_event_draft(ProviderEventContext {
                    workspace_id: None,
                    chat_thread_id: None,
                    process_id: None,
                    occurred_at_ms: 1,
                    schema_version: 1,
                    adapter_version: "test".to_owned(),
                });
            assert!(canonical.provider_item_id.is_none(), "{:?}", event.kind);
            assert!(canonical.provider_event_id.is_some(), "{:?}", event.kind);
        }

        let first = parse_claude_stream_json_lines(
            r#"{"type":"future_record","session_id":"s1","future_field":42}"#,
        )
        .unwrap()
        .pop()
        .unwrap();
        let second = parse_claude_stream_json_lines(
            r#"{"type":"future_record","session_id":"s1","future_field":42}"#,
        )
        .unwrap()
        .pop()
        .unwrap();
        assert_eq!(first.provider_event_id, second.provider_event_id);
    }

    #[test]
    fn claude_fixture_adapter_emits_lossless_common_effects_for_every_record() {
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
                content: "Summarize the fixture.".to_owned(),
                visible_content: None,
                kind: crate::archcar::protocol::ArchcarInputKind::User,
                delivery: crate::archcar::protocol::ArchcarInputDelivery::Auto,
            })
            .unwrap();

        let mut provider_event_count = 0;
        let mut all_effects = Vec::<HarnessEffect>::new();
        for line in BASIC_TURN_FIXTURE
            .lines()
            .chain(TOOLS_HOOKS_LIMITS_FIXTURE.lines())
            .chain(DEFERRED_AND_FAILED_RESULTS_FIXTURE.lines())
        {
            let effects = adapter
                .observe_native(NativeRecord {
                    provider_key: CLAUDE_PROVIDER_NAME,
                    payload: format!("{line}\n").into_bytes(),
                })
                .unwrap();
            provider_event_count += effects
                .iter()
                .filter(|effect| matches!(effect, HarnessEffect::ProviderEvent(_)))
                .count();
            all_effects.extend(effects);
        }

        assert_eq!(
            provider_event_count,
            BASIC_TURN_FIXTURE.lines().count()
                + TOOLS_HOOKS_LIMITS_FIXTURE.lines().count()
                + DEFERRED_AND_FAILED_RESULTS_FIXTURE.lines().count()
        );
        assert!(all_effects.iter().any(|effect| matches!(
            effect,
            HarnessEffect::CapabilitiesObserved(capabilities)
                if capabilities == &["streaming"]
        )));
        assert!(all_effects.iter().any(|effect| matches!(
            effect,
            HarnessEffect::Retry {
                delay_ms: Some(250),
                ..
            }
        )));
        assert!(all_effects
            .iter()
            .any(|effect| matches!(effect, HarnessEffect::RateLimited { .. })));
    }

    #[test]
    fn claude_fixture_same_text_replay_does_not_acknowledge_inactive_pending_input() {
        let mut adapter = ClaudeManagedAdapter::new(HarnessAdapterContext {
            session_id: 7,
            thread_id: 11,
            workspace: "fixture-workspace".to_owned(),
            native_session_id: None,
            controls: Default::default(),
        });
        adapter
            .encode_input(HarnessInput {
                local_input_id: "current-input".to_owned(),
                content: "Current prompt".to_owned(),
                visible_content: None,
                kind: crate::archcar::protocol::ArchcarInputKind::User,
                delivery: crate::archcar::protocol::ArchcarInputDelivery::Auto,
            })
            .unwrap();

        let replay = adapter
            .observe_native(NativeRecord {
                provider_key: CLAUDE_PROVIDER_NAME,
                payload: br#"{"type":"user","session_id":"s1","isReplay":true,"message":{"role":"user","content":[{"type":"text","text":"Historic prompt"}]}}
"#
                .to_vec(),
            })
            .unwrap();
        assert!(!replay.iter().any(|effect| matches!(
            effect,
            HarnessEffect::InputAcknowledged { .. } | HarnessEffect::TurnStarted { .. }
        )));

        let same_text_replay = adapter
            .observe_native(NativeRecord {
                provider_key: CLAUDE_PROVIDER_NAME,
                payload: br#"{"type":"user","session_id":"s1","isReplay":true,"message":{"role":"user","content":[{"type":"text","text":"Current prompt"}]}}
"#
                .to_vec(),
            })
            .unwrap();
        assert!(!same_text_replay.iter().any(|effect| matches!(
            effect,
            HarnessEffect::InputAcknowledged { .. } | HarnessEffect::TurnStarted { .. }
        )));

        let start = adapter
            .observe_native(NativeRecord {
                provider_key: CLAUDE_PROVIDER_NAME,
                payload: br#"{"type":"stream_event","session_id":"s1","event":{"type":"message_start","message":{"id":"m1"}}}
"#
                .to_vec(),
            })
            .unwrap();
        assert!(start.iter().any(|effect| matches!(
            effect,
            HarnessEffect::TurnStarted { local_input_id } if local_input_id == "current-input"
        )));
    }

    #[test]
    fn claude_fixture_mixed_final_snapshot_emits_assistant_and_reasoning_events() {
        let mut adapter = ClaudeManagedAdapter::new(HarnessAdapterContext {
            session_id: 7,
            thread_id: 11,
            workspace: "fixture-workspace".to_owned(),
            native_session_id: None,
            controls: Default::default(),
        });
        let raw = json!({
            "type": "assistant",
            "session_id": "s1",
            "uuid": "mixed-final-1",
            "message": {
                "id": "m1",
                "role": "assistant",
                "content": [
                    {"type": "thinking", "thinking": "Private reasoning"},
                    {"type": "text", "text": "Public answer"}
                ]
            }
        });
        let parsed = parse_claude_stream_json_lines(&format!("{raw}\n"))
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(parsed.kind, ClaudeProviderEventKind::AssistantMessage);
        assert_eq!(parsed.content_delta.as_deref(), Some("Public answer"));
        assert_eq!(parsed.reasoning_blocks.len(), 1);
        assert_eq!(parsed.reasoning_blocks[0].thinking, "Private reasoning");

        let effects = adapter
            .observe_native(NativeRecord {
                provider_key: CLAUDE_PROVIDER_NAME,
                payload: format!("{raw}\n").into_bytes(),
            })
            .unwrap();
        let events = effects
            .iter()
            .filter_map(|effect| match effect {
                HarnessEffect::ProviderEvent(event) => Some(event),
                _ => None,
            })
            .collect::<Vec<_>>();
        let assistant = events
            .iter()
            .find(|event| event.kind == ProviderEventKind::AssistantOutput)
            .unwrap();
        let reasoning = events
            .iter()
            .find(|event| event.kind == ProviderEventKind::PlanningReasoning)
            .unwrap();

        assert_eq!(assistant.normalized_payload["body"], "Public answer");
        assert_eq!(reasoning.normalized_payload["body"], "Private reasoning");
        assert_ne!(assistant.provider_item_id, reasoning.provider_item_id);
        assert_eq!(assistant.raw_json, raw);
        assert_eq!(reasoning.raw_json, raw);
    }

    #[test]
    fn claude_fixture_replay_acknowledges_matching_active_input() {
        let mut adapter = ClaudeManagedAdapter::new(HarnessAdapterContext {
            session_id: 7,
            thread_id: 11,
            workspace: "fixture-workspace".to_owned(),
            native_session_id: None,
            controls: Default::default(),
        });
        adapter
            .encode_input(HarnessInput {
                local_input_id: "active-input".to_owned(),
                content: "Active prompt".to_owned(),
                visible_content: None,
                kind: crate::archcar::protocol::ArchcarInputKind::User,
                delivery: crate::archcar::protocol::ArchcarInputDelivery::Auto,
            })
            .unwrap();
        adapter
            .observe_native(NativeRecord {
                provider_key: CLAUDE_PROVIDER_NAME,
                payload: br#"{"type":"stream_event","session_id":"s1","event":{"type":"message_start","message":{"id":"m1"}}}
"#
                .to_vec(),
            })
            .unwrap();

        let replay = adapter
            .observe_native(NativeRecord {
                provider_key: CLAUDE_PROVIDER_NAME,
                payload: br#"{"type":"user","session_id":"s1","isReplay":true,"message":{"role":"user","content":[{"type":"text","text":"Active prompt"}]}}
"#
                .to_vec(),
            })
            .unwrap();

        assert!(replay.iter().any(|effect| matches!(
            effect,
            HarnessEffect::InputAcknowledged { local_input_id }
                if local_input_id == "active-input"
        )));
        assert!(!replay
            .iter()
            .any(|effect| matches!(effect, HarnessEffect::TurnStarted { .. })));
    }

    #[test]
    fn claude_replayed_user_event_uses_visible_input_body() {
        let mut adapter = ClaudeManagedAdapter::new(HarnessAdapterContext {
            session_id: 7,
            thread_id: 11,
            workspace: "fixture-workspace".to_owned(),
            native_session_id: None,
            controls: Default::default(),
        });
        adapter
            .observe_native(NativeRecord {
                provider_key: CLAUDE_PROVIDER_NAME,
                payload: br#"{"type":"system","subtype":"init","session_id":"s1","model":"claude-test","capabilities":[]}
"#
                .to_vec(),
            })
            .unwrap();
        adapter
            .encode_input(HarnessInput {
                local_input_id: "active-input".to_owned(),
                content:
                    "what's up?\n\n<archductor_hidden_instruction>secret</archductor_hidden_instruction>"
                        .to_owned(),
                visible_content: Some("what's up?".to_owned()),
                kind: crate::archcar::protocol::ArchcarInputKind::User,
                delivery: crate::archcar::protocol::ArchcarInputDelivery::Auto,
            })
            .unwrap();

        let replay = adapter
            .observe_native(NativeRecord {
                provider_key: CLAUDE_PROVIDER_NAME,
                payload: br#"{"type":"user","session_id":"s1","isReplay":true,"message":{"role":"user","content":[{"type":"text","text":"what's up?\n\n<archductor_hidden_instruction>secret</archductor_hidden_instruction>"}]}}
"#
                .to_vec(),
            })
            .unwrap();
        let user_event = replay
            .iter()
            .find_map(|effect| match effect {
                HarnessEffect::ProviderEvent(event)
                    if event.kind == ProviderEventKind::UserInput =>
                {
                    Some(event)
                }
                _ => None,
            })
            .unwrap();

        assert_eq!(user_event.normalized_payload["body"], "what's up?");
    }

    #[test]
    fn claude_fixture_terminal_rate_limit_finishes_active_turn_once() {
        let mut adapter = ClaudeManagedAdapter::new(HarnessAdapterContext {
            session_id: 7,
            thread_id: 11,
            workspace: "fixture-workspace".to_owned(),
            native_session_id: None,
            controls: Default::default(),
        });
        adapter.tracker.initialized = true;
        adapter
            .encode_input(HarnessInput {
                local_input_id: "limited-input".to_owned(),
                content: "Trigger limit".to_owned(),
                visible_content: None,
                kind: crate::archcar::protocol::ArchcarInputKind::User,
                delivery: crate::archcar::protocol::ArchcarInputDelivery::Auto,
            })
            .unwrap();
        adapter
            .observe_native(NativeRecord {
                provider_key: CLAUDE_PROVIDER_NAME,
                payload: br#"{"type":"stream_event","session_id":"s1","event":{"type":"message_start","message":{"id":"m1"}}}
"#
                .to_vec(),
            })
            .unwrap();

        let effects = adapter
            .observe_native(NativeRecord {
                provider_key: CLAUDE_PROVIDER_NAME,
                payload: br#"{"type":"rate_limit_event","session_id":"s1","rate_limit_info":{"status":"rejected","rateLimitType":"five_hour"}}
"#
                .to_vec(),
            })
            .unwrap();

        assert!(effects.iter().any(|effect| matches!(
            effect,
            HarnessEffect::TurnCompleted {
                local_input_id,
                status: HarnessTurnStatus::Failed,
            } if local_input_id == "limited-input"
        )));
        assert!(effects
            .iter()
            .any(|effect| matches!(effect, HarnessEffect::Ready)));
    }

    #[test]
    fn claude_allowed_rate_limit_event_stays_hidden() {
        let mut adapter = ClaudeManagedAdapter::new(HarnessAdapterContext {
            session_id: 7,
            thread_id: 11,
            workspace: "fixture-workspace".to_owned(),
            native_session_id: None,
            controls: Default::default(),
        });

        let effects = adapter
            .observe_native(NativeRecord {
                provider_key: CLAUDE_PROVIDER_NAME,
                payload: br#"{"type":"rate_limit_event","session_id":"s1","rate_limit_info":{"status":"allowed","rateLimitType":"five_hour"}}
"#
                .to_vec(),
            })
            .unwrap();

        assert!(!effects
            .iter()
            .any(|effect| matches!(effect, HarnessEffect::RateLimited { .. })));
        assert!(!effects.iter().any(|effect| matches!(
            effect,
            HarnessEffect::ProviderEvent(event) if event.kind == ProviderEventKind::LimitFailure
        )));
    }

    #[test]
    fn claude_fixture_reasoning_stream_projects_one_canonical_reasoning_item() {
        let native = r#"{"type":"stream_event","session_id":"s1","event":{"type":"message_start","message":{"id":"m1"}}}
{"type":"stream_event","session_id":"s1","event":{"type":"content_block_start","index":1,"content_block":{"type":"thinking","thinking":""}}}
{"type":"stream_event","session_id":"s1","event":{"type":"content_block_delta","index":1,"delta":{"type":"thinking_delta","thinking":"Consider this"}}}
{"type":"stream_event","session_id":"s1","event":{"type":"content_block_stop","index":1}}"#;
        let events = parse_claude_stream_json_lines(native).unwrap();
        let reasoning = events
            .into_iter()
            .skip(1)
            .map(|event| {
                event.into_provider_event_draft(ProviderEventContext {
                    workspace_id: None,
                    chat_thread_id: None,
                    process_id: None,
                    occurred_at_ms: 1,
                    schema_version: 1,
                    adapter_version: "test".to_owned(),
                })
            })
            .collect::<Vec<_>>();

        assert!(reasoning
            .iter()
            .all(|event| event.kind == ProviderEventKind::PlanningReasoning));
        assert_eq!(reasoning[0].phase, ProviderEventPhase::Started);
        assert_eq!(reasoning[1].phase, ProviderEventPhase::Delta);
        assert_eq!(reasoning[2].phase, ProviderEventPhase::Completed);
        assert!(reasoning
            .windows(2)
            .all(|pair| pair[0].provider_item_id == pair[1].provider_item_id));
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
    fn claude_turn_tracker_drives_readiness_and_exactly_once_completion() {
        let mut adapter = ClaudeManagedAdapter::new(HarnessAdapterContext {
            session_id: 7,
            thread_id: 11,
            workspace: "fixture-workspace".to_owned(),
            native_session_id: None,
            controls: Default::default(),
        });
        let init = br#"{"type":"system","subtype":"init","session_id":"s1","model":"claude-sonnet-fixture","capabilities":["streaming"]}
"#;
        let replayed_user = br#"{"type":"user","session_id":"s1","isReplay":true,"message":{"role":"user","content":[{"type":"text","text":"hello tracker"}]}}
"#;
        let success_result = br#"{"type":"result","subtype":"success","session_id":"s1","result":"done","duration_ms":1}
"#;

        assert!(!adapter.tracker.ready());
        let init_effects = adapter
            .observe_native(NativeRecord {
                provider_key: CLAUDE_PROVIDER_NAME,
                payload: init.to_vec(),
            })
            .unwrap();
        assert!(init_effects.iter().any(|effect| matches!(
            effect,
            HarnessEffect::Initialized { native_session_id, model }
                if native_session_id == "s1" && model.as_deref() == Some("claude-sonnet-fixture")
        )));
        assert!(init_effects.iter().any(|effect| matches!(
            effect,
            HarnessEffect::CapabilitiesObserved(capabilities) if capabilities == &["streaming"]
        )));
        assert!(init_effects
            .iter()
            .any(|effect| matches!(effect, HarnessEffect::Ready)));
        assert!(adapter.tracker.ready());

        adapter
            .encode_input(HarnessInput {
                local_input_id: "local-input-1".to_owned(),
                content: "hello tracker".to_owned(),
                visible_content: None,
                kind: crate::archcar::protocol::ArchcarInputKind::User,
                delivery: crate::archcar::protocol::ArchcarInputDelivery::Auto,
            })
            .unwrap();
        assert!(!adapter.tracker.ready());

        let replay_effects = adapter
            .observe_native(NativeRecord {
                provider_key: CLAUDE_PROVIDER_NAME,
                payload: replayed_user.to_vec(),
            })
            .unwrap();
        assert!(replay_effects.iter().any(|effect| matches!(
            effect,
            HarnessEffect::InputAcknowledged { local_input_id }
                if local_input_id == "local-input-1"
        )));
        assert!(replay_effects.iter().any(|effect| matches!(
            effect,
            HarnessEffect::TurnStarted { local_input_id } if local_input_id == "local-input-1"
        )));

        let result_effects = adapter
            .observe_native(NativeRecord {
                provider_key: CLAUDE_PROVIDER_NAME,
                payload: success_result.to_vec(),
            })
            .unwrap();
        assert!(result_effects.iter().any(|effect| matches!(
            effect,
            HarnessEffect::TurnCompleted {
                local_input_id,
                status: HarnessTurnStatus::Success,
            } if local_input_id == "local-input-1"
        )));
        assert!(result_effects
            .iter()
            .any(|effect| matches!(effect, HarnessEffect::Ready)));
        assert!(adapter.tracker.ready());

        let duplicate_effects = adapter
            .observe_native(NativeRecord {
                provider_key: CLAUDE_PROVIDER_NAME,
                payload: success_result.to_vec(),
            })
            .unwrap();
        assert!(
            duplicate_effects.is_empty(),
            "duplicate terminal result must not produce lifecycle or provider effects: {duplicate_effects:?}"
        );
    }

    #[test]
    fn claude_turn_tracker_allows_reused_result_payload_for_new_local_turn() {
        let mut adapter = ClaudeManagedAdapter::new(HarnessAdapterContext {
            session_id: 7,
            thread_id: 11,
            workspace: "fixture-workspace".to_owned(),
            native_session_id: None,
            controls: Default::default(),
        });
        adapter
            .observe_native(NativeRecord {
                provider_key: CLAUDE_PROVIDER_NAME,
                payload: br#"{"type":"system","subtype":"init","session_id":"s1","model":"claude-sonnet-fixture"}
"#
                .to_vec(),
            })
            .unwrap();
        let success_result = br#"{"type":"result","subtype":"success","session_id":"s1","result":"done","duration_ms":1}
"#;

        for (local_input_id, content) in [
            ("local-input-1", "first identical result"),
            ("local-input-2", "second identical result"),
        ] {
            adapter
                .encode_input(HarnessInput {
                    local_input_id: local_input_id.to_owned(),
                    content: content.to_owned(),
                    visible_content: None,
                    kind: crate::archcar::protocol::ArchcarInputKind::User,
                    delivery: crate::archcar::protocol::ArchcarInputDelivery::Auto,
                })
                .unwrap();
            let mut start_effects = Vec::new();
            adapter
                .tracker
                .note_provider_turn_started(&mut start_effects);

            let result_effects = adapter
                .observe_native(NativeRecord {
                    provider_key: CLAUDE_PROVIDER_NAME,
                    payload: success_result.to_vec(),
                })
                .unwrap();
            assert!(result_effects.iter().any(|effect| matches!(
                effect,
                HarnessEffect::TurnCompleted {
                    local_input_id: completed_id,
                    status: HarnessTurnStatus::Success,
                } if completed_id == local_input_id
            )));
            assert!(adapter.tracker.ready());
        }
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
    fn capability_discovery_uses_contract_baseline_and_observed_init_values() {
        assert_eq!(
            CLAUDE_MANAGED_HARNESS_DESCRIPTOR.required_features,
            REQUIRED_HARNESS_FEATURES
        );
        let init = parse_claude_stream_json_lines(
            r#"{"type":"system","subtype":"init","session_id":"s1","capabilities":["streaming","future-capability"]}"#,
        )
        .unwrap()
        .pop()
        .unwrap();
        assert!(matches!(
            init.lifecycle_signal(),
            Some(ClaudeLifecycleSignal::Initialized(ClaudeInitMetadata {
                capabilities,
                ..
            })) if capabilities == ["streaming", "future-capability"]
        ));
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
        assert_eq!(events[0].kind, ClaudeProviderEventKind::Initialization);
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
    fn parses_claude_message_delta_thinking_token_usage() {
        let events = parse_claude_stream_json_lines(
            r#"{"type":"stream_event","uuid":"u1","session_id":"s1","event":{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":4,"output_tokens_details":{"thinking_tokens":17}}}}"#,
        )
        .unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, ClaudeProviderEventKind::MessageDelta);
        assert_eq!(events[0].usage.output_tokens, Some(4));
        assert_eq!(events[0].usage.thinking_tokens, Some(17));

        let canonical = events[0].clone().into_provider_event_draft(
            crate::provider_events::ProviderEventContext {
                workspace_id: None,
                chat_thread_id: Some(7),
                process_id: Some(9),
                occurred_at_ms: 42,
                schema_version: 1,
                adapter_version: "claude-stream-json-test".to_owned(),
            },
        );
        assert_eq!(canonical.normalized_payload["usage"]["thinking_tokens"], 17);

        let direct = parse_claude_stream_json_lines(
            r#"{"type":"stream_event","event":{"type":"message_delta","usage":{"thinking_tokens":18}}}"#,
        )
        .unwrap();
        assert_eq!(direct[0].usage.thinking_tokens, Some(18));
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
