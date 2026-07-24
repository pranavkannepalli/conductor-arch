use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command as ProcessCommand, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use tracing::{info, warn};

use crate::archcar::harness::{
    controller_for_kind, ensure_thread_for_kind, managed_harness_for_kind, provider_name,
    HarnessController,
};
use crate::archcar::harness_contract::{
    DesiredHarnessControls, HarnessAdapterContext, HarnessControl, HarnessControlPlan,
    HarnessEffect, HarnessInput, HarnessSignal, HarnessTurnStatus, ManagedHarnessAdapter,
    NativeRecord,
};
use crate::archcar::protocol::{
    session_harness_capabilities_for_descriptor, ArchcarEvent, ArchcarInputDelivery,
    ArchcarInputKind, SessionHarnessCapabilities,
};
use crate::codex_tui::codex_screen_ready_for_input;
use crate::harness::{normalize_agent_harness_options, FORCED_APPROVAL_MODE};
use crate::provider_adapters::claude_hooks::build_claude_hook_settings;
use crate::provider_adapters::claude_stream::{
    build_claude_stream_args, ClaudeManagedAdapter, ClaudeStreamLaunchConfig,
};
use crate::provider_adapters::codex_app_server::{
    parse_jsonl_message, write_initialize_request_with_id, write_initialized_notification,
    write_thread_start_request_with_id, write_turn_start_request_with_id,
    write_turn_steer_request_with_id, CodexAppServerInitializeParams, CodexAppServerMessage,
    CodexAppServerThreadStartParams, CodexAppServerTurnStartParams, CodexAppServerTurnSteerParams,
    CodexAppServerUserInput, CodexManagedAdapter, CODEX_APP_SERVER_DEFAULT_ARGS,
};
use crate::provider_events::{
    ProviderEventContext, ProviderEventDraft, ProviderEventKind, ProviderEventPhase,
};
use crate::provider_inputs::ProviderInputInput;
use crate::pty::PtySession;
use crate::runtime_session_store::RuntimeSessionStore;
use crate::session_state::{AgentSessionState, SessionStateMachine};
use crate::settings::PromptKind;
use crate::workspace::{
    ChatThreadRecord, ProcessStatus, SessionHarnessOptions, SessionKind, SessionLaunch,
    WorkspaceStore,
};
use serde_json::json;

pub const CODEX_RECOVERY_CURRENT_USER_MESSAGE_HEADER: &str = "Current user message:";

/// Builds the provider-facing first-turn text while keeping visible chat text separate.
///
/// General instructions are prepended only for the first visible user turn in a
/// durable chat thread. Follow-up turns keep the user's text unchanged so the
/// prompt snapshot does not leak into every provider input.
pub fn compose_first_turn_input(general: Option<&str>, visible: &str, first_turn: bool) -> String {
    if first_turn {
        if let Some(general) = general.map(str::trim).filter(|prompt| !prompt.is_empty()) {
            return format!("{general}\n\n{visible}");
        }
    }
    visible.to_owned()
}

pub fn compose_recovered_thread_input(recovery: Option<&str>, input: &str) -> String {
    let Some(recovery) = recovery.map(str::trim).filter(|value| !value.is_empty()) else {
        return input.to_owned();
    };
    format!("{recovery}\n\n{CODEX_RECOVERY_CURRENT_USER_MESSAGE_HEADER}\n{input}")
}

fn resolved_general_prompt(db_path: &std::path::Path, workspace: &str) -> Result<Option<String>> {
    WorkspaceStore::open_app(db_path)?.resolved_prompt(workspace, PromptKind::General)
}

fn durable_thread_is_first_user_turn(db_path: &std::path::Path, thread_id: i64) -> Result<bool> {
    Ok(!WorkspaceStore::open(db_path)?
        .list_chat_messages(thread_id)?
        .iter()
        .any(|message| message.role == "user"))
}

#[derive(Debug)]
pub enum SessionCommand {
    SendInput {
        input: String,
        visible_input: Option<String>,
        kind: ArchcarInputKind,
        delivery: ArchcarInputDelivery,
    },
    ApplyControl(HarnessControl),
    Resize {
        rows: u16,
        cols: u16,
    },
    InterruptTurn,
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
    pub capabilities: Option<SessionHarnessCapabilities>,
    pub screen: String,
}

#[derive(Clone)]
pub struct SessionHandle {
    pub snapshot: Arc<Mutex<SessionSnapshot>>,
    pub command_tx: Sender<SessionCommand>,
}

enum ManagedSessionConnection {
    Live(PtySession),
    CodexAppServer(ProviderProcessConnection),
    ClaudeStream(ProviderProcessConnection),
    Reattached {
        write: std::fs::File,
        output: Arc<Mutex<String>>,
        read_cursor: usize,
        pid: u32,
    },
}

struct ProviderProcessConnection {
    child: Child,
    stdin: ChildStdin,
    stdout_rx: Receiver<String>,
    next_read_line: usize,
    native_thread_id: Option<String>,
    cwd: PathBuf,
    model: Option<String>,
    approval_policy: Option<String>,
    reasoning_mode: Option<String>,
    effort_mode: Option<String>,
    personality: Option<String>,
    pending_recovery_context: Option<String>,
}

