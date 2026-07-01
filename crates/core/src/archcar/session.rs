use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use tracing::{info, warn};

use crate::archcar::harness::{controller_for_kind, ensure_thread_for_kind, HarnessController};
use crate::archcar::protocol::{ArchcarEvent, ArchcarInputKind};
use crate::codex_tui::ScreenMessage;
use crate::pty::PtySession;
use crate::workspace::{
    format_codex_raw_output, format_codex_screen_snapshot, ProcessStatus, SessionHarnessOptions,
    SessionKind, WorkspaceStore,
};

#[derive(Debug)]
pub enum SessionCommand {
    SendInput {
        input: String,
        kind: ArchcarInputKind,
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
        let snapshot = Arc::new(Mutex::new(snapshot_state));
        let (command_tx, command_rx) = mpsc::channel();
        let snapshot_for_thread = snapshot.clone();
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
        return Ok(SessionHandle {
            snapshot,
            command_tx,
        });
    }
    let thread_record = ensure_thread_for_kind(&store, &workspace, kind)?;
    let launch = controller.build_launch(&store, &workspace, harness)?;
    let pty = PtySession::spawn(
        launch.program.clone(),
        launch.args.clone(),
        &launch.cwd,
        launch.env.clone(),
        24,
        80,
    )
    .with_context(|| format!("spawn managed {:?} pty", kind))?;
    let pid = pty.process_id().context("pty has no process id")?;
    let process =
        store.record_session_process_for_thread(&workspace, thread_record.id, &launch, pid)?;
    let snapshot = Arc::new(Mutex::new(SessionSnapshot {
        session_id: process.id,
        thread_id: thread_record.id,
        workspace: workspace.clone(),
        kind,
        pid,
        status: ProcessStatus::Running,
        ready: false,
        screen: String::new(),
    }));
    let (command_tx, command_rx) = mpsc::channel();
    let snapshot_for_thread = snapshot.clone();
    thread::spawn(move || {
        run_session_loop(
            db_path,
            logs_dir,
            snapshot_for_thread,
            controller,
            ManagedSessionConnection::Live(pty),
            command_rx,
            event_tx,
        )
    });
    Ok(SessionHandle {
        snapshot,
        command_tx,
    })
}

fn adopt_running_session(
    store: &WorkspaceStore,
    workspace: &str,
    kind: SessionKind,
) -> Result<Option<(ManagedSessionConnection, SessionSnapshot)>> {
    let Some(process) = store.list_sessions(workspace)?.into_iter().find(|record| {
        record.status == ProcessStatus::Running
            && record.chat_thread_id.is_some()
            && record.command.starts_with("codex ")
            && kind == SessionKind::Codex
    }) else {
        return Ok(None);
    };
    let thread_id = process
        .chat_thread_id
        .context("running managed session missing chat_thread_id")?;
    let connection = ManagedSessionConnection::try_reattach_running(process.pid)?;
    let snapshot = SessionSnapshot {
        session_id: process.id,
        thread_id,
        workspace: workspace.to_owned(),
        kind,
        pid: process.pid,
        status: ProcessStatus::Running,
        ready: true,
        screen: String::new(),
    };
    Ok(Some((connection, snapshot)))
}

fn session_kind_from_command(command: &str) -> Option<SessionKind> {
    let trimmed = command.trim();
    if trimmed.starts_with("codex ") || trimmed == "codex" {
        Some(SessionKind::Codex)
    } else if trimmed.starts_with("claude ") || trimmed == "claude" {
        Some(SessionKind::Claude)
    } else {
        None
    }
}

pub fn restore_managed_session(
    db_path: std::path::PathBuf,
    _logs_dir: std::path::PathBuf,
    process_id: i64,
    event_tx: Sender<ArchcarEvent>,
) -> Result<Option<SessionHandle>> {
    let store = WorkspaceStore::open(&db_path)?;
    let process = match store.get_process_record(process_id) {
        Ok(process) => process,
        Err(_) => return Ok(None),
    };
    if process.status != ProcessStatus::Running {
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
    let snapshot = Arc::new(Mutex::new(SessionSnapshot {
        session_id: process.id,
        thread_id,
        workspace,
        kind,
        pid: process.pid,
        status: ProcessStatus::Running,
        ready: kind == SessionKind::Codex,
        screen: String::new(),
    }));
    let (command_tx, command_rx) = mpsc::channel();
    let snapshot_for_thread = Arc::clone(&snapshot);
    thread::spawn(move || {
        run_session_loop(
            db_path,
            _logs_dir,
            snapshot_for_thread,
            controller,
            connection,
            command_rx,
            event_tx,
        )
    });
    Ok(Some(SessionHandle {
        snapshot,
        command_tx,
    }))
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
            input
        ),
        ArchcarInputKind::User => format!(
            "\n[user input {}#{}]\n{}\n[/user input]\n",
            workspace, session_id, input
        ),
        ArchcarInputKind::ControlCommand => String::new(),
    }
}

