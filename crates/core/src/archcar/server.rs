use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::archcar::protocol::{
    archcar_event_summary, archcar_request_summary, archcar_response_summary, ArchcarEvent,
    ArchcarMessage, ArchcarRequest, ArchcarResponse, RpcEnvelope,
};
use crate::archcar::session::{
    restore_managed_session, spawn_managed_session, spawn_managed_session_for_thread, SessionHandle,
};
use crate::archcar::transport::{self, LocalListener, LocalStream};
use crate::paths::AppPaths;
use crate::provider_events::ProviderEventStore;
use crate::provider_projection::{
    provider_projection_from_records, provider_projection_item_is_relevant_chat_event,
    provider_projection_item_text,
};
use crate::workspace::{SessionKind, WorkspaceStore};

pub struct ArchcarServer {
    listener: LocalListener,
    state: Arc<Mutex<ServerState>>,
}

struct ServerState {
    db_path: PathBuf,
    logs_dir: PathBuf,
    queued_defaults: HashSet<String>,
    queued_threads: HashSet<i64>,
    sessions: HashMap<i64, SessionHandle>,
    subscribers: Vec<Sender<ArchcarEvent>>,
}

pub fn reconcile_managed_sessions_on_startup(paths: &AppPaths) -> Result<()> {
    let store = WorkspaceStore::open(&paths.database_path)?;
    for workspace in store.list()? {
        let records = store.list_sessions(&workspace.name)?;
        for kind in [SessionKind::Codex, SessionKind::Claude] {
            for record in persisted_running_session_candidates(&records, kind) {
                if !is_archcar_managed_persisted_session(&record, &paths.logs_dir) {
                    continue;
                }
                if archcar_process_alive(record.pid) {
                    continue;
                }
                let _ = store.mark_session_process_exited(record.id, None)?;
            }
        }
    }
    Ok(())
}

fn archcar_process_alive(pid: u32) -> bool {
    crate::platform::process_alive(pid)
}

