use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use tracing::{info, warn};

use crate::archcar::harness::{
    controller_for_kind, ensure_thread_for_kind, provider_name, HarnessController,
};
use crate::archcar::protocol::{ArchcarEvent, ArchcarInputKind};
use crate::codex_tui::codex_screen_ready_for_input;
use crate::provider_events::{
    ProviderEventContext, ProviderEventDraft, ProviderEventKind, ProviderEventPhase,
};
use crate::pty::PtySession;
use crate::runtime_session_store::RuntimeSessionStore;
use crate::session_state::{AgentSessionState, SessionStateMachine};
use crate::workspace::{
    ProcessStatus, SessionHarnessOptions, SessionKind, SessionLaunch, WorkspaceStore,
};
use serde_json::json;

#[derive(Debug)]
pub enum SessionCommand {
    SendInput {
        input: String,
        visible_input: Option<String>,
        kind: ArchcarInputKind,
    },
    Resize {
        rows: u16,
        cols: u16,
    },
    Kill,
}

#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    pub session_id: i64,
    pub thread_id: i64,
    pub workspace: String,
    pub kind: SessionKind,
    pub pid: u32,
    pub status: ProcessStatus,
    pub runtime_state: AgentSessionState,
    pub ready: bool,
    pub screen: String,
}

#[derive(Clone)]
pub struct SessionHandle {
    pub snapshot: Arc<Mutex<SessionSnapshot>>,
    pub command_tx: Sender<SessionCommand>,
}

enum ManagedSessionConnection {
    Live(PtySession),
    Reattached {
        write: std::fs::File,
        output: Arc<Mutex<String>>,
        read_cursor: usize,
        pid: u32,
    },
}

impl ManagedSessionConnection {
    fn try_reattach_running(pid: u32) -> Result<Self> {
        let path = terminal_device_path_for_pid(pid)?;
        let mut reader = OpenOptions::new().read(true).open(&path)?;
        let write = OpenOptions::new().write(true).open(&path)?;
        let output = Arc::new(Mutex::new(String::new()));
        let reader_output = Arc::clone(&output);
        thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Ok(mut output) = reader_output.lock() {
                            output.push_str(&String::from_utf8_lossy(&buffer[..n]));
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self::Reattached {
            write,
            output,
            read_cursor: 0,
            pid,
        })
    }

    fn send_line(&mut self, input: &str) -> Result<()> {
        match self {
            Self::Live(session) => session.send_line(input),
            Self::Reattached { write, .. } => {
                write.write_all(input.as_bytes())?;
                write.flush()?;
                thread::sleep(Duration::from_millis(20));
                write.write_all(b"\r")?;
                write.flush()?;
                Ok(())
            }
        }
    }

    fn stop(&mut self) -> Result<()> {
        match self {
            Self::Live(session) => session.stop(),
            Self::Reattached { .. } => Ok(()),
        }
    }

    fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        match self {
            Self::Live(session) => session.resize(rows, cols),
            Self::Reattached { .. } => Ok(()),
        }
    }

    fn has_exited(&mut self) -> Result<bool> {
        match self {
            Self::Live(session) => session.has_exited(),
            Self::Reattached { pid, .. } => Ok(!terminal_process_alive(*pid)),
        }
    }

    fn read_available(&mut self) -> String {
        match self {
            Self::Live(session) => session.read_available(),
            Self::Reattached {
                output,
                read_cursor,
                ..
            } => {
                let Ok(output) = output.lock() else {
                    return String::new();
                };
                let next = output.get(*read_cursor..).unwrap_or_default().to_owned();
                *read_cursor = output.len();
                next
            }
        }
    }

    fn visible_screen_text(&self) -> String {
        match self {
            Self::Live(session) => session.visible_screen_text(),
            Self::Reattached { .. } => String::new(),
        }
    }
}

pub fn spawn_managed_session(
    db_path: std::path::PathBuf,
    logs_dir: std::path::PathBuf,
    workspace: String,
    kind: SessionKind,
    harness: SessionHarnessOptions,
    event_tx: Sender<ArchcarEvent>,
) -> Result<SessionHandle> {
    let store = WorkspaceStore::open_with_logs(db_path.clone(), logs_dir.clone())?;
    let controller = controller_for_kind(kind);
    if let Some((connection, snapshot_state)) = adopt_running_session(&store, &workspace, kind)? {
        return Ok(start_session_handle(
            db_path,
            logs_dir,
            snapshot_state,
            controller,
            connection,
            event_tx,
        ));
    }
    let thread_record = ensure_thread_for_kind(&store, &workspace, kind)?;
    let launch = controller.build_launch(&store, &workspace, harness)?;
    spawn_live_managed_session(LiveSessionStart {
        db_path,
        logs_dir,
        store: &store,
        workspace,
        thread_id: thread_record.id,
        kind,
        launch,
        controller,
        event_tx,
    })
}

struct LiveSessionStart<'a> {
    db_path: PathBuf,
    logs_dir: PathBuf,
    store: &'a WorkspaceStore,
    workspace: String,
    thread_id: i64,
    kind: SessionKind,
    launch: SessionLaunch,
    controller: Box<dyn HarnessController>,
    event_tx: Sender<ArchcarEvent>,
}

fn spawn_live_managed_session(start: LiveSessionStart<'_>) -> Result<SessionHandle> {
    let pty = PtySession::spawn(
        start.launch.program.clone(),
        start.launch.args.clone(),
        &start.launch.cwd,
        start.launch.env.clone(),
        24,
        80,
    )
    .with_context(|| format!("spawn managed {:?} pty", start.kind))?;
    let pid = pty.process_id().context("pty has no process id")?;
    let process = start.store.record_session_process_for_thread(
        &start.workspace,
        start.thread_id,
        &start.launch,
        pid,
    )?;
    let snapshot = running_session_snapshot(
        process.id,
        start.thread_id,
        start.workspace,
        start.kind,
        pid,
        process_lifecycle_ready(start.kind),
    );
    Ok(start_session_handle(
        start.db_path,
        start.logs_dir,
        snapshot,
        start.controller,
        ManagedSessionConnection::Live(pty),
        start.event_tx,
    ))
}

