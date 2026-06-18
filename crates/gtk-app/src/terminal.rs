use anyhow::{Context, Result};
use gtk::prelude::*;
use gtk::{Box as GBox, Button, Entry, Label, Orientation, ScrolledWindow, TextBuffer, TextView};
use linux_conductor_core::pty::PtySession;
use linux_conductor_core::workspace::{SessionKind, TerminalLogMatch, WorkspaceStore};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

pub fn embedded_terminal_panel(
    database_path: PathBuf,
    workspace_name: &str,
    workspace_path: &Path,
    full_mode: bool,
) -> GBox {
    let root = GBox::new(Orientation::Vertical, 8);
    root.add_css_class("terminal-panel");

    let heading = Label::new(Some(if full_mode {
        "Big Terminal"
    } else {
        "Workspace Terminal"
    }));
    heading.add_css_class("section-title");
    heading.set_xalign(0.0);
    root.append(&heading);

    let transcript = TextView::new();
    transcript.set_editable(false);
    transcript.set_monospace(true);
    transcript.add_css_class("history-view");
    transcript.buffer().set_text(&initial_terminal_text(
        &database_path,
        workspace_name,
        workspace_path,
    ));

    let active_pty: Rc<RefCell<Option<TerminalSession>>> = Rc::new(RefCell::new(None));
    let buffer_for_poll = transcript.buffer();
    let pty_for_poll = active_pty.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if let Some(session) = pty_for_poll.borrow_mut().as_mut() {
            let output = session.session.read_available();
            if !output.is_empty() {
                if let Err(err) = session.append_output(&output) {
                    append_text(&buffer_for_poll, &format!("\n[pty log error]\n{err:#}\n"));
                }
                append_text(&buffer_for_poll, &output);
            }
        }
        glib::ControlFlow::Continue
    });

    let transcript_scroll = ScrolledWindow::new();
    transcript_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    transcript_scroll.set_vexpand(true);
    transcript_scroll.set_child(Some(&transcript));
    root.append(&transcript_scroll);

    let pty_controls = GBox::new(Orientation::Horizontal, 8);
    let start_pty_btn = Button::with_label("Start Shell");
    let stop_pty_btn = Button::with_label("Stop Shell");
    let db_for_pty = database_path.clone();
    let workspace_for_pty = workspace_name.to_owned();
    let pty_for_start = active_pty.clone();
    let buffer_for_start = transcript.buffer();
    let cols = if full_mode { 120 } else { 80 };
    start_pty_btn.connect_clicked(move |_| {
        if pty_for_start.borrow().is_some() {
            append_text(&buffer_for_start, "\n[pty already running]\n");
            return;
        }
        match WorkspaceStore::open(db_for_pty.clone()).and_then(|store| {
            let launch = store.session_launch(&workspace_for_pty, SessionKind::Shell)?;
            let command = display_command(&launch.program, &launch.args);
            let session = PtySession::spawn(
                launch.program,
                launch.args,
                &launch.cwd,
                launch.env,
                24,
                cols,
            )?;
            let pid = session
                .process_id()
                .context("PTY shell did not report a process id")?;
            let process = store.record_terminal_process(&workspace_for_pty, &command, pid)?;
            Ok(TerminalSession {
                session,
                database_path: db_for_pty.clone(),
                process_id: Some(process.id),
            })
        }) {
            Ok(terminal) => {
                *pty_for_start.borrow_mut() = Some(terminal);
                append_text(&buffer_for_start, "\n[pty shell started]\n");
            }
            Err(err) => append_text(&buffer_for_start, &format!("\n[pty error]\n{err:#}\n")),
        }
    });
    let pty_for_stop = active_pty.clone();
    let buffer_for_stop = transcript.buffer();
    stop_pty_btn.connect_clicked(move |_| {
        if let Some(session) = pty_for_stop.borrow_mut().take() {
            match session.stop() {
                Ok(()) => append_text(&buffer_for_stop, "\n[pty shell stopped]\n"),
                Err(err) => {
                    append_text(&buffer_for_stop, &format!("\n[pty stop error]\n{err:#}\n"))
                }
            }
        } else {
            append_text(&buffer_for_stop, "\n[no pty shell running]\n");
        }
    });
    pty_controls.append(&start_pty_btn);
    pty_controls.append(&stop_pty_btn);
    root.append(&pty_controls);

    let presets = GBox::new(Orientation::Horizontal, 8);
    for (label, command) in [
        ("Env", "env | sort | grep '^CONDUCTOR_'"),
        ("Git Status", "git status --short --branch"),
        ("Git Diff", "git diff --stat && git diff -- ."),
        (
            "Files",
            "find . -maxdepth 2 -type f | sort | sed 's#^./##' | head -80",
        ),
    ] {
        let button = Button::with_label(label);
        button.set_tooltip_text(Some(command));
        let db = database_path.clone();
        let workspace = workspace_name.to_owned();
        let buffer = transcript.buffer();
        let pty = active_pty.clone();
        button.connect_clicked(move |_| {
            send_or_run_terminal_command(
                db.clone(),
                workspace.clone(),
                command.to_owned(),
                buffer.clone(),
                pty.clone(),
            );
        });
        presets.append(&button);
    }
    root.append(&presets);

    let command_row = GBox::new(Orientation::Horizontal, 8);
    let entry = Entry::new();
    entry.set_placeholder_text(Some("workspace command"));
    entry.set_hexpand(true);
    let run_btn = Button::with_label("Run");
    let buffer = transcript.buffer();
    let workspace = workspace_name.to_owned();
    let db = database_path.clone();
    let pty = active_pty;
    let entry_clone = entry.clone();
    run_btn.connect_clicked(move |_| {
        let command = entry_clone.text().trim().to_owned();
        if command.is_empty() {
            return;
        }
        send_or_run_terminal_command(
            db.clone(),
            workspace.clone(),
            command,
            buffer.clone(),
            pty.clone(),
        );
        entry_clone.set_text("");
    });

    command_row.append(&entry);
    command_row.append(&run_btn);
    root.append(&command_row);

    let search_row = GBox::new(Orientation::Horizontal, 8);
    let search_entry = Entry::new();
    search_entry.set_placeholder_text(Some("search terminal history"));
    search_entry.set_hexpand(true);
    let search_btn = Button::with_label("Search Logs");
    let search_buffer = transcript.buffer();
    let search_workspace = workspace_name.to_owned();
    let search_db = database_path;
    let search_entry_clone = search_entry.clone();
    search_btn.connect_clicked(move |_| {
        let query = search_entry_clone.text().trim().to_owned();
        if query.is_empty() {
            return;
        }
        run_terminal_log_search(
            search_db.clone(),
            search_workspace.clone(),
            query,
            search_buffer.clone(),
        );
    });
    search_row.append(&search_entry);
    search_row.append(&search_btn);
    root.append(&search_row);
    root
}