impl ArchcarServer {
    pub fn bind(paths: AppPaths) -> Result<Self> {
        fs::create_dir_all(&paths.state_dir)?;
        let endpoint_path = paths.archcar_endpoint_path();
        let listener = transport::bind(&endpoint_path)
            .with_context(|| format!("bind archcar endpoint {}", endpoint_path.display()))?;
        let state = Arc::new(Mutex::new(ServerState {
            db_path: paths.database_path,
            logs_dir: paths.logs_dir,
            queued_defaults: HashSet::new(),
            queued_threads: HashSet::new(),
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

fn handle_connection(stream: LocalStream, state: Arc<Mutex<ServerState>>) -> Result<()> {
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
            "archcar local rpc"
        );
    } else {
        info!(
            %rpc_id,
            direction,
            message_type,
            summary = %summary,
            "archcar local rpc"
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
        ArchcarRequest::EnsureChatThreadSession {
            workspace,
            thread_id,
            kind,
            harness,
        } => ensure_chat_thread_session(
            state,
            workspace,
            thread_id,
            kind,
            harness.unwrap_or_default(),
        ),
        ArchcarRequest::SpawnSession {
            workspace,
            kind,
            harness,
        } => spawn_session(state, workspace, kind, harness.unwrap_or_default()),
        ArchcarRequest::SendInput {
            session_id,
            input,
            visible_input,
            kind,
        } => match load_or_restore_session_handle(state, session_id) {
            Ok(Some(handle)) => {
                match handle
                    .command_tx
                    .send(crate::archcar::session::SessionCommand::SendInput {
                        input,
                        visible_input,
                        kind,
                    }) {
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
        ArchcarRequest::InterruptTurn { session_id } => {
            match load_or_restore_session_handle(state, session_id) {
                Ok(Some(handle)) => {
                    let kind = handle.snapshot.lock().ok().map(|snapshot| snapshot.kind);
                    if kind != Some(crate::workspace::SessionKind::Codex) {
                        return ArchcarResponse::Error {
                            message: format!(
                                "interrupt_turn is only supported for codex sessions; got {kind:?}"
                            ),
                        };
                    }
                    match handle
                        .command_tx
                        .send(crate::archcar::session::SessionCommand::InterruptTurn)
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
        ArchcarRequest::SetSessionModel { session_id, model } => {
            match load_or_restore_session_handle(state, session_id) {
                Ok(Some(handle)) => {
                    let kind = handle.snapshot.lock().ok().map(|snapshot| snapshot.kind);
                    if kind != Some(crate::workspace::SessionKind::Codex) {
                        return ArchcarResponse::Error {
                            message: format!(
                                "set_session_model is only supported for codex sessions; got {kind:?}"
                            ),
                        };
                    }
                    match handle
                        .command_tx
                        .send(crate::archcar::session::SessionCommand::SetModel { model })
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
            match session_messages_for_thread(&db_path, thread_id) {
                Ok(messages) => ArchcarResponse::SessionMessages {
                    thread_id,
                    messages,
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

fn session_messages_for_thread(db_path: &Path, thread_id: i64) -> Result<Vec<ArchcarMessage>> {
    let store = WorkspaceStore::open(db_path)?;
    let mut persisted_messages: Vec<_> = store
        .list_chat_messages(thread_id)?
        .into_iter()
        .map(|message| ArchcarMessage {
            id: message.id,
            role: message.role,
            content: message.content,
            source: message.source,
            inline_event: None,
            context_usage: None,
        })
        .collect();
    persisted_messages.sort_by_key(|message| message.id);

    let provider_records = ProviderEventStore::new(db_path).list_for_chat_thread(thread_id)?;
    let projection = provider_projection_from_records(&provider_records);
    let mut messages = Vec::new();
    let mut next_provider_message_id = -1;
    let provider_items = projection
        .items
        .into_iter()
        .filter(provider_projection_item_is_relevant_chat_event)
        .collect::<Vec<_>>();
    let has_provider_user_anchors = provider_items.iter().any(|item| {
        item.render_class.role_label() == "user"
            && !provider_projection_item_text(item).trim().is_empty()
    });
    if !has_provider_user_anchors {
        messages.append(&mut persisted_messages);
    }
    for item in provider_items {
        let content = provider_projection_item_text(&item);
        let content = if item.render_class.role_label() == "assistant" {
            store.apply_agent_chat_metadata_directive(thread_id, &content)?
        } else {
            content
        };
        if content.trim().is_empty() {
            continue;
        }
        if item.render_class.role_label() == "user" {
            if let Some(index) = persisted_messages.iter().position(|message| {
                message.role == "user" && message.content.trim() == content.trim()
            }) {
                let matched = persisted_messages.remove(index);
                messages.extend(persisted_messages.drain(..index));
                messages.push(matched);
            } else if !messages.iter().any(|message: &ArchcarMessage| {
                semantic_roles_match(&message.role, "user")
                    && message.content.trim() == content.trim()
            }) {
                messages.push(ArchcarMessage {
                    id: next_provider_message_id,
                    role: "user".to_owned(),
                    content,
                    source: "provider_event".to_owned(),
                    inline_event: None,
                    context_usage: None,
                });
                next_provider_message_id -= 1;
            }
            continue;
        }
        if messages
            .iter()
            .chain(persisted_messages.iter())
            .any(|message: &ArchcarMessage| {
                message.source != "provider_event"
                    && semantic_roles_match(&message.role, item.render_class.role_label())
                    && message.content.trim() == content.trim()
            })
        {
            continue;
        }
        messages.push(ArchcarMessage {
            id: next_provider_message_id,
            role: item.render_class.role_label().to_owned(),
            content,
            source: "provider_event".to_owned(),
            inline_event: None,
            context_usage: None,
        });
        next_provider_message_id -= 1;
    }
    messages.extend(persisted_messages);

    Ok(messages)
}

fn semantic_roles_match(left: &str, right: &str) -> bool {
    left == right
        || matches!(
            (left, right),
            ("agent", "assistant") | ("assistant", "agent")
        )
}

fn ensure_default_session(
    state: &Arc<Mutex<ServerState>>,
    workspace: String,
    kind: SessionKind,
    harness: crate::workspace::SessionHarnessOptions,
) -> ArchcarResponse {
    if !matches!(kind, SessionKind::Codex | SessionKind::Claude) {
        return ArchcarResponse::Error {
            message: "only codex and claude auto-spawn are implemented".to_owned(),
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
    let queue_key = default_queue_key(&workspace, kind);
    if !guard.queued_defaults.insert(queue_key.clone()) {
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
                guard
                    .queued_defaults
                    .remove(&default_queue_key(&workspace_for_spawn, kind));
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
                guard
                    .queued_defaults
                    .remove(&default_queue_key(&workspace_for_spawn, kind));
                broadcast(
                    &mut guard,
                    ArchcarEvent::SessionError {
                        session_id: None,
                        thread_id: None,
                        message: detail,
                    },
                );
            }
        }
    });

    ArchcarResponse::SessionSpawnQueued { workspace, kind }
}

fn default_queue_key(workspace: &str, kind: SessionKind) -> String {
    let kind = match kind {
        SessionKind::Shell => "shell",
        SessionKind::Codex => "codex",
        SessionKind::Claude => "claude",
    };
    format!("{workspace}\0{kind}")
}

fn ensure_chat_thread_session(
    state: &Arc<Mutex<ServerState>>,
    workspace: String,
    thread_id: i64,
    kind: SessionKind,
    harness: crate::workspace::SessionHarnessOptions,
) -> ArchcarResponse {
    if !matches!(kind, SessionKind::Codex | SessionKind::Claude) {
        return ArchcarResponse::Error {
            message: "only codex and claude chat-thread auto-spawn are implemented".to_owned(),
        };
    }
    let mut guard = state.lock().unwrap();
    let db_path = guard.db_path.clone();
    let logs_dir = guard.logs_dir.clone();
    if let Some((session_id, pid, ready)) = guard
        .sessions
        .values()
        .filter_map(|handle| {
            let snapshot = handle.snapshot.lock().ok()?.clone();
            (snapshot.workspace == workspace
                && snapshot.kind == kind
                && snapshot.thread_id == thread_id
                && snapshot.status == crate::workspace::ProcessStatus::Running)
                .then_some((snapshot.session_id, snapshot.pid, snapshot.ready))
        })
        .max_by_key(|(session_id, _, _)| *session_id)
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

    if let Err(err) = validate_chat_thread_workspace(&db_path, &workspace, thread_id, kind) {
        let message = format!("{err:#}");
        let mut guard = state.lock().unwrap();
        broadcast(
            &mut guard,
            ArchcarEvent::SessionError {
                session_id: None,
                thread_id: Some(thread_id),
                message: message.clone(),
            },
        );
        return ArchcarResponse::Error { message };
    }

    let mut guard = state.lock().unwrap();
    if !guard.queued_threads.insert(thread_id) {
        return ArchcarResponse::SessionSpawnQueued { workspace, kind };
    }
    drop(guard);

    if let Some(response) = restore_thread_session_from_store(state, &workspace, thread_id, kind) {
        if let Ok(mut guard) = state.lock() {
            guard.queued_threads.remove(&thread_id);
        }
        return response;
    }

    let mut guard = state.lock().unwrap();
    let state_for_spawn = state.clone();
    broadcast(
        &mut guard,
        ArchcarEvent::SessionSpawnQueued {
            workspace: workspace.clone(),
            kind,
        },
    );
    info!(%workspace, thread_id, ?kind, "archcar queued chat-thread session spawn");
    drop(guard);

    let workspace_for_spawn = workspace.clone();
    std::thread::spawn(move || {
        let (event_tx, event_rx) = mpsc::channel();
        match spawn_managed_session_for_thread(
            db_path,
            logs_dir,
            workspace_for_spawn.clone(),
            thread_id,
            kind,
            harness,
            event_tx,
        ) {
            Ok(handle) => {
                let session_id = handle.snapshot.lock().unwrap().session_id;
                info!(%workspace_for_spawn, thread_id, session_id, ?kind, "archcar spawned chat-thread managed session");
                let mut guard = state_for_spawn.lock().unwrap();
                guard.sessions.insert(session_id, handle);
                guard.queued_threads.remove(&thread_id);
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
                error!(%workspace_for_spawn, thread_id, ?kind, error = %detail, "archcar failed to spawn chat-thread managed session");
                let mut guard = state_for_spawn.lock().unwrap();
                guard.queued_threads.remove(&thread_id);
                broadcast(
                    &mut guard,
                    ArchcarEvent::SessionError {
                        session_id: None,
                        thread_id: Some(thread_id),
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

fn restore_thread_session_from_store(
    state: &Arc<Mutex<ServerState>>,
    workspace: &str,
    thread_id: i64,
    kind: SessionKind,
) -> Option<ArchcarResponse> {
    let db_path = state.lock().ok()?.db_path.clone();
    let store = match WorkspaceStore::open(&db_path) {
        Ok(store) => store,
        Err(err) => {
            warn!(
                workspace,
                thread_id,
                ?kind,
                error = %format!("{err:#}"),
                "archcar failed to open workspace store for persisted thread restore"
            );
            return None;
        }
    };
    let records = match store.list_thread_processes(thread_id) {
        Ok(records) => records,
        Err(err) => {
            warn!(
                workspace,
                thread_id,
                ?kind,
                error = %format!("{err:#}"),
                "archcar failed to list persisted thread sessions for restore"
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
                    || snapshot.thread_id != thread_id
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
                    thread_id,
                    ?kind,
                    session_id = snapshot.session_id,
                    pid = snapshot.pid,
                    "archcar restored persisted chat-thread session"
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
                    thread_id,
                    ?kind,
                    session_id = record.id,
                    error = %format!("{err:#}"),
                    "archcar failed to restore persisted thread session candidate"
                );
            }
        }
    }
    None
}

fn validate_chat_thread_workspace(
    db_path: &std::path::Path,
    workspace: &str,
    thread_id: i64,
    kind: SessionKind,
) -> Result<()> {
    let store = WorkspaceStore::open(db_path)?;
    let workspace_record = store.get_workspace_record_by_name(workspace)?;
    let thread_record = store.get_chat_thread_record(thread_id)?;
    anyhow::ensure!(
        thread_record.workspace_id == workspace_record.id,
        "chat thread {thread_id} does not belong to workspace {workspace}"
    );
    anyhow::ensure!(
        thread_record.provider == crate::archcar::harness::provider_name(kind),
        "chat thread {thread_id} is not a {:?} thread",
        kind
    );
    Ok(())
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
                        thread_id: None,
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
    use crate::provider_events::{ProviderEventDraft, ProviderEventKind, ProviderEventPhase};
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    #[cfg(unix)]
    use std::time::{Duration, Instant};

    use crate::paths::AppPaths;
    use crate::repository::{AddRepository, RepositoryStore};
    use crate::workspace::{CreateWorkspace, ProcessStatus};
    use serde_json::json;

    #[test]
    fn ensure_default_session_debounces_repeat_requests() {
        let state = Arc::new(Mutex::new(ServerState {
            db_path: PathBuf::from("/tmp/does-not-matter.db"),
            logs_dir: PathBuf::from("/tmp/does-not-matter-logs"),
            queued_defaults: HashSet::new(),
            queued_threads: HashSet::new(),
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
    fn ensure_default_session_queue_is_scoped_by_workspace_and_kind() {
        let (event_tx, event_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(ServerState {
            db_path: PathBuf::from("/tmp/does-not-matter.db"),
            logs_dir: PathBuf::from("/tmp/does-not-matter-logs"),
            queued_defaults: HashSet::from([default_queue_key("berlin", SessionKind::Codex)]),
            queued_threads: HashSet::new(),
            sessions: HashMap::new(),
            subscribers: vec![event_tx],
        }));

        let claude = ensure_default_session(
            &state,
            "berlin".to_owned(),
            SessionKind::Claude,
            crate::workspace::SessionHarnessOptions::default(),
        );

        assert!(matches!(
            claude,
            ArchcarResponse::SessionSpawnQueued {
                kind: SessionKind::Claude,
                ..
            }
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Ok(ArchcarEvent::SessionSpawnQueued {
                kind: SessionKind::Claude,
                ..
            })
        ));
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
            queued_threads: HashSet::new(),
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
                visible_input: None,
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
                visible_input: None,
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
    fn session_messages_project_provider_events_into_semantic_messages() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("state.db");
        let store = seeded_workspace_store(&db_path, &temp.path().join("logs"), temp.path());
        let thread = store
            .create_chat_thread("berlin", "codex", "Codex", None)
            .unwrap();
        store
            .append_chat_message(thread.id, "user", "Run tests", "cli")
            .unwrap();
        let provider_store = ProviderEventStore::new(&db_path);
        provider_store
            .upsert_event(&provider_event(
                thread.id,
                "assistant-1",
                ProviderEventKind::AssistantOutput,
                ProviderEventPhase::Completed,
                "agent_message",
                "Assistant",
                "Tests passed",
            ))
            .unwrap();
        provider_store
            .upsert_event(&provider_event(
                thread.id,
                "reasoning-1",
                ProviderEventKind::PlanningReasoning,
                ProviderEventPhase::Progress,
                "reasoning_summary",
                "Reasoning",
                "Checking failure output",
            ))
            .unwrap();
        provider_store
            .upsert_event(&provider_event(
                thread.id,
                "turn-1",
                ProviderEventKind::Turn,
                ProviderEventPhase::Started,
                "turn_started",
                "Turn started",
                "raw lifecycle",
            ))
            .unwrap();

        let messages = session_messages_for_thread(&db_path, thread.id).unwrap();

        assert_eq!(
            messages
                .iter()
                .map(|message| (message.role.as_str(), message.content.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("user", "Run tests"),
                ("assistant", "Tests passed"),
                ("reasoning", "Reasoning\nChecking failure output"),
            ]
        );
    }

    #[test]
    fn session_messages_preserve_assistant_edge_whitespace() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("state.db");
        let store = seeded_workspace_store(&db_path, &temp.path().join("logs"), temp.path());
        let thread = store
            .create_chat_thread("berlin", "codex", "Codex", None)
            .unwrap();
        ProviderEventStore::new(&db_path)
            .upsert_event(&provider_event(
                thread.id,
                "assistant-whitespace",
                ProviderEventKind::AssistantOutput,
                ProviderEventPhase::Completed,
                "agent_message",
                "Assistant",
                "  indented reply\n",
            ))
            .unwrap();

        let messages = session_messages_for_thread(&db_path, thread.id).unwrap();

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "assistant");
        assert_eq!(messages[0].content, "  indented reply\n");
    }

    #[test]
    fn session_messages_update_mcp_startup_status_as_one_provider_message() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("state.db");
        let store = seeded_workspace_store(&db_path, &temp.path().join("logs"), temp.path());
        let thread = store
            .create_chat_thread("berlin", "codex", "Codex", None)
            .unwrap();
        let provider_store = ProviderEventStore::new(&db_path);
        provider_store
            .upsert_event(&provider_event(
                thread.id,
                "mcp-startup-status",
                ProviderEventKind::Mcp,
                ProviderEventPhase::Progress,
                "mcpServer/startupStatus/updated",
                "MCP loading",
                "",
            ))
            .unwrap();
        provider_store
            .upsert_event(&provider_event(
                thread.id,
                "mcp-startup-status",
                ProviderEventKind::Mcp,
                ProviderEventPhase::Completed,
                "mcpServer/startupStatus/updated",
                "MCP loaded",
                "github: ready",
            ))
            .unwrap();

        let messages = session_messages_for_thread(&db_path, thread.id).unwrap();

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "tool");
        assert_eq!(messages[0].source, "provider_event");
        assert_eq!(messages[0].content, "MCP loaded\ngithub: ready");
        assert!(!messages[0]
            .content
            .contains("mcpServer/startupStatus/updated"));
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
            queued_threads: HashSet::new(),
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
    fn ensure_chat_thread_session_does_not_reuse_other_thread() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        let db_path = temp.path().join("state.db");
        let workspace_parent = temp.path().join("workspaces/demo");
        RepositoryStore::open(&db_path)
            .unwrap()
            .add(AddRepository {
                name: Some("demo".to_owned()),
                root_path: repo_path,
                default_branch: Some("main".to_owned()),
                remote_name: "origin".to_owned(),
                workspace_parent_path: Some(workspace_parent),
            })
            .unwrap();
        let store = WorkspaceStore::open(&db_path).unwrap();
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let requested_thread = store
            .create_chat_thread("berlin", "codex", "Codex Chat 2", None)
            .unwrap();
        let snapshot = crate::archcar::session::SessionSnapshot {
            session_id: 9,
            thread_id: requested_thread.id + 1,
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
            db_path,
            logs_dir: temp.path().join("logs"),
            queued_defaults: HashSet::new(),
            queued_threads: HashSet::new(),
            sessions,
            subscribers: Vec::new(),
        }));

        let response = ensure_chat_thread_session(
            &state,
            "berlin".to_owned(),
            requested_thread.id,
            SessionKind::Codex,
            crate::workspace::SessionHarnessOptions::default(),
        );

        assert_eq!(
            response,
            ArchcarResponse::SessionSpawnQueued {
                workspace: "berlin".to_owned(),
                kind: SessionKind::Codex,
            }
        );
    }

    #[test]
    fn ensure_chat_thread_session_validates_workspace_before_queue_dedupe() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        let db_path = temp.path().join("state.db");
        let workspace_parent = temp.path().join("workspaces/demo");
        RepositoryStore::open(&db_path)
            .unwrap()
            .add(AddRepository {
                name: Some("demo".to_owned()),
                root_path: repo_path,
                default_branch: Some("main".to_owned()),
                remote_name: "origin".to_owned(),
                workspace_parent_path: Some(workspace_parent),
            })
            .unwrap();
        let store = WorkspaceStore::open(&db_path).unwrap();
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "tokyo".to_owned(),
                branch: "lc/tokyo".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let requested_thread = store
            .create_chat_thread("berlin", "codex", "Codex Chat", None)
            .unwrap();
        let state = Arc::new(Mutex::new(ServerState {
            db_path,
            logs_dir: temp.path().join("logs"),
            queued_defaults: HashSet::new(),
            queued_threads: HashSet::from([requested_thread.id]),
            sessions: HashMap::new(),
            subscribers: Vec::new(),
        }));

        let response = ensure_chat_thread_session(
            &state,
            "tokyo".to_owned(),
            requested_thread.id,
            SessionKind::Codex,
            crate::workspace::SessionHarnessOptions::default(),
        );

        assert!(matches!(response, ArchcarResponse::Error { .. }));
    }

    #[test]
    fn session_messages_merge_persisted_inputs_at_provider_user_anchors() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("state.db");
        let store = seeded_workspace_store(&db_path, &temp.path().join("logs"), temp.path());
        let thread = store
            .create_chat_thread("berlin", "codex", "Codex", None)
            .unwrap();
        store
            .append_chat_message(thread.id, "user", "first", "user_send")
            .unwrap();
        store
            .append_chat_message(thread.id, "user", "second", "user_send")
            .unwrap();
        let event_store = ProviderEventStore::new(&db_path);
        for (sequence, kind, item_id, body) in [
            (1, ProviderEventKind::UserInput, "user-1", "first"),
            (
                2,
                ProviderEventKind::AssistantOutput,
                "assistant-1",
                "answer one",
            ),
            (3, ProviderEventKind::UserInput, "user-2", "second"),
            (
                4,
                ProviderEventKind::AssistantOutput,
                "assistant-2",
                "answer two",
            ),
        ] {
            event_store
                .upsert_event(&ProviderEventDraft {
                    provider: "codex".to_owned(),
                    provider_event_id: Some(format!("event-{sequence}")),
                    provider_item_id: Some(item_id.to_owned()),
                    provider_thread_id: Some("thread-1".to_owned()),
                    provider_turn_id: None,
                    parent_provider_item_id: None,
                    parent_provider_thread_id: None,
                    workspace_id: None,
                    chat_thread_id: Some(thread.id),
                    process_id: None,
                    phase: ProviderEventPhase::Completed,
                    kind,
                    provider_subtype: Some("test".to_owned()),
                    provider_sequence: Some(sequence),
                    occurred_at_ms: sequence as u64,
                    normalized_payload: json!({
                        "title": if kind == ProviderEventKind::UserInput { "User" } else { "Assistant" },
                        "body": body
                    }),
                    raw_json: json!({"sequence": sequence}),
                    schema_version: 1,
                    adapter_version: "test".to_owned(),
                })
                .unwrap();
        }

        let messages = session_messages_for_thread(&db_path, thread.id).unwrap();
        let rendered = messages
            .iter()
            .map(|message| format!("{}:{}", message.role, message.content))
            .collect::<Vec<_>>();

        assert_eq!(
            rendered,
            vec![
                "user:first",
                "assistant:answer one",
                "user:second",
                "assistant:answer two",
            ]
        );
    }

    #[test]
    fn session_messages_emit_history_before_matching_provider_anchor() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("state.db");
        let store = seeded_workspace_store(&db_path, &temp.path().join("logs"), temp.path());
        let thread = store
            .create_chat_thread("berlin", "codex", "Codex", None)
            .unwrap();
        store
            .append_chat_message(thread.id, "user", "old prompt", "user_send")
            .unwrap();
        store
            .append_chat_message(thread.id, "agent", "old answer", "agent")
            .unwrap();
        store
            .append_chat_message(thread.id, "user", "new prompt", "user_send")
            .unwrap();
        let event_store = ProviderEventStore::new(&db_path);
        for (sequence, kind, item_id, body) in [
            (1, ProviderEventKind::UserInput, "user-1", "new prompt"),
            (
                2,
                ProviderEventKind::AssistantOutput,
                "assistant-1",
                "new answer",
            ),
        ] {
            event_store
                .upsert_event(&provider_event_with_sequence(
                    thread.id, sequence, kind, item_id, body,
                ))
                .unwrap();
        }

        let rendered = session_messages_for_thread(&db_path, thread.id)
            .unwrap()
            .into_iter()
            .map(|message| format!("{}:{}", message.role, message.content))
            .collect::<Vec<_>>();

        assert_eq!(
            rendered,
            vec![
                "user:old prompt",
                "agent:old answer",
                "user:new prompt",
                "assistant:new answer",
            ]
        );
    }

    #[test]
    fn session_messages_synthesize_provider_user_without_persisted_anchor() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("state.db");
        let store = seeded_workspace_store(&db_path, &temp.path().join("logs"), temp.path());
        let thread = store
            .create_chat_thread("berlin", "codex", "Codex", None)
            .unwrap();
        let event_store = ProviderEventStore::new(&db_path);
        for (sequence, kind, item_id, body) in [
            (1, ProviderEventKind::UserInput, "user-1", "native prompt"),
            (
                2,
                ProviderEventKind::AssistantOutput,
                "assistant-1",
                "native answer",
            ),
        ] {
            event_store
                .upsert_event(&provider_event_with_sequence(
                    thread.id, sequence, kind, item_id, body,
                ))
                .unwrap();
        }

        let rendered = session_messages_for_thread(&db_path, thread.id)
            .unwrap()
            .into_iter()
            .map(|message| format!("{}:{}:{}", message.role, message.source, message.content))
            .collect::<Vec<_>>();

        assert_eq!(
            rendered,
            vec![
                "user:provider_event:native prompt",
                "assistant:provider_event:native answer",
            ]
        );
    }

    #[test]
    fn session_messages_ignore_empty_provider_user_anchors() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("state.db");
        let store = seeded_workspace_store(&db_path, &temp.path().join("logs"), temp.path());
        let thread = store
            .create_chat_thread("berlin", "codex", "Codex", None)
            .unwrap();
        store
            .append_chat_message(thread.id, "user", "persisted prompt", "user_send")
            .unwrap();
        let event_store = ProviderEventStore::new(&db_path);
        event_store
            .upsert_event(&provider_event_with_sequence(
                thread.id,
                1,
                ProviderEventKind::UserInput,
                "user-1",
                "",
            ))
            .unwrap();

        let rendered = session_messages_for_thread(&db_path, thread.id)
            .unwrap()
            .into_iter()
            .map(|message| format!("{}:{}:{}", message.role, message.source, message.content))
            .collect::<Vec<_>>();

        assert_eq!(rendered, vec!["user:user_send:persisted prompt"]);
    }

    #[test]
    fn session_messages_dedupe_provider_assistant_against_remaining_agent_row() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("state.db");
        let store = seeded_workspace_store(&db_path, &temp.path().join("logs"), temp.path());
        let thread = store
            .create_chat_thread("berlin", "codex", "Codex", None)
            .unwrap();
        store
            .append_chat_message(thread.id, "user", "prompt", "user_send")
            .unwrap();
        store
            .append_chat_message(thread.id, "agent", "same answer", "agent")
            .unwrap();
        let event_store = ProviderEventStore::new(&db_path);
        for (sequence, kind, item_id, body) in [
            (1, ProviderEventKind::UserInput, "user-1", "prompt"),
            (
                2,
                ProviderEventKind::AssistantOutput,
                "assistant-1",
                "same answer",
            ),
        ] {
            event_store
                .upsert_event(&provider_event_with_sequence(
                    thread.id, sequence, kind, item_id, body,
                ))
                .unwrap();
        }

        let rendered = session_messages_for_thread(&db_path, thread.id)
            .unwrap()
            .into_iter()
            .map(|message| format!("{}:{}:{}", message.role, message.source, message.content))
            .collect::<Vec<_>>();

        assert_eq!(
            rendered,
            vec!["user:user_send:prompt", "agent:agent:same answer"]
        );
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
                "user.name=Archductor",
                "-c",
                "user.email=archductor@example.test",
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

    #[cfg(unix)]
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
    fn reconcile_startup_marks_dead_managed_claude_sessions_exited() {
        let temp = tempfile::tempdir().unwrap();
        let paths = app_paths(temp.path());
        let store = seeded_workspace_store(&paths.database_path, &paths.logs_dir, temp.path());
        let thread = store
            .create_chat_thread("berlin", "claude", "Claude", None)
            .unwrap();
        let launch = store.session_launch("berlin", SessionKind::Claude).unwrap();
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
            database_path: root.join("data/archductor.db"),
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

    fn provider_event(
        thread_id: i64,
        item_id: &str,
        kind: ProviderEventKind,
        phase: ProviderEventPhase,
        subtype: &str,
        title: &str,
        body: &str,
    ) -> ProviderEventDraft {
        ProviderEventDraft {
            provider: "codex".to_owned(),
            provider_event_id: Some(format!("evt-{item_id}")),
            provider_item_id: Some(item_id.to_owned()),
            provider_thread_id: Some("thread-1".to_owned()),
            provider_turn_id: Some("turn-1".to_owned()),
            parent_provider_item_id: None,
            parent_provider_thread_id: None,
            workspace_id: None,
            chat_thread_id: Some(thread_id),
            process_id: None,
            phase,
            kind,
            provider_subtype: Some(subtype.to_owned()),
            provider_sequence: Some(1),
            occurred_at_ms: 42,
            normalized_payload: json!({"title": title, "body": body}),
            raw_json: json!({"method": subtype, "params": {"body": body}}),
            schema_version: 1,
            adapter_version: "test".to_owned(),
        }
    }

    fn provider_event_with_sequence(
        thread_id: i64,
        sequence: u64,
        kind: ProviderEventKind,
        item_id: &str,
        body: &str,
    ) -> ProviderEventDraft {
        ProviderEventDraft {
            provider: "codex".to_owned(),
            provider_event_id: Some(format!("event-{sequence}")),
            provider_item_id: Some(item_id.to_owned()),
            provider_thread_id: Some("thread-1".to_owned()),
            provider_turn_id: None,
            parent_provider_item_id: None,
            parent_provider_thread_id: None,
            workspace_id: None,
            chat_thread_id: Some(thread_id),
            process_id: None,
            phase: ProviderEventPhase::Completed,
            kind,
            provider_subtype: Some("test".to_owned()),
            provider_sequence: Some(sequence as i64),
            occurred_at_ms: sequence,
            normalized_payload: json!({
                "title": if kind == ProviderEventKind::UserInput { "User" } else { "Assistant" },
                "body": body
            }),
            raw_json: json!({"sequence": sequence}),
            schema_version: 1,
            adapter_version: "test".to_owned(),
        }
    }

    #[cfg(unix)]
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

    #[cfg(unix)]
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

    #[cfg(unix)]
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
        let mut command = crate::platform::shell_command("exit 0");
        let mut child = command
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