fn process_lifecycle_ready(kind: SessionKind) -> bool {
    matches!(kind, SessionKind::Codex | SessionKind::Shell)
}

struct RuntimeProviderEventInput<'a> {
    kind: SessionKind,
    session_id: i64,
    thread_id: i64,
    identity_suffix: Option<&'a str>,
    provider_sequence: Option<u64>,
    phase: ProviderEventPhase,
    event_kind: ProviderEventKind,
    subtype: &'a str,
    title: &'a str,
    body: &'a str,
}

fn runtime_provider_event(input: RuntimeProviderEventInput<'_>) -> ProviderEventDraft {
    let identity_suffix = input
        .identity_suffix
        .map(|suffix| format!(":{suffix}"))
        .unwrap_or_default();
    ProviderEventDraft {
        provider: provider_name(input.kind).to_owned(),
        provider_event_id: Some(format!(
            "archcar:{}:{}{}",
            input.session_id, input.subtype, identity_suffix
        )),
        provider_item_id: Some(format!(
            "archcar-session-{}-{}{}",
            input.session_id, input.subtype, identity_suffix
        )),
        provider_thread_id: Some(input.thread_id.to_string()),
        provider_turn_id: None,
        parent_provider_item_id: None,
        parent_provider_thread_id: None,
        workspace_id: None,
        chat_thread_id: Some(input.thread_id),
        process_id: Some(input.session_id),
        phase: input.phase,
        kind: input.event_kind,
        provider_subtype: Some(input.subtype.to_owned()),
        provider_sequence: input.provider_sequence.map(|sequence| sequence as i64),
        occurred_at_ms: ProviderEventContext::runtime(
            None,
            Some(input.thread_id),
            Some(input.session_id),
            "",
        )
        .occurred_at_ms,
        normalized_payload: json!({
            "title": input.title,
            "body": input.body,
        }),
        raw_json: json!({
            "source": "archcar",
            "session_id": input.session_id,
            "thread_id": input.thread_id,
            "identity_suffix": input.identity_suffix,
            "kind": format!("{:?}", input.kind),
            "phase": input.phase.as_str(),
            "event_kind": input.event_kind.as_str(),
            "subtype": input.subtype,
            "title": input.title,
            "body": input.body,
        }),
        schema_version: 1,
        adapter_version: "archcar-runtime".to_owned(),
    }
}

fn append_runtime_provider_event(
    runtime_store: &RuntimeSessionStore,
    event: ProviderEventDraft,
    context: &str,
) {
    let session_id = event.process_id;
    let thread_id = event.chat_thread_id;
    let provider = event.provider.clone();
    let subtype = event.provider_subtype.clone();
    let phase = event.phase.as_str();
    if let Err(err) = runtime_store.append_provider_event(&event) {
        warn!(
            session_id,
            thread_id,
            provider = %provider,
            subtype = subtype.as_deref().unwrap_or("unknown"),
            phase,
            error = %format!("{err:#}"),
            context,
            "archcar runtime provider event persistence failed"
        );
    }
}

fn start_session_handle(
    db_path: PathBuf,
    logs_dir: PathBuf,
    snapshot_state: SessionSnapshot,
    controller: Box<dyn HarnessController>,
    connection: ManagedSessionConnection,
    event_tx: Sender<ArchcarEvent>,
) -> SessionHandle {
    let snapshot = Arc::new(Mutex::new(snapshot_state));
    let (command_tx, command_rx) = mpsc::channel();
    let snapshot_for_thread = Arc::clone(&snapshot);
    thread::spawn(move || {
        run_session_loop(
            db_path,
            logs_dir,
            snapshot_for_thread,
            controller,
            connection,
            command_rx,
            event_tx,
        )
    });
    SessionHandle {
        snapshot,
        command_tx,
    }
}

fn running_session_snapshot(
    session_id: i64,
    thread_id: i64,
    workspace: String,
    kind: SessionKind,
    pid: u32,
    ready: bool,
) -> SessionSnapshot {
    SessionSnapshot {
        session_id,
        thread_id,
        workspace,
        kind,
        pid,
        status: ProcessStatus::Running,
        runtime_state: AgentSessionState::Running,
        ready,
        screen: String::new(),
    }
}

pub fn spawn_managed_session_for_thread(
    db_path: std::path::PathBuf,
    logs_dir: std::path::PathBuf,
    workspace: String,
    thread_id: i64,
    kind: SessionKind,
    harness: SessionHarnessOptions,
    event_tx: Sender<ArchcarEvent>,
) -> Result<SessionHandle> {
    let store = WorkspaceStore::open_with_logs(db_path.clone(), logs_dir.clone())?;
    let thread_record = store.get_chat_thread_record(thread_id)?;
    let workspace_record = store.get_workspace_record_by_name(&workspace)?;
    anyhow::ensure!(
        thread_record.workspace_id == workspace_record.id,
        "chat thread {thread_id} does not belong to workspace {workspace}"
    );
    anyhow::ensure!(
        thread_record.provider == provider_name(kind),
        "chat thread {thread_id} is not a {:?} thread",
        kind
    );
    let controller = controller_for_kind(kind);
    let launch = build_thread_session_launch(
        &store,
        &workspace,
        kind,
        harness,
        thread_record.native_thread_id.as_deref(),
        controller.as_ref(),
    )?;
    spawn_live_managed_session(LiveSessionStart {
        db_path,
        logs_dir,
        store: &store,
        workspace,
        thread_id,
        kind,
        launch,
        controller,
        event_tx,
    })
}

