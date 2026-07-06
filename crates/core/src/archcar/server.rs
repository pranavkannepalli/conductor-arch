use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::archcar::protocol::{
    archcar_event_summary, archcar_request_summary, archcar_response_summary, ArchcarEvent,
    ArchcarMessage, ArchcarRequest, ArchcarResponse, RpcEnvelope,
};
use crate::archcar::session::{restore_managed_session, spawn_managed_session, SessionHandle};
use crate::paths::AppPaths;
use crate::workspace::{SessionKind, WorkspaceStore};

pub struct ArchcarServer {
    listener: UnixListener,
    state: Arc<Mutex<ServerState>>,
}

struct ServerState {
    db_path: PathBuf,
    logs_dir: PathBuf,
    queued_defaults: HashSet<String>,
    sessions: HashMap<i64, SessionHandle>,
    subscribers: Vec<Sender<ArchcarEvent>>,
}

pub fn reconcile_managed_sessions_on_startup(paths: &AppPaths) -> Result<()> {
    let store = WorkspaceStore::open(&paths.database_path)?;
    for workspace in store.list()? {
        let records = store.list_sessions(&workspace.name)?;
        for record in persisted_running_session_candidates(&records, SessionKind::Codex) {
            if !is_archcar_managed_persisted_session(&record, &paths.logs_dir) {
                continue;
            }
            if archcar_process_alive(record.pid) {
                continue;
            }
            let _ = store.mark_session_process_exited(record.id, None)?;
        }
    }
    Ok(())
}