fn send_or_run_terminal_command(
    database_path: PathBuf,
    workspace_name: String,
    command: String,
    buffer: TextBuffer,
    pty: Rc<RefCell<Option<TerminalSession>>>,
) {
    if let Some(session) = pty.borrow_mut().as_mut() {
        let command_line = format!("\n$ {command}\n");
        append_text(&buffer, &command_line);
        if let Err(err) = session.append_output(&command_line) {
            append_text(&buffer, &format!("[pty log error]\n{err:#}\n"));
        }
        if let Err(err) = session.session.write(&format!("{command}\n")) {
            append_text(&buffer, &format!("[pty write error]\n{err:#}\n"));
        }
        return;
    }
    run_terminal_command(database_path, workspace_name, command, buffer);
}

fn run_terminal_command(
    database_path: PathBuf,
    workspace_name: String,
    command: String,
    buffer: TextBuffer,
) {
    append_text(&buffer, &format!("\n$ {command}\n[running]\n"));
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let message = match WorkspaceStore::open(database_path)
            .and_then(|store| store.terminal_command(&workspace_name, &command))
        {
            Ok(result) => {
                let mut text = String::new();
                if !result.stdout.is_empty() {
                    text.push_str(&result.stdout);
                }
                if !result.stderr.is_empty() {
                    text.push_str("\n[stderr]\n");
                    text.push_str(&result.stderr);
                }
                text.push_str(&format!(
                    "\n[exit {}]\n",
                    result
                        .exit_code
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "signal".to_owned())
                ));
                text
            }
            Err(err) => format!("[error]\n{err:#}\n"),
        };
        let _ = tx.send(message);
    });

    glib::timeout_add_local(Duration::from_millis(100), move || match rx.try_recv() {
        Ok(message) => {
            append_text(&buffer, &message);
            glib::ControlFlow::Break
        }
        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(mpsc::TryRecvError::Disconnected) => {
            append_text(&buffer, "[error]\nterminal worker disconnected\n");
            glib::ControlFlow::Break
        }
    });
}

fn run_terminal_log_search(
    database_path: PathBuf,
    workspace_name: String,
    query: String,
    buffer: TextBuffer,
) {
    append_text(
        &buffer,
        &format!("\n[terminal search] {query}\n[searching]\n"),
    );
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let message = match WorkspaceStore::open(database_path)
            .and_then(|store| store.search_terminal_logs(&workspace_name, &query))
        {
            Ok(matches) => format_terminal_search_results(&query, &matches),
            Err(err) => format!("[terminal search error]\n{err:#}\n"),
        };
        let _ = tx.send(message);
    });

    glib::timeout_add_local(Duration::from_millis(100), move || match rx.try_recv() {
        Ok(message) => {
            append_text(&buffer, &message);
            glib::ControlFlow::Break
        }
        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(mpsc::TryRecvError::Disconnected) => {
            append_text(&buffer, "[error]\nterminal search worker disconnected\n");
            glib::ControlFlow::Break
        }
    });
}

