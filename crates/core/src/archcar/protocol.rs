use serde::{Deserialize, Serialize};

use crate::archcar::harness_contract::HarnessDescriptor;
use crate::archcar::harness_contract::{ProviderInteractionDraft, ProviderInteractionResolution};
use crate::codex_tui::{CodexContextUsage, CodexInlineEvent};
use crate::provider_events::ProviderEventRecord;
use crate::provider_interactions::ProviderInteractionRecord;
use crate::session_state::AgentSessionState;
use crate::workspace::{ChatEventRecord, ChatMessageRecord, SessionHarnessOptions, SessionKind};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RpcEnvelope<T> {
    pub id: String,
    pub payload: T,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchcarInputKind {
    User,
    ReviewPrompt,
    ControlCommand,
    RawTerminal,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchcarInputDelivery {
    #[default]
    Auto,
    Immediate,
}

impl ArchcarInputDelivery {
    fn is_auto(&self) -> bool {
        *self == Self::Auto
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Immediate => "immediate",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ArchcarRequest {
    EnsureWorkspaceDefaultSession {
        workspace: String,
        kind: SessionKind,
        harness: Option<SessionHarnessOptions>,
    },
    EnsureChatThreadSession {
        workspace: String,
        thread_id: i64,
        kind: SessionKind,
        harness: Option<SessionHarnessOptions>,
    },
    SpawnSession {
        workspace: String,
        kind: SessionKind,
        harness: Option<SessionHarnessOptions>,
    },
    SendInput {
        session_id: i64,
        input: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        visible_input: Option<String>,
        kind: ArchcarInputKind,
        #[serde(default, skip_serializing_if = "ArchcarInputDelivery::is_auto")]
        delivery: ArchcarInputDelivery,
    },
    InterruptTurn {
        session_id: i64,
    },
    SetSessionModel {
        session_id: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    SetSessionEffort {
        session_id: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effort: Option<String>,
    },
    SetSessionPermissionMode {
        session_id: i64,
        mode: String,
    },
    ResizeSession {
        session_id: i64,
        rows: u16,
        cols: u16,
    },
    GetSessionStatus {
        session_id: i64,
    },
    GetSessionScreen {
        session_id: i64,
    },
    GetSessionMessages {
        thread_id: i64,
    },
    GetChatSnapshot {
        thread_id: i64,
    },
    QueueChatInput {
        thread_id: i64,
        input: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        visible_input: Option<String>,
        kind: ArchcarInputKind,
        session_kind: SessionKind,
    },
    ListQueuedChatInputs {
        thread_id: i64,
    },
    RemoveQueuedChatInput {
        queue_id: i64,
    },
    KillSession {
        session_id: i64,
    },
    RegisterProviderInteraction {
        interaction: ProviderInteractionDraft,
    },
    GetProviderInteraction {
        interaction_id: String,
    },
    ListProviderInteractions {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread_id: Option<i64>,
        pending_only: bool,
    },
    ResolveProviderInteraction {
        interaction_id: String,
        resolution: ProviderInteractionResolution,
    },
    ConsumeProviderInteraction {
        interaction_id: String,
        native_response: serde_json::Value,
    },
    Subscribe,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ArchcarResponse {
    Ack,
    SessionSpawnQueued {
        workspace: String,
        kind: SessionKind,
    },
    SessionSpawned {
        session_id: i64,
        thread_id: i64,
        workspace: String,
        kind: SessionKind,
        pid: u32,
    },
    SessionStatus {
        session_id: i64,
        status: String,
        runtime_state: AgentSessionState,
        ready: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capabilities: Option<SessionHarnessCapabilities>,
    },
    SessionScreen {
        session_id: i64,
        screen: String,
    },
    SessionMessages {
        thread_id: i64,
        messages: Vec<ArchcarMessage>,
    },
    ChatSnapshot {
        snapshot: ArchcarChatSnapshot,
    },
    QueuedChatInput {
        input: QueuedArchcarInput,
    },
    QueuedChatInputs {
        thread_id: i64,
        inputs: Vec<QueuedArchcarInput>,
    },
    ProviderInteraction {
        interaction: ProviderInteractionRecord,
    },
    ProviderInteractions {
        interactions: Vec<ProviderInteractionRecord>,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionHarnessCapabilities {
    pub contract_version: u16,
    pub required: Vec<String>,
    pub optional: Vec<SessionCapabilitySupport>,
    pub observed_native: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionCapabilitySupport {
    pub name: String,
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

pub fn session_harness_capabilities_for_descriptor(
    descriptor: &HarnessDescriptor,
    observed_native: Vec<String>,
) -> SessionHarnessCapabilities {
    SessionHarnessCapabilities {
        contract_version: descriptor.contract_version,
        required: descriptor
            .required_features
            .iter()
            .map(|feature| feature.as_str().to_owned())
            .collect(),
        optional: descriptor
            .optional_capabilities
            .iter()
            .map(|(capability, support)| SessionCapabilitySupport {
                name: capability.as_str().to_owned(),
                mode: support.as_str().to_owned(),
                reason: support.reason().map(str::to_owned),
            })
            .collect(),
        observed_native,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchcarMessage {
    pub id: i64,
    pub role: String,
    pub content: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline_event: Option<CodexInlineEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_usage: Option<CodexContextUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueuedArchcarInput {
    pub id: i64,
    pub thread_id: i64,
    pub input: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_input: Option<String>,
    pub kind: ArchcarInputKind,
    pub session_kind: SessionKind,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchcarChatSnapshot {
    pub thread_id: i64,
    pub messages: Vec<ChatMessageRecord>,
    pub events: Vec<ChatEventRecord>,
    pub provider_events: Vec<ProviderEventRecord>,
    pub queued_inputs: Vec<QueuedArchcarInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_session: Option<ArchcarChatLiveSession>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchcarChatLiveSession {
    pub session_id: i64,
    pub status: String,
    pub runtime_state: AgentSessionState,
    pub ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<SessionHarnessCapabilities>,
}

pub fn archcar_request_summary(request: &ArchcarRequest) -> String {
    match request {
        ArchcarRequest::EnsureWorkspaceDefaultSession {
            workspace, kind, ..
        } => {
            format!(
                "ensure_workspace_default_session workspace={workspace} kind={}",
                session_kind_label(*kind)
            )
        }
        ArchcarRequest::EnsureChatThreadSession {
            workspace,
            thread_id,
            kind,
            ..
        } => {
            format!(
                "ensure_chat_thread_session workspace={workspace} thread_id={thread_id} kind={}",
                session_kind_label(*kind)
            )
        }
        ArchcarRequest::SpawnSession {
            workspace, kind, ..
        } => {
            format!(
                "spawn_session workspace={workspace} kind={}",
                session_kind_label(*kind)
            )
        }
        ArchcarRequest::SendInput {
            session_id,
            input,
            visible_input: _,
            kind,
            delivery,
        } => format!(
            "send_input session_id={session_id} kind={} delivery={} chars={}",
            input_kind_label(kind),
            delivery.as_str(),
            input.chars().count()
        ),
        ArchcarRequest::InterruptTurn { session_id } => {
            format!("interrupt_turn session_id={session_id}")
        }
        ArchcarRequest::SetSessionModel { session_id, model } => format!(
            "set_session_model session_id={session_id} model={}",
            if model
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
            {
                "set"
            } else {
                "default"
            }
        ),
        ArchcarRequest::SetSessionEffort { session_id, effort } => format!(
            "set_session_effort session_id={session_id} effort={}",
            effort.as_deref().unwrap_or("default")
        ),
        ArchcarRequest::SetSessionPermissionMode { session_id, mode } => {
            format!("set_session_permission_mode session_id={session_id} mode={mode}")
        }
        ArchcarRequest::ResizeSession {
            session_id,
            rows,
            cols,
        } => format!("resize_session session_id={session_id} rows={rows} cols={cols}"),
        ArchcarRequest::GetSessionStatus { session_id } => {
            format!("get_session_status session_id={session_id}")
        }
        ArchcarRequest::GetSessionScreen { session_id } => {
            format!("get_session_screen session_id={session_id}")
        }
        ArchcarRequest::GetSessionMessages { thread_id } => {
            format!("get_session_messages thread_id={thread_id}")
        }
        ArchcarRequest::GetChatSnapshot { thread_id } => {
            format!("get_chat_snapshot thread_id={thread_id}")
        }
        ArchcarRequest::QueueChatInput {
            thread_id,
            input,
            kind,
            session_kind,
            ..
        } => format!(
            "queue_chat_input thread_id={thread_id} kind={} session_kind={} chars={}",
            input_kind_label(kind),
            session_kind_label(*session_kind),
            input.chars().count()
        ),
        ArchcarRequest::ListQueuedChatInputs { thread_id } => {
            format!("list_queued_chat_inputs thread_id={thread_id}")
        }
        ArchcarRequest::RemoveQueuedChatInput { queue_id } => {
            format!("remove_queued_chat_input queue_id={queue_id}")
        }
        ArchcarRequest::KillSession { session_id } => {
            format!("kill_session session_id={session_id}")
        }
        ArchcarRequest::RegisterProviderInteraction { interaction } => format!(
            "register_provider_interaction provider={} session_id={} thread_id={} kind={:?} native_id={} request_bytes={}",
            interaction.provider_key,
            interaction.session_id,
            interaction.thread_id,
            interaction.kind,
            interaction.native_id,
            interaction.native_request.to_string().len()
        ),
        ArchcarRequest::GetProviderInteraction { interaction_id } => {
            format!("get_provider_interaction id={interaction_id}")
        }
        ArchcarRequest::ListProviderInteractions {
            thread_id,
            pending_only,
        } => format!(
            "list_provider_interactions thread_id={thread_id:?} pending_only={pending_only}"
        ),
        ArchcarRequest::ResolveProviderInteraction {
            interaction_id,
            resolution,
        } => format!(
            "resolve_provider_interaction id={interaction_id} {}",
            provider_interaction_resolution_summary(resolution)
        ),
        ArchcarRequest::ConsumeProviderInteraction {
            interaction_id,
            native_response,
        } => format!(
            "consume_provider_interaction id={interaction_id} response_bytes={}",
            native_response.to_string().len()
        ),
        ArchcarRequest::Subscribe => "subscribe".to_owned(),
    }
}

fn provider_interaction_resolution_summary(resolution: &ProviderInteractionResolution) -> String {
    match resolution {
        ProviderInteractionResolution::Approve => "resolution=approve".to_owned(),
        ProviderInteractionResolution::Deny { reason } => format!(
            "resolution=deny denial_reason_chars={}",
            reason.as_deref().unwrap_or_default().chars().count()
        ),
        ProviderInteractionResolution::Answer { answers } => format!(
            "resolution=answer answer_count={} answer_chars={}",
            answers.len(),
            answers
                .iter()
                .map(|(_, answer)| answer.chars().count())
                .sum::<usize>()
        ),
        ProviderInteractionResolution::Defer => "resolution=defer".to_owned(),
    }
}

pub fn archcar_response_summary(response: &ArchcarResponse) -> String {
    match response {
        ArchcarResponse::Ack => "ack".to_owned(),
        ArchcarResponse::SessionSpawnQueued { workspace, kind } => format!(
            "session_spawn_queued workspace={workspace} kind={}",
            session_kind_label(*kind)
        ),
        ArchcarResponse::SessionSpawned {
            session_id,
            thread_id,
            workspace,
            kind,
            pid,
        } => format!(
            "session_spawned workspace={workspace} kind={} session_id={session_id} thread_id={thread_id} pid={pid}",
            session_kind_label(*kind)
        ),
        ArchcarResponse::SessionStatus {
            session_id,
            status,
            runtime_state,
            ready,
            capabilities: _,
        } => format!(
            "session_status session_id={session_id} status={status} state={} ready={ready}",
            runtime_state.as_str()
        ),
        ArchcarResponse::SessionScreen { session_id, screen } => format!(
            "session_screen session_id={session_id} chars={}",
            screen.chars().count()
        ),
        ArchcarResponse::SessionMessages { thread_id, messages } => format!(
            "session_messages thread_id={thread_id} count={}",
            messages.len()
        ),
        ArchcarResponse::ChatSnapshot { snapshot } => format!(
            "chat_snapshot thread_id={} messages={} events={} provider_events={} queued_inputs={} live_session={}",
            snapshot.thread_id,
            snapshot.messages.len(),
            snapshot.events.len(),
            snapshot.provider_events.len(),
            snapshot.queued_inputs.len(),
            snapshot.live_session.is_some()
        ),
        ArchcarResponse::QueuedChatInput { input } => {
            format!("queued_chat_input id={} thread_id={}", input.id, input.thread_id)
        }
        ArchcarResponse::QueuedChatInputs { thread_id, inputs } => {
            format!("queued_chat_inputs thread_id={thread_id} count={}", inputs.len())
        }
        ArchcarResponse::ProviderInteraction { interaction } => format!(
            "provider_interaction id={} kind={:?} status={:?}",
            interaction.id, interaction.kind, interaction.status
        ),
        ArchcarResponse::ProviderInteractions { interactions } => {
            format!("provider_interactions count={}", interactions.len())
        }
        ArchcarResponse::Error { message } => {
            format!("error chars={}", message.chars().count())
        }
    }
}

pub fn archcar_event_summary(event: &ArchcarEvent) -> String {
    match event {
        ArchcarEvent::SessionSpawnQueued { workspace, kind } => format!(
            "session_spawn_queued workspace={workspace} kind={}",
            session_kind_label(*kind)
        ),
        ArchcarEvent::SessionStarted {
            session_id,
            thread_id,
            workspace,
            kind,
            pid,
        } => format!(
            "session_started workspace={workspace} kind={} session_id={session_id} thread_id={thread_id} pid={pid}",
            session_kind_label(*kind)
        ),
        ArchcarEvent::SessionReady {
            session_id,
            thread_id,
        } => format!("session_ready session_id={session_id} thread_id={thread_id}"),
        ArchcarEvent::SessionCapabilitiesChanged {
            session_id,
            thread_id,
            capabilities,
        } => format!(
            "session_capabilities_changed session_id={session_id} thread_id={thread_id} required={} optional={} observed_native={}",
            capabilities.required.len(),
            capabilities.optional.len(),
            capabilities.observed_native.len()
        ),
        ArchcarEvent::TurnCompleted {
            session_id,
            thread_id,
            status,
        } => format!(
            "turn_completed session_id={session_id} thread_id={thread_id} status={}",
            status.as_deref().unwrap_or("unknown")
        ),
        ArchcarEvent::SessionScreenUpdated { session_id } => {
            format!("session_screen_updated session_id={session_id}")
        }
        ArchcarEvent::SessionMessagesUpdated { thread_id } => {
            format!("session_messages_updated thread_id={thread_id}")
        }
        ArchcarEvent::ChatQueueUpdated { thread_id } => {
            format!("chat_queue_updated thread_id={thread_id}")
        }
        ArchcarEvent::SessionExited {
            session_id,
            exit_code,
        } => format!("session_exited session_id={session_id} exit_code={exit_code:?}"),
        ArchcarEvent::SessionError {
            session_id,
            thread_id,
            message,
        } => format!(
            "session_error session_id={session_id:?} thread_id={thread_id:?} chars={}",
            message.chars().count()
        ),
        ArchcarEvent::ProviderInteractionRequested { interaction } => format!(
            "provider_interaction_requested id={} kind={:?} status={:?}",
            interaction.id, interaction.kind, interaction.status
        ),
        ArchcarEvent::ProviderInteractionResolved { interaction } => format!(
            "provider_interaction_resolved id={} kind={:?} status={:?}",
            interaction.id, interaction.kind, interaction.status
        ),
    }
}

fn session_kind_label(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Shell => "Shell",
        SessionKind::Codex => "Codex",
        SessionKind::Claude => "Claude",
    }
}

fn input_kind_label(kind: &ArchcarInputKind) -> &'static str {
    match kind {
        ArchcarInputKind::User => "user",
        ArchcarInputKind::ReviewPrompt => "review_prompt",
        ArchcarInputKind::ControlCommand => "control_command",
        ArchcarInputKind::RawTerminal => "raw_terminal",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ArchcarEvent {
    SessionSpawnQueued {
        workspace: String,
        kind: SessionKind,
    },
    SessionStarted {
        session_id: i64,
        thread_id: i64,
        workspace: String,
        kind: SessionKind,
        pid: u32,
    },
    SessionReady {
        session_id: i64,
        thread_id: i64,
    },
    SessionCapabilitiesChanged {
        session_id: i64,
        thread_id: i64,
        capabilities: SessionHarnessCapabilities,
    },
    TurnCompleted {
        session_id: i64,
        thread_id: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
    SessionScreenUpdated {
        session_id: i64,
    },
    SessionMessagesUpdated {
        thread_id: i64,
    },
    ChatQueueUpdated {
        thread_id: i64,
    },
    SessionExited {
        session_id: i64,
        exit_code: Option<i32>,
    },
    SessionError {
        session_id: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread_id: Option<i64>,
        message: String,
    },
    ProviderInteractionRequested {
        interaction: ProviderInteractionRecord,
    },
    ProviderInteractionResolved {
        interaction: ProviderInteractionRecord,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archcar::harness_contract::{
        ProviderInteractionKind, ProviderInteractionResolution,
    };
    use crate::codex_tui::{CodexContextUsage, CodexInlineEvent, CodexToolCall};
    use crate::provider_interactions::ProviderInteractionStatus;

    #[test]
    fn protocol_round_trips_spawn_event() {
        let envelope = RpcEnvelope {
            id: "1".to_owned(),
            payload: ArchcarEvent::SessionStarted {
                session_id: 4,
                thread_id: 6,
                workspace: "berlin".to_owned(),
                kind: SessionKind::Codex,
                pid: 123,
            },
        };
        let json = serde_json::to_string(&envelope).unwrap();
        let decoded: RpcEnvelope<ArchcarEvent> = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, envelope);
    }

    #[test]
    fn request_summary_describes_send_input() {
        let request = ArchcarRequest::SendInput {
            session_id: 9,
            input: "run tests".to_owned(),
            visible_input: None,
            kind: ArchcarInputKind::User,
            delivery: ArchcarInputDelivery::Immediate,
        };

        assert_eq!(
            archcar_request_summary(&request),
            "send_input session_id=9 kind=user delivery=immediate chars=9"
        );
    }

    #[test]
    fn send_input_delivery_defaults_to_auto_and_round_trips_immediate() {
        let legacy = r#"{"type":"send_input","session_id":9,"input":"run tests","kind":"user"}"#;
        let decoded: ArchcarRequest = serde_json::from_str(legacy).unwrap();
        let ArchcarRequest::SendInput { delivery, .. } = decoded else {
            panic!("expected send input");
        };
        assert_eq!(delivery, ArchcarInputDelivery::Auto);

        let immediate = ArchcarRequest::SendInput {
            session_id: 9,
            input: "adjust course".to_owned(),
            visible_input: None,
            kind: ArchcarInputKind::User,
            delivery: ArchcarInputDelivery::Immediate,
        };
        let json = serde_json::to_string(&immediate).unwrap();
        assert!(json.contains("\"delivery\":\"immediate\""));
        assert_eq!(
            serde_json::from_str::<ArchcarRequest>(&json).unwrap(),
            immediate
        );
    }

    #[test]
    fn queued_chat_input_protocol_round_trips() {
        let request = ArchcarRequest::QueueChatInput {
            thread_id: 42,
            input: "run tests".to_owned(),
            visible_input: Some("visible run tests".to_owned()),
            kind: ArchcarInputKind::User,
            session_kind: SessionKind::Codex,
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"type\":\"queue_chat_input\""));
        assert_eq!(
            archcar_request_summary(&request),
            "queue_chat_input thread_id=42 kind=user session_kind=Codex chars=9"
        );
        assert_eq!(
            serde_json::from_str::<ArchcarRequest>(&json).unwrap(),
            request
        );

        let event = ArchcarEvent::ChatQueueUpdated { thread_id: 42 };
        assert_eq!(
            archcar_event_summary(&event),
            "chat_queue_updated thread_id=42"
        );
        assert_eq!(
            serde_json::from_str::<ArchcarEvent>(&serde_json::to_string(&event).unwrap()).unwrap(),
            event
        );

        for visible_input in [Some("visible run tests".to_owned()), None] {
            let queued = QueuedArchcarInput {
                id: 7,
                thread_id: 42,
                input: "run tests".to_owned(),
                visible_input: visible_input.clone(),
                kind: ArchcarInputKind::User,
                session_kind: SessionKind::Codex,
                created_at: "2026-07-23T12:00:00Z".to_owned(),
                updated_at: "2026-07-23T12:00:01Z".to_owned(),
            };
            let json = serde_json::to_string(&queued).unwrap();
            assert_eq!(
                serde_json::from_str::<QueuedArchcarInput>(&json).unwrap(),
                queued
            );

            let response = ArchcarResponse::QueuedChatInput {
                input: queued.clone(),
            };
            let json = serde_json::to_string(&response).unwrap();
            assert_eq!(
                serde_json::from_str::<ArchcarResponse>(&json).unwrap(),
                response
            );

            let list = ArchcarResponse::QueuedChatInputs {
                thread_id: 42,
                inputs: vec![queued],
            };
            let json = serde_json::to_string(&list).unwrap();
            assert_eq!(
                serde_json::from_str::<ArchcarResponse>(&json).unwrap(),
                list
            );
        }
    }

    #[test]
    fn request_summary_describes_and_round_trips_set_session_model() {
        let request = ArchcarRequest::SetSessionModel {
            session_id: 9,
            model: Some("gpt-5.6-terra".to_owned()),
        };

        assert_eq!(
            archcar_request_summary(&request),
            "set_session_model session_id=9 model=set"
        );
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"type\":\"set_session_model\""));
        let decoded: ArchcarRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, request);

        let reset = ArchcarRequest::SetSessionModel {
            session_id: 9,
            model: None,
        };
        assert_eq!(
            archcar_request_summary(&reset),
            "set_session_model session_id=9 model=default"
        );
        let json = serde_json::to_string(&reset).unwrap();
        assert!(!json.contains("\"model\""));
        let decoded: ArchcarRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, reset);
    }

    #[test]
    fn set_session_effort_and_permission_mode_requests_round_trip() {
        let effort = ArchcarRequest::SetSessionEffort {
            session_id: 7,
            effort: Some("high".to_owned()),
        };
        assert_eq!(
            archcar_request_summary(&effort),
            "set_session_effort session_id=7 effort=high"
        );
        let json = serde_json::to_string(&effort).unwrap();
        assert!(json.contains("\"type\":\"set_session_effort\""));
        assert_eq!(
            serde_json::from_str::<ArchcarRequest>(&json).unwrap(),
            effort
        );

        let permission = ArchcarRequest::SetSessionPermissionMode {
            session_id: 7,
            mode: "default".to_owned(),
        };
        assert_eq!(
            archcar_request_summary(&permission),
            "set_session_permission_mode session_id=7 mode=default"
        );
        let json = serde_json::to_string(&permission).unwrap();
        assert!(json.contains("\"type\":\"set_session_permission_mode\""));
        assert_eq!(
            serde_json::from_str::<ArchcarRequest>(&json).unwrap(),
            permission
        );
    }

    #[test]
    fn request_summary_describes_resize_session() {
        let request = ArchcarRequest::ResizeSession {
            session_id: 9,
            rows: 33,
            cols: 111,
        };

        assert_eq!(
            archcar_request_summary(&request),
            "resize_session session_id=9 rows=33 cols=111"
        );
    }

    #[test]
    fn provider_interaction_protocol_round_trips_and_summarizes_without_bodies() {
        let request = ArchcarRequest::RegisterProviderInteraction {
            interaction: ProviderInteractionDraft {
                provider_key: "claude".to_owned(),
                workspace: "berlin".to_owned(),
                thread_id: 7,
                session_id: 11,
                native_session_id: Some("session-1".to_owned()),
                native_id: "tool-1".to_owned(),
                kind: ProviderInteractionKind::UserQuestion,
                title: "Question".to_owned(),
                detail: "secret detail".to_owned(),
                choices: vec!["yes".to_owned()],
                native_request: serde_json::json!({"prompt":"secret"}),
            },
        };

        let summary = archcar_request_summary(&request);
        assert!(summary.contains("register_provider_interaction"));
        assert!(summary.contains("request_bytes="));
        assert!(!summary.contains("secret"));

        let json = serde_json::to_string(&request).unwrap();
        assert_eq!(
            serde_json::from_str::<ArchcarRequest>(&json).unwrap(),
            request
        );

        let resolve = ArchcarRequest::ResolveProviderInteraction {
            interaction_id: "interaction-1".to_owned(),
            resolution: ProviderInteractionResolution::Deny {
                reason: Some("contains secret token".to_owned()),
            },
        };
        let resolve_summary = archcar_request_summary(&resolve);
        assert!(resolve_summary.contains("interaction-1"));
        assert!(resolve_summary.contains("resolution=deny"));
        assert!(resolve_summary.contains("denial_reason_chars="));
        assert!(!resolve_summary.contains("secret"));
    }

    #[test]
    fn provider_interaction_event_summary_reports_identity_and_status() {
        let interaction = ProviderInteractionRecord {
            id: "interaction-1".to_owned(),
            provider_key: "claude".to_owned(),
            workspace: "berlin".to_owned(),
            thread_id: 7,
            session_id: 11,
            native_session_id: None,
            native_id: "tool-1".to_owned(),
            kind: ProviderInteractionKind::Permission,
            title: "Permission".to_owned(),
            detail: "secret".to_owned(),
            choices: Vec::new(),
            native_request: serde_json::json!({"secret": true}),
            request_fingerprint: "abc".to_owned(),
            status: ProviderInteractionStatus::Pending,
            resolution: None,
            native_response: None,
            error: None,
            created_at: "1".to_owned(),
            resolved_at: None,
            consumed_at: None,
        };
        let event = ArchcarEvent::ProviderInteractionRequested {
            interaction: interaction.clone(),
        };

        let summary = archcar_event_summary(&event);
        assert!(summary.contains("interaction-1"));
        assert!(summary.contains("Pending"));
        assert!(!summary.contains("secret"));
        assert_eq!(
            serde_json::from_str::<ArchcarEvent>(&serde_json::to_string(&event).unwrap()).unwrap(),
            event
        );
    }

    #[test]
    fn request_summary_describes_and_round_trips_interrupt_turn() {
        let request = ArchcarRequest::InterruptTurn { session_id: 9 };

        assert_eq!(
            archcar_request_summary(&request),
            "interrupt_turn session_id=9"
        );
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"type\":\"interrupt_turn\""));
        let decoded: ArchcarRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn archcar_message_skips_absent_codex_metadata_and_round_trips_present_metadata() {
        let message = ArchcarMessage {
            id: 1,
            role: "assistant".to_owned(),
            content: "Running tests".to_owned(),
            source: "codex".to_owned(),
            inline_event: None,
            context_usage: None,
        };
        let json = serde_json::to_string(&message).unwrap();
        assert!(!json.contains("inline_event"));
        assert!(!json.contains("context_usage"));
        let decoded: ArchcarMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, message);

        let message = ArchcarMessage {
            inline_event: Some(CodexInlineEvent::Tool(CodexToolCall {
                namespace: "web".to_owned(),
                name: "run".to_owned(),
                marker: "web.run".to_owned(),
            })),
            context_usage: Some(CodexContextUsage {
                percent: Some(42),
                used_tokens: None,
                total_tokens: None,
            }),
            ..message
        };
        let json = serde_json::to_string(&message).unwrap();
        assert!(json.contains("inline_event"));
        assert!(json.contains("context_usage"));
        let decoded: ArchcarMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, message);
    }

    #[test]
    fn response_summary_describes_spawned_session() {
        let response = ArchcarResponse::SessionSpawned {
            session_id: 7,
            thread_id: 3,
            workspace: "hoi-an".to_owned(),
            kind: SessionKind::Codex,
            pid: 4242,
        };

        assert_eq!(
            archcar_response_summary(&response),
            "session_spawned workspace=hoi-an kind=Codex session_id=7 thread_id=3 pid=4242"
        );
    }

    #[test]
    fn response_summaries_omit_screen_and_message_bodies() {
        let screen_response = ArchcarResponse::SessionScreen {
            session_id: 7,
            screen: "prompt with OPENAI_API_KEY=sk-secret".to_owned(),
        };
        let messages_response = ArchcarResponse::SessionMessages {
            thread_id: 3,
            messages: vec![ArchcarMessage {
                id: 1,
                role: "assistant".to_owned(),
                content: "staged review prompt: keep this private".to_owned(),
                source: "agent_screen_parse".to_owned(),
                inline_event: None,
                context_usage: None,
            }],
        };

        let screen_summary = archcar_response_summary(&screen_response);
        let messages_summary = archcar_response_summary(&messages_response);

        assert_eq!(screen_summary, "session_screen session_id=7 chars=36");
        assert_eq!(messages_summary, "session_messages thread_id=3 count=1");
        assert!(!screen_summary.contains("sk-secret"));
        assert!(!messages_summary.contains("staged review prompt"));
    }

    #[test]
    fn event_summary_describes_ready_session() {
        let event = ArchcarEvent::SessionReady {
            session_id: 11,
            thread_id: 5,
        };

        assert_eq!(
            archcar_event_summary(&event),
            "session_ready session_id=11 thread_id=5"
        );
    }

    #[test]
    fn event_summary_describes_completed_turn_boundary() {
        let event = ArchcarEvent::TurnCompleted {
            session_id: 11,
            thread_id: 5,
            status: Some("cancelled".to_owned()),
        };

        assert_eq!(
            archcar_event_summary(&event),
            "turn_completed session_id=11 thread_id=5 status=cancelled"
        );
    }

    #[test]
    fn session_capabilities_event_serializes_descriptor_payload() {
        let capabilities = SessionHarnessCapabilities {
            contract_version: 1,
            required: vec!["preflight".to_owned(), "streaming_events".to_owned()],
            optional: vec![SessionCapabilitySupport {
                name: "goals".to_owned(),
                mode: "native".to_owned(),
                reason: None,
            }],
            observed_native: vec!["streaming".to_owned()],
        };
        let event = ArchcarEvent::SessionCapabilitiesChanged {
            session_id: 11,
            thread_id: 5,
            capabilities: capabilities.clone(),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"session_capabilities_changed\""));
        assert!(json.contains("\"observed_native\":[\"streaming\"]"));
        assert_eq!(serde_json::from_str::<ArchcarEvent>(&json).unwrap(), event);
        assert_eq!(
            archcar_event_summary(&event),
            "session_capabilities_changed session_id=11 thread_id=5 required=2 optional=1 observed_native=1"
        );
    }

    #[test]
    fn session_status_response_carries_typed_runtime_state() {
        let response = ArchcarResponse::SessionStatus {
            session_id: 11,
            status: "running".to_owned(),
            ready: false,
            runtime_state: crate::session_state::AgentSessionState::ToolRunning,
            capabilities: Some(SessionHarnessCapabilities {
                contract_version: 1,
                required: vec!["preflight".to_owned()],
                optional: Vec::new(),
                observed_native: Vec::new(),
            }),
        };

        let encoded = serde_json::to_string(&response).unwrap();

        assert!(encoded.contains("\"runtime_state\":\"tool_running\""));
        assert!(encoded.contains("\"capabilities\""));
        assert_eq!(
            archcar_response_summary(&response),
            "session_status session_id=11 status=running state=tool_running ready=false"
        );
    }
}