fn archcar_process_alive(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

impl ArchcarServer {
    pub fn bind(paths: AppPaths) -> Result<Self> {
        fs::create_dir_all(&paths.state_dir)?;
        let socket_path = paths.archcar_socket_path();
        if socket_path.exists() {
            let _ = fs::remove_file(&socket_path);
        }
        let listener = UnixListener::bind(&socket_path)
            .with_context(|| format!("bind archcar socket {}", socket_path.display()))?;
        let state = Arc::new(Mutex::new(ServerState {
            db_path: paths.database_path,
            logs_dir: paths.logs_dir,
            queued_defaults: HashSet::new(),
            sessions: HashMap::new(),
            subscribers: Vec::new(),
        }));
        Ok(Self { listener, state })
    }

    pub fn serve(self) -> Result<()> {
        for stream in self.listener.incoming() {
            let stream = stream?;
            let state = self.state.clone();
            std::thread::spawn(move || {
                let _ = handle_connection(stream, state);
            });
        }
        Ok(())
    }
}

fn handle_connection(stream: UnixStream, state: Arc<Mutex<ServerState>>) -> Result<()> {
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.trim().is_empty() {
        return Ok(());
    }
    let envelope: RpcEnvelope<ArchcarRequest> = serde_json::from_str(&line)?;
    log_archcar_rpc(
        &envelope.id,
        "recv",
        "request",
        archcar_request_summary(&envelope.payload),
        line.trim_end(),
    );
    match envelope.payload {
        ArchcarRequest::Subscribe => {
            let (tx, rx) = mpsc::channel();
            state.lock().unwrap().subscribers.push(tx);
            while let Ok(event) = rx.recv() {
                let envelope = RpcEnvelope {
                    id: Uuid::new_v4().to_string(),
                    payload: event,
                };
                let line = serde_json::to_string(&envelope)?;
                log_archcar_rpc(
                    &envelope.id,
                    "send",
                    "event",
                    archcar_event_summary(&envelope.payload),
                    &line,
                );
                writer.write_all(line.as_bytes())?;
                writer.write_all(b"\n")?;
                writer.flush()?;
            }
        }
        request => {
            let response = dispatch_request(request, &state);
            let envelope = RpcEnvelope {
                id: envelope.id,
                payload: response,
            };
            let line = serde_json::to_string(&envelope)?;
            log_archcar_rpc(
                &envelope.id,
                "send",
                "response",
                archcar_response_summary(&envelope.payload),
                &line,
            );
            writer.write_all(line.as_bytes())?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
    }
    Ok(())
}

fn log_archcar_rpc(
    rpc_id: &str,
    direction: &str,
    message_type: &str,
    summary: String,
    raw_payload: &str,
) {
    if let Some(payload) = archcar_rpc_log_payload(raw_payload) {
        info!(
            %rpc_id,
            direction,
            message_type,
            summary = %summary,
            payload = %payload,
            "archcar unix rpc"
        );
    } else {
        info!(
            %rpc_id,
            direction,
            message_type,
            summary = %summary,
            "archcar unix rpc"
        );
    }
}

fn archcar_rpc_log_payload(raw_payload: &str) -> Option<String> {
    archcar_rpc_log_payload_for_flag(
        raw_payload,
        crate::env_flags::enabled("ARCHDUCTOR_LOG_ARCHCAR_PAYLOADS"),
    )
}

fn archcar_rpc_log_payload_for_flag(raw_payload: &str, enabled: bool) -> Option<String> {
    enabled.then(|| crate::redaction::redact_sensitive_text(raw_payload))
}

fn dispatch_request(request: ArchcarRequest, state: &Arc<Mutex<ServerState>>) -> ArchcarResponse {
    match request {
        ArchcarRequest::EnsureWorkspaceDefaultSession {
            workspace,
            kind,
            harness,
        } => ensure_default_session(state, workspace, kind, harness.unwrap_or_default()),
        ArchcarRequest::SpawnSession {
            workspace,
            kind,
            harness,
        } => spawn_session(state, workspace, kind, harness.unwrap_or_default()),
        ArchcarRequest::SendInput {
            session_id,
            input,
            kind,
        } => match load_or_restore_session_handle(state, session_id) {
            Ok(Some(handle)) => {
                match handle
                    .command_tx
                    .send(crate::archcar::session::SessionCommand::SendInput { input, kind })
                {
                    Ok(_) => ArchcarResponse::Ack,
                    Err(err) => ArchcarResponse::Error {
                        message: err.to_string(),
                    },
                }
            }
            Ok(None) => ArchcarResponse::Error {
                message: format!("unknown session {session_id}"),
            },
            Err(err) => ArchcarResponse::Error {
                message: err.to_string(),
            },
        },
        ArchcarRequest::ResizeSession {
            session_id,
            rows,
            cols,
        } => match load_or_restore_session_handle(state, session_id) {
            Ok(Some(handle)) => {
                match handle
                    .command_tx
                    .send(crate::archcar::session::SessionCommand::Resize { rows, cols })
                {
                    Ok(_) => ArchcarResponse::Ack,
                    Err(err) => ArchcarResponse::Error {
                        message: err.to_string(),
                    },
                }
            }
            Ok(None) => ArchcarResponse::Error {
                message: format!("unknown session {session_id}"),
            },
            Err(err) => ArchcarResponse::Error {
                message: err.to_string(),
            },
        },
        ArchcarRequest::GetSessionStatus { session_id } => {
            match load_or_restore_session_handle(state, session_id) {
                Ok(Some(handle)) => {
                    let snapshot = handle.snapshot.lock().unwrap().clone();
                    ArchcarResponse::SessionStatus {
                        session_id,
                        status: snapshot.status.as_str().to_owned(),
                        runtime_state: snapshot.runtime_state,
                        ready: snapshot.ready,
                    }
                }
                Ok(None) => ArchcarResponse::Error {
                    message: format!("unknown session {session_id}"),
                },
                Err(err) => ArchcarResponse::Error {
                    message: err.to_string(),
                },
            }
        }
        ArchcarRequest::GetSessionScreen { session_id } => {
            match load_or_restore_session_handle(state, session_id) {
                Ok(Some(handle)) => {
                    let snapshot = handle.snapshot.lock().unwrap().clone();
                    ArchcarResponse::SessionScreen {
                        session_id,
                        screen: snapshot.screen,
                    }
                }
                Ok(None) => ArchcarResponse::Error {
                    message: format!("unknown session {session_id}"),
                },
                Err(err) => ArchcarResponse::Error {
                    message: err.to_string(),
                },
            }
        }
        ArchcarRequest::GetSessionMessages { thread_id } => {
            let db_path = state.lock().unwrap().db_path.clone();
            match WorkspaceStore::open(&db_path)
                .and_then(|store| store.list_chat_messages(thread_id))
            {
                Ok(messages) => ArchcarResponse::SessionMessages {
                    thread_id,
                    messages: messages
                        .into_iter()
                        .map(|message| ArchcarMessage {
                            id: message.id,
                            role: message.role,
                            content: message.content,
                            source: message.source,
                            inline_event: None,
                            context_usage: None,
                        })
                        .collect(),
                },
                Err(err) => ArchcarResponse::Error {
                    message: err.to_string(),
                },
            }
        }
        ArchcarRequest::KillSession { session_id } => {
            match load_or_restore_session_handle(state, session_id) {
                Ok(Some(handle)) => {
                    match handle
                        .command_tx
                        .send(crate::archcar::session::SessionCommand::Kill)
                    {
                        Ok(_) => ArchcarResponse::Ack,
                        Err(err) => ArchcarResponse::Error {
                            message: err.to_string(),
                        },
                    }
                }
                Ok(None) => ArchcarResponse::Error {
                    message: format!("unknown session {session_id}"),
                },
                Err(err) => ArchcarResponse::Error {
                    message: err.to_string(),
                },
            }
        }
        ArchcarRequest::Subscribe => ArchcarResponse::Error {
            message: "subscribe must use a persistent connection".to_owned(),
        },
    }
}

fn ensure_default_session(
    state: &Arc<Mutex<ServerState>>,
    workspace: String,
    kind: SessionKind,
    harness: crate::workspace::SessionHarnessOptions,
) -> ArchcarResponse {
    if kind != SessionKind::Codex {
        return ArchcarResponse::Error {
            message: "only codex auto-spawn is implemented".to_owned(),
        };
    }
    let mut guard = state.lock().unwrap();
    if let Some((session_id, thread_id, pid, ready)) = guard
        .sessions
        .values()
        .filter_map(|handle| {
            let snapshot = handle.snapshot.lock().ok()?.clone();
            (snapshot.workspace == workspace
                && snapshot.kind == kind
                && snapshot.status == crate::workspace::ProcessStatus::Running)
                .then_some((
                    snapshot.session_id,
                    snapshot.thread_id,
                    snapshot.pid,
                    snapshot.ready,
                ))
        })
        .max_by_key(|(session_id, _, _, _)| *session_id)
    {
        if ready {
            broadcast(
                &mut guard,
                ArchcarEvent::SessionReady {
                    session_id,
                    thread_id,
                },
            );
        }
        return ArchcarResponse::SessionSpawned {
            session_id,
            thread_id,
            workspace,
            kind,
            pid,
        };
    }
    drop(guard);

    if let Some(response) = restore_workspace_session_from_store(state, &workspace, kind) {
        return response;
    }

    let mut guard = state.lock().unwrap();
    if !guard.queued_defaults.insert(workspace.clone()) {
        return ArchcarResponse::SessionSpawnQueued { workspace, kind };
    }
    let db_path = guard.db_path.clone();
    let logs_dir = guard.logs_dir.clone();
    let state_for_spawn = state.clone();
    broadcast(
        &mut guard,
        ArchcarEvent::SessionSpawnQueued {
            workspace: workspace.clone(),
            kind,
        },
    );
    info!(%workspace, ?kind, "archcar queued default session spawn");
    drop(guard);

    let workspace_for_spawn = workspace.clone();
    std::thread::spawn(move || {
        let (event_tx, event_rx) = mpsc::channel();
        match spawn_managed_session(
            db_path,
            logs_dir,
            workspace_for_spawn.clone(),
            kind,
            harness,
            event_tx,
        ) {
            Ok(handle) => {
                let session_id = handle.snapshot.lock().unwrap().session_id;
                info!(%workspace_for_spawn, session_id, ?kind, "archcar spawned managed session");
                let mut guard = state_for_spawn.lock().unwrap();
                guard.sessions.insert(session_id, handle);
                guard.queued_defaults.remove(&workspace_for_spawn);
                drop(guard);
                while let Ok(event) = event_rx.recv() {
                    let mut guard = state_for_spawn.lock().unwrap();
                    if let ArchcarEvent::SessionExited { session_id, .. } = &event {
                        guard.sessions.remove(session_id);
                    }
                    broadcast(&mut guard, event);
                }
            }
            Err(err) => {
                let detail = format!("{err:#}");
                error!(%workspace_for_spawn, ?kind, error = %detail, "archcar failed to spawn managed session");
                let mut guard = state_for_spawn.lock().unwrap();
                guard.queued_defaults.remove(&workspace_for_spawn);
                broadcast(
                    &mut guard,
                    ArchcarEvent::SessionError {
                        session_id: None,
                        message: detail,
                    },
                );
            }
        }
    });

    ArchcarResponse::SessionSpawnQueued { workspace, kind }
}

fn restore_workspace_session_from_store(
    state: &Arc<Mutex<ServerState>>,
    workspace: &str,
    kind: SessionKind,
) -> Option<ArchcarResponse> {
    let db_path = state.lock().ok()?.db_path.clone();
    let store = match WorkspaceStore::open(&db_path) {
        Ok(store) => store,
        Err(err) => {
            warn!(
                workspace,
                ?kind,
                error = %format!("{err:#}"),
                "archcar failed to open workspace store for persisted session restore"
            );
            return None;
        }
    };
    let records = match store.list_sessions(workspace) {
        Ok(records) => records,
        Err(err) => {
            warn!(
                workspace,
                ?kind,
                error = %format!("{err:#}"),
                "archcar failed to list persisted sessions for restore"
            );
            return None;
        }
    };

    for record in persisted_running_session_candidates(&records, kind) {
        match load_or_restore_session_handle(state, record.id) {
            Ok(Some(handle)) => {
                let snapshot = match handle.snapshot.lock() {
                    Ok(snapshot) => snapshot.clone(),
                    Err(_) => continue,
                };
                if snapshot.workspace != workspace
                    || snapshot.kind != kind
                    || snapshot.status != crate::workspace::ProcessStatus::Running
                {
                    continue;
                }

                let mut guard = match state.lock() {
                    Ok(guard) => guard,
                    Err(_) => return None,
                };
                if snapshot.ready {
                    broadcast(
                        &mut guard,
                        ArchcarEvent::SessionReady {
                            session_id: snapshot.session_id,
                            thread_id: snapshot.thread_id,
                        },
                    );
                }
                info!(
                    workspace,
                    ?kind,
                    session_id = snapshot.session_id,
                    thread_id = snapshot.thread_id,
                    pid = snapshot.pid,
                    "archcar restored persisted workspace session"
                );
                return Some(ArchcarResponse::SessionSpawned {
                    session_id: snapshot.session_id,
                    thread_id: snapshot.thread_id,
                    workspace: snapshot.workspace,
                    kind: snapshot.kind,
                    pid: snapshot.pid,
                });
            }
            Ok(None) => {}
            Err(err) => {
                warn!(
                    workspace,
                    ?kind,
                    session_id = record.id,
                    error = %format!("{err:#}"),
                    "archcar failed to restore persisted session candidate"
                );
            }
        }
    }

    None
}

fn persisted_running_session_candidates(
    records: &[crate::workspace::ProcessRecord],
    kind: SessionKind,
) -> Vec<crate::workspace::ProcessRecord> {
    records
        .iter()
        .filter(|record| {
            record.status == crate::workspace::ProcessStatus::Running
                && record.chat_thread_id.is_some()
                && session_kind_matches_command(&record.command, kind)
        })
        .cloned()
        .collect()
}

fn is_archcar_managed_persisted_session(
    record: &crate::workspace::ProcessRecord,
    state_logs_dir: &Path,
) -> bool {
    record.log_path.starts_with(state_logs_dir)
}

fn session_kind_matches_command(command: &str, kind: SessionKind) -> bool {
    let trimmed = command.trim();
    match kind {
        SessionKind::Codex => trimmed == "codex" || trimmed.starts_with("codex "),
        SessionKind::Claude => trimmed == "claude" || trimmed.starts_with("claude "),
        SessionKind::Shell => {
            !(trimmed == "codex"
                || trimmed.starts_with("codex ")
                || trimmed == "claude"
                || trimmed.starts_with("claude "))
        }
    }
}

fn spawn_session(
    state: &Arc<Mutex<ServerState>>,
    workspace: String,
    kind: SessionKind,
    harness: crate::workspace::SessionHarnessOptions,
) -> ArchcarResponse {
    let mut guard = state.lock().unwrap();
    let db_path = guard.db_path.clone();
    let logs_dir = guard.logs_dir.clone();
    let state_for_spawn = state.clone();
    broadcast(
        &mut guard,
        ArchcarEvent::SessionSpawnQueued {
            workspace: workspace.clone(),
            kind,
        },
    );
    info!(%workspace, ?kind, "archcar queued explicit session spawn");
    drop(guard);

    let workspace_for_spawn = workspace.clone();
    std::thread::spawn(move || {
        let (event_tx, event_rx) = mpsc::channel();
        match spawn_managed_session(
            db_path,
            logs_dir,
            workspace_for_spawn.clone(),
            kind,
            harness,
            event_tx,
        ) {
            Ok(handle) => {
                let session_id = handle.snapshot.lock().unwrap().session_id;
                info!(%workspace_for_spawn, session_id, ?kind, "archcar spawned explicit managed session");
                let mut guard = state_for_spawn.lock().unwrap();
                guard.sessions.insert(session_id, handle);
                drop(guard);
                while let Ok(event) = event_rx.recv() {
                    let mut guard = state_for_spawn.lock().unwrap();
                    if let ArchcarEvent::SessionExited { session_id, .. } = &event {
                        guard.sessions.remove(session_id);
                    }
                    broadcast(&mut guard, event);
                }
            }
            Err(err) => {
                let detail = format!("{err:#}");
                error!(%workspace_for_spawn, ?kind, error = %detail, "archcar failed to spawn explicit managed session");
                let mut guard = state_for_spawn.lock().unwrap();
                broadcast(
                    &mut guard,
                    ArchcarEvent::SessionError {
                        session_id: None,
                        message: detail,
                    },
                );
            }
        }
    });

    ArchcarResponse::SessionSpawnQueued { workspace, kind }
}

fn load_or_restore_session_handle(
    state: &Arc<Mutex<ServerState>>,
    session_id: i64,
) -> Result<Option<SessionHandle>> {
    if let Some(handle) = state.lock().unwrap().sessions.get(&session_id).cloned() {
        return Ok(Some(handle));
    }

    let (db_path, logs_dir) = {
        let guard = state.lock().unwrap();
        (guard.db_path.clone(), guard.logs_dir.clone())
    };
    let (event_tx, event_rx) = mpsc::channel();
    let Some(handle) = restore_managed_session(db_path, logs_dir, session_id, event_tx)? else {
        warn!(
            session_id,
            "archcar could not restore unknown session from persistent state"
        );
        return Ok(None);
    };

    let inserted = {
        let mut guard = state.lock().unwrap();
        if let Some(existing) = guard.sessions.get(&session_id).cloned() {
            return Ok(Some(existing));
        }
        guard.sessions.insert(session_id, handle.clone());
        info!(session_id, "archcar restored session into active state");
        true
    };

    if inserted {
        let state_for_events = Arc::clone(state);
        std::thread::spawn(move || {
            while let Ok(event) = event_rx.recv() {
                let mut guard = state_for_events.lock().unwrap();
                if let ArchcarEvent::SessionExited { session_id, .. } = &event {
                    guard.sessions.remove(session_id);
                }
                broadcast(&mut guard, event);
            }
        });
    }

    Ok(Some(handle))
}

fn broadcast(state: &mut ServerState, event: ArchcarEvent) {
    state
        .subscribers
        .retain(|subscriber| subscriber.send(event.clone()).is_ok());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archcar::protocol::ArchcarInputKind;
    use std::fs;
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use crate::paths::AppPaths;
    use crate::repository::{AddRepository, RepositoryStore};
    use crate::workspace::{CreateWorkspace, ProcessStatus};

    #[test]
    fn ensure_default_session_debounces_repeat_requests() {
        let state = Arc::new(Mutex::new(ServerState {
            db_path: PathBuf::from("/tmp/does-not-matter.db"),
            logs_dir: PathBuf::from("/tmp/does-not-matter-logs"),
            queued_defaults: HashSet::new(),
            sessions: HashMap::new(),
            subscribers: Vec::new(),
        }));
        let first = ensure_default_session(
            &state,
            "berlin".to_owned(),
            SessionKind::Codex,
            crate::workspace::SessionHarnessOptions::default(),
        );
        let second = ensure_default_session(
            &state,
            "berlin".to_owned(),
            SessionKind::Codex,
            crate::workspace::SessionHarnessOptions::default(),
        );
        assert_eq!(
            first,
            ArchcarResponse::SessionSpawnQueued {
                workspace: "berlin".to_owned(),
                kind: SessionKind::Codex,
            }
        );
        assert_eq!(
            second,
            ArchcarResponse::SessionSpawnQueued {
                workspace: "berlin".to_owned(),
                kind: SessionKind::Codex,
            }
        );
    }

    #[test]
    fn explicit_spawn_session_accepts_shell_runtime_requests() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("state.db");
        let logs_dir = temp.path().join("logs");
        let state = Arc::new(Mutex::new(ServerState {
            db_path,
            logs_dir,
            queued_defaults: HashSet::new(),
            sessions: HashMap::new(),
            subscribers: Vec::new(),
        }));

        let response = spawn_session(
            &state,
            "missing-workspace".to_owned(),
            SessionKind::Shell,
            crate::workspace::SessionHarnessOptions::default(),
        );

        assert_ne!(
            response,
            ArchcarResponse::Error {
                message: "only codex auto-spawn is implemented".to_owned(),
            }
        );
    }

    #[test]
    fn archcar_rpc_log_payload_is_omitted_by_default_for_send_input() {
        let envelope = RpcEnvelope {
            id: "abc".to_owned(),
            payload: ArchcarRequest::SendInput {
                session_id: 42,
                input: "paste OPENAI_API_KEY=sk-secret into session".to_owned(),
                kind: ArchcarInputKind::User,
            },
        };
        let line = serde_json::to_string(&envelope).unwrap();

        assert_eq!(archcar_rpc_log_payload(&line), None);
        assert_eq!(
            archcar_request_summary(&envelope.payload),
            "send_input session_id=42 kind=user chars=43"
        );
    }

    #[test]
    fn archcar_rpc_log_payload_redacts_sensitive_values_when_payload_logging_is_enabled() {
        let envelope = RpcEnvelope {
            id: "abc".to_owned(),
            payload: ArchcarRequest::SendInput {
                session_id: 42,
                input: "paste OPENAI_API_KEY=sk-secret bearer ghp_secret --password swordfish"
                    .to_owned(),
                kind: ArchcarInputKind::User,
            },
        };
        let line = serde_json::to_string(&envelope).unwrap();

        let payload = archcar_rpc_log_payload_for_flag(&line, true).unwrap();

        assert!(payload.contains("[redacted]"));
        assert!(!payload.contains("sk-secret"));
        assert!(!payload.contains("ghp_secret"));
        assert!(!payload.contains("swordfish"));
    }

    #[test]
    fn ensure_default_session_reuses_existing_running_session() {
        let snapshot = crate::archcar::session::SessionSnapshot {
            session_id: 9,
            thread_id: 4,
            workspace: "berlin".to_owned(),
            kind: SessionKind::Codex,
            pid: 12345,
            status: crate::workspace::ProcessStatus::Running,
            runtime_state: crate::session_state::AgentSessionState::WaitingForInput,
            ready: true,
            screen: String::new(),
        };
        let (command_tx, _command_rx) = mpsc::channel();
        let mut sessions = HashMap::new();
        sessions.insert(
            snapshot.session_id,
            crate::archcar::session::SessionHandle {
                snapshot: Arc::new(Mutex::new(snapshot)),
                command_tx,
            },
        );
        let state = Arc::new(Mutex::new(ServerState {
            db_path: PathBuf::from("/tmp/does-not-matter.db"),
            logs_dir: PathBuf::from("/tmp/does-not-matter-logs"),
            queued_defaults: HashSet::new(),
            sessions,
            subscribers: Vec::new(),
        }));

        let response = ensure_default_session(
            &state,
            "berlin".to_owned(),
            SessionKind::Codex,
            crate::workspace::SessionHarnessOptions::default(),
        );

        assert_eq!(
            response,
            ArchcarResponse::SessionSpawned {
                session_id: 9,
                thread_id: 4,
                workspace: "berlin".to_owned(),
                kind: SessionKind::Codex,
                pid: 12345,
            }
        );
        assert!(state.lock().unwrap().queued_defaults.is_empty());
    }

    #[test]
    fn persisted_running_session_candidates_preserve_store_descending_order() {
        let records = vec![
            crate::workspace::ProcessRecord {
                id: 6,
                workspace_id: 1,
                chat_thread_id: Some(60),
                kind: crate::workspace::ProcessKind::Session,
                command: "codex".to_owned(),
                pid: 666,
                log_path: "/tmp/6.log".into(),
                status: crate::workspace::ProcessStatus::Exited,
                started_at: "2026-06-28T00:00:03Z".to_owned(),
                exit_code: Some(0),
                ended_at: Some("2026-06-28T00:00:04Z".to_owned()),
                session_harness_metadata: None,
                session_resume_id: None,
            },
            crate::workspace::ProcessRecord {
                id: 5,
                workspace_id: 1,
                chat_thread_id: Some(50),
                kind: crate::workspace::ProcessKind::Session,
                command: "codex resume --last".to_owned(),
                pid: 555,
                log_path: "/tmp/5.log".into(),
                status: crate::workspace::ProcessStatus::Running,
                started_at: "2026-06-28T00:00:02Z".to_owned(),
                exit_code: None,
                ended_at: None,
                session_harness_metadata: None,
                session_resume_id: None,
            },
            crate::workspace::ProcessRecord {
                id: 4,
                workspace_id: 1,
                chat_thread_id: Some(40),
                kind: crate::workspace::ProcessKind::Session,
                command: "claude".to_owned(),
                pid: 444,
                log_path: "/tmp/4.log".into(),
                status: crate::workspace::ProcessStatus::Running,
                started_at: "2026-06-28T00:00:01Z".to_owned(),
                exit_code: None,
                ended_at: None,
                session_harness_metadata: None,
                session_resume_id: None,
            },
            crate::workspace::ProcessRecord {
                id: 3,
                workspace_id: 1,
                chat_thread_id: Some(30),
                kind: crate::workspace::ProcessKind::Session,
                command: "codex --no-alt-screen".to_owned(),
                pid: 333,
                log_path: "/tmp/3.log".into(),
                status: crate::workspace::ProcessStatus::Running,
                started_at: "2026-06-28T00:00:00Z".to_owned(),
                exit_code: None,
                ended_at: None,
                session_harness_metadata: None,
                session_resume_id: None,
            },
        ];

        assert_eq!(
            persisted_running_session_candidates(&records, SessionKind::Codex)
                .into_iter()
                .map(|record| record.id)
                .collect::<Vec<_>>(),
            vec![5, 3]
        );
        assert_eq!(
            persisted_running_session_candidates(&records, SessionKind::Claude)
                .into_iter()
                .map(|record| record.id)
                .collect::<Vec<_>>(),
            vec![4]
        );
    }

    #[test]
    fn reconcile_startup_leaves_live_managed_codex_sessions_running() {
        let temp = tempfile::tempdir().unwrap();
        let paths = app_paths(temp.path());
        let store = seeded_workspace_store(&paths.database_path, &paths.logs_dir, temp.path());
        let thread = store
            .create_chat_thread("berlin", "codex", "Codex", None)
            .unwrap();
        let launch = store.session_launch("berlin", SessionKind::Codex).unwrap();
        let mut child = spawn_fake_managed_codex_process();
        let process = store
            .record_session_process_for_thread("berlin", thread.id, &launch, child.id())
            .unwrap();
        assert!(
            wait_for_fake_codex_child_alive(child.id()),
            "fake codex child should be alive before reconciliation"
        );

        reconcile_managed_sessions_on_startup(&paths).unwrap();

        assert!(
            child.try_wait().unwrap().is_none(),
            "startup reconciliation should not signal live pids"
        );
        let reconciled = store.get_process_record(process.id).unwrap();
        assert_eq!(reconciled.status, ProcessStatus::Running);
        assert!(reconciled.ended_at.is_none());
        assert!(reconciled.log_path.starts_with(&paths.logs_dir));

        terminate_test_child(&mut child);
    }

    #[test]
    fn reconcile_startup_marks_dead_managed_codex_sessions_exited() {
        let temp = tempfile::tempdir().unwrap();
        let paths = app_paths(temp.path());
        let store = seeded_workspace_store(&paths.database_path, &paths.logs_dir, temp.path());
        let thread = store
            .create_chat_thread("berlin", "codex", "Codex", None)
            .unwrap();
        let launch = store.session_launch("berlin", SessionKind::Codex).unwrap();
        let process = store
            .record_session_process_for_thread("berlin", thread.id, &launch, exited_child_pid())
            .unwrap();

        reconcile_managed_sessions_on_startup(&paths).unwrap();

        let reconciled = store.get_process_record(process.id).unwrap();
        assert_eq!(reconciled.status, ProcessStatus::Exited);
        assert!(reconciled.ended_at.is_some());
        assert!(reconciled.log_path.starts_with(&paths.logs_dir));
    }

    #[test]
    fn reconcile_startup_leaves_data_dir_codex_sessions_untouched() {
        let temp = tempfile::tempdir().unwrap();
        let paths = app_paths(temp.path());
        let data_logs_dir = paths.data_dir.join("logs");
        let store = seeded_workspace_store(&paths.database_path, &data_logs_dir, temp.path());
        let thread = store
            .create_chat_thread("berlin", "codex", "Codex", None)
            .unwrap();
        let launch = store.session_launch("berlin", SessionKind::Codex).unwrap();
        let process = store
            .record_session_process_for_thread("berlin", thread.id, &launch, std::process::id())
            .unwrap();
        assert!(process.log_path.starts_with(&data_logs_dir));
        assert_eq!(process.status, ProcessStatus::Running);

        reconcile_managed_sessions_on_startup(&paths).unwrap();

        let unchanged = store.get_process_record(process.id).unwrap();
        assert_eq!(unchanged.status, ProcessStatus::Running);
        assert_eq!(unchanged.ended_at, None);
    }

    #[test]
    fn reconcile_startup_leaves_non_managed_sessions_untouched() {
        let temp = tempfile::tempdir().unwrap();
        let paths = app_paths(temp.path());
        let store = seeded_workspace_store(
            &paths.database_path,
            &paths.data_dir.join("logs"),
            temp.path(),
        );
        let launch = store.session_launch("berlin", SessionKind::Shell).unwrap();
        let process = store
            .record_session_process("berlin", &launch, std::process::id())
            .unwrap();
        assert_eq!(process.status, ProcessStatus::Running);

        reconcile_managed_sessions_on_startup(&paths).unwrap();

        let unchanged = store.get_process_record(process.id).unwrap();
        assert_eq!(unchanged.status, ProcessStatus::Running);
        assert_eq!(unchanged.ended_at, None);
    }

    fn app_paths(root: &Path) -> AppPaths {
        let state_dir = root.join("state");
        AppPaths {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            state_dir: state_dir.clone(),
            cache_dir: root.join("cache"),
            database_path: root.join("data/linux-archductor.db"),
            logs_dir: state_dir.join("logs"),
        }
    }

    fn seeded_workspace_store(db_path: &Path, logs_dir: &Path, root: &Path) -> WorkspaceStore {
        let repo_path = init_repo(root.join("demo"));
        RepositoryStore::open(db_path)
            .unwrap()
            .add(AddRepository {
                name: Some("demo".to_owned()),
                root_path: repo_path,
                default_branch: Some("main".to_owned()),
                remote_name: "origin".to_owned(),
                workspace_parent_path: Some(root.join("workspaces/demo")),
            })
            .unwrap();
        let store = WorkspaceStore::open_with_logs(db_path, logs_dir).unwrap();
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        store
    }

    fn init_repo(path: PathBuf) -> PathBuf {
        fs::create_dir(&path).unwrap();
        Command::new("git")
            .args(["init", "--initial-branch", "main"])
            .arg(&path)
            .status()
            .unwrap();
        fs::write(path.join("README.md"), "demo\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&path)
            .args(["add", "."])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "initial",
            ])
            .status()
            .unwrap();
        path
    }

    fn spawn_fake_managed_codex_process() -> std::process::Child {
        let mut command = Command::new("bash");
        command
            .arg("-lc")
            .arg("exec -a codex bash -lc 'while :; do sleep 1; done' --no-alt-screen")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command.process_group(0);
        command.spawn().unwrap()
    }

    fn terminate_test_child(child: &mut std::process::Child) {
        let _ = Command::new("kill")
            .arg("-KILL")
            .arg(format!("-{}", child.id()))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("kill")
            .arg("-KILL")
            .arg(child.id().to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = child.wait();
    }

    fn wait_for_fake_codex_child_alive(pid: u32) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            let alive = Command::new("kill")
                .arg("-0")
                .arg(pid.to_string())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|status| status.success())
                .unwrap_or(false);
            if alive {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        false
    }

    fn exited_child_pid() -> u32 {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("exit 0")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        child.wait().unwrap();
        pid
    }
}
