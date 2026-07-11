use crate::codex_tui::{
    codex_screen_ready_for_input, detect_directory_trust_prompt, CodexParseBenchmark,
    CodexParseCursor,
};
use crate::session_event::{
    parse_codex_screen_event_delta, SessionCommandOutputStatus, SessionEvent, SessionEventPayload,
    SessionEventSource, SessionEventStatus, SessionPromptOption, SessionPromptStyle,
};
use crate::session_state::{AgentSessionState, SessionStateMachine};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtyChunkInput {
    pub sequence: u64,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct SessionPipelineInput {
    pub chunks: Vec<PtyChunkInput>,
    pub screen: String,
    pub benchmark: CodexParseBenchmark,
    pub previous_cursor: Option<CodexParseCursor>,
    pub previous_state: AgentSessionState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPipelineOutput {
    pub chunk_range: Option<(u64, u64)>,
    pub normalized_text: String,
    pub events: Vec<SessionEvent>,
    pub cursor: CodexParseCursor,
    pub state: AgentSessionState,
    pub ready_for_input: bool,
    pub trust_prompt: bool,
}

pub fn process_codex_pty_pipeline(input: SessionPipelineInput) -> SessionPipelineOutput {
    let chunk_range = input
        .chunks
        .iter()
        .map(|chunk| chunk.sequence)
        .min()
        .zip(input.chunks.iter().map(|chunk| chunk.sequence).max());
    let normalized_text = input
        .chunks
        .iter()
        .map(|chunk| chunk.text.as_str())
        .collect::<String>();
    let trust_prompt = detect_directory_trust_prompt(&input.screen);
    let delta = parse_codex_screen_event_delta(
        &input.screen,
        &input.benchmark,
        input.previous_cursor.as_ref(),
    );
    let mut events = delta.events;
    let ready_for_input =
        codex_ready_for_input(&input.screen) && !events.iter().any(is_running_tool_event);

    if trust_prompt && !events.iter().any(is_prompt_event) {
        events.push(codex_trust_prompt_event(&input.screen));
    }
    if ready_for_input && input.previous_state != AgentSessionState::WaitingForInput {
        events.push(SessionEvent::new(
            SessionEventSource::System,
            None,
            SessionEventPayload::StatusChange {
                status: SessionEventStatus::WaitingForInput,
                message: Some("ready for input".to_owned()),
            },
        ));
    }

    let mut machine = SessionStateMachine::from_state(input.previous_state);
    machine.apply_events(events.iter());

    SessionPipelineOutput {
        chunk_range,
        normalized_text,
        events,
        cursor: delta.cursor,
        state: machine.state(),
        ready_for_input,
        trust_prompt,
    }
}

fn is_prompt_event(event: &SessionEvent) -> bool {
    matches!(event.payload, SessionEventPayload::Prompt { .. })
}

fn is_running_tool_event(event: &SessionEvent) -> bool {
    matches!(
        event.payload,
        SessionEventPayload::CommandOutput {
            status: SessionCommandOutputStatus::Running,
            ..
        }
    )
}

fn codex_ready_for_input(screen: &str) -> bool {
    codex_screen_ready_for_input(screen)
}

fn codex_trust_prompt_event(screen: &str) -> SessionEvent {
    let text = screen
        .lines()
        .find(|line| line.contains("Do you trust the contents of this directory?"))
        .unwrap_or("Do you trust the contents of this directory?")
        .trim()
        .to_owned();
    SessionEvent::new(
        SessionEventSource::System,
        Some(text.clone()),
        SessionEventPayload::Prompt {
            style: SessionPromptStyle::Confirmation,
            text,
            options: vec![
                SessionPromptOption {
                    label: "Yes, continue".to_owned(),
                    value: "yes".to_owned(),
                },
                SessionPromptOption {
                    label: "No, exit".to_owned(),
                    value: "no".to_owned(),
                },
            ],
        },
    )
}