fn format_terminal_search_results(query: &str, matches: &[TerminalLogMatch]) -> String {
    let mut text = format!("\n[terminal search] {query}\n");
    if matches.is_empty() {
        text.push_str("No terminal transcript matches.\n");
        return text;
    }
    for item in matches {
        let file_name = item
            .log_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("terminal.log");
        text.push_str(&format!(
            "#{} {} {}:{}\n{}\n",
            item.process_id, item.command, file_name, item.line_number, item.line
        ));
    }
    text
}

fn initial_terminal_text(
    database_path: &Path,
    workspace_name: &str,
    workspace_path: &Path,
) -> String {
    let restored = WorkspaceStore::open(database_path)
        .and_then(|store| store.read_latest_terminal_log(workspace_name))
        .ok()
        .filter(|log| !log.trim().is_empty());
    format_initial_terminal_text(workspace_name, workspace_path, restored.as_deref())
}

fn format_initial_terminal_text(
    workspace_name: &str,
    workspace_path: &Path,
    restored_transcript: Option<&str>,
) -> String {
    let mut text = format!(
        "Workspace terminal\nworkspace: {}\npath: {}\n\nCommands run here execute inside the workspace with CONDUCTOR_* environment variables.",
        workspace_name,
        workspace_path.display()
    );
    if let Some(transcript) = restored_transcript {
        text.push_str("\n\n[restored latest terminal transcript]\n");
        text.push_str(&terminal_display_text(transcript));
    }
    text
}

fn append_text(buffer: &TextBuffer, text: &str) {
    let mut end = buffer.end_iter();
    buffer.insert(&mut end, &terminal_display_text(text));
}

fn terminal_display_text(text: &str) -> String {
    let mut rendered = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            rendered.push(ch);
            continue;
        }

        match chars.peek().copied() {
            Some('[') => {
                chars.next();
                for code in chars.by_ref() {
                    if ('@'..='~').contains(&code) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                while let Some(code) = chars.next() {
                    if code == '\u{7}' {
                        break;
                    }
                    if code == '\u{1b}' && matches!(chars.peek(), Some('\\')) {
                        chars.next();
                        break;
                    }
                }
            }
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    rendered
}

struct TerminalSession {
    session: PtySession,
    database_path: PathBuf,
    process_id: Option<i64>,
}

impl TerminalSession {
    fn stop(mut self) -> Result<()> {
        self.session.stop()?;
        self.mark_stopped(Some(143))
    }

    fn mark_stopped(&mut self, exit_code: Option<i32>) -> Result<()> {
        let Some(process_id) = self.process_id.take() else {
            return Ok(());
        };
        WorkspaceStore::open(self.database_path.clone())?
            .mark_terminal_process_stopped(process_id, exit_code)?;
        Ok(())
    }

    fn append_output(&self, output: &str) -> Result<()> {
        let Some(process_id) = self.process_id else {
            return Ok(());
        };
        WorkspaceStore::open(self.database_path.clone())?
            .append_terminal_process_output(process_id, output)
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.session.stop();
        let _ = self.mark_stopped(Some(143));
    }
}

fn display_command(program: &Path, args: &[String]) -> String {
    std::iter::once(program.display().to_string())
        .chain(args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_search_results_render_process_line_and_empty_state() {
        let matches = vec![TerminalLogMatch {
            process_id: 42,
            command: "/bin/sh".to_owned(),
            log_path: PathBuf::from("/tmp/logs/terminal.log"),
            line_number: 3,
            line: "needle found".to_owned(),
        }];

        let rendered = format_terminal_search_results("needle", &matches);

        assert!(rendered.contains("[terminal search] needle"));
        assert!(rendered.contains("#42 /bin/sh"));
        assert!(rendered.contains("terminal.log:3"));
        assert!(rendered.contains("needle found"));
        assert_eq!(
            format_terminal_search_results("missing", &[]),
            "\n[terminal search] missing\nNo terminal transcript matches.\n"
        );
    }

    #[test]
    fn restored_terminal_transcript_is_included_in_initial_text() {
        let text = format_initial_terminal_text(
            "berlin",
            Path::new("/tmp/workspaces/berlin"),
            Some("last shell output\n"),
        );

        assert!(text.contains("workspace: berlin"));
        assert!(text.contains("[restored latest terminal transcript]"));
        assert!(text.contains("last shell output"));
    }

    #[test]
    fn terminal_display_text_strips_common_ansi_escape_sequences() {
        let rendered = terminal_display_text("\u{1b}[32mok\u{1b}[0m\r\u{1b}[Kdone\n");

        assert_eq!(rendered, "ok\rdone\n");
    }
}