fn build_thread_session_launch(
    store: &WorkspaceStore,
    workspace: &str,
    kind: SessionKind,
    harness: SessionHarnessOptions,
    native_thread_id: Option<&str>,
    controller: &dyn HarnessController,
) -> Result<crate::workspace::SessionLaunch> {
    if kind == SessionKind::Codex {
        if let Some(native_thread_id) = native_thread_id {
            return store.session_launch_with_options_and_resume(
                workspace,
                SessionKind::Codex,
                harness,
                Some(native_thread_id),
            );
        }
    }
    controller.build_launch(store, workspace, harness)
}

fn adopt_running_session(
    store: &WorkspaceStore,
    workspace: &str,
    kind: SessionKind,
) -> Result<Option<(ManagedSessionConnection, SessionSnapshot)>> {
    for process in store
        .list_sessions(workspace)?
        .into_iter()
        .filter(|record| {
            record.status == ProcessStatus::Running
                && record.chat_thread_id.is_some()
                && session_kind_from_command(&record.command) == Some(kind)
                && kind == SessionKind::Codex
                && store.owns_process_log_path(&record.log_path)
        })
    {
        if !terminal_process_alive(process.pid) {
            let _ = store.mark_session_process_exited(process.id, None);
            continue;
        }
        let thread_id = process
            .chat_thread_id
            .context("running managed session missing chat_thread_id")?;
        let connection = match ManagedSessionConnection::try_reattach_running(process.pid) {
            Ok(connection) => connection,
            Err(err) => {
                warn!(
                    session_id = process.id,
                    pid = process.pid,
                    error = %format!("{err:#}"),
                    "archcar could not adopt running session"
                );
                continue;
            }
        };
        let snapshot = running_session_snapshot(
            process.id,
            thread_id,
            workspace.to_owned(),
            kind,
            process.pid,
            true,
        );
        return Ok(Some((connection, snapshot)));
    }
    Ok(None)
}

fn session_kind_from_command(command: &str) -> Option<SessionKind> {
    let trimmed = command.trim();
    if trimmed.starts_with("codex ") || trimmed == "codex" {
        Some(SessionKind::Codex)
    } else if trimmed.starts_with("claude ") || trimmed == "claude" {
        Some(SessionKind::Claude)
    } else if !trimmed.is_empty() {
        Some(SessionKind::Shell)
    } else {
        None
    }
}

pub fn restore_managed_session(
    db_path: std::path::PathBuf,
    logs_dir: std::path::PathBuf,
    process_id: i64,
    event_tx: Sender<ArchcarEvent>,
) -> Result<Option<SessionHandle>> {
    let store = WorkspaceStore::open_with_logs(&db_path, &logs_dir)?;
    let process = match store.get_process_record(process_id) {
        Ok(process) => process,
        Err(_) => return Ok(None),
    };
    if process.status != ProcessStatus::Running {
        return Ok(None);
    }
    if !store.owns_process_log_path(&process.log_path) {
        return Ok(None);
    }
    if !terminal_process_alive(process.pid) {
        let _ = store.mark_session_process_exited(process.id, None);
        return Ok(None);
    }
    let Some(kind) = session_kind_from_command(&process.command) else {
        return Ok(None);
    };
    let Some(thread_id) = process.chat_thread_id else {
        return Ok(None);
    };
    let workspace = store.get_workspace_record(process.workspace_id)?.name;
    let controller = controller_for_kind(kind);
    let connection = ManagedSessionConnection::try_reattach_running(process.pid)?;
    let snapshot = running_session_snapshot(
        process.id,
        thread_id,
        workspace,
        kind,
        process.pid,
        kind == SessionKind::Codex,
    );
    Ok(Some(start_session_handle(
        db_path, logs_dir, snapshot, controller, connection, event_tx,
    )))
}

fn format_input_audit_log(
    workspace: &str,
    session_id: i64,
    input: &str,
    kind: &ArchcarInputKind,
) -> String {
    match kind {
        ArchcarInputKind::ReviewPrompt => format!(
            "\n[staged review prompt]\n{}\n[/staged review prompt]\n",
            crate::redaction::redact_sensitive_text(input)
        ),
        ArchcarInputKind::User => format!(
            "\n[user input {}#{}]\n{}\n[/user input]\n",
            workspace,
            session_id,
            crate::redaction::redact_sensitive_text(input)
        ),
        ArchcarInputKind::ControlCommand => String::new(),
    }
}

fn should_persist_screen_output(
    kind: SessionKind,
    last_fingerprint: &mut Option<String>,
    screen: &str,
) -> bool {
    let fingerprint = screen_persistence_fingerprint(kind, screen);
    if last_fingerprint.as_deref() == Some(fingerprint.as_str()) {
        return false;
    }
    *last_fingerprint = Some(fingerprint);
    true
}

