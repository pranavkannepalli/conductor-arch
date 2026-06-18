use gtk::prelude::*;
use gtk::{Box as GBox, Button, Entry, Label, Orientation, ScrolledWindow, TextBuffer, TextView};
use linux_conductor_core::pty::PtySession;
use linux_conductor_core::workspace::{SessionKind, WorkspaceStore};
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
    transcript.buffer().set_text(&format!(
        "Workspace terminal\nworkspace: {}\npath: {}\n\nCommands run here execute inside the workspace with CONDUCTOR_* environment variables.",
        workspace_name,
        workspace_path.display()
    ));

    let active_pty: Rc<RefCell<Option<PtySession>>> = Rc::new(RefCell::new(None));
    let buffer_for_poll = transcript.buffer();
    let pty_for_poll = active_pty.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if let Some(session) = pty_for_poll.borrow_mut().as_mut() {
            let output = session.read_available();
            if !output.is_empty() {
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
        match WorkspaceStore::open(db_for_pty.clone())
            .and_then(|store| store.session_launch(&workspace_for_pty, SessionKind::Shell))
            .and_then(|launch| {
                PtySession::spawn(
                    launch.program,
                    launch.args,
                    &launch.cwd,
                    launch.env,
                    24,
                    cols,
                )
            }) {
            Ok(session) => {
                *pty_for_start.borrow_mut() = Some(session);
                append_text(&buffer_for_start, "\n[pty shell started]\n");
            }
            Err(err) => append_text(&buffer_for_start, &format!("\n[pty error]\n{err:#}\n")),
        }
    });
    let pty_for_stop = active_pty.clone();
    let buffer_for_stop = transcript.buffer();
    stop_pty_btn.connect_clicked(move |_| {
        if let Some(mut session) = pty_for_stop.borrow_mut().take() {
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
    let db = database_path;
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
    root
}

fn send_or_run_terminal_command(
    database_path: PathBuf,
    workspace_name: String,
    command: String,
    buffer: TextBuffer,
    pty: Rc<RefCell<Option<PtySession>>>,
) {
    if let Some(session) = pty.borrow_mut().as_mut() {
        append_text(&buffer, &format!("\n$ {command}\n"));
        if let Err(err) = session.write(&format!("{command}\n")) {
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

fn append_text(buffer: &TextBuffer, text: &str) {
    let mut end = buffer.end_iter();
    buffer.insert(&mut end, text);
}