static PROVIDER_NATIVE_SESSION_LAUNCH_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn provider_native_session_launch_guard(kind: SessionKind) -> Option<MutexGuard<'static, ()>> {
    matches!(kind, SessionKind::Codex | SessionKind::Claude).then(|| {
        PROVIDER_NATIVE_SESSION_LAUNCH_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    })
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
            Self::CodexAppServer(_) | Self::ClaudeStream(_) => {
                anyhow::bail!("provider-native sessions do not accept terminal input")
            }
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

    fn write_raw(&mut self, input: &str) -> Result<()> {
        match self {
            Self::Live(session) => session.write(input),
            Self::CodexAppServer(_) | Self::ClaudeStream(_) => {
                anyhow::bail!("provider-native sessions do not accept terminal input")
            }
            Self::Reattached { write, .. } => {
                write.write_all(input.as_bytes())?;
                write.flush()?;
                Ok(())
            }
        }
    }

    fn stop(&mut self) -> Result<()> {
        match self {
            Self::Live(session) => session.stop(),
            Self::CodexAppServer(connection) | Self::ClaudeStream(connection) => {
                connection.child.kill()?;
                Ok(())
            }
            Self::Reattached { pid, .. } => stop_reattached_process(*pid),
        }
    }

    fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        match self {
            Self::Live(session) => session.resize(rows, cols),
            Self::CodexAppServer(_) | Self::ClaudeStream(_) => Ok(()),
            Self::Reattached { .. } => Ok(()),
        }
    }

    fn has_exited(&mut self) -> Result<bool> {
        match self {
            Self::Live(session) => session.has_exited(),
            Self::CodexAppServer(connection) | Self::ClaudeStream(connection) => {
                Ok(connection.child.try_wait()?.is_some())
            }
            Self::Reattached { pid, .. } => Ok(!terminal_process_alive(*pid)),
        }
    }

    fn read_available(&mut self) -> String {
        match self {
            Self::Live(session) => session.read_available(),
            Self::CodexAppServer(connection) | Self::ClaudeStream(connection) => {
                let mut output = String::new();
                while let Ok(line) = connection.stdout_rx.try_recv() {
                    output.push_str(&line);
                    output.push('\n');
                }
                output
            }
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
            Self::CodexAppServer(_) | Self::ClaudeStream(_) => String::new(),
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
    let _provider_native_launch_guard = provider_native_session_launch_guard(kind);
    let store = open_managed_session_store(db_path.clone(), logs_dir.clone())?;
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
    let ThreadSessionLaunch {
        launch,
        provider_port_reservation,
        startup_recovery_context,
    } = build_thread_session_launch(
        &store,
        &workspace,
        &thread_record,
        kind,
        harness,
        controller.as_ref(),
    )?;
    spawn_live_managed_session(LiveSessionStart {
        db_path,
        logs_dir,
        store: &store,
        workspace,
        thread_id: thread_record.id,
        kind,
        launch,
        startup_recovery_context,
        provider_port_reservation,
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
    startup_recovery_context: Option<String>,
    provider_port_reservation: Option<TcpListener>,
    controller: Box<dyn HarnessController>,
    event_tx: Sender<ArchcarEvent>,
}

fn spawn_live_managed_session(start: LiveSessionStart<'_>) -> Result<SessionHandle> {
    if matches!(start.kind, SessionKind::Codex | SessionKind::Claude) {
        return spawn_provider_native_managed_session(start);
    }

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

fn spawn_provider_native_managed_session(start: LiveSessionStart<'_>) -> Result<SessionHandle> {
    let _provider_port_reservation = start.provider_port_reservation;
    let mut command = ProcessCommand::new(&start.launch.program);
    command
        .args(&start.launch.args)
        .current_dir(&start.launch.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    for (key, value) in &start.launch.env {
        command.env(key, value);
    }
    crate::platform::configure_new_process_group(&mut command);

    let mut child = command
        .spawn()
        .with_context(|| format!("spawn managed {:?} provider process", start.kind))?;
    let pid = child.id();
    let stdin = child
        .stdin
        .take()
        .context("provider process stdin was not piped")?;
    let stdout = child
        .stdout
        .take()
        .context("provider process stdout was not piped")?;
    let stdout_rx = spawn_stdout_line_reader(stdout);
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
        false,
    );
    let connection = ProviderProcessConnection {
        child,
        stdin,
        stdout_rx,
        next_read_line: 0,
        native_thread_id: start.launch.session_resume_id.clone(),
        cwd: start.launch.cwd.clone(),
        model: model_from_harness_metadata(start.launch.harness_metadata.as_deref()),
        approval_policy: approval_from_harness_metadata(start.launch.harness_metadata.as_deref()),
        reasoning_mode: reasoning_from_harness_metadata(start.launch.harness_metadata.as_deref()),
        effort_mode: effort_from_harness_metadata(start.launch.harness_metadata.as_deref()),
        personality: personality_from_harness_metadata(start.launch.harness_metadata.as_deref()),
        pending_recovery_context: start.startup_recovery_context.clone(),
    };
    let connection = match start.kind {
        SessionKind::Codex => ManagedSessionConnection::CodexAppServer(connection),
        SessionKind::Claude => ManagedSessionConnection::ClaudeStream(connection),
        SessionKind::Shell => unreachable!("shell sessions use PTY"),
    };

    Ok(start_session_handle(
        start.db_path,
        start.logs_dir,
        snapshot,
        start.controller,
        connection,
        start.event_tx,
    ))
}

fn spawn_stdout_line_reader(stdout: std::process::ChildStdout) -> Receiver<String> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    rx
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

fn apply_harness_effect(
    runtime_store: &RuntimeSessionStore,
    snapshot: &Arc<Mutex<SessionSnapshot>>,
    event_tx: &Sender<ArchcarEvent>,
    started: &SessionSnapshot,
    native_thread_id: &mut Option<String>,
    effect: HarnessEffect,
) {
    match effect {
        HarnessEffect::Initialized {
            native_session_id,
            model,
        } => {
            *native_thread_id = Some(native_session_id.clone());
            let _ =
                runtime_store.update_chat_thread_native_id(started.thread_id, &native_session_id);
            info!(
                session_id = started.session_id,
                thread_id = started.thread_id,
                kind = ?started.kind,
                native_session_id = %native_session_id,
                model = model.as_deref().unwrap_or("unknown"),
                "managed harness initialized"
            );
        }
        HarnessEffect::Ready => {
            mark_snapshot_ready(snapshot);
            info!(
                session_id = started.session_id,
                thread_id = started.thread_id,
                kind = ?started.kind,
                "managed harness ready"
            );
            let _ = event_tx.send(ArchcarEvent::SessionReady {
                session_id: started.session_id,
                thread_id: started.thread_id,
            });
        }
        HarnessEffect::InputAcknowledged { local_input_id } => {
            info!(
                session_id = started.session_id,
                thread_id = started.thread_id,
                kind = ?started.kind,
                local_input_id = %local_input_id,
                "managed harness input acknowledged"
            );
            if let Err(err) = runtime_store.mark_provider_input_acknowledged(&local_input_id, None)
            {
                let _ = event_tx.send(ArchcarEvent::SessionError {
                    session_id: Some(started.session_id),
                    thread_id: Some(started.thread_id),
                    message: format!("Provider input acknowledgement persistence failed: {err:#}"),
                });
            }
        }
        HarnessEffect::TurnCompleted {
            local_input_id,
            status,
        } => {
            info!(
                session_id = started.session_id,
                thread_id = started.thread_id,
                kind = ?started.kind,
                local_input_id = %local_input_id,
                status = harness_turn_status_label(status),
                "managed harness turn completed"
            );
            if let Err(err) = runtime_store.mark_provider_input_terminal(&local_input_id) {
                let _ = event_tx.send(ArchcarEvent::SessionError {
                    session_id: Some(started.session_id),
                    thread_id: Some(started.thread_id),
                    message: format!("Provider input terminal persistence failed: {err:#}"),
                });
                return;
            }
            let _ = event_tx.send(ArchcarEvent::TurnCompleted {
                session_id: started.session_id,
                thread_id: started.thread_id,
                status: Some(harness_turn_status_label(status).to_owned()),
            });
        }
        HarnessEffect::ProviderEvent(draft) => {
            append_runtime_provider_event(runtime_store, draft, "provider_native_event");
            let _ = event_tx.send(ArchcarEvent::SessionMessagesUpdated {
                thread_id: started.thread_id,
            });
        }
        HarnessEffect::InteractionRequested(draft) => {
            match runtime_store.register_provider_interaction(draft) {
                Ok(interaction) => {
                    let _ =
                        event_tx.send(ArchcarEvent::ProviderInteractionRequested { interaction });
                }
                Err(err) => {
                    let _ = event_tx.send(ArchcarEvent::SessionError {
                        session_id: Some(started.session_id),
                        thread_id: Some(started.thread_id),
                        message: format!("Provider interaction registration failed: {err:#}"),
                    });
                }
            }
        }
        HarnessEffect::Warning(message) => {
            let _ = event_tx.send(ArchcarEvent::SessionError {
                session_id: Some(started.session_id),
                thread_id: Some(started.thread_id),
                message,
            });
        }
        HarnessEffect::Fatal(message) => {
            mark_provider_session_failed(runtime_store, snapshot, event_tx, started, message);
        }
        HarnessEffect::CapabilitiesObserved(observed_native) => {
            let capabilities = capabilities_for_kind(started.kind, observed_native);
            if let Some(capabilities) = capabilities {
                if let Ok(mut guard) = snapshot.lock() {
                    guard.capabilities = Some(capabilities.clone());
                }
                info!(
                    session_id = started.session_id,
                    thread_id = started.thread_id,
                    kind = ?started.kind,
                    required_count = capabilities.required.len(),
                    optional_count = capabilities.optional.len(),
                    observed_native = ?capabilities.observed_native,
                    "managed harness capabilities observed"
                );
                let _ = event_tx.send(ArchcarEvent::SessionCapabilitiesChanged {
                    session_id: started.session_id,
                    thread_id: started.thread_id,
                    capabilities,
                });
            }
        }
        HarnessEffect::TurnStarted { local_input_id } => {
            info!(
                session_id = started.session_id,
                thread_id = started.thread_id,
                kind = ?started.kind,
                local_input_id = %local_input_id,
                "managed harness turn started"
            );
        }
        HarnessEffect::Retry { message, delay_ms } => {
            let provider_message = bounded_redacted_provider_message(&message);
            info!(
                session_id = started.session_id,
                thread_id = started.thread_id,
                kind = ?started.kind,
                delay_ms = ?delay_ms,
                provider_message = %provider_message,
                "managed harness retry"
            );
        }
        HarnessEffect::RateLimited {
            message,
            retry_after_ms,
        } => {
            info!(
                session_id = started.session_id,
                thread_id = started.thread_id,
                kind = ?started.kind,
                retry_after_ms = ?retry_after_ms,
                provider_message = %message,
                "managed harness rate limited"
            );
        }
        HarnessEffect::InteractionResolved { .. } | HarnessEffect::ResumeRequired => {}
    }
}

fn bounded_redacted_provider_message(message: &str) -> String {
    crate::local_chat::truncate_chars(&crate::redaction::redact_sensitive_text(message), 512)
}

#[cfg(test)]
mod provider_log_tests {
    use super::bounded_redacted_provider_message;

    #[test]
    fn retry_provider_messages_are_redacted_and_bounded() {
        let message = format!("Authorization: Bearer secret-token {}", "x".repeat(600));
        let logged = bounded_redacted_provider_message(&message);
        assert!(!logged.contains("secret-token"));
        assert!(logged.chars().count() <= 515);
        assert!(logged.ends_with("..."));
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
        capabilities: capabilities_for_kind(kind, Vec::new()),
        screen: String::new(),
    }
}

fn capabilities_for_kind(
    kind: SessionKind,
    observed_native: Vec<String>,
) -> Option<SessionHarnessCapabilities> {
    managed_harness_for_kind(kind).map(|harness| {
        session_harness_capabilities_for_descriptor(harness.descriptor(), observed_native)
    })
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
    let _provider_native_launch_guard = provider_native_session_launch_guard(kind);
    let store = open_managed_session_store(db_path.clone(), logs_dir.clone())?;
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
    let ThreadSessionLaunch {
        launch,
        provider_port_reservation,
        startup_recovery_context,
    } = build_thread_session_launch(
        &store,
        &workspace,
        &thread_record,
        kind,
        harness,
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
        startup_recovery_context,
        provider_port_reservation,
        controller,
        event_tx,
    })
}

struct ThreadSessionLaunch {
    launch: SessionLaunch,
    startup_recovery_context: Option<String>,
    provider_port_reservation: Option<TcpListener>,
}

fn build_thread_session_launch(
    store: &WorkspaceStore,
    workspace: &str,
    thread_record: &ChatThreadRecord,
    kind: SessionKind,
    harness: SessionHarnessOptions,
    controller: &dyn HarnessController,
) -> Result<ThreadSessionLaunch> {
    if kind == SessionKind::Codex {
        return codex_app_server_session_launch(store, workspace, thread_record, harness);
    }
    if kind == SessionKind::Claude {
        return claude_stream_session_launch(store, workspace, thread_record, harness);
    }
    Ok(ThreadSessionLaunch {
        launch: controller.build_launch(store, workspace, harness)?,
        startup_recovery_context: None,
        provider_port_reservation: None,
    })
}

fn codex_app_server_session_launch(
    store: &WorkspaceStore,
    workspace: &str,
    thread_record: &ChatThreadRecord,
    harness: SessionHarnessOptions,
) -> Result<ThreadSessionLaunch> {
    let harness = normalize_agent_harness_options(harness);
    let mut launch = store.session_launch_with_options(
        workspace,
        SessionKind::Codex,
        SessionHarnessOptions::default(),
    )?;
    launch.args = CODEX_APP_SERVER_DEFAULT_ARGS
        .iter()
        .map(|arg| (*arg).to_owned())
        .collect();
    launch.harness_metadata = non_interactive_harness_metadata("codex-app-server", &harness);
    let resume_plan = codex_resume_plan_for_thread(store, thread_record.id, thread_record)?;
    launch.session_resume_id = resume_plan.native_thread_id;
    let provider_port_reservation =
        assign_provider_native_thread_port(store, workspace, thread_record, &mut launch)?;
    Ok(ThreadSessionLaunch {
        launch,
        startup_recovery_context: resume_plan.recovery_context,
        provider_port_reservation: Some(provider_port_reservation),
    })
}

struct CodexResumePlan {
    native_thread_id: Option<String>,
    recovery_context: Option<String>,
}

fn codex_resume_plan_for_thread(
    store: &WorkspaceStore,
    thread_id: i64,
    thread_record: &ChatThreadRecord,
) -> Result<CodexResumePlan> {
    let Some(native_thread_id) = thread_record.native_thread_id.as_deref() else {
        return Ok(CodexResumePlan {
            native_thread_id: None,
            recovery_context: None,
        });
    };
    if store.codex_native_thread_id_has_rollout(native_thread_id)? {
        return Ok(CodexResumePlan {
            native_thread_id: Some(native_thread_id.to_owned()),
            recovery_context: None,
        });
    }
    let recovery_context = store.codex_recovery_context_for_thread(thread_id, native_thread_id)?;
    let _ = store.clear_chat_thread_native_id(thread_id)?;
    Ok(CodexResumePlan {
        native_thread_id: None,
        recovery_context: Some(recovery_context),
    })
}

fn claude_stream_session_launch(
    store: &WorkspaceStore,
    workspace: &str,
    thread_record: &ChatThreadRecord,
    harness: SessionHarnessOptions,
) -> Result<ThreadSessionLaunch> {
    let harness = normalize_agent_harness_options(harness);
    let mut launch = store.session_launch_with_options(
        workspace,
        SessionKind::Claude,
        SessionHarnessOptions::default(),
    )?;
    let hook_settings = std::env::current_exe().ok().and_then(|executable| {
        serde_json::to_string(&build_claude_hook_settings(&executable, thread_record.id)).ok()
    });
    launch.args = build_claude_stream_args(&ClaudeStreamLaunchConfig {
        persistent_input: true,
        replay_user_messages: true,
        resume: thread_record.native_thread_id.clone(),
        permission_mode: claude_stream_permission_mode(&harness),
        model: sanitize_harness_text(harness.model.as_deref()),
        effort: claude_stream_effort_mode(&harness),
        append_system_prompt: None,
        settings_json: hook_settings,
    });
    launch.harness_metadata = non_interactive_harness_metadata("claude-stream-json", &harness);
    launch.session_resume_id = thread_record.native_thread_id.clone();
    let provider_port_reservation =
        assign_provider_native_thread_port(store, workspace, thread_record, &mut launch)?;
    Ok(ThreadSessionLaunch {
        launch,
        startup_recovery_context: None,
        provider_port_reservation: Some(provider_port_reservation),
    })
}

const PROVIDER_NATIVE_PORT_START: u16 = 43000;
const PROVIDER_NATIVE_PORT_METADATA_KEY: &str = "port";
const PROVIDER_NATIVE_PORT_ENV: &str = "ARCHDUCTOR_PROVIDER_PORT";

fn assign_provider_native_thread_port(
    store: &WorkspaceStore,
    workspace: &str,
    thread_record: &ChatThreadRecord,
    launch: &mut SessionLaunch,
) -> Result<TcpListener> {
    let reservation = reserve_provider_native_thread_port(store, workspace, thread_record.id)?;
    let port = reservation
        .local_addr()
        .context("read reserved provider-native port")?
        .port();
    set_launch_env(
        launch,
        PROVIDER_NATIVE_PORT_ENV,
        OsString::from(port.to_string()),
    );
    launch.harness_metadata = append_metadata_entry(
        launch.harness_metadata.take(),
        PROVIDER_NATIVE_PORT_METADATA_KEY,
        &port.to_string(),
    );
    Ok(reservation)
}

fn reserve_provider_native_thread_port(
    store: &WorkspaceStore,
    workspace: &str,
    thread_id: i64,
) -> Result<TcpListener> {
    let occupied_ports = provider_native_occupied_ports(store, workspace, thread_id)?;
    for port in PROVIDER_NATIVE_PORT_START..=u16::MAX {
        if occupied_ports.contains(&port) {
            continue;
        }
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            return Ok(listener);
        }
    }

    anyhow::bail!(
        "no available provider-native ports at or above {} for workspace {workspace}",
        PROVIDER_NATIVE_PORT_START
    )
}

#[cfg(test)]
fn provider_native_thread_port_with_checker(
    store: &WorkspaceStore,
    workspace: &str,
    thread_id: i64,
    port_available: impl Fn(u16) -> bool,
) -> Result<u16> {
    let occupied_ports = provider_native_occupied_ports(store, workspace, thread_id)?;

    for port in PROVIDER_NATIVE_PORT_START..=u16::MAX {
        if occupied_ports.contains(&port) {
            continue;
        }
        if port_available(port) {
            return Ok(port);
        }
    }

    anyhow::bail!(
        "no available provider-native ports at or above {} for workspace {workspace}",
        PROVIDER_NATIVE_PORT_START
    )
}

fn provider_native_occupied_ports(
    store: &WorkspaceStore,
    workspace: &str,
    thread_id: i64,
) -> Result<std::collections::HashSet<u16>> {
    let _workspace_record = store.get_workspace_record_by_name(workspace)?;
    Ok(store
        .list_running_sessions()?
        .into_iter()
        .filter(|process| process.chat_thread_id != Some(thread_id))
        .filter(|process| terminal_process_alive(process.pid))
        .filter_map(|process| {
            metadata_value(
                process.session_harness_metadata.as_deref(),
                PROVIDER_NATIVE_PORT_METADATA_KEY,
            )
            .and_then(|value| value.parse::<u16>().ok())
        })
        .collect::<std::collections::HashSet<_>>())
}

fn set_launch_env(launch: &mut SessionLaunch, key: &str, value: OsString) {
    if let Some((_, existing)) = launch.env.iter_mut().find(|(name, _)| name == key) {
        *existing = value;
    } else {
        launch.env.push((key.to_owned(), value));
    }
}

fn append_metadata_entry(metadata: Option<String>, key: &str, value: &str) -> Option<String> {
    let entry = format!("{key}={}", sanitize_metadata_value(value));
    match metadata {
        Some(metadata) if !metadata.trim().is_empty() => Some(format!("{metadata};{entry}")),
        _ => Some(entry),
    }
}

fn non_interactive_harness_metadata(
    harness_name: &str,
    harness: &SessionHarnessOptions,
) -> Option<String> {
    let mut entries = vec![format!("harness={harness_name}")];
    if harness.plan_mode {
        entries.push("plan=true".to_owned());
    }
    if harness.fast_mode {
        entries.push("fast=true".to_owned());
    }
    if let Some(value) = sanitize_harness_text(harness.model.as_deref()) {
        entries.push(format!("model={}", sanitize_metadata_value(&value)));
    }
    entries.push(format!("approval={FORCED_APPROVAL_MODE}"));
    if let Some(value) = sanitize_harness_text(harness.reasoning_mode.as_deref()) {
        entries.push(format!("reasoning={}", sanitize_metadata_value(&value)));
    }
    if let Some(value) = sanitize_harness_text(harness.effort_mode.as_deref()) {
        entries.push(format!("effort={}", sanitize_metadata_value(&value)));
    }
    if let Some(value) = sanitize_harness_text(harness.codex_personality.as_deref()) {
        entries.push(format!("personality={}", sanitize_metadata_value(&value)));
    }
    Some(entries.join(";"))
}

fn claude_stream_permission_mode(_harness: &SessionHarnessOptions) -> Option<String> {
    Some("bypassPermissions".to_owned())
}

fn claude_stream_effort_mode(harness: &SessionHarnessOptions) -> Option<String> {
    sanitize_harness_text(harness.effort_mode.as_deref()).or_else(|| {
        if harness.fast_mode {
            Some("low".to_owned())
        } else {
            sanitize_harness_text(harness.reasoning_mode.as_deref())
        }
    })
}

fn sanitize_harness_text(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn set_provider_connection_model(
    connection: &mut ProviderProcessConnection,
    model: Option<String>,
) {
    connection.model = sanitize_harness_text(model.as_deref());
}

fn set_provider_connection_effort(
    connection: &mut ProviderProcessConnection,
    effort: Option<String>,
) {
    connection.effort_mode = sanitize_harness_text(effort.as_deref());
}

fn set_provider_connection_permission_mode(
    connection: &mut ProviderProcessConnection,
    _permission_mode: Option<String>,
) {
    connection.approval_policy = Some(FORCED_APPROVAL_MODE.to_owned());
}

fn apply_provider_connection_controls(
    connection: &mut ProviderProcessConnection,
    controls: DesiredHarnessControls,
) {
    set_provider_connection_model(connection, controls.model);
    set_provider_connection_effort(connection, controls.effort);
    set_provider_connection_permission_mode(connection, controls.permission_mode);
}

fn apply_provider_control_plan(
    runtime_store: &RuntimeSessionStore,
    snapshot: &Arc<Mutex<SessionSnapshot>>,
    event_tx: &Sender<ArchcarEvent>,
    started: &SessionSnapshot,
    connection: &mut ProviderProcessConnection,
    plan: HarnessControlPlan,
) {
    match plan {
        HarnessControlPlan::NativeWrite(native_write) => {
            if let Err(err) = connection
                .stdin
                .write_all(&native_write.payload)
                .and_then(|_| connection.stdin.flush())
            {
                let _ = connection.child.kill();
                mark_provider_session_failed(
                    runtime_store,
                    snapshot,
                    event_tx,
                    started,
                    format!("Provider control write failed: {err:#}"),
                );
            }
        }
        HarnessControlPlan::Signal(HarnessSignal::TerminateProcessGroup) => {
            let _ = crate::platform::terminate_process_group(connection.child.id(), false);
        }
        HarnessControlPlan::Signal(HarnessSignal::InterruptProcessGroup) => {
            let _ = crate::platform::interrupt_process_group(connection.child.id());
        }
        HarnessControlPlan::RestartRequired(controls) => {
            if started.kind == SessionKind::Claude && connection.native_thread_id.is_none() {
                let _ = event_tx.send(ArchcarEvent::SessionError {
                    session_id: Some(started.session_id),
                    thread_id: Some(started.thread_id),
                    message: "Claude session has no native session id to resume".to_owned(),
                });
                return;
            }
            apply_provider_connection_controls(connection, controls);
        }
        HarnessControlPlan::Emulated(effect) => {
            let mut native_thread_id = connection.native_thread_id.clone();
            apply_harness_effect(
                runtime_store,
                snapshot,
                event_tx,
                started,
                &mut native_thread_id,
                effect,
            );
            connection.native_thread_id = native_thread_id;
        }
        HarnessControlPlan::Unsupported { reason } => {
            let _ = event_tx.send(ArchcarEvent::SessionError {
                session_id: Some(started.session_id),
                thread_id: Some(started.thread_id),
                message: reason,
            });
        }
    }
}

fn sanitize_metadata_value(value: &str) -> String {
    value.replace(';', ",").replace('\n', " ")
}

fn model_from_harness_metadata(metadata: Option<&str>) -> Option<String> {
    metadata_value(metadata, "model")
}

fn approval_from_harness_metadata(metadata: Option<&str>) -> Option<String> {
    let _ = metadata;
    Some(FORCED_APPROVAL_MODE.to_owned())
}

fn reasoning_from_harness_metadata(metadata: Option<&str>) -> Option<String> {
    metadata_value(metadata, "reasoning")
}

fn effort_from_harness_metadata(metadata: Option<&str>) -> Option<String> {
    metadata_value(metadata, "effort")
}

fn personality_from_harness_metadata(metadata: Option<&str>) -> Option<String> {
    metadata_value(metadata, "personality")
}

fn codex_sandbox_for_approval(approval_policy: Option<&str>) -> Option<&'static str> {
    let _ = approval_policy;
    Some("danger-full-access")
}

fn metadata_value(metadata: Option<&str>, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    metadata?
        .split(';')
        .find_map(|entry| entry.strip_prefix(&prefix).map(ToOwned::to_owned))
}

fn provider_native_process(metadata: Option<&str>) -> bool {
    matches!(
        metadata_value(metadata, "harness").as_deref(),
        Some("codex-app-server" | "claude-stream-json")
    )
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
                && !provider_native_process(record.session_harness_metadata.as_deref())
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
    let store = open_managed_session_store(&db_path, &logs_dir)?;
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
    if provider_native_process(process.session_harness_metadata.as_deref()) {
        if terminal_process_alive(process.pid) {
            terminate_process(process.pid);
        }
        let _ = store.mark_session_process_exited(process.id, None);
        return Ok(None);
    }
    if !terminal_process_alive(process.pid) {
        let _ = store.mark_session_process_exited(process.id, None);
        return Ok(None);
    }
    let Some(kind) = session_kind_from_command(&process.command) else {
        return Ok(None);
    };
    if kind == SessionKind::Shell {
        if terminate_process_and_wait(process.pid) {
            let _ = store.mark_session_process_exited(process.id, None);
        } else {
            warn!(
                session_id = process.id,
                pid = process.pid,
                "archcar restore left shell session running after termination failed"
            );
        }
        return Ok(None);
    }
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

fn open_managed_session_store(
    db_path: impl AsRef<std::path::Path>,
    logs_dir: impl AsRef<std::path::Path>,
) -> Result<WorkspaceStore> {
    WorkspaceStore::open_app_with_logs(db_path, logs_dir)
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
        ArchcarInputKind::ControlCommand | ArchcarInputKind::RawTerminal => String::new(),
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

fn provider_input_local_id(session_id: i64, sequence: u64) -> String {
    format!("session-{session_id}-input-{sequence}")
}

fn session_ready_for_visible_screen(kind: SessionKind, screen: &str) -> bool {
    match kind {
        SessionKind::Codex => codex_screen_ready_for_input(screen),
        _ => false,
    }
}

fn session_ready_for_visible_screen_after_busy(
    kind: SessionKind,
    screen: &str,
    saw_busy_since_input: bool,
) -> bool {
    match kind {
        SessionKind::Codex => saw_busy_since_input && codex_screen_ready_for_input(screen),
        _ => session_ready_for_visible_screen(kind, screen),
    }
}

fn session_busy_for_visible_screen(kind: SessionKind, screen: &str) -> bool {
    match kind {
        SessionKind::Codex => screen.contains("Working ("),
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
    match pty {
        ManagedSessionConnection::CodexAppServer(connection) => {
            return run_codex_app_server_session_loop(
                db_path, snapshot, connection, command_rx, event_tx,
            );
        }
        ManagedSessionConnection::ClaudeStream(connection) => {
            return run_claude_stream_session_loop(
                db_path, snapshot, connection, command_rx, event_tx,
            );
        }
        _ => {}
    }

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
    let mut codex_busy_since_input = false;
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
                    delivery: _,
                } => {
                    let current = snapshot.lock().unwrap().clone();
                    if kind == ArchcarInputKind::RawTerminal {
                        match pty.write_raw(&input) {
                            Ok(()) => {
                                if let Ok(mut state) = snapshot.lock() {
                                    state.ready = true;
                                    state.runtime_state = AgentSessionState::WaitingForInput;
                                }
                            }
                            Err(err) => {
                                let message = format!("Raw terminal input failed: {err:#}");
                                warn!(
                                    session_id = current.session_id,
                                    thread_id = current.thread_id,
                                    kind = ?kind,
                                    chars = input.chars().count(),
                                    error = %err,
                                    "archcar session raw terminal write failed"
                                );
                                let _ = event_tx.send(ArchcarEvent::SessionError {
                                    session_id: Some(current.session_id),
                                    thread_id: Some(current.thread_id),
                                    message,
                                });
                                let stopped = match pty.stop() {
                                    Ok(()) => true,
                                    Err(stop_err) => {
                                        warn!(
                                            session_id = current.session_id,
                                            thread_id = current.thread_id,
                                            error = %stop_err,
                                            "archcar session stop after raw terminal write failure failed"
                                        );
                                        pty.has_exited().unwrap_or(false)
                                    }
                                };
                                if stopped {
                                    let _ = runtime_store
                                        .mark_session_process_exited(current.session_id, None);
                                    mark_snapshot_exited(&snapshot, None);
                                    let _ = event_tx.send(ArchcarEvent::SessionExited {
                                        session_id: current.session_id,
                                        exit_code: None,
                                    });
                                } else if let Ok(mut state) = snapshot.lock() {
                                    state.ready = false;
                                    state.runtime_state = AgentSessionState::Failed;
                                }
                                return;
                            }
                        }
                        continue;
                    }
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
                        ArchcarInputKind::RawTerminal => unreachable!("raw terminal handled above"),
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
                                ArchcarInputKind::RawTerminal => {
                                    unreachable!("raw terminal handled above")
                                }
                            },
                            title: match kind {
                                ArchcarInputKind::User => "User input",
                                ArchcarInputKind::ReviewPrompt => "Review prompt",
                                ArchcarInputKind::ControlCommand => "Control command",
                                ArchcarInputKind::RawTerminal => {
                                    unreachable!("raw terminal handled above")
                                }
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
                                    codex_busy_since_input = false;
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
                SessionCommand::InterruptTurn => {}
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
                SessionCommand::ApplyControl(_) => {}
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
                if !state.ready && session_busy_for_visible_screen(state.kind, &state.screen) {
                    codex_busy_since_input = true;
                }
                if !state.ready
                    && session_ready_for_visible_screen_after_busy(
                        state.kind,
                        &state.screen,
                        codex_busy_since_input,
                    )
                {
                    state.ready = true;
                    state.runtime_state = AgentSessionState::WaitingForInput;
                    codex_busy_since_input = false;
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

fn run_codex_app_server_session_loop(
    db_path: PathBuf,
    snapshot: Arc<Mutex<SessionSnapshot>>,
    mut connection: ProviderProcessConnection,
    command_rx: Receiver<SessionCommand>,
    event_tx: Sender<ArchcarEvent>,
) {
    let started = snapshot.lock().unwrap().clone();
    let general_prompt = match resolved_general_prompt(&db_path, &started.workspace) {
        Ok(prompt) => prompt,
        Err(err) => {
            let _ = event_tx.send(ArchcarEvent::SessionError {
                session_id: Some(started.session_id),
                thread_id: Some(started.thread_id),
                message: format!("Could not resolve general prompt: {err:#}"),
            });
            None
        }
    };
    let runtime_store = RuntimeSessionStore::new(db_path.clone());
    let _ = event_tx.send(ArchcarEvent::SessionStarted {
        session_id: started.session_id,
        thread_id: started.thread_id,
        workspace: started.workspace.clone(),
        kind: started.kind,
        pid: started.pid,
    });
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
            body: "codex app-server",
        }),
        "codex_app_server_session_started",
    );
    let _ = runtime_store.append_provider_native_output(
        started.session_id,
        "codex-app-server",
        "transport started: codex app-server stdio JSONL",
    );

    if let Err(err) = start_codex_app_server_lifecycle(&mut connection, &started) {
        mark_provider_session_failed(&runtime_store, &snapshot, &event_tx, &started, err);
        return;
    }
    let mut startup_phase = CodexStartupPhase::InitializePending;
    let startup_request_id = 2_u64;
    let mut provider_thread_id = connection.native_thread_id.clone();
    let mut user_input_sequence = runtime_store
        .max_runtime_input_provider_sequence(started.session_id)
        .unwrap_or(0);
    let mut fallback_request_sequence = 0_u64;
    let mut active_turn_id: Option<String> = None;
    let mut active_input_ids = Vec::<String>::new();
    let mut pending_input_requests = HashMap::<u64, PendingCodexInputRequest>::new();
    let mut control_adapter = CodexManagedAdapter::new(HarnessAdapterContext {
        session_id: started.session_id,
        thread_id: started.thread_id,
        workspace: started.workspace.clone(),
        native_session_id: connection.native_thread_id.clone(),
        controls: DesiredHarnessControls {
            model: connection.model.clone(),
            effort: connection.effort_mode.clone(),
            permission_mode: connection.approval_policy.clone(),
        },
    });

    loop {
        while let Ok(line) = connection.stdout_rx.try_recv() {
            connection.next_read_line += 1;
            let _ = runtime_store.append_provider_native_output(
                started.session_id,
                "codex-app-server",
                &line,
            );
            match parse_jsonl_message(&line, connection.next_read_line) {
                Ok(message) => {
                    persist_codex_app_server_message(&runtime_store, &message, &started);
                    if startup_phase == CodexStartupPhase::InitializePending
                        && codex_response_id(&message) == Some(1)
                    {
                        if let Some(error) = codex_response_error(&message) {
                            let _ = connection.child.kill();
                            mark_provider_session_failed(
                                &runtime_store,
                                &snapshot,
                                &event_tx,
                                &started,
                                error,
                            );
                            return;
                        }
                        if let Err(err) =
                            continue_codex_app_server_startup_after_initialize(&mut connection)
                        {
                            let _ = connection.child.kill();
                            mark_provider_session_failed(
                                &runtime_store,
                                &snapshot,
                                &event_tx,
                                &started,
                                err,
                            );
                            return;
                        }
                        startup_phase = CodexStartupPhase::ThreadPending;
                        info!(
                            session_id = started.session_id,
                            thread_id = started.thread_id,
                            "started codex app-server lifecycle"
                        );
                    }
                    if startup_phase == CodexStartupPhase::ThreadPending
                        && codex_response_id(&message) == Some(startup_request_id)
                    {
                        if let Some(error) = codex_response_error(&message) {
                            let _ = connection.child.kill();
                            mark_provider_session_failed(
                                &runtime_store,
                                &snapshot,
                                &event_tx,
                                &started,
                                error,
                            );
                            return;
                        }
                    }
                    if let Some(native_id) =
                        codex_thread_id_from_startup_response(&message, startup_request_id)
                    {
                        startup_phase = CodexStartupPhase::Ready;
                        provider_thread_id = Some(native_id.clone());
                        connection.native_thread_id = Some(native_id.clone());
                        let _ = runtime_store
                            .update_chat_thread_native_id(started.thread_id, &native_id);
                        apply_harness_effect(
                            &runtime_store,
                            &snapshot,
                            &event_tx,
                            &started,
                            &mut connection.native_thread_id,
                            HarnessEffect::Ready,
                        );
                    }
                    if let Some(request_id) = codex_response_id(&message) {
                        if let Some(pending) = pending_input_requests.remove(&request_id) {
                            let response_error = codex_response_error(&message);
                            match codex_input_response_action(&pending, response_error.is_some()) {
                                CodexInputResponseAction::Complete => {
                                    if let Err(err) = runtime_store
                                        .mark_provider_input_acknowledged(
                                            &pending.local_input_id,
                                            Some(&request_id.to_string()),
                                        )
                                    {
                                        mark_provider_session_failed(
                                            &runtime_store,
                                            &snapshot,
                                            &event_tx,
                                            &started,
                                            format!(
                                                "Codex input acknowledgement persistence failed: {err:#}"
                                            ),
                                        );
                                        return;
                                    }
                                    active_input_ids.push(pending.local_input_id);
                                }
                                CodexInputResponseAction::RetryStart => {
                                    let Some(thread_id) = provider_thread_id.as_deref() else {
                                        let _ = event_tx.send(ArchcarEvent::SessionError {
                                            session_id: Some(started.session_id),
                                            thread_id: Some(started.thread_id),
                                            message:
                                                "Codex app-server thread is not initialized yet"
                                                    .to_owned(),
                                        });
                                        continue;
                                    };
                                    fallback_request_sequence =
                                        fallback_request_sequence.saturating_add(1);
                                    let fallback_request_id =
                                        next_fallback_request_id(fallback_request_sequence);
                                    let params = codex_turn_start_params(
                                        &connection,
                                        thread_id,
                                        &pending.input,
                                    );
                                    if let Err(err) = write_turn_start_request_with_id(
                                        &mut connection.stdin,
                                        fallback_request_id,
                                        &params,
                                    ) {
                                        let _ = connection.child.kill();
                                        mark_provider_session_failed(
                                            &runtime_store,
                                            &snapshot,
                                            &event_tx,
                                            &started,
                                            format!(
                                                "Codex immediate turn fallback failed: {err:#}"
                                            ),
                                        );
                                        return;
                                    }
                                    pending_input_requests.insert(
                                        fallback_request_id,
                                        PendingCodexInputRequest {
                                            kind: CodexInputRequestKind::ImmediateFallbackStart,
                                            local_input_id: pending.local_input_id,
                                            input: pending.input,
                                        },
                                    );
                                }
                                CodexInputResponseAction::ReportError => {
                                    if let Some(error) = response_error.as_deref() {
                                        let _ = runtime_store.mark_provider_input_failed(
                                            &pending.local_input_id,
                                            error,
                                        );
                                    }
                                    mark_snapshot_running_not_ready(&snapshot);
                                    let _ = event_tx.send(ArchcarEvent::SessionError {
                                        session_id: Some(started.session_id),
                                        thread_id: Some(started.thread_id),
                                        message: response_error.unwrap_or_else(|| {
                                            "Codex app-server input request failed".to_owned()
                                        }),
                                    });
                                }
                            }
                        }
                    }
                    if let Some(turn_id) = codex_turn_id_from_message(&message) {
                        active_turn_id = Some(turn_id);
                        control_adapter.set_active_turn_id(active_turn_id.clone());
                    }
                    if message.method.as_deref() == Some("turn/completed") {
                        active_turn_id = None;
                        control_adapter.set_active_turn_id(None);
                        for local_input_id in active_input_ids.drain(..) {
                            if let Err(err) =
                                runtime_store.mark_provider_input_terminal(&local_input_id)
                            {
                                mark_provider_session_failed(
                                    &runtime_store,
                                    &snapshot,
                                    &event_tx,
                                    &started,
                                    format!("Codex input terminal persistence failed: {err:#}"),
                                );
                                return;
                            }
                        }
                        let _ = event_tx.send(ArchcarEvent::TurnCompleted {
                            session_id: started.session_id,
                            thread_id: started.thread_id,
                            status: codex_turn_completed_status(&message),
                        });
                        apply_harness_effect(
                            &runtime_store,
                            &snapshot,
                            &event_tx,
                            &started,
                            &mut connection.native_thread_id,
                            HarnessEffect::Ready,
                        );
                    }
                    let _ = event_tx.send(ArchcarEvent::SessionMessagesUpdated {
                        thread_id: started.thread_id,
                    });
                }
                Err(err) => warn!(
                    session_id = started.session_id,
                    error = %format!("{err:#}"),
                    "failed to parse codex app-server message"
                ),
            }
        }

        while let Ok(command) = command_rx.try_recv() {
            match command {
                SessionCommand::SendInput {
                    input,
                    visible_input,
                    kind,
                    delivery,
                } => {
                    let first_turn = if kind == ArchcarInputKind::ControlCommand {
                        false
                    } else {
                        match durable_thread_is_first_user_turn(&db_path, started.thread_id) {
                            Ok(first_turn) => first_turn,
                            Err(err) => {
                                let _ = event_tx.send(ArchcarEvent::SessionError {
                                    session_id: Some(started.session_id),
                                    thread_id: Some(started.thread_id),
                                    message: format!(
                                        "Could not prepare Codex input history: {err:#}"
                                    ),
                                });
                                continue;
                            }
                        }
                    };
                    let provider_input =
                        compose_first_turn_input(general_prompt.as_deref(), &input, first_turn);
                    let persisted_input = visible_input.as_deref().unwrap_or(&input).to_owned();
                    let Some(thread_id) = provider_thread_id.as_deref() else {
                        let _ = event_tx.send(ArchcarEvent::SessionError {
                            session_id: Some(started.session_id),
                            thread_id: Some(started.thread_id),
                            message: "Codex app-server thread is not initialized yet".to_owned(),
                        });
                        continue;
                    };
                    let next_input_sequence = user_input_sequence + 1;
                    let local_input_id =
                        provider_input_local_id(started.session_id, next_input_sequence);
                    let request_id = next_turn_request_id(next_input_sequence);
                    let auto_active = snapshot.lock().map(|state| !state.ready).unwrap_or(false);
                    let route = codex_input_route(delivery, active_turn_id.as_deref(), auto_active);
                    let recovery_context = matches!(route, CodexInputRoute::Start)
                        .then(|| connection.pending_recovery_context.as_deref())
                        .flatten();
                    let uses_recovery_context = recovery_context.is_some();
                    let provider_input =
                        compose_recovered_thread_input(recovery_context, &provider_input);
                    let request_kind = match (&route, delivery) {
                        (CodexInputRoute::Start, _) => CodexInputRequestKind::Start,
                        (CodexInputRoute::Steer { .. }, ArchcarInputDelivery::Immediate) => {
                            CodexInputRequestKind::ImmediateSteer
                        }
                        (CodexInputRoute::Steer { .. }, ArchcarInputDelivery::Auto) => {
                            CodexInputRequestKind::AutoSteer
                        }
                    };
                    if let Err(err) = enqueue_provider_input(
                        &runtime_store,
                        &started,
                        &local_input_id,
                        &input,
                        visible_input.as_deref(),
                        &kind,
                        delivery,
                        connection.native_thread_id.clone(),
                    ) {
                        mark_provider_session_failed(
                            &runtime_store,
                            &snapshot,
                            &event_tx,
                            &started,
                            format!("Codex input persistence failed: {err:#}"),
                        );
                        continue;
                    }
                    let write_result = match route {
                        CodexInputRoute::Steer { expected_turn_id } => {
                            write_turn_steer_request_with_id(
                                &mut connection.stdin,
                                request_id,
                                &CodexAppServerTurnSteerParams {
                                    thread_id: thread_id.to_owned(),
                                    input: vec![CodexAppServerUserInput::Text {
                                        text: provider_input.clone(),
                                    }],
                                    expected_turn_id: Some(expected_turn_id),
                                },
                            )
                        }
                        CodexInputRoute::Start => {
                            let params =
                                codex_turn_start_params(&connection, thread_id, &provider_input);
                            write_turn_start_request_with_id(
                                &mut connection.stdin,
                                request_id,
                                &params,
                            )
                        }
                    };
                    if let Err(err) = write_result {
                        let _ = runtime_store
                            .mark_provider_input_failed(&local_input_id, &err.to_string());
                        let _ = connection.child.kill();
                        mark_provider_session_failed(
                            &runtime_store,
                            &snapshot,
                            &event_tx,
                            &started,
                            format!("Codex turn input failed: {err:#}"),
                        );
                    } else if let Err(err) =
                        runtime_store.mark_provider_input_written(&local_input_id)
                    {
                        let _ = connection.child.kill();
                        mark_provider_session_failed(
                            &runtime_store,
                            &snapshot,
                            &event_tx,
                            &started,
                            format!("Codex input written-state persistence failed: {err:#}"),
                        );
                    } else {
                        if uses_recovery_context {
                            connection.pending_recovery_context = None;
                        }
                        pending_input_requests.insert(
                            request_id,
                            PendingCodexInputRequest {
                                kind: request_kind,
                                local_input_id: local_input_id.clone(),
                                input: provider_input.clone(),
                            },
                        );
                        user_input_sequence = next_input_sequence;
                        persist_runtime_user_input(
                            &runtime_store,
                            &started,
                            &provider_input,
                            Some(&persisted_input),
                            &kind,
                            user_input_sequence,
                        );
                        let _ = event_tx.send(ArchcarEvent::SessionMessagesUpdated {
                            thread_id: started.thread_id,
                        });
                        if let Ok(mut state) = snapshot.lock() {
                            state.ready = false;
                            state.runtime_state = AgentSessionState::Running;
                        }
                    }
                }
                SessionCommand::Kill => {
                    let _ = connection.child.kill();
                }
                SessionCommand::InterruptTurn => {
                    control_adapter.set_native_session_id(provider_thread_id.clone());
                    let plan = control_adapter.plan_control(HarnessControl::Interrupt);
                    apply_provider_control_plan(
                        &runtime_store,
                        &snapshot,
                        &event_tx,
                        &started,
                        &mut connection,
                        plan,
                    );
                    if let Ok(mut state) = snapshot.lock() {
                        state.ready = false;
                        state.runtime_state = AgentSessionState::Running;
                    }
                }
                SessionCommand::ApplyControl(control) => {
                    let plan = control_adapter.plan_control(control);
                    apply_provider_control_plan(
                        &runtime_store,
                        &snapshot,
                        &event_tx,
                        &started,
                        &mut connection,
                        plan,
                    );
                }
                SessionCommand::Resize { .. } => {}
            }
        }

        match connection.child.try_wait() {
            Ok(Some(status)) => {
                let code = status.code();
                let _ = runtime_store.mark_session_process_exited(started.session_id, code);
                mark_snapshot_exited(&snapshot, code);
                let _ = event_tx.send(ArchcarEvent::SessionExited {
                    session_id: started.session_id,
                    exit_code: code,
                });
                break;
            }
            Ok(None) => {}
            Err(err) => {
                mark_provider_session_failed(&runtime_store, &snapshot, &event_tx, &started, err);
                break;
            }
        }

        thread::sleep(Duration::from_millis(50));
    }
}

fn run_claude_stream_session_loop(
    db_path: PathBuf,
    snapshot: Arc<Mutex<SessionSnapshot>>,
    mut connection: ProviderProcessConnection,
    command_rx: Receiver<SessionCommand>,
    event_tx: Sender<ArchcarEvent>,
) {
    let started = snapshot.lock().unwrap().clone();
    let general_prompt = match resolved_general_prompt(&db_path, &started.workspace) {
        Ok(prompt) => prompt,
        Err(err) => {
            let _ = event_tx.send(ArchcarEvent::SessionError {
                session_id: Some(started.session_id),
                thread_id: Some(started.thread_id),
                message: format!("Could not resolve general prompt: {err:#}"),
            });
            None
        }
    };
    let runtime_store = RuntimeSessionStore::new(db_path.clone());
    let _ = event_tx.send(ArchcarEvent::SessionStarted {
        session_id: started.session_id,
        thread_id: started.thread_id,
        workspace: started.workspace.clone(),
        kind: started.kind,
        pid: started.pid,
    });
    let _ = runtime_store.append_provider_native_output(
        started.session_id,
        "claude-stream-json",
        "transport started: claude -p stream-json",
    );
    let mut user_input_sequence = runtime_store
        .max_runtime_input_provider_sequence(started.session_id)
        .unwrap_or(0);
    let mut adapter = ClaudeManagedAdapter::new(HarnessAdapterContext {
        session_id: started.session_id,
        thread_id: started.thread_id,
        workspace: started.workspace.clone(),
        native_session_id: connection.native_thread_id.clone(),
        controls: DesiredHarnessControls {
            model: connection.model.clone(),
            effort: connection.effort_mode.clone(),
            permission_mode: connection.approval_policy.clone(),
        },
    });

    loop {
        drain_claude_stdout(
            &runtime_store,
            &snapshot,
            &event_tx,
            &started,
            &mut connection,
            &mut adapter,
        );

        while let Ok(command) = command_rx.try_recv() {
            match command {
                SessionCommand::SendInput {
                    input,
                    visible_input,
                    kind,
                    delivery,
                } => {
                    let first_turn = if kind == ArchcarInputKind::ControlCommand {
                        false
                    } else {
                        match durable_thread_is_first_user_turn(&db_path, started.thread_id) {
                            Ok(first_turn) => first_turn,
                            Err(err) => {
                                let _ = event_tx.send(ArchcarEvent::SessionError {
                                    session_id: Some(started.session_id),
                                    thread_id: Some(started.thread_id),
                                    message: format!(
                                        "Could not prepare Claude input history: {err:#}"
                                    ),
                                });
                                continue;
                            }
                        }
                    };
                    let provider_input =
                        compose_first_turn_input(general_prompt.as_deref(), &input, first_turn);
                    let persisted_input = visible_input.as_deref().unwrap_or(&input).to_owned();
                    let next_input_sequence = user_input_sequence + 1;
                    let local_input_id =
                        provider_input_local_id(started.session_id, next_input_sequence);
                    if let Err(err) = enqueue_provider_input(
                        &runtime_store,
                        &started,
                        &local_input_id,
                        &provider_input,
                        Some(&persisted_input),
                        &kind,
                        delivery,
                        connection.native_thread_id.clone(),
                    ) {
                        mark_provider_session_failed(
                            &runtime_store,
                            &snapshot,
                            &event_tx,
                            &started,
                            format!("Claude input persistence failed: {err:#}"),
                        );
                        continue;
                    }
                    let native_write = adapter.encode_input(HarnessInput {
                        local_input_id: local_input_id.clone(),
                        content: provider_input.clone(),
                        visible_content: Some(persisted_input.clone()),
                        kind: kind.clone(),
                        delivery,
                    });
                    let write_result = native_write.and_then(|native_write| {
                        anyhow::ensure!(
                            native_write.provider_key == "claude",
                            "Claude adapter encoded input for {}",
                            native_write.provider_key,
                        );
                        connection.stdin.write_all(&native_write.payload)?;
                        connection.stdin.flush()?;
                        Ok(())
                    });
                    if let Err(err) = write_result {
                        let _ = runtime_store
                            .mark_provider_input_failed(&local_input_id, &err.to_string());
                        adapter.settle_failed_input_write(&local_input_id);
                        let _ = connection.child.kill();
                        mark_provider_session_failed(
                            &runtime_store,
                            &snapshot,
                            &event_tx,
                            &started,
                            format!("Claude stream input failed: {err:#}"),
                        );
                    } else if let Err(err) =
                        runtime_store.mark_provider_input_written(&local_input_id)
                    {
                        adapter.settle_failed_input_write(&local_input_id);
                        let _ = connection.child.kill();
                        mark_provider_session_failed(
                            &runtime_store,
                            &snapshot,
                            &event_tx,
                            &started,
                            format!("Claude input written-state persistence failed: {err:#}"),
                        );
                    } else {
                        user_input_sequence = next_input_sequence;
                        persist_runtime_user_input(
                            &runtime_store,
                            &started,
                            &provider_input,
                            Some(&persisted_input),
                            &kind,
                            user_input_sequence,
                        );
                        if let Ok(mut state) = snapshot.lock() {
                            state.ready = false;
                            state.runtime_state = AgentSessionState::Running;
                        }
                    }
                }
                SessionCommand::Kill => {
                    let _ = connection.child.kill();
                }
                SessionCommand::InterruptTurn => {
                    let plan = adapter.plan_control(HarnessControl::Interrupt);
                    apply_provider_control_plan(
                        &runtime_store,
                        &snapshot,
                        &event_tx,
                        &started,
                        &mut connection,
                        plan,
                    );
                }
                SessionCommand::ApplyControl(control) => {
                    let plan = adapter.plan_control(control);
                    apply_provider_control_plan(
                        &runtime_store,
                        &snapshot,
                        &event_tx,
                        &started,
                        &mut connection,
                        plan,
                    );
                }
                SessionCommand::Resize { .. } => {}
            }
        }

        match connection.child.try_wait() {
            Ok(Some(status)) => {
                drain_claude_stdout(
                    &runtime_store,
                    &snapshot,
                    &event_tx,
                    &started,
                    &mut connection,
                    &mut adapter,
                );
                let code = status.code();
                let _ = runtime_store.mark_session_process_exited(started.session_id, code);
                mark_snapshot_exited(&snapshot, code);
                let _ = event_tx.send(ArchcarEvent::SessionExited {
                    session_id: started.session_id,
                    exit_code: code,
                });
                break;
            }
            Ok(None) => {}
            Err(err) => {
                mark_provider_session_failed(&runtime_store, &snapshot, &event_tx, &started, err);
                break;
            }
        }

        thread::sleep(Duration::from_millis(50));
    }
}

fn drain_claude_stdout(
    runtime_store: &RuntimeSessionStore,
    snapshot: &Arc<Mutex<SessionSnapshot>>,
    event_tx: &Sender<ArchcarEvent>,
    started: &SessionSnapshot,
    connection: &mut ProviderProcessConnection,
    adapter: &mut ClaudeManagedAdapter,
) {
    while let Ok(line) = connection.stdout_rx.try_recv() {
        let _ = runtime_store.append_provider_native_output(
            started.session_id,
            "claude-stream-json",
            &line,
        );
        match adapter.observe_native(NativeRecord {
            provider_key: "claude",
            payload: line.into_bytes(),
        }) {
            Ok(effects) => {
                for effect in effects {
                    apply_harness_effect(
                        runtime_store,
                        snapshot,
                        event_tx,
                        started,
                        &mut connection.native_thread_id,
                        effect,
                    );
                }
            }
            Err(err) => warn!(
                session_id = started.session_id,
                error = %format!("{err:#}"),
                "failed to parse claude stream-json message"
            ),
        }
    }
}

fn start_codex_app_server_lifecycle(
    connection: &mut ProviderProcessConnection,
    _started: &SessionSnapshot,
) -> Result<()> {
    write_initialize_request_with_id(
        &mut connection.stdin,
        1,
        &CodexAppServerInitializeParams {
            client_name: "archductor".to_owned(),
            client_title: Some("Archductor".to_owned()),
            client_version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            workspace_root: None,
        },
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexStartupPhase {
    InitializePending,
    ThreadPending,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CodexInputRoute {
    Start,
    Steer { expected_turn_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexInputRequestKind {
    Start,
    AutoSteer,
    ImmediateSteer,
    ImmediateFallbackStart,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingCodexInputRequest {
    kind: CodexInputRequestKind,
    local_input_id: String,
    input: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexInputResponseAction {
    Complete,
    RetryStart,
    ReportError,
}

fn codex_input_route(
    delivery: ArchcarInputDelivery,
    active_turn_id: Option<&str>,
    auto_active: bool,
) -> CodexInputRoute {
    let should_steer = delivery == ArchcarInputDelivery::Immediate || auto_active;
    if should_steer {
        if let Some(turn_id) = active_turn_id {
            return CodexInputRoute::Steer {
                expected_turn_id: turn_id.to_owned(),
            };
        }
    }
    CodexInputRoute::Start
}

fn codex_input_response_action(
    pending: &PendingCodexInputRequest,
    failed: bool,
) -> CodexInputResponseAction {
    if !failed {
        CodexInputResponseAction::Complete
    } else if pending.kind == CodexInputRequestKind::ImmediateSteer {
        CodexInputResponseAction::RetryStart
    } else {
        CodexInputResponseAction::ReportError
    }
}

fn continue_codex_app_server_startup_after_initialize(
    connection: &mut ProviderProcessConnection,
) -> Result<()> {
    write_initialized_notification(&mut connection.stdin)?;
    if let Some(native_thread_id) = connection.native_thread_id.as_deref() {
        write_provider_json_line(
            &mut connection.stdin,
            &json!({
                "id": 2,
                "method": "thread/resume",
                "params": {
                    "threadId": native_thread_id,
                    "cwd": connection.cwd.to_string_lossy(),
                    "serviceName": "archductor",
                },
            }),
        )?;
    } else {
        write_thread_start_request_with_id(
            &mut connection.stdin,
            2,
            &CodexAppServerThreadStartParams {
                model: connection.model.clone(),
                cwd: Some(connection.cwd.clone()),
                approval_policy: connection.approval_policy.clone(),
                sandbox: codex_sandbox_for_approval(connection.approval_policy.as_deref())
                    .map(str::to_owned),
                service_name: Some("archductor".to_owned()),
            },
        )?;
    }
    Ok(())
}

fn persist_codex_app_server_message(
    runtime_store: &RuntimeSessionStore,
    message: &CodexAppServerMessage,
    started: &SessionSnapshot,
) {
    let event = codex_app_server_provider_event_for_session(message, started);
    if !should_persist_provider_native_event(event.kind) {
        return;
    }
    if let Err(err) = runtime_store.append_provider_event(&event) {
        warn!(
            session_id = started.session_id,
            thread_id = started.thread_id,
            error = %format!("{err:#}"),
            "failed to persist codex app-server provider event"
        );
    }
}

fn should_persist_provider_native_event(kind: ProviderEventKind) -> bool {
    kind != ProviderEventKind::UserInput
}

fn codex_app_server_provider_event_for_session(
    message: &CodexAppServerMessage,
    started: &SessionSnapshot,
) -> ProviderEventDraft {
    let mut event =
        message
            .to_provider_event_draft()
            .into_provider_event_draft(ProviderEventContext::runtime(
                None,
                Some(started.thread_id),
                Some(started.session_id),
                "codex-app-server",
            ));
    if let Some(provider_event_id) = event.provider_event_id.take() {
        event.provider_event_id = Some(format!("{}:{provider_event_id}", started.session_id));
    }
    event
}

fn codex_response_id(message: &CodexAppServerMessage) -> Option<u64> {
    message.id.as_ref().and_then(|id| id.as_u64())
}

fn codex_response_error(message: &CodexAppServerMessage) -> Option<String> {
    let error = message.value.get("error")?;
    let message = error
        .get("message")
        .and_then(serde_json::Value::as_str)
        .or_else(|| error.as_str())
        .unwrap_or("Codex app-server request failed");
    Some(message.to_owned())
}

fn codex_turn_completed_status(message: &CodexAppServerMessage) -> Option<String> {
    for pointer in [
        "/params/status",
        "/params/turn/status",
        "/params/finalStatus",
        "/params/final_status",
        "/result/status",
        "/result/turn/status",
    ] {
        if let Some(status) = message
            .value
            .pointer(pointer)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|status| !status.is_empty())
        {
            return Some(status.to_owned());
        }
    }
    None
}

fn harness_turn_status_label(status: HarnessTurnStatus) -> &'static str {
    match status {
        HarnessTurnStatus::Success => "success",
        HarnessTurnStatus::Failed => "failed",
        HarnessTurnStatus::Interrupted => "interrupted",
        HarnessTurnStatus::Deferred => "deferred",
    }
}

fn codex_turn_id_from_message(message: &CodexAppServerMessage) -> Option<String> {
    for pointer in [
        "/params/turn/id",
        "/params/turnId",
        "/params/turn_id",
        "/result/turn/id",
        "/result/turnId",
        "/result/turn_id",
    ] {
        if let Some(turn_id) = message
            .value
            .pointer(pointer)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|turn_id| !turn_id.is_empty())
        {
            return Some(turn_id.to_owned());
        }
    }
    None
}

fn codex_thread_id_from_startup_response(
    message: &CodexAppServerMessage,
    startup_request_id: u64,
) -> Option<String> {
    (codex_response_id(message) == Some(startup_request_id))
        .then(|| {
            message
                .value
                .pointer("/result/thread/id")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
        .flatten()
}

fn codex_turn_effort(connection: &ProviderProcessConnection) -> Option<String> {
    connection
        .effort_mode
        .clone()
        .or_else(|| connection.reasoning_mode.clone())
}

fn codex_turn_start_params(
    connection: &ProviderProcessConnection,
    thread_id: &str,
    input: &str,
) -> CodexAppServerTurnStartParams {
    CodexAppServerTurnStartParams {
        thread_id: thread_id.to_owned(),
        input: vec![CodexAppServerUserInput::Text {
            text: input.to_owned(),
        }],
        cwd: Some(connection.cwd.clone()),
        approval_policy: connection.approval_policy.clone(),
        sandbox_policy: codex_sandbox_for_approval(connection.approval_policy.as_deref())
            .map(|_| json!({"type": "dangerFullAccess"})),
        model: connection.model.clone(),
        effort: codex_turn_effort(connection),
        summary: None,
        personality: connection.personality.clone(),
    }
}

fn persist_runtime_user_input(
    runtime_store: &RuntimeSessionStore,
    started: &SessionSnapshot,
    input: &str,
    visible_input: Option<&str>,
    kind: &ArchcarInputKind,
    sequence: u64,
) {
    let persisted_input = visible_input.unwrap_or(input);
    let (role, source) = match kind {
        ArchcarInputKind::User => ("user", "user_send"),
        ArchcarInputKind::ReviewPrompt => ("user", "staged_review_send"),
        ArchcarInputKind::ControlCommand => ("system", "control_command"),
        ArchcarInputKind::RawTerminal => ("system", "raw_terminal"),
    };
    let log_text = format_input_audit_log(
        &started.workspace,
        started.session_id,
        persisted_input,
        kind,
    );
    let _ = runtime_store.append_input_and_audit_log(
        started.thread_id,
        started.session_id,
        role,
        persisted_input,
        source,
        &log_text,
    );
    append_runtime_provider_event(
        runtime_store,
        runtime_provider_event(RuntimeProviderEventInput {
            kind: started.kind,
            session_id: started.session_id,
            thread_id: started.thread_id,
            identity_suffix: Some(&user_input_identity_suffix(sequence)),
            provider_sequence: Some(sequence),
            phase: ProviderEventPhase::Started,
            event_kind: ProviderEventKind::UserInput,
            subtype: source,
            title: match kind {
                ArchcarInputKind::User => "User input",
                ArchcarInputKind::ReviewPrompt => "Review prompt",
                ArchcarInputKind::ControlCommand => "Control command",
                ArchcarInputKind::RawTerminal => "Raw terminal input",
            },
            body: persisted_input,
        }),
        "provider_native_user_input",
    );
}

fn mark_snapshot_ready(snapshot: &Arc<Mutex<SessionSnapshot>>) {
    if let Ok(mut state) = snapshot.lock() {
        state.ready = true;
        state.runtime_state = AgentSessionState::WaitingForInput;
    }
}

#[allow(clippy::too_many_arguments)]
fn enqueue_provider_input(
    runtime_store: &RuntimeSessionStore,
    started: &SessionSnapshot,
    local_input_id: &str,
    input: &str,
    visible_input: Option<&str>,
    kind: &ArchcarInputKind,
    delivery: ArchcarInputDelivery,
    native_session_id: Option<String>,
) -> Result<()> {
    runtime_store.enqueue_provider_input(ProviderInputInput {
        id: local_input_id.to_owned(),
        provider: session_kind_provider_key(started.kind).to_owned(),
        thread_id: started.thread_id,
        process_id: started.session_id,
        native_session_id,
        input_kind: input_kind_storage_label(kind).to_owned(),
        delivery: delivery.as_str().to_owned(),
        provider_input: input.to_owned(),
        visible_input: visible_input.map(str::to_owned),
    })?;
    Ok(())
}

fn session_kind_provider_key(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Codex => "codex",
        SessionKind::Claude => "claude",
        SessionKind::Shell => "shell",
    }
}

fn input_kind_storage_label(kind: &ArchcarInputKind) -> &'static str {
    match kind {
        ArchcarInputKind::User => "user",
        ArchcarInputKind::ReviewPrompt => "review_prompt",
        ArchcarInputKind::ControlCommand => "control_command",
        ArchcarInputKind::RawTerminal => "raw_terminal",
    }
}

fn mark_snapshot_running_not_ready(snapshot: &Arc<Mutex<SessionSnapshot>>) {
    if let Ok(mut state) = snapshot.lock() {
        state.status = ProcessStatus::Running;
        state.ready = false;
        state.runtime_state = AgentSessionState::Running;
    }
}

fn mark_snapshot_exited(snapshot: &Arc<Mutex<SessionSnapshot>>, exit_code: Option<i32>) {
    if let Ok(mut state) = snapshot.lock() {
        state.status = ProcessStatus::Exited;
        let mut machine = SessionStateMachine::from_state(state.runtime_state);
        machine.mark_exited(exit_code);
        state.runtime_state = machine.state();
    }
}

fn mark_provider_session_failed(
    runtime_store: &RuntimeSessionStore,
    snapshot: &Arc<Mutex<SessionSnapshot>>,
    event_tx: &Sender<ArchcarEvent>,
    started: &SessionSnapshot,
    err: impl std::fmt::Display,
) {
    let error_body = err.to_string();
    append_runtime_provider_event(
        runtime_store,
        runtime_provider_event(RuntimeProviderEventInput {
            kind: started.kind,
            session_id: started.session_id,
            thread_id: started.thread_id,
            identity_suffix: None,
            provider_sequence: None,
            phase: ProviderEventPhase::Failed,
            event_kind: ProviderEventKind::ThreadSession,
            subtype: "session_error",
            title: "Session error",
            body: &error_body,
        }),
        "provider_native_session_error",
    );
    if let Ok(mut state) = snapshot.lock() {
        state.ready = false;
        state.runtime_state = AgentSessionState::Failed;
    }
    let _ = event_tx.send(ArchcarEvent::SessionError {
        session_id: Some(started.session_id),
        thread_id: Some(started.thread_id),
        message: error_body,
    });
}

fn next_turn_request_id(sequence: u64) -> u64 {
    sequence.saturating_add(10)
}

fn next_fallback_request_id(sequence: u64) -> u64 {
    sequence.saturating_add(2_000_000)
}

fn write_provider_json_line<W: Write>(writer: &mut W, value: &serde_json::Value) -> Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn terminal_process_alive(process_id: u32) -> bool {
    crate::platform::process_alive(process_id)
}

fn stop_reattached_process(process_id: u32) -> Result<()> {
    if !terminal_process_alive(process_id) {
        return Ok(());
    }
    request_process_stop(process_id);
    if wait_for_process_exit(process_id, Duration::from_secs(3)) {
        return Ok(());
    }
    force_process_stop(process_id);
    if wait_for_process_exit(process_id, Duration::from_millis(500)) {
        return Ok(());
    }
    hard_kill_process(process_id);
    if wait_for_process_exit(process_id, Duration::from_secs(1)) {
        return Ok(());
    }
    anyhow::bail!("process {process_id} remained alive after forced termination")
}

fn terminate_process_and_wait(process_id: u32) -> bool {
    terminate_process(process_id);
    if wait_for_process_exit(process_id, Duration::from_secs(1)) {
        return true;
    }
    hard_kill_process(process_id);
    wait_for_process_exit(process_id, Duration::from_secs(1))
}

fn wait_for_process_exit(process_id: u32, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if !terminal_process_alive(process_id) {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    !terminal_process_alive(process_id)
}

fn request_process_stop(process_id: u32) {
    if crate::platform::terminate_process_group(process_id, false).unwrap_or(false) {
        return;
    }
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .arg("-TERM")
            .arg(process_id.to_string())
            .status();
    }
    #[cfg(windows)]
    {
        let _ = crate::platform::terminate_process_tree(process_id, false);
    }
}

fn force_process_stop(process_id: u32) {
    if crate::platform::terminate_process_group(process_id, true).unwrap_or(false) {
        return;
    }
    hard_kill_process(process_id);
}

fn hard_kill_process(process_id: u32) {
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .arg("-KILL")
            .arg(process_id.to_string())
            .status();
    }
    #[cfg(windows)]
    {
        let _ = crate::platform::terminate_process_tree(process_id, true);
    }
}

pub(crate) fn terminate_process(process_id: u32) {
    force_process_stop(process_id);
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
    use crate::workspace::{CreateWorkspace, ProcessRecord};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn general_prompt_is_hidden_prefix_only_on_first_turn() {
        assert_eq!(
            compose_first_turn_input(Some("Keep changes focused."), "Fix auth", true),
            "Keep changes focused.\n\nFix auth"
        );
        assert_eq!(
            compose_first_turn_input(Some("Keep changes focused."), "Run tests", false),
            "Run tests"
        );
    }

    #[test]
    fn recovered_thread_context_is_hidden_prefix_for_provider_input() {
        assert_eq!(compose_recovered_thread_input(None, "continue"), "continue");
        assert_eq!(
            compose_recovered_thread_input(Some("prior context"), "continue"),
            "prior context\n\nCurrent user message:\ncontinue"
        );
    }

    #[test]
    fn provider_echoes_do_not_persist_as_visible_user_messages() {
        assert!(!should_persist_provider_native_event(
            ProviderEventKind::UserInput
        ));
        assert!(should_persist_provider_native_event(
            ProviderEventKind::AssistantOutput
        ));
    }

    #[cfg(unix)]
    #[test]
    fn reattached_stop_sends_sigterm_before_force_kill() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("term.marker");
        let script = format!(
            "trap 'echo term > {}; exit 0' TERM; while :; do sleep 1; done",
            marker.display()
        );
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let write = fs::OpenOptions::new()
            .write(true)
            .open("/dev/null")
            .unwrap();
        let mut session = ManagedSessionConnection::Reattached {
            write,
            output: Arc::new(Mutex::new(String::new())),
            read_cursor: 0,
            pid: child.id(),
        };

        thread::sleep(Duration::from_millis(100));
        session.stop().unwrap();
        let _ = child.wait();

        assert_eq!(fs::read_to_string(marker).unwrap(), "term\n");
    }

    #[cfg(unix)]
    #[test]
    fn raw_terminal_write_failure_emits_error_and_terminates_session() {
        let temp = tempfile::tempdir().unwrap();
        let store = seeded_workspace_store(temp.path());
        let thread = store
            .create_chat_thread("berlin", "shell", "Shell", None)
            .unwrap();
        let launch = store.session_launch("berlin", SessionKind::Shell).unwrap();
        let mut child = ProcessCommand::new("sleep").arg("30").spawn().unwrap();
        let process = store
            .record_session_process_for_thread("berlin", thread.id, &launch, child.id())
            .unwrap();
        let write = fs::OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .unwrap();
        let connection = ManagedSessionConnection::Reattached {
            write,
            output: Arc::new(Mutex::new(String::new())),
            read_cursor: 0,
            pid: child.id(),
        };
        let snapshot = Arc::new(Mutex::new(running_session_snapshot(
            process.id,
            thread.id,
            "berlin".to_owned(),
            SessionKind::Shell,
            child.id(),
            true,
        )));
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let db_path = temp.path().join("state.db");
        let logs_dir = temp.path().join("logs");
        let snapshot_for_loop = Arc::clone(&snapshot);
        let loop_thread = thread::spawn(move || {
            run_session_loop(
                db_path,
                logs_dir,
                snapshot_for_loop,
                controller_for_kind(SessionKind::Shell),
                connection,
                command_rx,
                event_tx,
            )
        });

        recv_archcar_event_until(
            &event_rx,
            |event| matches!(event, ArchcarEvent::SessionStarted { session_id, .. } if *session_id == process.id),
        );
        command_tx
            .send(SessionCommand::SendInput {
                input: "raw".to_owned(),
                visible_input: None,
                kind: ArchcarInputKind::RawTerminal,
                delivery: ArchcarInputDelivery::Immediate,
            })
            .unwrap();

        recv_archcar_event_until(
            &event_rx,
            |event| matches!(event, ArchcarEvent::SessionError { session_id, message, .. } if *session_id == Some(process.id) && message.contains("Raw terminal input failed")),
        );
        loop_thread.join().unwrap();
        let _ = child.wait();
        let exited = store.get_process_record(process.id).unwrap();
        assert_eq!(exited.status, ProcessStatus::Exited);
    }

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
                subtype: "user_send",
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
                subtype: "user_send",
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
    fn provider_input_local_ids_are_session_scoped() {
        assert_eq!(provider_input_local_id(7, 3), "session-7-input-3");
        assert_ne!(provider_input_local_id(1, 1), provider_input_local_id(2, 1));
    }

    #[test]
    fn fatal_harness_effect_persists_chat_visible_session_error() {
        let temp = tempfile::tempdir().unwrap();
        let store = seeded_workspace_store(temp.path());
        let thread = store
            .create_chat_thread("berlin", "claude", "Claude", None)
            .unwrap();
        let process = record_running_thread_session_with_port(&store, &thread, 43991);
        let db_path = temp.path().join("state.db");
        let runtime_store = RuntimeSessionStore::new(db_path.clone());
        let started = running_session_snapshot(
            process.id,
            thread.id,
            "berlin".to_owned(),
            SessionKind::Claude,
            process.pid,
            false,
        );
        let snapshot = Arc::new(Mutex::new(started.clone()));
        let (event_tx, event_rx) = mpsc::channel();
        let mut native_thread_id = None;

        apply_harness_effect(
            &runtime_store,
            &snapshot,
            &event_tx,
            &started,
            &mut native_thread_id,
            HarnessEffect::Fatal(
                "Claude authentication failed. You are not logged in. Run `claude auth login`."
                    .to_owned(),
            ),
        );

        let event = event_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(matches!(
            event,
            ArchcarEvent::SessionError { message, .. }
                if message.contains("not logged in")
        ));
        let records = crate::provider_events::ProviderEventStore::new(&db_path)
            .list_for_process(process.id)
            .unwrap();
        assert!(records.iter().any(|event| {
            event.kind == ProviderEventKind::ThreadSession
                && event.phase == ProviderEventPhase::Failed
                && event.provider_subtype.as_deref() == Some("session_error")
                && event
                    .normalized_payload
                    .get("body")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|body| body.contains("not logged in"))
        }));
        assert_eq!(
            snapshot.lock().unwrap().runtime_state,
            AgentSessionState::Failed
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
    fn managed_claude_runtime_uses_one_adapter_for_input_and_output_state() {
        let source = include_str!("session.rs");
        let loop_source = source
            .split_once("fn run_claude_stream_session_loop")
            .unwrap()
            .1
            .split_once("fn start_codex_app_server_lifecycle")
            .unwrap()
            .0;

        assert!(loop_source.contains("adapter.encode_input(HarnessInput"));
        assert!(loop_source.contains("adapter.observe_native(NativeRecord"));
        assert!(loop_source.contains("adapter.settle_failed_input_write"));
        assert!(!loop_source.contains("ClaudeStreamParser"));
    }

    #[test]
    fn managed_claude_init_after_first_input_does_not_restore_readiness() {
        let temp = tempfile::tempdir().unwrap();
        let store = seeded_workspace_store(temp.path());
        let thread = store
            .create_chat_thread("berlin", "claude", "Claude", None)
            .unwrap();
        let mut child = ProcessCommand::new("bash")
            .args(["-lc", "cat >/dev/null"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        let stdin = child.stdin.take().unwrap();
        let process = record_thread_session_with_port_and_pid(&store, &thread, 43991, pid);
        let (native_tx, native_rx) = mpsc::channel();
        let connection = ProviderProcessConnection {
            child,
            stdin,
            stdout_rx: native_rx,
            next_read_line: 0,
            native_thread_id: None,
            cwd: PathBuf::from("/tmp/workspace"),
            model: Some("claude-sonnet-fixture".to_owned()),
            approval_policy: None,
            reasoning_mode: None,
            effort_mode: None,
            personality: None,
            pending_recovery_context: None,
        };
        let snapshot = Arc::new(Mutex::new(running_session_snapshot(
            process.id,
            thread.id,
            "berlin".to_owned(),
            SessionKind::Claude,
            pid,
            false,
        )));
        let snapshot_for_loop = Arc::clone(&snapshot);
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let db_path = temp.path().join("state.db");
        let loop_thread = thread::spawn(move || {
            run_claude_stream_session_loop(
                db_path,
                snapshot_for_loop,
                connection,
                command_rx,
                event_tx,
            )
        });

        recv_archcar_event_until(
            &event_rx,
            |event| matches!(event, ArchcarEvent::SessionStarted { session_id, .. } if *session_id == process.id),
        );
        assert!(!snapshot.lock().unwrap().ready);

        command_tx
            .send(SessionCommand::SendInput {
                input: "fixture input".to_owned(),
                visible_input: None,
                kind: ArchcarInputKind::User,
                delivery: ArchcarInputDelivery::Auto,
            })
            .unwrap();
        wait_for_snapshot_readiness(&snapshot, false);

        let fixture = include_str!("../../tests/fixtures/claude_stream/basic_turn.jsonl");
        let init = fixture
            .lines()
            .find(|line| {
                serde_json::from_str::<serde_json::Value>(line)
                    .is_ok_and(|record| record["subtype"] == "init")
            })
            .unwrap();
        native_tx.send(init.to_owned()).unwrap();
        recv_archcar_event_until(
            &event_rx,
            |event| matches!(event, ArchcarEvent::SessionMessagesUpdated { thread_id } if *thread_id == thread.id),
        );
        let ready_after_init = snapshot.lock().unwrap().ready;

        let result = fixture
            .lines()
            .find(|line| {
                serde_json::from_str::<serde_json::Value>(line)
                    .is_ok_and(|record| record["type"] == "result")
            })
            .unwrap();
        native_tx.send(result.to_owned()).unwrap();
        recv_archcar_event_until(
            &event_rx,
            |event| matches!(event, ArchcarEvent::SessionMessagesUpdated { thread_id } if *thread_id == thread.id),
        );
        wait_for_snapshot_readiness(&snapshot, true);

        command_tx.send(SessionCommand::Kill).unwrap();
        loop_thread.join().unwrap();

        assert!(
            !ready_after_init,
            "startup init must not restore readiness after the first input"
        );
    }

    #[test]
    fn managed_claude_lifecycle_waits_for_init_and_completes_turn_once() {
        let temp = tempfile::tempdir().unwrap();
        let fake_claude = temp.path().join("claude");
        fs::write(
            &fake_claude,
            r#"#!/usr/bin/env bash
printf '%s\n' '{"type":"system","subtype":"hook_started","session_id":"fake-session","hook":"startup"}'
while [ ! -f "$ARCHDUCTOR_TEST_INIT_GATE" ]; do sleep 0.02; done
printf '%s\n' '{"type":"system","subtype":"init","session_id":"fake-session","model":"claude-sonnet-fixture","capabilities":["streaming"]}'
IFS= read -r _line
printf '%s\n' '{"type":"user","session_id":"fake-session","isReplay":true,"message":{"role":"user","content":[{"type":"text","text":"hello lifecycle"}]}}'
printf '%s\n' '{"type":"assistant","session_id":"fake-session","message":{"id":"m1","role":"assistant","content":[{"type":"text","text":"done"}]}}'
printf '%s\n' '{"type":"result","subtype":"success","session_id":"fake-session","result":"ok","duration_ms":1}'
"#,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&fake_claude).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&fake_claude, perms).unwrap();
        }

        let init_gate = temp.path().join("allow-claude-init");
        let store = seeded_workspace_store(temp.path());
        let thread = store
            .create_chat_thread("berlin", "claude", "Claude", None)
            .unwrap();
        let mut child = ProcessCommand::new(&fake_claude)
            .env("ARCHDUCTOR_TEST_INIT_GATE", &init_gate)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let process = record_thread_session_with_port_and_pid(&store, &thread, 43992, pid);
        let connection = ProviderProcessConnection {
            child,
            stdin,
            stdout_rx: spawn_stdout_line_reader(stdout),
            next_read_line: 0,
            native_thread_id: None,
            cwd: PathBuf::from("/tmp/workspace"),
            model: Some("claude-sonnet-fixture".to_owned()),
            approval_policy: None,
            reasoning_mode: None,
            effort_mode: None,
            personality: None,
            pending_recovery_context: None,
        };
        let snapshot = Arc::new(Mutex::new(running_session_snapshot(
            process.id,
            thread.id,
            "berlin".to_owned(),
            SessionKind::Claude,
            pid,
            false,
        )));
        let snapshot_for_loop = Arc::clone(&snapshot);
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let db_path = temp.path().join("state.db");
        let loop_thread = thread::spawn(move || {
            run_claude_stream_session_loop(
                db_path,
                snapshot_for_loop,
                connection,
                command_rx,
                event_tx,
            )
        });

        recv_archcar_event_until(
            &event_rx,
            |event| matches!(event, ArchcarEvent::SessionStarted { session_id, .. } if *session_id == process.id),
        );
        let no_ready_deadline = std::time::Instant::now() + Duration::from_millis(180);
        while std::time::Instant::now() < no_ready_deadline {
            if let Ok(event) = event_rx.recv_timeout(Duration::from_millis(20)) {
                assert!(
                    !matches!(event, ArchcarEvent::SessionReady { session_id, .. } if session_id == process.id),
                    "managed Claude must not become ready before system/init: {event:?}"
                );
            }
        }
        assert!(!snapshot.lock().unwrap().ready);
        fs::write(&init_gate, b"ready").unwrap();

        recv_archcar_event_until(
            &event_rx,
            |event| matches!(event, ArchcarEvent::SessionReady { session_id, .. } if *session_id == process.id),
        );
        assert!(snapshot.lock().unwrap().ready);

        command_tx
            .send(SessionCommand::SendInput {
                input: "hello lifecycle".to_owned(),
                visible_input: None,
                kind: ArchcarInputKind::User,
                delivery: ArchcarInputDelivery::Auto,
            })
            .unwrap();
        wait_for_snapshot_readiness(&snapshot, false);

        let mut completed_count = 0;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let event = event_rx
                .recv_timeout(remaining)
                .expect("timed out waiting for managed Claude result");
            match event {
                ArchcarEvent::TurnCompleted {
                    session_id,
                    thread_id,
                    status,
                } if session_id == process.id && thread_id == thread.id => {
                    completed_count += 1;
                    assert_eq!(status.as_deref(), Some("success"));
                }
                ArchcarEvent::SessionExited { session_id, .. } if session_id == process.id => {
                    break;
                }
                _ => {}
            }
        }
        loop_thread.join().unwrap();

        assert_eq!(
            completed_count, 1,
            "managed Claude must emit exactly one turn_completed for one delivered local input"
        );
        assert!(snapshot.lock().unwrap().ready);
    }

    #[test]
    fn codex_process_lifecycle_is_ready_without_screen_semantic_detection() {
        assert!(process_lifecycle_ready(SessionKind::Codex));
        assert!(process_lifecycle_ready(SessionKind::Shell));
        assert!(!process_lifecycle_ready(SessionKind::Claude));
    }

    #[test]
    fn managed_codex_sessions_launch_app_server_not_terminal_ui() {
        let temp = tempfile::tempdir().unwrap();
        let store = seeded_workspace_store(temp.path());
        let thread = store
            .create_chat_thread("berlin", "codex", "Codex", None)
            .unwrap();

        let ThreadSessionLaunch {
            launch,
            provider_port_reservation: _reservation,
            startup_recovery_context: _,
        } = codex_app_server_session_launch(
            &store,
            "berlin",
            &thread,
            SessionHarnessOptions {
                model: Some("gpt-5.4".to_owned()),
                approval_mode: Some("never".to_owned()),
                reasoning_mode: Some("high".to_owned()),
                ..SessionHarnessOptions::default()
            },
        )
        .unwrap();

        assert_eq!(launch.program, PathBuf::from("codex"));
        assert_eq!(launch.args, vec!["app-server"]);
        assert_eq!(launch.env_value("ARCHDUCTOR_PORT"), Some("42000"));
        let port = launch.env_value(PROVIDER_NATIVE_PORT_ENV).unwrap();
        let expected_metadata = format!(
            "harness=codex-app-server;model=gpt-5.4;approval=never;reasoning=high;port={port}"
        );
        assert_eq!(
            launch.harness_metadata.as_deref(),
            Some(expected_metadata.as_str())
        );
        assert!(launch.session_resume_id.is_none());
    }

    #[test]
    fn codex_app_server_lifecycle_waits_for_initialize_response_before_thread_start() {
        let mut child = ProcessCommand::new("bash")
            .args(["-lc", "cat"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stdout_rx = spawn_stdout_line_reader(stdout);
        let mut connection = ProviderProcessConnection {
            child,
            stdin,
            stdout_rx,
            next_read_line: 0,
            native_thread_id: None,
            cwd: PathBuf::from("/tmp/workspace"),
            model: Some("gpt-5.4".to_owned()),
            approval_policy: Some("never".to_owned()),
            reasoning_mode: None,
            effort_mode: None,
            personality: None,
            pending_recovery_context: None,
        };
        let snapshot =
            running_session_snapshot(7, 11, "berlin".to_owned(), SessionKind::Codex, 123, false);

        start_codex_app_server_lifecycle(&mut connection, &snapshot).unwrap();

        let first = connection
            .stdout_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        let first: serde_json::Value = serde_json::from_str(&first).unwrap();

        assert_eq!(first["method"], "initialize");
        assert_eq!(first["id"], 1);
        assert!(first.get("jsonrpc").is_none());
        assert!(connection
            .stdout_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err());

        continue_codex_app_server_startup_after_initialize(&mut connection).unwrap();

        let second = connection
            .stdout_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        let third = connection
            .stdout_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        let second: serde_json::Value = serde_json::from_str(&second).unwrap();
        let third: serde_json::Value = serde_json::from_str(&third).unwrap();

        assert_eq!(second, json!({"method": "initialized", "params": {}}));
        assert_eq!(third["method"], "thread/start");
        assert_eq!(third["id"], 2);
        assert_eq!(third["params"]["cwd"], "/tmp/workspace");
        assert_eq!(third["params"]["model"], "gpt-5.4");
        assert_eq!(third["params"]["approvalPolicy"], "never");
        assert_eq!(third["params"]["sandbox"], "danger-full-access");

        let _ = connection.child.kill();
    }

    #[test]
    fn codex_app_server_lifecycle_resumes_existing_native_thread() {
        let mut child = ProcessCommand::new("bash")
            .args(["-lc", "cat"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stdout_rx = spawn_stdout_line_reader(stdout);
        let mut connection = ProviderProcessConnection {
            child,
            stdin,
            stdout_rx,
            next_read_line: 0,
            native_thread_id: Some("thr_existing".to_owned()),
            cwd: PathBuf::from("/tmp/workspace"),
            model: None,
            approval_policy: None,
            reasoning_mode: None,
            effort_mode: None,
            personality: None,
            pending_recovery_context: None,
        };
        let snapshot =
            running_session_snapshot(7, 11, "berlin".to_owned(), SessionKind::Codex, 123, false);

        start_codex_app_server_lifecycle(&mut connection, &snapshot).unwrap();

        let first = connection
            .stdout_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        let first: serde_json::Value = serde_json::from_str(&first).unwrap();

        assert_eq!(first["method"], "initialize");
        assert!(connection
            .stdout_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err());

        continue_codex_app_server_startup_after_initialize(&mut connection).unwrap();

        let second = connection
            .stdout_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        let third = connection
            .stdout_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        let second: serde_json::Value = serde_json::from_str(&second).unwrap();
        let third: serde_json::Value = serde_json::from_str(&third).unwrap();

        assert_eq!(second, json!({"method": "initialized", "params": {}}));
        assert_eq!(third["method"], "thread/resume");
        assert_eq!(third["id"], 2);
        assert_eq!(third["params"]["threadId"], "thr_existing");
        assert_eq!(third["params"]["cwd"], "/tmp/workspace");
        assert_eq!(third["params"]["serviceName"], "archductor");
        assert!(third["params"].get("approvalPolicy").is_none());
        assert!(third["params"].get("sandbox").is_none());

        let _ = connection.child.kill();
    }

    #[test]
    fn codex_response_event_ids_are_scoped_to_runtime_session() {
        let temp = tempfile::tempdir().unwrap();
        let store = seeded_workspace_store(temp.path());
        let thread = store
            .create_chat_thread("berlin", "codex", "Codex", None)
            .unwrap();
        let db_path = temp.path().join("state.db");
        let message = parse_jsonl_message(r#"{"id":2,"result":{"ok":true}}"#, 1).unwrap();
        let first_process = record_thread_session_with_port_and_pid(&store, &thread, 43000, 123);
        let second_process = record_thread_session_with_port_and_pid(&store, &thread, 43001, 456);
        let first = running_session_snapshot(
            first_process.id,
            thread.id,
            "berlin".to_owned(),
            SessionKind::Codex,
            123,
            false,
        );
        let second = running_session_snapshot(
            second_process.id,
            thread.id,
            "berlin".to_owned(),
            SessionKind::Codex,
            456,
            false,
        );

        let event_store = crate::provider_events::ProviderEventStore::new(db_path);
        event_store
            .upsert_event(&codex_app_server_provider_event_for_session(
                &message, &first,
            ))
            .unwrap();
        event_store
            .upsert_event(&codex_app_server_provider_event_for_session(
                &message, &second,
            ))
            .unwrap();

        let events = event_store.list_for_chat_thread(thread.id).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].provider_event_id.as_deref(),
            Some(format!("{}:2", first_process.id).as_str())
        );
        assert_eq!(
            events[1].provider_event_id.as_deref(),
            Some(format!("{}:2", second_process.id).as_str())
        );
    }

    #[test]
    fn codex_turn_completed_status_reads_final_status_payload() {
        let message = parse_jsonl_message(
            r#"{"method":"turn/completed","params":{"turn":{"id":"turn-1","status":"cancelled"}}}"#,
            1,
        )
        .unwrap();

        assert_eq!(
            codex_turn_completed_status(&message).as_deref(),
            Some("cancelled")
        );
    }

    #[test]
    fn codex_model_update_changes_future_turn_model() {
        let mut child = ProcessCommand::new("bash")
            .args(["-lc", "cat"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stdout_rx = spawn_stdout_line_reader(stdout);
        let mut connection = ProviderProcessConnection {
            child,
            stdin,
            stdout_rx,
            next_read_line: 0,
            native_thread_id: Some("thr_existing".to_owned()),
            cwd: PathBuf::from("/tmp/workspace"),
            model: Some("gpt-5.6-sol".to_owned()),
            approval_policy: None,
            reasoning_mode: None,
            effort_mode: None,
            personality: None,
            pending_recovery_context: None,
        };

        apply_provider_connection_controls(
            &mut connection,
            DesiredHarnessControls {
                model: Some(" gpt-5.6-terra ".to_owned()),
                ..DesiredHarnessControls::default()
            },
        );

        assert_eq!(connection.model.as_deref(), Some("gpt-5.6-terra"));

        apply_provider_connection_controls(
            &mut connection,
            DesiredHarnessControls {
                model: Some("   ".to_owned()),
                ..DesiredHarnessControls::default()
            },
        );

        assert_eq!(connection.model, None);

        let _ = connection.child.kill();
    }

    #[test]
    fn codex_sandbox_always_forces_bypass_approval() {
        assert_eq!(
            approval_from_harness_metadata(None),
            Some("never".to_owned())
        );
        assert_eq!(codex_sandbox_for_approval(None), Some("danger-full-access"));
        assert_eq!(
            codex_sandbox_for_approval(Some("on-request")),
            Some("danger-full-access")
        );
        assert_eq!(
            codex_sandbox_for_approval(Some("never")),
            Some("danger-full-access")
        );
    }

    #[test]
    fn managed_claude_sessions_launch_stream_json_not_interactive_ui() {
        let temp = tempfile::tempdir().unwrap();
        let store = seeded_workspace_store(temp.path());
        let thread = store
            .create_chat_thread("berlin", "claude", "Claude", None)
            .unwrap();
        let thread = store
            .update_chat_thread_native_id(thread.id, "claude-session-1")
            .unwrap();

        let ThreadSessionLaunch {
            launch,
            provider_port_reservation: _reservation,
            startup_recovery_context: _,
        } = claude_stream_session_launch(
            &store,
            "berlin",
            &thread,
            SessionHarnessOptions {
                model: Some("claude-sonnet-5".to_owned()),
                approval_mode: Some("never".to_owned()),
                reasoning_mode: Some("low".to_owned()),
                ..SessionHarnessOptions::default()
            },
        )
        .unwrap();

        assert_eq!(launch.program, PathBuf::from("claude"));
        assert!(launch
            .args
            .windows(2)
            .any(|args| args == ["--resume", "claude-session-1"]));
        assert!(launch
            .args
            .windows(2)
            .any(|args| args == ["--permission-mode", "bypassPermissions"]));
        assert!(launch
            .args
            .windows(2)
            .any(|args| args == ["--model", "claude-sonnet-5"]));
        assert!(launch
            .args
            .windows(2)
            .any(|args| args == ["--effort", "low"]));
        let settings = launch
            .args
            .windows(2)
            .find_map(|args| (args[0] == "--settings").then_some(args[1].as_str()))
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(settings).unwrap()["hooks"]
                ["PermissionRequest"][0]["matcher"],
            ".*"
        );
        assert!(!launch.args.iter().any(|arg| arg == "--bare"));
        assert_eq!(launch.env_value("ARCHDUCTOR_PORT"), Some("42000"));
        let port = launch.env_value(PROVIDER_NATIVE_PORT_ENV).unwrap();
        let expected_metadata = format!(
            "harness=claude-stream-json;model=claude-sonnet-5;approval=never;reasoning=low;port={port}"
        );
        assert_eq!(
            launch.harness_metadata.as_deref(),
            Some(expected_metadata.as_str())
        );
        assert_eq!(
            launch.session_resume_id.as_deref(),
            Some("claude-session-1")
        );
    }

    #[test]
    fn provider_native_thread_launches_continue_past_initial_port_block() {
        let temp = tempfile::tempdir().unwrap();
        let store = seeded_workspace_store(temp.path());
        let threads = (0..10)
            .map(|index| {
                let provider = if index % 2 == 0 { "codex" } else { "claude" };
                store
                    .create_chat_thread("berlin", provider, &format!("Chat {index}"), None)
                    .unwrap()
            })
            .collect::<Vec<_>>();

        let ports = threads
            .iter()
            .map(|thread| {
                let port =
                    provider_native_thread_port_with_checker(&store, "berlin", thread.id, |_| true)
                        .unwrap();
                record_running_thread_session_with_port(&store, thread, port);
                port.to_string()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            ports,
            vec![
                "43000", "43001", "43002", "43003", "43004", "43005", "43006", "43007", "43008",
                "43009"
            ]
        );

        let eleventh = store
            .create_chat_thread("berlin", "codex", "Chat 10", None)
            .unwrap();
        assert_eq!(
            provider_native_thread_port_with_checker(&store, "berlin", eleventh.id, |_| true)
                .unwrap(),
            43010
        );
    }

    #[test]
    fn provider_native_thread_ports_start_above_common_dev_ports() {
        let temp = tempfile::tempdir().unwrap();
        let store = seeded_workspace_store(temp.path());
        let thread = store
            .create_chat_thread("berlin", "codex", "Codex", None)
            .unwrap();

        let port = provider_native_thread_port_with_checker(&store, "berlin", thread.id, |_| true)
            .unwrap();

        assert!(port > 8080);
        assert!(![3000, 5173, 8080].contains(&port));
    }

    #[test]
    fn provider_native_thread_launch_reserves_port_until_process_recorded() {
        let temp = tempfile::tempdir().unwrap();
        let store = seeded_workspace_store(temp.path());
        let first = store
            .create_chat_thread("berlin", "codex", "First", None)
            .unwrap();
        let second = store
            .create_chat_thread("berlin", "claude", "Second", None)
            .unwrap();

        let reservation = reserve_provider_native_thread_port(&store, "berlin", first.id).unwrap();
        assert_eq!(reservation.local_addr().unwrap().port(), 43000);

        let second_reservation =
            reserve_provider_native_thread_port(&store, "berlin", second.id).unwrap();

        let second_port = second_reservation.local_addr().unwrap().port();
        assert_ne!(second_port, reservation.local_addr().unwrap().port());
        assert!(second_port >= 43001);
    }

    #[test]
    fn provider_native_thread_launch_reuses_released_reserved_port() {
        let temp = tempfile::tempdir().unwrap();
        let store = seeded_workspace_store(temp.path());
        let first = store
            .create_chat_thread("berlin", "codex", "First", None)
            .unwrap();
        let second = store
            .create_chat_thread("berlin", "codex", "Second", None)
            .unwrap();
        let first_process = record_running_thread_session_with_port(&store, &first, 43000);
        record_running_thread_session_with_port(&store, &second, 43001);
        store
            .mark_session_process_exited(first_process.id, Some(0))
            .unwrap();
        store.close_chat_thread(first.id).unwrap();
        let replacement = store
            .create_chat_thread("berlin", "claude", "Replacement", None)
            .unwrap();

        assert_eq!(
            provider_native_thread_port_with_checker(&store, "berlin", replacement.id, |_| true)
                .unwrap(),
            43000
        );
    }

    #[test]
    fn provider_native_thread_launch_reuses_stale_reserved_port_after_archcar_restart() {
        let temp = tempfile::tempdir().unwrap();
        let store = seeded_workspace_store(temp.path());
        let stale = store
            .create_chat_thread("berlin", "codex", "Stale", None)
            .unwrap();
        record_thread_session_with_port_and_pid(&store, &stale, 43000, exited_child_pid());
        let replacement = store
            .create_chat_thread("berlin", "claude", "Replacement", None)
            .unwrap();

        assert_eq!(
            provider_native_thread_port_with_checker(&store, "berlin", replacement.id, |_| true)
                .unwrap(),
            43000
        );
    }

    #[test]
    fn provider_native_thread_launch_uses_lowest_free_port_across_workspaces() {
        let temp = tempfile::tempdir().unwrap();
        let store = seeded_workspace_store(temp.path());
        let berlin = store
            .create_chat_thread("berlin", "codex", "Berlin", None)
            .unwrap();
        record_running_thread_session_with_port(&store, &berlin, 43000);
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "tokyo".to_owned(),
                branch: "lc/tokyo".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let tokyo = store
            .create_chat_thread("tokyo", "claude", "Tokyo", None)
            .unwrap();

        assert_eq!(
            provider_native_thread_port_with_checker(&store, "tokyo", tokyo.id, |_| true).unwrap(),
            43001
        );
    }

    #[test]
    fn provider_native_thread_launch_skips_unavailable_reserved_ports() {
        let temp = tempfile::tempdir().unwrap();
        let store = seeded_workspace_store(temp.path());
        let first = store
            .create_chat_thread("berlin", "codex", "First", None)
            .unwrap();
        record_running_thread_session_with_port(&store, &first, 43000);
        let thread = store
            .create_chat_thread("berlin", "codex", "Codex", None)
            .unwrap();

        assert_eq!(
            provider_native_thread_port_with_checker(&store, "berlin", thread.id, |port| {
                port != 43001
            })
            .unwrap(),
            43002
        );
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
    fn codex_screen_readiness_requires_busy_transition_after_input() {
        let ready_screen = "\
› Follow up

  gpt-5.6-sol medium · ~/archductor/workspaces/demo";
        let working_screen = "\
› Follow up

• Working (12s • esc to interrupt)

  gpt-5.6-sol medium · ~/archductor/workspaces/demo";

        assert!(!session_ready_for_visible_screen_after_busy(
            SessionKind::Codex,
            ready_screen,
            false
        ));
        assert!(!session_ready_for_visible_screen_after_busy(
            SessionKind::Codex,
            working_screen,
            true
        ));
        assert!(session_ready_for_visible_screen_after_busy(
            SessionKind::Codex,
            ready_screen,
            true
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

        let ThreadSessionLaunch { launch, .. } = build_thread_session_launch(
            &store,
            "berlin",
            &store
                .create_chat_thread("berlin", "codex", "Codex", None)
                .unwrap(),
            SessionKind::Codex,
            SessionHarnessOptions::default(),
            controller.as_ref(),
        )
        .unwrap();

        assert_eq!(launch.args, vec!["app-server"]);
        assert!(launch.session_resume_id.is_none());
    }

    #[test]
    fn codex_thread_launch_with_native_id_resumes_that_session() {
        let _guard = env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let store = seeded_workspace_store(temp.path());
        let controller = controller_for_kind(SessionKind::Codex);
        let fake_home = temp.path().join("home");
        let rollout_dir = fake_home.join(".codex/sessions/2026/07/18");
        fs::create_dir_all(&rollout_dir).unwrap();
        fs::write(
            rollout_dir.join("rollout-valid.jsonl"),
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"session_id\":\"codex-native-thread\",\"cwd\":\"{}\"}}}}\n",
                store
                    .list()
                    .unwrap()
                    .into_iter()
                    .find(|workspace| workspace.name == "berlin")
                    .unwrap()
                    .path
                    .display()
            ),
        )
        .unwrap();
        let previous_home = std::env::var_os("HOME");
        std::env::set_var("HOME", &fake_home);

        let ThreadSessionLaunch { launch, .. } = build_thread_session_launch(
            &store,
            "berlin",
            &store
                .create_chat_thread("berlin", "codex", "Codex", None)
                .and_then(|thread| {
                    store.update_chat_thread_native_id(thread.id, "codex-native-thread")
                })
                .unwrap(),
            SessionKind::Codex,
            SessionHarnessOptions::default(),
            controller.as_ref(),
        )
        .unwrap();

        assert_eq!(launch.args, vec!["app-server"]);
        assert_eq!(
            launch.session_resume_id.as_deref(),
            Some("codex-native-thread")
        );

        if let Some(previous) = previous_home {
            std::env::set_var("HOME", previous);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn codex_thread_launch_clears_native_id_when_rollout_is_missing() {
        let _guard = env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let store = seeded_workspace_store(temp.path());
        let controller = controller_for_kind(SessionKind::Codex);
        let fake_home = temp.path().join("home");
        fs::create_dir_all(fake_home.join(".codex/sessions")).unwrap();
        let previous_home = std::env::var_os("HOME");
        std::env::set_var("HOME", &fake_home);
        let thread = store
            .create_chat_thread("berlin", "codex", "Codex", None)
            .and_then(|thread| store.update_chat_thread_native_id(thread.id, "missing-rollout"))
            .unwrap();
        store
            .append_chat_message(thread.id, "user", "continue the fix", "user_send")
            .unwrap();

        let ThreadSessionLaunch {
            launch,
            startup_recovery_context,
            ..
        } = build_thread_session_launch(
            &store,
            "berlin",
            &thread,
            SessionKind::Codex,
            SessionHarnessOptions::default(),
            controller.as_ref(),
        )
        .unwrap();

        assert_eq!(launch.args, vec!["app-server"]);
        assert!(launch.session_resume_id.is_none());
        let recovery_context = startup_recovery_context.unwrap();
        assert!(recovery_context.contains("Missing Codex rollout thread id: missing-rollout"));
        assert!(recovery_context.contains("- user: continue the fix"));
        assert!(store
            .get_chat_thread_record(thread.id)
            .unwrap()
            .native_thread_id
            .is_none());

        if let Some(previous) = previous_home {
            std::env::set_var("HOME", previous);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn managed_session_store_uses_app_shared_provider_settings() {
        let _guard = env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        drop(seeded_workspace_store(temp.path()));
        let config_home = temp.path().join("xdg/config");
        let settings_path = config_home.join("archductor/settings.toml");
        fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        fs::write(
            settings_path,
            "codex_executable_path = \"/shared/bin/codex\"\n",
        )
        .unwrap();
        let previous_config_home = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", &config_home);

        let store =
            open_managed_session_store(temp.path().join("state.db"), temp.path().join("logs"))
                .unwrap();

        if let Some(previous) = previous_config_home {
            std::env::set_var("XDG_CONFIG_HOME", previous);
        } else {
            std::env::remove_var("XDG_CONFIG_HOME");
        }
        assert_eq!(
            store
                .session_launch("berlin", SessionKind::Codex)
                .unwrap()
                .program,
            PathBuf::from("/shared/bin/codex")
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
    fn restore_managed_session_terminates_provider_native_records_instead_of_pty_restore() {
        let temp = tempfile::tempdir().unwrap();
        let store = seeded_workspace_store(temp.path());
        let thread = store
            .create_chat_thread("berlin", "codex", "Codex", None)
            .unwrap();
        let mut launch = store.session_launch("berlin", SessionKind::Codex).unwrap();
        launch.harness_metadata = Some("harness=codex-app-server;port=43000".to_owned());
        let mut child = ProcessCommand::new("sleep").arg("30").spawn().unwrap();
        let process = store
            .record_session_process_for_thread("berlin", thread.id, &launch, child.id())
            .unwrap();
        let (event_tx, _event_rx) = mpsc::channel();

        let restored = restore_managed_session(
            temp.path().join("state.db"),
            temp.path().join("logs"),
            process.id,
            event_tx,
        )
        .unwrap();

        assert!(restored.is_none());
        let exited = store.get_process_record(process.id).unwrap();
        assert_eq!(exited.status, ProcessStatus::Exited);
        let _ = child.wait();
    }

    #[cfg(unix)]
    #[test]
    fn restore_managed_session_terminates_shell_records_instead_of_pty_restore() {
        let temp = tempfile::tempdir().unwrap();
        let store = seeded_workspace_store(temp.path());
        let thread = store
            .create_chat_thread("berlin", "shell", "Shell", None)
            .unwrap();
        let launch = store.session_launch("berlin", SessionKind::Shell).unwrap();
        let mut child = ProcessCommand::new("/bin/sh")
            .args(["-c", "while :; do sleep 1; done"])
            .spawn()
            .unwrap();
        let process = store
            .record_session_process_for_thread("berlin", thread.id, &launch, child.id())
            .unwrap();
        let (event_tx, _event_rx) = mpsc::channel();

        let restored = restore_managed_session(
            temp.path().join("state.db"),
            temp.path().join("logs"),
            process.id,
            event_tx,
        )
        .unwrap();

        assert!(restored.is_none());
        assert!(wait_for_process_exit(child.id(), Duration::from_secs(5)));
        let _ = child.wait();
        let exited = store.get_process_record(process.id).unwrap();
        assert_eq!(exited.status, ProcessStatus::Exited);
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

    fn recv_archcar_event_until(
        receiver: &Receiver<ArchcarEvent>,
        predicate: impl Fn(&ArchcarEvent) -> bool,
    ) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let event = receiver
                .recv_timeout(remaining)
                .expect("timed out waiting for archcar event");
            if predicate(&event) {
                return;
            }
        }
    }

    fn wait_for_snapshot_readiness(snapshot: &Arc<Mutex<SessionSnapshot>>, expected: bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if snapshot.lock().unwrap().ready == expected {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for snapshot readiness={expected}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn record_running_thread_session_with_port(
        store: &WorkspaceStore,
        thread: &ChatThreadRecord,
        port: u16,
    ) -> ProcessRecord {
        record_thread_session_with_port_and_pid(store, thread, port, std::process::id())
    }

    fn record_thread_session_with_port_and_pid(
        store: &WorkspaceStore,
        thread: &ChatThreadRecord,
        port: u16,
        pid: u32,
    ) -> ProcessRecord {
        let kind = match thread.provider.as_str() {
            "codex" => SessionKind::Codex,
            "claude" => SessionKind::Claude,
            other => panic!("unexpected provider {other}"),
        };
        let mut launch = store.session_launch("berlin", kind).unwrap();
        launch.harness_metadata = Some(format!(
            "harness={};port={port}",
            match kind {
                SessionKind::Codex => "codex-app-server",
                SessionKind::Claude => "claude-stream-json",
                SessionKind::Shell => "shell",
            }
        ));
        store
            .record_session_process_for_thread("berlin", thread.id, &launch, pid)
            .unwrap()
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

    #[test]
    fn codex_immediate_input_routes_to_active_turn_or_new_turn() {
        assert_eq!(
            codex_input_route(ArchcarInputDelivery::Immediate, Some("turn-7"), false),
            CodexInputRoute::Steer {
                expected_turn_id: "turn-7".to_owned(),
            }
        );
        assert_eq!(
            codex_input_route(ArchcarInputDelivery::Immediate, None, false),
            CodexInputRoute::Start
        );
    }

    #[test]
    fn codex_immediate_steer_error_retries_once_as_start() {
        let immediate = PendingCodexInputRequest {
            kind: CodexInputRequestKind::ImmediateSteer,
            local_input_id: "input-1".to_owned(),
            input: "adjust course".to_owned(),
        };
        assert_eq!(
            codex_input_response_action(&immediate, true),
            CodexInputResponseAction::RetryStart
        );

        let fallback = PendingCodexInputRequest {
            kind: CodexInputRequestKind::ImmediateFallbackStart,
            local_input_id: "input-1".to_owned(),
            input: "adjust course".to_owned(),
        };
        assert_eq!(
            codex_input_response_action(&fallback, true),
            CodexInputResponseAction::ReportError
        );
        assert_eq!(
            codex_input_response_action(&fallback, false),
            CodexInputResponseAction::Complete
        );
    }
}