fn screen_persistence_fingerprint(kind: SessionKind, screen: &str) -> String {
    if kind != SessionKind::Codex {
        return screen.to_owned();
    }

    screen
        .lines()
        .map(normalize_codex_working_status_for_fingerprint)
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_codex_working_status_for_fingerprint(line: &str) -> String {
    let Some(working_start) = line.find("Working (") else {
        return line.to_owned();
    };
    let Some(status_end_offset) = line[working_start..].find("esc to interrupt)") else {
        return line.to_owned();
    };
    let status_end = working_start + status_end_offset + "esc to interrupt)".len();
    let mut prefix = line[..working_start].to_owned();
    if let Some((idx, ch)) = prefix
        .char_indices()
        .rev()
        .find(|(_, ch)| !ch.is_whitespace())
    {
        if matches!(ch, '•' | '◦') {
            prefix.replace_range(idx..idx + ch.len_utf8(), "•");
        }
    }
    format!(
        "{prefix}Working ([elapsed] • esc to interrupt){}",
        &line[status_end..]
    )
}

fn user_input_identity_suffix(sequence: u64) -> String {
    format!("input-{sequence}")
}

fn session_ready_for_visible_screen(kind: SessionKind, screen: &str) -> bool {
    match kind {
        SessionKind::Codex => codex_screen_ready_for_input(screen),
        _ => false,
    }
}

fn write_pty_screen_snapshot(
    logs_dir: &std::path::Path,
    source: &str,
    process_id: i64,
    screen: &str,
) {
    if !pty_screen_snapshot_logging_enabled() {
        return;
    }
    let path = logs_dir.join("pty-screens.log");
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let _ = writeln!(
        file,
        "=== [unix_ms={ts}] source={source} process_id={process_id} ===\n{}\n===",
        screen.trim_end_matches('\n')
    );
}

fn pty_screen_snapshot_logging_enabled() -> bool {
    crate::env_flags::enabled("ARCHDUCTOR_LOG_PTY_SCREENS")
}

fn should_attempt_native_thread_resolution(kind: SessionKind, already_resolved: bool) -> bool {
    kind == SessionKind::Codex && !already_resolved
}

fn run_session_loop(
    db_path: std::path::PathBuf,
    logs_dir: std::path::PathBuf,
    snapshot: Arc<Mutex<SessionSnapshot>>,
    _controller: Box<dyn HarnessController>,
    mut pty: ManagedSessionConnection,
    command_rx: Receiver<SessionCommand>,
    event_tx: Sender<ArchcarEvent>,
) {
    let started = snapshot.lock().unwrap().clone();
    let _ = event_tx.send(ArchcarEvent::SessionStarted {
        session_id: started.session_id,
        thread_id: started.thread_id,
        workspace: started.workspace.clone(),
        kind: started.kind,
        pid: started.pid,
    });
    if started.ready {
        let _ = event_tx.send(ArchcarEvent::SessionReady {
            session_id: started.session_id,
            thread_id: started.thread_id,
        });
    }
    let mut last_screen = String::new();
    let mut last_persisted_screen_fingerprint = None;
    let mut native_thread_id_resolved = false;
    let runtime_store = RuntimeSessionStore::new(db_path.clone());
    let mut user_input_sequence =
        match runtime_store.max_runtime_input_provider_sequence(started.session_id) {
            Ok(sequence) => sequence,
            Err(err) => {
                warn!(
                    session_id = started.session_id,
                    thread_id = started.thread_id,
                    error = %format!("{err:#}"),
                    "archcar could not seed runtime input sequence"
                );
                let _ = event_tx.send(ArchcarEvent::SessionError {
                    session_id: Some(started.session_id),
                    thread_id: Some(started.thread_id),
                    message: format!("Could not seed runtime input sequence: {err:#}"),
                });
                return;
            }
        };
    append_runtime_provider_event(
        &runtime_store,
        runtime_provider_event(RuntimeProviderEventInput {
            kind: started.kind,
            session_id: started.session_id,
            thread_id: started.thread_id,
            identity_suffix: None,
            provider_sequence: None,
            phase: ProviderEventPhase::Started,
            event_kind: ProviderEventKind::ThreadSession,
            subtype: "session_started",
            title: "Session started",
            body: provider_name(started.kind),
        }),
        "session_started",
    );
    loop {
        while let Ok(command) = command_rx.try_recv() {
            match command {
                SessionCommand::SendInput {
                    input,
                    visible_input,
                    kind,
                } => {
                    let current = snapshot.lock().unwrap().clone();
                    let persisted_input = visible_input.as_deref().unwrap_or(&input);
                    info!(
                        session_id = current.session_id,
                        thread_id = current.thread_id,
                        workspace = %current.workspace,
                        kind = ?kind,
                        chars = input.chars().count(),
                        "archcar session send_input dequeued"
                    );
                    let (role, source) = match kind {
                        ArchcarInputKind::User => ("user", "user_send"),
                        ArchcarInputKind::ReviewPrompt => ("user", "staged_review_send"),
                        ArchcarInputKind::ControlCommand => ("system", "control_command"),
                    };
                    let log_text = format_input_audit_log(
                        &current.workspace,
                        current.session_id,
                        persisted_input,
                        &kind,
                    );
                    let _ = runtime_store.append_input_and_audit_log(
                        current.thread_id,
                        current.session_id,
                        role,
                        persisted_input,
                        source,
                        &log_text,
                    );
                    user_input_sequence += 1;
                    let user_input_identity = user_input_identity_suffix(user_input_sequence);
                    append_runtime_provider_event(
                        &runtime_store,
                        runtime_provider_event(RuntimeProviderEventInput {
                            kind: current.kind,
                            session_id: current.session_id,
                            thread_id: current.thread_id,
                            identity_suffix: Some(&user_input_identity),
                            provider_sequence: Some(user_input_sequence),
                            phase: ProviderEventPhase::Started,
                            event_kind: ProviderEventKind::UserInput,
                            subtype: match kind {
                                ArchcarInputKind::User => "user_input",
                                ArchcarInputKind::ReviewPrompt => "review_prompt",
                                ArchcarInputKind::ControlCommand => "control_command",
                            },
                            title: match kind {
                                ArchcarInputKind::User => "User input",
                                ArchcarInputKind::ReviewPrompt => "Review prompt",
                                ArchcarInputKind::ControlCommand => "Control command",
                            },
                            body: persisted_input,
                        }),
                        "user_input",
                    );
                    match pty.send_line(&input) {
                        Ok(()) => {
                            match current.kind {
                                SessionKind::Codex => {
                                    if let Ok(mut state) = snapshot.lock() {
                                        state.ready = false;
                                        state.runtime_state = AgentSessionState::Running;
                                    }
                                }
                                SessionKind::Shell => {
                                    if let Ok(mut state) = snapshot.lock() {
                                        state.ready = true;
                                        state.runtime_state = AgentSessionState::WaitingForInput;
                                    }
                                    let _ = event_tx.send(ArchcarEvent::SessionReady {
                                        session_id: current.session_id,
                                        thread_id: current.thread_id,
                                    });
                                }
                                SessionKind::Claude => {}
                            }
                            info!(
                                session_id = current.session_id,
                                thread_id = current.thread_id,
                                kind = ?kind,
                                chars = input.chars().count(),
                                "archcar session send_input wrote to pty"
                            );
                        }
                        Err(err) => {
                            warn!(
                                session_id = current.session_id,
                                thread_id = current.thread_id,
                                kind = ?kind,
                                chars = input.chars().count(),
                                error = %err,
                                "archcar session send_input pty write failed"
                            );
                        }
                    }
                }
                SessionCommand::Kill => {
                    let current = snapshot.lock().unwrap().clone();
                    info!(
                        session_id = current.session_id,
                        thread_id = current.thread_id,
                        workspace = %current.workspace,
                        "archcar session kill dequeued"
                    );
                    if let Err(err) = pty.stop() {
                        warn!(
                            session_id = current.session_id,
                            thread_id = current.thread_id,
                            error = %err,
                            "archcar session kill failed"
                        );
                    }
                }
                SessionCommand::Resize { rows, cols } => {
                    let current = snapshot.lock().unwrap().clone();
                    info!(
                        session_id = current.session_id,
                        thread_id = current.thread_id,
                        rows,
                        cols,
                        "archcar session resize dequeued"
                    );
                    if let Err(err) = pty.resize(rows, cols) {
                        warn!(
                            session_id = current.session_id,
                            thread_id = current.thread_id,
                            rows,
                            cols,
                            error = %err,
                            "archcar session resize failed"
                        );
                    }
                }
            }
        }

        let raw = pty.read_available();
        if !raw.is_empty() {
            let current = snapshot.lock().unwrap().clone();
            let _ = runtime_store.append_raw_output(current.session_id, current.kind, &raw);
        }

        let screen = pty.visible_screen_text();
        if !screen.is_empty() && screen != last_screen {
            let current = snapshot.lock().unwrap().clone();
            let persist_screen = should_persist_screen_output(
                current.kind,
                &mut last_persisted_screen_fingerprint,
                &screen,
            );
            if persist_screen {
                let _ =
                    runtime_store.append_screen_output(current.session_id, current.kind, &screen);
                write_pty_screen_snapshot(
                    &logs_dir,
                    "archcar-session-loop",
                    current.session_id,
                    &screen,
                );
            }
            let ready_event = {
                let mut state = snapshot.lock().unwrap();
                state.screen = screen.clone();
                let _ = event_tx.send(ArchcarEvent::SessionScreenUpdated {
                    session_id: state.session_id,
                });
                if !state.ready && session_ready_for_visible_screen(state.kind, &state.screen) {
                    state.ready = true;
                    state.runtime_state = AgentSessionState::WaitingForInput;
                    Some((state.session_id, state.thread_id))
                } else {
                    None
                }
            };
            if let Some((session_id, thread_id)) = ready_event {
                let _ = event_tx.send(ArchcarEvent::SessionReady {
                    session_id,
                    thread_id,
                });
            }
            last_screen = screen;
        }

        let current = snapshot.lock().unwrap().clone();
        if should_attempt_native_thread_resolution(current.kind, native_thread_id_resolved) {
            native_thread_id_resolved = runtime_store
                .resolve_codex_native_thread_id_for_process(current.session_id)
                .ok()
                .flatten()
                .is_some();
        }

        match pty.has_exited() {
            Ok(true) => {
                let current = snapshot.lock().unwrap().clone();
                append_runtime_provider_event(
                    &runtime_store,
                    runtime_provider_event(RuntimeProviderEventInput {
                        kind: current.kind,
                        session_id: current.session_id,
                        thread_id: current.thread_id,
                        identity_suffix: None,
                        provider_sequence: None,
                        phase: ProviderEventPhase::Completed,
                        event_kind: ProviderEventKind::ThreadSession,
                        subtype: "session_started",
                        title: "Session exited",
                        body: provider_name(current.kind),
                    }),
                    "session_exited",
                );
                let _ = runtime_store.mark_session_process_exited(current.session_id, None);
                if let Ok(mut state) = snapshot.lock() {
                    state.status = ProcessStatus::Exited;
                    let mut machine = SessionStateMachine::from_state(state.runtime_state);
                    machine.mark_exited(None);
                    state.runtime_state = machine.state();
                }
                let _ = event_tx.send(ArchcarEvent::SessionExited {
                    session_id: current.session_id,
                    exit_code: None,
                });
                break;
            }
            Ok(false) => {}
            Err(err) => {
                let current = snapshot.lock().unwrap().clone();
                let error_body = err.to_string();
                append_runtime_provider_event(
                    &runtime_store,
                    runtime_provider_event(RuntimeProviderEventInput {
                        kind: current.kind,
                        session_id: current.session_id,
                        thread_id: current.thread_id,
                        identity_suffix: None,
                        provider_sequence: None,
                        phase: ProviderEventPhase::Failed,
                        event_kind: ProviderEventKind::ThreadSession,
                        subtype: "session_started",
                        title: "Session error",
                        body: &error_body,
                    }),
                    "session_error",
                );
                let _ = event_tx.send(ArchcarEvent::SessionError {
                    session_id: Some(current.session_id),
                    thread_id: Some(current.thread_id),
                    message: err.to_string(),
                });
                break;
            }
        }

        thread::sleep(Duration::from_millis(100));
    }
}

fn terminal_process_alive(process_id: u32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(process_id.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn terminal_device_path_for_pid(process_id: u32) -> Result<PathBuf> {
    let fd = format!("/proc/{process_id}/fd/0");
    let target = fs::read_link(&fd)?;
    let path = target
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("terminal fd path is not valid UTF-8"))?
        .to_owned();
    anyhow::ensure!(
        !path.is_empty() && path.starts_with("/dev/pts/"),
        "process {process_id} is not attached to a PTY slave"
    );
    Ok(PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::{AddRepository, RepositoryStore};
    use crate::workspace::CreateWorkspace;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    #[test]
    fn user_and_review_inputs_write_auditable_logs() {
        assert_eq!(
            format_input_audit_log("berlin", 7, "run tests", &ArchcarInputKind::User),
            "\n[user input berlin#7]\nrun tests\n[/user input]\n"
        );
        assert_eq!(
            format_input_audit_log("berlin", 7, "review body", &ArchcarInputKind::ReviewPrompt),
            "\n[staged review prompt]\nreview body\n[/staged review prompt]\n"
        );
        assert!(format_input_audit_log(
            "berlin",
            7,
            "/model gpt-5.6-sol",
            &ArchcarInputKind::ControlCommand
        )
        .is_empty());
    }

    #[test]
    fn input_audit_logs_redact_sensitive_values() {
        let log = format_input_audit_log(
            "berlin",
            7,
            "use OPENAI_API_KEY=sk-secret bearer ghp_secret --password swordfish",
            &ArchcarInputKind::User,
        );

        assert!(log.contains("[redacted]"));
        assert!(!log.contains("sk-secret"));
        assert!(!log.contains("ghp_secret"));
        assert!(!log.contains("swordfish"));
    }

    #[test]
    fn codex_outputs_use_canonical_wrappers() {
        assert_eq!(
            crate::runtime_session_store::format_session_raw_output(SessionKind::Codex, "hello"),
            "[codex raw]\nhello\n[/codex raw]\n"
        );
        assert_eq!(
            crate::runtime_session_store::format_session_screen_output(
                SessionKind::Codex,
                "hello\n"
            ),
            "[codex screen]\nhello\n[/codex screen]\n"
        );
        assert_eq!(
            crate::runtime_session_store::format_session_raw_output(SessionKind::Shell, "plain"),
            "plain"
        );
        assert_eq!(
            crate::runtime_session_store::format_session_screen_output(SessionKind::Shell, "plain"),
            "plain"
        );
    }

    #[test]
    fn runtime_provider_input_events_use_discrete_identities() {
        let first = runtime_provider_event(RuntimeProviderEventInput {
            kind: SessionKind::Codex,
            session_id: 7,
            thread_id: 11,
            identity_suffix: Some("input-1"),
            provider_sequence: Some(1),
            phase: ProviderEventPhase::Started,
            event_kind: ProviderEventKind::UserInput,
            subtype: "user_input",
            title: "User input",
            body: "first",
        });
        let second = runtime_provider_event(RuntimeProviderEventInput {
            kind: SessionKind::Codex,
            session_id: 7,
            thread_id: 11,
            identity_suffix: Some("input-2"),
            provider_sequence: Some(2),
            phase: ProviderEventPhase::Started,
            event_kind: ProviderEventKind::UserInput,
            subtype: "user_input",
            title: "User input",
            body: "second",
        });

        assert_ne!(first.provider_event_id, second.provider_event_id);
        assert_ne!(first.provider_item_id, second.provider_item_id);
    }

    #[test]
    fn runtime_provider_input_sequence_resumes_from_highest_persisted_sequence() {
        let temp = tempfile::tempdir().unwrap();
        let store = seeded_workspace_store(temp.path());
        let thread = store
            .create_chat_thread("berlin", "codex", "Codex", None)
            .unwrap();
        let launch = store.session_launch("berlin", SessionKind::Codex).unwrap();
        let process = store
            .record_session_process_for_thread("berlin", thread.id, &launch, exited_child_pid())
            .unwrap();
        let db_path = temp.path().join("state.db");
        let first_runtime_store = RuntimeSessionStore::new(db_path.clone());
        append_runtime_provider_event(
            &first_runtime_store,
            runtime_provider_event(RuntimeProviderEventInput {
                kind: SessionKind::Codex,
                session_id: process.id,
                thread_id: thread.id,
                identity_suffix: Some("input-1"),
                provider_sequence: Some(1),
                phase: ProviderEventPhase::Started,
                event_kind: ProviderEventKind::UserInput,
                subtype: "user_input",
                title: "User input",
                body: "first",
            }),
            "test_first_input",
        );
        append_runtime_provider_event(
            &first_runtime_store,
            runtime_provider_event(RuntimeProviderEventInput {
                kind: SessionKind::Codex,
                session_id: process.id,
                thread_id: thread.id,
                identity_suffix: Some("input-3"),
                provider_sequence: Some(3),
                phase: ProviderEventPhase::Started,
                event_kind: ProviderEventKind::UserInput,
                subtype: "user_input",
                title: "User input",
                body: "third",
            }),
            "test_third_input",
        );

        let restored_runtime_store = RuntimeSessionStore::new(db_path);
        let restored_sequence = restored_runtime_store
            .max_runtime_input_provider_sequence(process.id)
            .unwrap();
        let second_suffix = user_input_identity_suffix(restored_sequence + 1);
        append_runtime_provider_event(
            &restored_runtime_store,
            runtime_provider_event(RuntimeProviderEventInput {
                kind: SessionKind::Codex,
                session_id: process.id,
                thread_id: thread.id,
                identity_suffix: Some(&second_suffix),
                provider_sequence: Some(restored_sequence + 1),
                phase: ProviderEventPhase::Started,
                event_kind: ProviderEventKind::UserInput,
                subtype: "user_input",
                title: "User input",
                body: "second",
            }),
            "test_second_input",
        );

        assert_eq!(second_suffix, "input-4");
        assert_eq!(
            restored_runtime_store
                .max_runtime_input_provider_sequence(process.id)
                .unwrap(),
            4
        );
        assert_eq!(
            crate::provider_events::ProviderEventStore::new(temp.path().join("state.db"))
                .list_for_chat_thread(thread.id)
                .unwrap()
                .into_iter()
                .filter(|event| event.kind == ProviderEventKind::UserInput)
                .count(),
            3
        );
    }

    #[test]
    fn archcar_loop_keeps_pty_screens_out_of_normal_semantic_pipeline() {
        let source = include_str!("session.rs");
        let old_pipeline = concat!("persist_codex", "_pipeline_update");
        let screen_message_parser = concat!("parse_codex", "_screen_messages");
        let message_update_helper = concat!("should_emit", "_message_update");

        assert!(source.contains("append_pty_chunk"));
        assert!(
            !source.contains(old_pipeline),
            "archcar runtime loop must not persist Codex semantics from PTY screens"
        );
        assert!(
            !source.contains(screen_message_parser),
            "archcar runtime loop must not parse Codex screen text into messages"
        );
        assert!(
            !source.contains(message_update_helper),
            "archcar runtime loop must not emit semantic message updates from PTY screens"
        );
        assert!(
            source.contains("append_provider_event"),
            "archcar runtime loop should write canonical provider events instead"
        );
    }

    #[test]
    fn codex_process_lifecycle_is_ready_without_screen_semantic_detection() {
        assert!(process_lifecycle_ready(SessionKind::Codex));
        assert!(process_lifecycle_ready(SessionKind::Shell));
        assert!(!process_lifecycle_ready(SessionKind::Claude));
    }

    #[test]
    fn archcar_runtime_loop_uses_runtime_session_store_boundary() {
        let source = include_str!("session.rs");
        let broad_store_open = concat!("WorkspaceStore::", "open(&db_path)");

        assert!(source.contains("RuntimeSessionStore::new"));
        assert!(
            !source.contains(broad_store_open),
            "archcar runtime loop should use the narrow runtime session store boundary"
        );
    }

    #[test]
    fn screen_persistence_ignores_codex_timer_repaints_without_semantic_parsing() {
        let first_screen = "\
› Explain this codebase

• Explored
  └ Read main.rs, server.rs

• Working (2m 05s • esc to interrupt) · 1 background terminal running · /ps to …";
        let timer_repaint = "\
› Explain this codebase

• Explored
  └ Read main.rs, server.rs

◦ Working (2m 06s • esc to interrupt) · 1 background terminal running · /ps to …";
        let real_update = "\
› Explain this codebase

• Explored
  └ Read main.rs, server.rs

• Found a real issue in session.rs.

• Working (2m 07s • esc to interrupt) · 1 background terminal running · /ps to …";
        let background_terminal_update = "\
› Explain this codebase

• Explored
  └ Read main.rs, server.rs

• Working (2m 07s • esc to interrupt) · 2 background terminals running · /ps to …";

        let mut last_persisted = None;
        assert!(should_persist_screen_output(
            SessionKind::Codex,
            &mut last_persisted,
            first_screen
        ));
        assert!(!should_persist_screen_output(
            SessionKind::Codex,
            &mut last_persisted,
            first_screen
        ));
        assert!(!should_persist_screen_output(
            SessionKind::Codex,
            &mut last_persisted,
            timer_repaint
        ));
        assert!(should_persist_screen_output(
            SessionKind::Codex,
            &mut last_persisted,
            background_terminal_update
        ));
        assert!(should_persist_screen_output(
            SessionKind::Codex,
            &mut last_persisted,
            real_update
        ));

        assert!(timer_repaint.contains("Working (2m 06s"));
    }

    #[test]
    fn codex_screen_readiness_marks_session_ready_without_semantic_message_parsing() {
        let ready_screen = "\
› Follow up

  gpt-5.6-sol medium · ~/archductor/workspaces/demo";
        let working_screen = "\
› Follow up

• Working (12s • esc to interrupt)

  gpt-5.6-sol medium · ~/archductor/workspaces/demo";

        assert!(session_ready_for_visible_screen(
            SessionKind::Codex,
            ready_screen
        ));
        assert!(!session_ready_for_visible_screen(
            SessionKind::Codex,
            working_screen
        ));
        assert!(!session_ready_for_visible_screen(
            SessionKind::Shell,
            ready_screen
        ));
    }

    #[test]
    fn native_thread_resolution_stays_codex_only() {
        assert!(should_attempt_native_thread_resolution(
            SessionKind::Codex,
            false
        ));
        assert!(!should_attempt_native_thread_resolution(
            SessionKind::Codex,
            true
        ));
        assert!(!should_attempt_native_thread_resolution(
            SessionKind::Shell,
            false
        ));
    }

    #[test]
    fn codex_thread_launch_without_native_id_starts_clean_session() {
        let temp = tempfile::tempdir().unwrap();
        let store = seeded_workspace_store(temp.path());
        let controller = controller_for_kind(SessionKind::Codex);

        let launch = build_thread_session_launch(
            &store,
            "berlin",
            SessionKind::Codex,
            SessionHarnessOptions::default(),
            None,
            controller.as_ref(),
        )
        .unwrap();

        assert!(!launch.args.iter().any(|arg| arg == "resume"));
        assert!(!launch.args.iter().any(|arg| arg == "--last"));
        assert!(launch.session_resume_id.is_none());
    }

    #[test]
    fn codex_thread_launch_with_native_id_resumes_that_session() {
        let temp = tempfile::tempdir().unwrap();
        let store = seeded_workspace_store(temp.path());
        let controller = controller_for_kind(SessionKind::Codex);

        let launch = build_thread_session_launch(
            &store,
            "berlin",
            SessionKind::Codex,
            SessionHarnessOptions::default(),
            Some("codex-native-thread"),
            controller.as_ref(),
        )
        .unwrap();

        assert!(launch.args.iter().any(|arg| arg == "resume"));
        assert!(launch.args.iter().any(|arg| arg == "codex-native-thread"));
        assert_eq!(
            launch.session_resume_id.as_deref(),
            Some("codex-native-thread")
        );
    }

    #[test]
    fn session_kind_detection_restores_runtime_supported_harnesses() {
        assert_eq!(
            session_kind_from_command("codex --no-alt-screen"),
            Some(SessionKind::Codex)
        );
        assert_eq!(session_kind_from_command("codex"), Some(SessionKind::Codex));
        assert_eq!(
            session_kind_from_command("claude --print"),
            Some(SessionKind::Claude)
        );
        assert_eq!(
            session_kind_from_command("bash -lc ls"),
            Some(SessionKind::Shell)
        );
        assert_eq!(session_kind_from_command(""), None);
    }

    #[test]
    fn adopt_running_session_marks_dead_codex_record_exited() {
        let temp = tempfile::tempdir().unwrap();
        let store = seeded_workspace_store(temp.path());
        let thread = store
            .create_chat_thread("berlin", "codex", "Codex", None)
            .unwrap();
        let launch = store.session_launch("berlin", SessionKind::Codex).unwrap();
        let process = store
            .record_session_process_for_thread("berlin", thread.id, &launch, exited_child_pid())
            .unwrap();

        let adopted = adopt_running_session(&store, "berlin", SessionKind::Codex).unwrap();

        assert!(adopted.is_none());
        let reconciled = store.get_process_record(process.id).unwrap();
        assert_eq!(reconciled.status, ProcessStatus::Exited);
        assert!(reconciled.ended_at.is_some());
    }

    #[test]
    fn adopt_running_session_ignores_non_archcar_owned_records() {
        let temp = tempfile::tempdir().unwrap();
        let data_logs_dir = temp.path().join("data-logs");
        let archcar_logs_dir = temp.path().join("state-logs");
        let data_store = seeded_workspace_store_with_logs(temp.path(), &data_logs_dir);
        let thread = data_store
            .create_chat_thread("berlin", "codex", "Codex", None)
            .unwrap();
        let launch = data_store
            .session_launch("berlin", SessionKind::Codex)
            .unwrap();
        let process = data_store
            .record_session_process_for_thread("berlin", thread.id, &launch, exited_child_pid())
            .unwrap();
        assert!(process.log_path.starts_with(&data_logs_dir));

        let archcar_store =
            WorkspaceStore::open_with_logs(temp.path().join("state.db"), &archcar_logs_dir)
                .unwrap();
        let adopted = adopt_running_session(&archcar_store, "berlin", SessionKind::Codex).unwrap();

        assert!(adopted.is_none());
        let unchanged = data_store.get_process_record(process.id).unwrap();
        assert_eq!(unchanged.status, ProcessStatus::Running);
        assert!(unchanged.ended_at.is_none());
    }

    #[test]
    fn restore_managed_session_ignores_non_archcar_owned_records() {
        let temp = tempfile::tempdir().unwrap();
        let data_logs_dir = temp.path().join("data-logs");
        let archcar_logs_dir = temp.path().join("state-logs");
        let store = seeded_workspace_store_with_logs(temp.path(), &data_logs_dir);
        let thread = store
            .create_chat_thread("berlin", "codex", "Codex", None)
            .unwrap();
        let launch = store.session_launch("berlin", SessionKind::Codex).unwrap();
        let process = store
            .record_session_process_for_thread("berlin", thread.id, &launch, std::process::id())
            .unwrap();
        assert!(process.log_path.starts_with(&data_logs_dir));

        let (event_tx, _event_rx) = mpsc::channel();
        let restored = restore_managed_session(
            temp.path().join("state.db"),
            archcar_logs_dir,
            process.id,
            event_tx,
        )
        .unwrap();

        assert!(restored.is_none());
        let unchanged = store.get_process_record(process.id).unwrap();
        assert_eq!(unchanged.status, ProcessStatus::Running);
        assert!(unchanged.ended_at.is_none());
    }

    #[test]
    fn pty_screen_snapshot_writer_is_disabled_without_explicit_flag() {
        let temp = tempfile::tempdir().unwrap();
        write_pty_screen_snapshot(temp.path(), "archcar", 7, "hello\nworld\n");
        assert!(!temp.path().join("pty-screens.log").exists());
    }

    fn seeded_workspace_store(root: &Path) -> WorkspaceStore {
        seeded_workspace_store_with_logs(root, &root.join("logs"))
    }

    fn seeded_workspace_store_with_logs(root: &Path, logs_dir: &Path) -> WorkspaceStore {
        let repo_path = init_repo(root.join("demo"));
        let db_path = root.join("state.db");
        RepositoryStore::open(&db_path)
            .unwrap()
            .add(AddRepository {
                name: Some("demo".to_owned()),
                root_path: repo_path,
                default_branch: Some("main".to_owned()),
                remote_name: "origin".to_owned(),
                workspace_parent_path: Some(root.join("workspaces/demo")),
            })
            .unwrap();
        let store = WorkspaceStore::open_with_logs(&db_path, logs_dir).unwrap();
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
