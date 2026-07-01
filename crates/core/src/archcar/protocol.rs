use serde::{Deserialize, Serialize};

use crate::workspace::{SessionHarnessOptions, SessionKind};

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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ArchcarRequest {
    EnsureWorkspaceDefaultSession {
        workspace: String,
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
        kind: ArchcarInputKind,
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
    KillSession {
        session_id: i64,
    },
    Subscribe,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
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
        ready: bool,
    },
    SessionScreen {
        session_id: i64,
        screen: String,
    },
    SessionMessages {
        thread_id: i64,
        messages: Vec<ArchcarMessage>,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchcarMessage {
    pub id: i64,
    pub role: String,
    pub content: String,
    pub source: String,
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
            kind,
        } => format!(
            "send_input session_id={session_id} kind={} chars={}",
            input_kind_label(kind),
            input.chars().count()
        ),
        ArchcarRequest::GetSessionStatus { session_id } => {
            format!("get_session_status session_id={session_id}")
        }
        ArchcarRequest::GetSessionScreen { session_id } => {
            format!("get_session_screen session_id={session_id}")
        }
        ArchcarRequest::GetSessionMessages { thread_id } => {
            format!("get_session_messages thread_id={thread_id}")
        }
        ArchcarRequest::KillSession { session_id } => {
            format!("kill_session session_id={session_id}")
        }
        ArchcarRequest::Subscribe => "subscribe".to_owned(),
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
            ready,
        } => format!("session_status session_id={session_id} status={status} ready={ready}"),
        ArchcarResponse::SessionScreen { session_id, screen } => format!(
            "session_screen session_id={session_id} chars={}",
            screen.chars().count()
        ),
        ArchcarResponse::SessionMessages { thread_id, messages } => format!(
            "session_messages thread_id={thread_id} count={}",
            messages.len()
        ),
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
        ArchcarEvent::SessionScreenUpdated { session_id } => {
            format!("session_screen_updated session_id={session_id}")
        }
        ArchcarEvent::SessionMessagesUpdated { thread_id } => {
            format!("session_messages_updated thread_id={thread_id}")
        }
        ArchcarEvent::SessionExited {
            session_id,
            exit_code,
        } => format!("session_exited session_id={session_id} exit_code={exit_code:?}"),
        ArchcarEvent::SessionError {
            session_id,
            message,
        } => format!(
            "session_error session_id={session_id:?} chars={}",
            message.chars().count()
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
    SessionScreenUpdated {
        session_id: i64,
    },
    SessionMessagesUpdated {
        thread_id: i64,
    },
    SessionExited {
        session_id: i64,
        exit_code: Option<i32>,
    },
    SessionError {
        session_id: Option<i64>,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

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
            kind: ArchcarInputKind::User,
        };

        assert_eq!(
            archcar_request_summary(&request),
            "send_input session_id=9 kind=user chars=9"
        );
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
}
