use anyhow::{Context, Result};
use gtk::prelude::*;
use gtk::{
    Box as GBox, Button, ComboBoxText, Entry, Label, Orientation, ScrolledWindow, TextBuffer,
    TextView,
};
use linux_conductor_core::pty::PtySession;
use linux_conductor_core::workspace::{
    ProcessRecord, SessionKind, TerminalLogMatch, WorkspaceStore,
};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use crate::refresh::{RefreshHub, RefreshScope};

const TERMINAL_SCROLLBACK_LINES: usize = 2_000;
const TERMINAL_SCROLLBACK_TRIM_MARKER: &str = "[terminal scrollback trimmed]\n";

pub fn embedded_terminal_panel(
    database_path: PathBuf,
    workspace_name: &str,
    workspace_path: &Path,
    full_mode: bool,
    refresh_hub: RefreshHub,
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

    let active_ptys: Rc<RefCell<Vec<Option<TerminalSession>>>> = Rc::new(RefCell::new(Vec::new()));
    let last_pty_size: Rc<RefCell<Option<(u16, u16)>>> = Rc::new(RefCell::new(None));
    let buffer_for_poll = transcript.buffer();
    let ptys_for_poll = active_ptys.clone();
    let transcript_for_poll = transcript.clone();
    let last_size_for_poll = last_pty_size.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        for session in ptys_for_poll.borrow_mut().iter_mut().flatten() {
            let size = terminal_size_from_pixels(
                transcript_for_poll.allocated_width(),
                transcript_for_poll.allocated_height(),
            );
            if *last_size_for_poll.borrow() != Some(size) {
                if let Err(err) = session.resize(size.0, size.1) {
                    append_text(
                        &buffer_for_poll,
                        &format!("\n[pty resize error]\n{err:#}\n"),
                    );
                } else {
                    *last_size_for_poll.borrow_mut() = Some(size);
                }
            }
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
    let active_pty_combo = ComboBoxText::new();
    active_pty_combo.set_hexpand(true);
    active_pty_combo.set_visible(false);
    let terminal_tabs = GBox::new(Orientation::Horizontal, 6);
    terminal_tabs.add_css_class("terminal-tab-strip");
    let tab_buttons: Rc<RefCell<Vec<Button>>> = Rc::new(RefCell::new(Vec::new()));
    let db_for_pty = database_path.clone();
    let workspace_for_pty = workspace_name.to_owned();
    let ptys_for_start = active_ptys.clone();
    let buffer_for_start = transcript.buffer();
    let refresh_for_start = refresh_hub.clone();
    let active_pty_combo_for_start = active_pty_combo.clone();
    let terminal_tabs_for_start = terminal_tabs.clone();
    let tab_buttons_for_start = tab_buttons.clone();
    let last_size_for_start = last_pty_size.clone();
    let cols = if full_mode { 120 } else { 80 };
    start_pty_btn.connect_clicked(move |_| {
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
            Ok((
                TerminalSession {
                    session,
                    database_path: db_for_pty.clone(),
                    process_id: Some(process.id),
                },
                process.id,
            ))
        }) {
            Ok((terminal, process_id)) => {
                let mut sessions = ptys_for_start.borrow_mut();
                let index = sessions.len();
                sessions.push(Some(terminal));
                active_pty_combo_for_start.append(
                    Some(&index.to_string()),
                    &active_terminal_session_option_label(index, Some(process_id)),
                );
                active_pty_combo_for_start.set_active(Some(index as u32));
                let tab =
                    Button::with_label(&active_terminal_tab_label(index, Some(process_id), true));
                tab.add_css_class("flat");
                let active_pty_combo_for_tab = active_pty_combo_for_start.clone();
                let tab_buttons_for_tab = tab_buttons_for_start.clone();
                tab.connect_clicked(move |_| {
                    active_pty_combo_for_tab.set_active(Some(index as u32));
                    set_terminal_tab_active(&tab_buttons_for_tab.borrow(), Some(index));
                });
                terminal_tabs_for_start.append(&tab);
                tab_buttons_for_start.borrow_mut().push(tab);
                set_terminal_tab_active(&tab_buttons_for_start.borrow(), Some(index));
                *last_size_for_start.borrow_mut() = None;
                append_text(
                    &buffer_for_start,
                    &format!("\n[pty shell {} started]\n", index + 1),
                );
                refresh_for_start.refresh(terminal_process_refresh_scope());
            }
            Err(err) => append_text(&buffer_for_start, &format!("\n[pty error]\n{err:#}\n")),
        }
    });
    let ptys_for_stop = active_ptys.clone();
    let buffer_for_stop = transcript.buffer();
    let refresh_for_stop = refresh_hub.clone();
    let active_pty_combo_for_stop = active_pty_combo.clone();
    let tab_buttons_for_stop = tab_buttons.clone();
    stop_pty_btn.connect_clicked(move |_| {
        let Some(active_id) = active_pty_combo_for_stop.active_id() else {
            append_text(&buffer_for_stop, "\n[no pty shell selected]\n");
            return;
        };
        let Ok(index) = active_id.as_str().parse::<usize>() else {
            append_text(&buffer_for_stop, "\n[selected pty shell is invalid]\n");
            return;
        };
        let mut sessions = ptys_for_stop.borrow_mut();
        let Some(session_slot) = sessions.get_mut(index) else {
            append_text(&buffer_for_stop, "\n[selected pty shell is missing]\n");
            return;
        };
        if let Some(session) = session_slot.take() {
            let process_id = session.process_id;
            match session.stop() {
                Ok(()) => append_text(
                    &buffer_for_stop,
                    &format!("\n[pty shell {} stopped]\n", index + 1),
                ),
                Err(err) => {
                    append_text(&buffer_for_stop, &format!("\n[pty stop error]\n{err:#}\n"))
                }
            }
            if let Some(tab) = tab_buttons_for_stop.borrow().get(index) {
                tab.set_label(&active_terminal_tab_label(index, process_id, false));
            }
            set_terminal_tab_active(&tab_buttons_for_stop.borrow(), Some(index));
            refresh_for_stop.refresh(terminal_process_refresh_scope());
        } else {
            append_text(&buffer_for_stop, "\n[selected pty shell already stopped]\n");
        }
    });
    root.append(&terminal_tabs);
    pty_controls.append(&start_pty_btn);
    pty_controls.append(&stop_pty_btn);
    pty_controls.append(&active_pty_combo);
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
        let ptys = active_ptys.clone();
        let active_pty_combo_for_command = active_pty_combo.clone();
        button.connect_clicked(move |_| {
            send_or_run_terminal_command(
                db.clone(),
                workspace.clone(),
                command.to_owned(),
                buffer.clone(),
                ptys.clone(),
                active_pty_combo_for_command.clone(),
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
    let ptys = active_ptys;
    let active_pty_combo_for_run = active_pty_combo;
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
            ptys.clone(),
            active_pty_combo_for_run.clone(),
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
    let history_btn = Button::with_label("Show History");
    let history_combo = ComboBoxText::new();
    history_combo.set_hexpand(true);
    let load_history_btn = Button::with_label("Load Transcript");
    let history_records: Rc<RefCell<Vec<ProcessRecord>>> = Rc::new(RefCell::new(Vec::new()));
    let search_buffer = transcript.buffer();
    let history_buffer = transcript.buffer();
    let load_history_buffer = transcript.buffer();
    let search_workspace = workspace_name.to_owned();
    let history_workspace = workspace_name.to_owned();
    let load_history_workspace = workspace_name.to_owned();
    let history_db = database_path.clone();
    let search_db = database_path;
    let load_history_db = history_db.clone();
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
    let history_combo_for_load = history_combo.clone();
    let history_records_for_load = history_records.clone();
    load_history_btn.connect_clicked(move |_| {
        let Some(active_id) = history_combo_for_load.active_id() else {
            append_text(
                &load_history_buffer,
                "\n[terminal history]\nSelect a terminal session first.\n",
            );
            return;
        };
        let Ok(process_id) = active_id.as_str().parse::<i64>() else {
            append_text(
                &load_history_buffer,
                "\n[terminal history]\nSelected terminal session is invalid.\n",
            );
            return;
        };
        let Some(record) = history_records_for_load
            .borrow()
            .iter()
            .find(|record| record.id == process_id)
            .cloned()
        else {
            append_text(
                &load_history_buffer,
                "\n[terminal history]\nSelected terminal session is no longer loaded.\n",
            );
            return;
        };
        run_terminal_transcript_load(
            load_history_db.clone(),
            load_history_workspace.clone(),
            record,
            load_history_buffer.clone(),
        );
    });
    let history_combo_for_history = history_combo.clone();
    let history_records_for_history = history_records;
    history_btn.connect_clicked(move |_| {
        run_terminal_history(
            history_db.clone(),
            history_workspace.clone(),
            history_buffer.clone(),
            history_combo_for_history.clone(),
            history_records_for_history.clone(),
        );
    });
    search_row.append(&search_entry);
    search_row.append(&search_btn);
    search_row.append(&history_btn);
    search_row.append(&history_combo);
    search_row.append(&load_history_btn);
    root.append(&search_row);
    root
}

fn send_or_run_terminal_command(
    database_path: PathBuf,
    workspace_name: String,
    command: String,
    buffer: TextBuffer,
    ptys: Rc<RefCell<Vec<Option<TerminalSession>>>>,
    active_pty_combo: ComboBoxText,
) {
    if let Some(active_id) = active_pty_combo.active_id() {
        let Ok(index) = active_id.as_str().parse::<usize>() else {
            append_text(&buffer, "\n[selected pty shell is invalid]\n");
            return;
        };
        let mut sessions = ptys.borrow_mut();
        let Some(session_slot) = sessions.get_mut(index) else {
            append_text(&buffer, "\n[selected pty shell is missing]\n");
            return;
        };
        let Some(session) = session_slot.as_mut() else {
            append_text(&buffer, "\n[selected pty shell is stopped]\n");
            return;
        };
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

fn run_terminal_history(
    database_path: PathBuf,
    workspace_name: String,
    buffer: TextBuffer,
    history_combo: ComboBoxText,
    history_records: Rc<RefCell<Vec<ProcessRecord>>>,
) {
    append_text(&buffer, "\n[terminal history]\n[loading]\n");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = WorkspaceStore::open(database_path)
            .and_then(|store| store.list_terminals(&workspace_name));
        let _ = tx.send(result);
    });

    glib::timeout_add_local(Duration::from_millis(100), move || match rx.try_recv() {
        Ok(Ok(records)) => {
            history_combo.remove_all();
            for record in &records {
                history_combo.append(
                    Some(&record.id.to_string()),
                    &terminal_history_option_label(record),
                );
            }
            if !records.is_empty() {
                history_combo.set_active(Some(0));
            }
            *history_records.borrow_mut() = records.clone();
            append_text(&buffer, &format_terminal_history(&records));
            glib::ControlFlow::Break
        }
        Ok(Err(err)) => {
            append_text(&buffer, &format!("[terminal history error]\n{err:#}\n"));
            glib::ControlFlow::Break
        }
        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(mpsc::TryRecvError::Disconnected) => {
            append_text(&buffer, "[error]\nterminal history worker disconnected\n");
            glib::ControlFlow::Break
        }
    });
}

fn run_terminal_transcript_load(
    database_path: PathBuf,
    workspace_name: String,
    record: ProcessRecord,
    buffer: TextBuffer,
) {
    append_text(
        &buffer,
        &format!("\n[terminal transcript #{}]\n[loading]\n", record.id),
    );
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let message = match WorkspaceStore::open(database_path)
            .and_then(|store| store.read_terminal_log(&workspace_name, record.id))
        {
            Ok(transcript) => format_selected_terminal_transcript(&record, &transcript),
            Err(err) => format!("[terminal transcript error]\n{err:#}\n"),
        };
        let _ = tx.send(message);
    });

    glib::timeout_add_local(Duration::from_millis(100), move || match rx.try_recv() {
        Ok(message) => {
            buffer.set_text(&message);
            glib::ControlFlow::Break
        }
        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(mpsc::TryRecvError::Disconnected) => {
            append_text(
                &buffer,
                "[error]\nterminal transcript worker disconnected\n",
            );
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

fn format_terminal_history(records: &[ProcessRecord]) -> String {
    let mut text = "\n[terminal history]\n".to_owned();
    if records.is_empty() {
        text.push_str("No terminal shells recorded.\n");
        return text;
    }

    for record in records {
        let file_name = record
            .log_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("terminal.log");
        text.push_str(&format!(
            "#{} {} pid={} exit={} log={} started={}\n{}\n",
            record.id,
            record.status.as_str(),
            record.pid,
            terminal_exit_label(record.exit_code),
            file_name,
            record.started_at,
            record.command
        ));
    }
    text
}

fn terminal_history_option_label(record: &ProcessRecord) -> String {
    let file_name = record
        .log_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("terminal.log");
    format!(
        "#{} {} pid={} {}",
        record.id,
        record.status.as_str(),
        record.pid,
        file_name
    )
}

fn active_terminal_session_option_label(index: usize, process_id: Option<i64>) -> String {
    match process_id {
        Some(process_id) => format!("Shell {} #{}", index + 1, process_id),
        None => format!("Shell {}", index + 1),
    }
}

fn active_terminal_tab_label(index: usize, process_id: Option<i64>, running: bool) -> String {
    let status = if running { "running" } else { "stopped" };
    match process_id {
        Some(process_id) => format!("Shell {} #{} {}", index + 1, process_id, status),
        None => format!("Shell {} {}", index + 1, status),
    }
}

fn set_terminal_tab_active(tabs: &[Button], active_index: Option<usize>) {
    for (index, tab) in tabs.iter().enumerate() {
        if Some(index) == active_index {
            tab.add_css_class("suggested-action");
        } else {
            tab.remove_css_class("suggested-action");
        }
    }
}

fn format_selected_terminal_transcript(record: &ProcessRecord, transcript: &str) -> String {
    trim_terminal_scrollback(
        &format!(
            "[terminal transcript #{}]\nstatus={} pid={} exit={} started={}\ncommand: {}\n\n{}",
            record.id,
            record.status.as_str(),
            record.pid,
            terminal_exit_label(record.exit_code),
            record.started_at,
            record.command,
            terminal_display_text(transcript)
        ),
        TERMINAL_SCROLLBACK_LINES,
    )
}

fn terminal_exit_label(exit_code: Option<i32>) -> String {
    exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "-".to_owned())
}

fn terminal_process_refresh_scope() -> RefreshScope {
    RefreshScope::Workspace
}

fn terminal_size_from_pixels(width: i32, height: i32) -> (u16, u16) {
    let cols = (width.max(0) / 8).clamp(20, u16::MAX as i32) as u16;
    let rows = (height.max(0) / 20).clamp(4, u16::MAX as i32) as u16;
    (rows, cols)
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
    trim_terminal_buffer(buffer, TERMINAL_SCROLLBACK_LINES);
}

fn trim_terminal_buffer(buffer: &TextBuffer, max_lines: usize) {
    let text = buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string();
    let trimmed = trim_terminal_scrollback(&text, max_lines);
    if trimmed != text {
        buffer.set_text(&trimmed);
    }
}

fn trim_terminal_scrollback(text: &str, max_lines: usize) -> String {
    if max_lines == 0 {
        return TERMINAL_SCROLLBACK_TRIM_MARKER.to_owned();
    }

    let trailing_newline = text.ends_with('\n');
    let lines = text.lines().collect::<Vec<_>>();
    if lines.len() <= max_lines {
        return text.to_owned();
    }

    let mut trimmed = TERMINAL_SCROLLBACK_TRIM_MARKER.to_owned();
    trimmed.push_str(&lines[lines.len() - max_lines..].join("\n"));
    if trailing_newline {
        trimmed.push('\n');
    }
    trimmed
}

fn terminal_display_text(text: &str) -> String {
    let mut rendered = Vec::new();
    let mut cursor = None;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            cursor = Some(line_start(&rendered));
            continue;
        }
        if ch == '\n' {
            if let Some(position) = cursor {
                if rendered.get(position) == Some(&'\n') {
                    cursor = None;
                    continue;
                }
            }
            cursor = None;
            rendered.push(ch);
            continue;
        }
        if ch != '\u{1b}' {
            push_terminal_display_char(&mut rendered, &mut cursor, ch);
            continue;
        }

        match chars.peek().copied() {
            Some('[') => {
                chars.next();
                let mut sequence = String::new();
                let mut final_code = None;
                for code in chars.by_ref() {
                    if ('@'..='~').contains(&code) {
                        final_code = Some(code);
                        break;
                    }
                    sequence.push(code);
                }
                match final_code {
                    Some('A') => {
                        cursor = Some(move_terminal_display_cursor_up(
                            &rendered,
                            cursor.unwrap_or(rendered.len()),
                            csi_first_number(&sequence, 1),
                        ));
                    }
                    Some('G') => {
                        cursor = Some(move_terminal_display_cursor_to_column(
                            &rendered,
                            cursor.unwrap_or(rendered.len()),
                            csi_first_number(&sequence, 1),
                        ));
                    }
                    Some('H') => {
                        cursor = Some(move_terminal_display_cursor_to_position(
                            &rendered,
                            csi_numbers(&sequence),
                        ));
                    }
                    Some('J') => {
                        if csi_first_number(&sequence, 0) == 2 {
                            rendered.clear();
                            cursor = Some(0);
                        }
                    }
                    Some('K') => {
                        clear_terminal_display_line(&mut rendered, cursor);
                    }
                    _ => {}
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
    rendered.into_iter().collect()
}

fn push_terminal_display_char(rendered: &mut Vec<char>, cursor: &mut Option<usize>, ch: char) {
    let Some(position) = *cursor else {
        rendered.push(ch);
        return;
    };
    if position < rendered.len() && rendered[position] != '\n' {
        rendered[position] = ch;
    } else if position <= rendered.len() {
        rendered.insert(position, ch);
    } else {
        rendered.push(ch);
    }
    *cursor = Some(position + 1);
}

fn clear_terminal_display_line(rendered: &mut Vec<char>, cursor: Option<usize>) {
    let Some(start) = cursor else {
        return;
    };
    let end = rendered[start..]
        .iter()
        .position(|ch| *ch == '\n')
        .map(|offset| start + offset)
        .unwrap_or(rendered.len());
    rendered.drain(start..end);
}

fn move_terminal_display_cursor_up(rendered: &[char], cursor: usize, lines: usize) -> usize {
    let column = cursor.saturating_sub(line_start_before(rendered, cursor));
    let mut start = line_start_before(rendered, cursor);
    for _ in 0..lines {
        if start == 0 {
            break;
        }
        start = line_start_before(rendered, start.saturating_sub(1));
    }
    (start + column).min(line_end_after(rendered, start))
}

fn move_terminal_display_cursor_to_column(
    rendered: &[char],
    cursor: usize,
    column: usize,
) -> usize {
    let start = line_start_before(rendered, cursor);
    let target = start + column.saturating_sub(1);
    target.min(line_end_after(rendered, start))
}

fn csi_first_number(sequence: &str, default: usize) -> usize {
    sequence
        .split(';')
        .next()
        .and_then(|part| part.parse::<usize>().ok())
        .filter(|number| *number > 0)
        .unwrap_or(default)
}

fn csi_numbers(sequence: &str) -> Vec<usize> {
    sequence
        .split(';')
        .filter_map(|part| part.parse::<usize>().ok())
        .collect()
}

fn move_terminal_display_cursor_to_position(rendered: &[char], numbers: Vec<usize>) -> usize {
    let row = numbers.first().copied().unwrap_or(1).max(1);
    let column = numbers.get(1).copied().unwrap_or(1).max(1);
    let mut start = 0;
    for _ in 1..row {
        start = match rendered[start.min(rendered.len())..]
            .iter()
            .position(|ch| *ch == '\n')
        {
            Some(offset) => start + offset + 1,
            None => rendered.len(),
        };
    }
    (start + column - 1).min(line_end_after(rendered, start))
}

fn line_start(rendered: &[char]) -> usize {
    rendered
        .iter()
        .rposition(|ch| *ch == '\n')
        .map(|index| index + 1)
        .unwrap_or(0)
}

fn line_start_before(rendered: &[char], cursor: usize) -> usize {
    rendered[..cursor.min(rendered.len())]
        .iter()
        .rposition(|ch| *ch == '\n')
        .map(|index| index + 1)
        .unwrap_or(0)
}

fn line_end_after(rendered: &[char], start: usize) -> usize {
    rendered[start.min(rendered.len())..]
        .iter()
        .position(|ch| *ch == '\n')
        .map(|offset| start + offset)
        .unwrap_or(rendered.len())
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

    fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        self.session.resize(rows, cols)
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
    use linux_conductor_core::workspace::{ProcessKind, ProcessStatus};

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
        let rendered = terminal_display_text("\u{1b}[32mok\u{1b}[0m \u{1b}]0;title\u{7}done\n");

        assert_eq!(rendered, "ok done\n");
    }

    #[test]
    fn terminal_display_text_applies_carriage_return_line_updates() {
        let rendered = terminal_display_text("Downloading 10%\rDownloading 100%\nnext\n");

        assert_eq!(rendered, "Downloading 100%\nnext\n");
    }

    #[test]
    fn terminal_display_text_applies_cursor_up_line_redraws() {
        let rendered = terminal_display_text("step 1\nstep 2\n\u{1b}[1A\u{1b}[2Kdone\n");

        assert_eq!(rendered, "step 1\ndone\n");
    }

    #[test]
    fn terminal_display_text_applies_clear_screen_and_cursor_home() {
        let rendered = terminal_display_text("old line\n\u{1b}[2J\u{1b}[Hfresh\n");

        assert_eq!(rendered, "fresh\n");
    }

    #[test]
    fn terminal_size_from_pixels_clamps_to_minimum_grid() {
        assert_eq!(terminal_size_from_pixels(960, 480), (24, 120));
        assert_eq!(terminal_size_from_pixels(1, 1), (4, 20));
    }

    #[test]
    fn terminal_scrollback_keeps_latest_lines_with_marker() {
        let rendered = trim_terminal_scrollback("one\ntwo\nthree\nfour\n", 2);

        assert_eq!(rendered, "[terminal scrollback trimmed]\nthree\nfour\n");
    }

    #[test]
    fn terminal_history_summary_lists_terminal_records() {
        let records = vec![ProcessRecord {
            id: 7,
            workspace_id: 1,
            kind: ProcessKind::Terminal,
            command: "/bin/bash".to_owned(),
            pid: 4242,
            log_path: PathBuf::from("/tmp/logs/terminal-4242.log"),
            status: ProcessStatus::Exited,
            started_at: "2026-06-18T02:00:00Z".to_owned(),
            exit_code: Some(0),
            ended_at: Some("2026-06-18T02:05:00Z".to_owned()),
        }];

        let rendered = format_terminal_history(&records);

        assert!(rendered.contains("[terminal history]"));
        assert!(rendered.contains("#7 exited pid=4242 exit=0"));
        assert!(rendered.contains("terminal-4242.log"));
        assert!(rendered.contains("/bin/bash"));
    }

    #[test]
    fn active_terminal_session_option_labels_include_process_id() {
        assert_eq!(
            active_terminal_session_option_label(1, Some(42)),
            "Shell 2 #42"
        );
        assert_eq!(active_terminal_session_option_label(0, None), "Shell 1");
    }

    #[test]
    fn active_terminal_tab_labels_include_state() {
        assert_eq!(
            active_terminal_tab_label(1, Some(42), true),
            "Shell 2 #42 running"
        );
        assert_eq!(
            active_terminal_tab_label(0, Some(7), false),
            "Shell 1 #7 stopped"
        );
    }

    #[test]
    fn selected_terminal_transcript_renders_session_header() {
        let record = ProcessRecord {
            id: 7,
            workspace_id: 1,
            kind: ProcessKind::Terminal,
            command: "/bin/bash".to_owned(),
            pid: 4242,
            log_path: PathBuf::from("/tmp/logs/terminal-4242.log"),
            status: ProcessStatus::Exited,
            started_at: "2026-06-18T02:00:00Z".to_owned(),
            exit_code: Some(0),
            ended_at: Some("2026-06-18T02:05:00Z".to_owned()),
        };

        let rendered = format_selected_terminal_transcript(&record, "hello\n");

        assert!(rendered.contains("[terminal transcript #7]"));
        assert!(rendered.contains("status=exited pid=4242 exit=0"));
        assert!(rendered.contains("/bin/bash"));
        assert!(rendered.contains("hello"));
    }

    #[test]
    fn terminal_process_changes_refresh_workspace_scope() {
        assert!(matches!(
            terminal_process_refresh_scope(),
            crate::refresh::RefreshScope::Workspace
        ));
    }
}