fn format_session_raw_output(kind: SessionKind, raw: &str) -> String {
    match kind {
        SessionKind::Codex => format_codex_raw_output(raw),
        _ => raw.to_owned(),
    }
}

fn format_session_screen_output(kind: SessionKind, screen: &str) -> String {
    match kind {
        SessionKind::Codex => format_codex_screen_snapshot(screen),
        _ => screen.to_owned(),
    }
}

fn write_pty_screen_snapshot(
    logs_dir: &std::path::Path,
    source: &str,
    process_id: i64,
    screen: &str,
) {
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

fn should_emit_message_update(
    controller: &dyn HarnessController,
    last_messages: &[ScreenMessage],
    screen: &str,
) -> Option<Vec<ScreenMessage>> {
    let parsed = controller.parse_messages(screen);
    (parsed != last_messages).then_some(parsed)
}

fn should_attempt_native_thread_resolution(kind: SessionKind, already_resolved: bool) -> bool {
    kind == SessionKind::Codex && !already_resolved
}

fn run_session_loop(
    db_path: std::path::PathBuf,
    logs_dir: std::path::PathBuf,
    snapshot: Arc<Mutex<SessionSnapshot>>,
    controller: Box<dyn HarnessController>,
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
    let mut trust_answered = false;
    let mut last_screen = String::new();
    let mut last_messages = Vec::new();
    let mut native_thread_id_resolved = false;
    loop {
        while let Ok(command) = command_rx.try_recv() {
            match command {
                SessionCommand::SendInput { input, kind } => {
                    let current = snapshot.lock().unwrap().clone();
                    info!(
                        session_id = current.session_id,
                        thread_id = current.thread_id,
                        workspace = %current.workspace,
                        kind = ?kind,
                        chars = input.chars().count(),
                        "archcar session send_input dequeued"
                    );
                    if let Ok(store) = WorkspaceStore::open(&db_path) {
                        let (role, source) = match kind {
                            ArchcarInputKind::User => ("user", "user_send"),
                            ArchcarInputKind::ReviewPrompt => ("user", "staged_review_send"),
                            ArchcarInputKind::ControlCommand => ("system", "control_command"),
                        };
                        let _ = store.append_chat_message(current.thread_id, role, &input, source);
                        let log_text = format_input_audit_log(
                            &current.workspace,
                            current.session_id,
                            &input,
                            &kind,
                        );
                        let _ = store.append_session_process_output(current.session_id, &log_text);
                    }
                    match pty.send_line(&input) {
                        Ok(()) => {
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
            }
        }

        let raw = pty.read_available();
        if !raw.is_empty() {
            let current = snapshot.lock().unwrap().clone();
            if let Ok(store) = WorkspaceStore::open(&db_path) {
                let formatted = format_session_raw_output(current.kind, &raw);
                let _ = store.append_session_process_output(current.session_id, &formatted);
            }
        }

        let screen = pty.visible_screen_text();
        if !screen.is_empty() && screen != last_screen {
            if !trust_answered {
                if let Some(input) = controller.startup_input(&screen) {
                    let _ = pty.send_line(&input);
                    trust_answered = true;
                }
            }
            let current = snapshot.lock().unwrap().clone();
            if let Ok(store) = WorkspaceStore::open(&db_path) {
                let formatted = format_session_screen_output(current.kind, &screen);
                let _ = store.append_session_process_output(current.session_id, &formatted);
            }
            write_pty_screen_snapshot(
                &logs_dir,
                "archcar-session-loop",
                current.session_id,
                &screen,
            );
            let parsed_messages =
                should_emit_message_update(controller.as_ref(), &last_messages, &screen);
            {
                let mut state = snapshot.lock().unwrap();
                state.screen = screen.clone();
                if !state.ready && controller.detect_ready(&screen) {
                    state.ready = true;
                    let _ = event_tx.send(ArchcarEvent::SessionReady {
                        session_id: state.session_id,
                        thread_id: state.thread_id,
                    });
                }
                let _ = event_tx.send(ArchcarEvent::SessionScreenUpdated {
                    session_id: state.session_id,
                });
                if let Some(parsed_messages) = parsed_messages {
                    last_messages = parsed_messages;
                    let _ = event_tx.send(ArchcarEvent::SessionMessagesUpdated {
                        thread_id: state.thread_id,
                    });
                }
            }
            last_screen = screen;
        }

        let current = snapshot.lock().unwrap().clone();
        if should_attempt_native_thread_resolution(current.kind, native_thread_id_resolved) {
            if let Ok(store) = WorkspaceStore::open(&db_path) {
                native_thread_id_resolved = store
                    .resolve_codex_native_thread_id_for_process(current.session_id)
                    .ok()
                    .flatten()
                    .is_some();
            }
        }

        match pty.has_exited() {
            Ok(true) => {
                let current = snapshot.lock().unwrap().clone();
                if let Ok(store) = WorkspaceStore::open(&db_path) {
                    let _ = store.mark_session_process_exited(current.session_id, None);
                }
                if let Ok(mut state) = snapshot.lock() {
                    state.status = ProcessStatus::Exited;
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
                let _ = event_tx.send(ArchcarEvent::SessionError {
                    session_id: Some(current.session_id),
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
    use crate::archcar::harness::{CodexHarnessController, ShellHarnessController};

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
            "/model gpt-5",
            &ArchcarInputKind::ControlCommand
        )
        .is_empty());
    }

    #[test]
    fn codex_outputs_use_canonical_wrappers() {
        assert_eq!(
            format_session_raw_output(SessionKind::Codex, "hello"),
            "[codex raw]\nhello\n[/codex raw]\n"
        );
        assert_eq!(
            format_session_screen_output(SessionKind::Codex, "hello\n"),
            "[codex screen]\nhello\n[/codex screen]\n"
        );
        assert_eq!(
            format_session_raw_output(SessionKind::Shell, "plain"),
            "plain"
        );
        assert_eq!(
            format_session_screen_output(SessionKind::Shell, "plain"),
            "plain"
        );
    }

    #[test]
    fn message_update_events_follow_parsed_transcript_changes() {
        let controller = CodexHarnessController;
        let first_screen = "╭─ You\n│ run tests\n╰─\n╭─ Codex\n│ Running now.\n╰─";
        let repeated_screen =
            "╭─ You\n│ run tests\n╰─\n╭─ Codex\n│ Running now.\n╰─\nstatus: spinner";
        let changed_screen =
            "╭─ You\n│ run tests\n╰─\n╭─ Codex\n│ Running now.\n│ Tests passed.\n╰─";

        let first = should_emit_message_update(&controller, &[], first_screen).unwrap();
        assert_eq!(first.len(), 2);

        let repeated = should_emit_message_update(&controller, &first, repeated_screen);
        assert!(repeated.is_none());

        let changed = should_emit_message_update(&controller, &first, changed_screen).unwrap();
        assert_eq!(changed.len(), 2);
        assert_ne!(changed, first);
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
    fn session_kind_detection_only_restores_supported_harnesses() {
        assert_eq!(
            session_kind_from_command("codex --no-alt-screen"),
            Some(SessionKind::Codex)
        );
        assert_eq!(
            session_kind_from_command("claude --print"),
            Some(SessionKind::Claude)
        );
        assert_eq!(session_kind_from_command("bash -lc ls"), None);
    }

    #[test]
    fn pty_screen_snapshot_writer_appends_expected_marker() {
        let temp = tempfile::tempdir().unwrap();
        write_pty_screen_snapshot(temp.path(), "archcar", 7, "hello\nworld\n");
        let body = std::fs::read_to_string(temp.path().join("pty-screens.log")).unwrap();
        assert!(body.contains("source=archcar process_id=7"));
        assert!(body.contains("hello\nworld"));
    }

    #[test]
    fn empty_shell_parsing_does_not_emit_message_updates() {
        let controller = ShellHarnessController;
        let parsed = should_emit_message_update(&controller, &[], "plain shell output");
        assert!(parsed.is_none());
    }
}
