use anyhow::Context;
use archductor_core::archcar::protocol::{ArchcarInputKind, ArchcarRequest};
use archductor_core::workspace::{
    ProcessRecord, ProcessStatus, SessionKind, TerminalLogMatch, TerminalSessionSummary,
    WorkspaceStore,
};
use futures_channel::oneshot;
use gtk::prelude::*;
use gtk::{
    Box as GBox, Button, ComboBoxText, CssProvider, Entry, Label, ListBox, Orientation,
    ScrolledWindow, TextBuffer, TextView, STYLE_PROVIDER_PRIORITY_APPLICATION,
};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::buttons::text_button;
use crate::refresh::{RefreshEvent, RefreshHub};
use crate::toast::ToastManager;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

const TERMINAL_SCROLLBACK_LINES: usize = 2_000;
const TERMINAL_SCROLLBACK_TRIM_MARKER: &str = "[terminal scrollback trimmed]\n";
const TERMINAL_SEARCH_CONTEXT_LINES: usize = 4;
const TERMINAL_TAIL_PREVIEW_LINES: usize = 160;
const TERMINAL_HEAD_PREVIEW_LINES: usize = 160;
const TERMINAL_LINE_JUMP_CONTEXT: usize = 20;
const TERMINAL_LINE_JUMP_PAGE_SIZE: usize = 160;
const TERMINAL_MIN_SCROLLBACK_LINES: usize = 100;
const TERMINAL_MAX_SCROLLBACK_LINES: usize = 20_000;

thread_local! {
    static TERMINAL_BUFFER_SCROLLBACK: RefCell<HashMap<usize, usize>> = RefCell::new(HashMap::new());
}

fn run_terminal_worker<T, W, F, D>(work: W, on_result: F, on_disconnect: D)
where
    T: Send + 'static,
    W: FnOnce() -> T + Send + 'static,
    F: FnOnce(T) + 'static,
    D: FnOnce() + 'static,
{
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    glib::spawn_future_local(async move {
        match rx.await {
            Ok(value) => on_result(value),
            Err(_) => on_disconnect(),
        }
    });
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TerminalPreferences {
    pub font: Option<String>,
    pub scrollback_lines: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TerminalCommandPreset {
    pub label: String,
    pub command: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TerminalRefreshOutcome {
    pub process_changed: bool,
    pub transcript_changed: bool,
}

type RefreshTerminalSurface = Rc<dyn Fn() -> TerminalRefreshOutcome>;

impl Default for TerminalPreferences {
    fn default() -> Self {
        Self {
            font: None,
            scrollback_lines: TERMINAL_SCROLLBACK_LINES,
        }
    }
}

impl TerminalPreferences {
    pub(crate) fn from_config(font: Option<&str>, scrollback_lines: Option<u32>) -> Self {
        let font = font
            .map(str::trim)
            .filter(|font| !font.is_empty())
            .map(ToOwned::to_owned);
        let scrollback_lines = scrollback_lines
            .and_then(|lines| {
                let lines = lines as usize;
                (lines >= TERMINAL_MIN_SCROLLBACK_LINES).then_some(lines)
            })
            .unwrap_or(TERMINAL_SCROLLBACK_LINES)
            .min(TERMINAL_MAX_SCROLLBACK_LINES);
        Self {
            font,
            scrollback_lines,
        }
    }

    fn summary(&self) -> String {
        let font = self.font.as_deref().unwrap_or("system monospace");
        format!("font: {font}\nscrollback: {} lines", self.scrollback_lines)
    }
}

#[derive(Clone)]
struct TerminalTabState {
    process_id: i64,
    attached: bool,
    status: ProcessStatus,
}

impl TerminalTabState {
    fn is_running(&self) -> bool {
        self.status == ProcessStatus::Running
    }
}

fn terminal_action_row() -> GBox {
    let row = GBox::new(Orientation::Horizontal, 8);
    row.add_css_class("action-row");
    row
}

fn terminal_action_stack() -> GBox {
    let stack = GBox::new(Orientation::Vertical, 8);
    stack.add_css_class("action-stack");
    stack
}

fn terminal_toolbar_label(text: &str) -> Label {
    let label = Label::new(Some(text));
    label.add_css_class("toolbar-label");
    label.set_xalign(0.0);
    label
}

fn terminal_secondary_button(label: &str) -> Button {
    let button = text_button(label);
    button.add_css_class("secondary-action");
    button
}

fn terminal_flat_button(label: &str) -> Button {
    let button = text_button(label);
    button.add_css_class("flat-action");
    button
}

fn terminal_destructive_button(label: &str) -> Button {
    let button = text_button(label);
    button.add_css_class("destructive-action");
    button
}

pub fn embedded_terminal_panel(
    database_path: PathBuf,
    workspace_name: &str,
    workspace_path: &Path,
    full_mode: bool,
    refresh_hub: RefreshHub,
    preferences: TerminalPreferences,
    command_presets: Vec<TerminalCommandPreset>,
    toast_manager: ToastManager,
) -> GBox {
    let root = GBox::new(Orientation::Vertical, 8);
    root.add_css_class("terminal-panel");
    root.add_css_class("session-tool-surface");
    if full_mode {
        root.set_vexpand(true);
    }

    if !full_mode {
        let heading = Label::new(Some("Workspace Terminal"));
        heading.add_css_class("section-title");
        heading.set_xalign(0.0);
        root.append(&heading);
    }

    let transcript = TextView::new();
    transcript.set_editable(false);
    transcript.set_monospace(true);
    transcript.add_css_class("history-view");
    transcript.add_css_class("terminal-transcript-dark");
    apply_terminal_preferences(&transcript, &preferences);
    set_terminal_buffer_scrollback(&transcript.buffer(), preferences.scrollback_lines);
    transcript.buffer().set_text(&initial_terminal_text(
        &database_path,
        workspace_name,
        workspace_path,
        &preferences,
    ));

    let transcript_scroll = ScrolledWindow::new();
    transcript_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    transcript_scroll.set_vexpand(true);
    if full_mode {
        transcript_scroll.set_min_content_height(420);
    }
    transcript_scroll.set_child(Some(&transcript));
    root.append(&transcript_scroll);

    let controls = terminal_action_row();
    let start_btn = text_button("Start Shell");
    start_btn.add_css_class("suggested-action");
    let resize_btn = text_button("Resize Shell");
    let stop_btn = terminal_destructive_button("Stop Shell");
    controls.append(&start_btn);
    controls.append(&resize_btn);
    controls.append(&stop_btn);
    root.append(&controls);

    let db_for_start = database_path.clone();
    let workspace_for_start = workspace_name.to_owned();
    let buffer_for_start = transcript.buffer();
    let toast_for_start = toast_manager.clone();
    let refresh_for_start = refresh_hub.clone();
    start_btn.connect_clicked(move |_| {
        let request = ArchcarRequest::SpawnSession {
            workspace: workspace_for_start.clone(),
            kind: SessionKind::Shell,
            harness: None,
        };
        crate::archcar_async::spawn_archcar_request(
            archductor_core::paths::AppPaths::from_env(),
            request,
        );
        append_text(
            &buffer_for_start,
            "\n[terminal] shell start requested through archcar runtime\n",
        );
        if let Err(err) = WorkspaceStore::open_app(db_for_start.clone())
            .and_then(|store| store.reconcile_session_processes())
            .map(|_| ())
        {
            tracing::warn!(workspace = %workspace_for_start, error = %err, "terminal runtime reconcile failed after shell start");
            let message = terminal_runtime_error_text(&err);
            toast_for_start.error(message.trim().to_owned());
            append_text(&buffer_for_start, &message);
        }
        notify_terminal_refresh(
            &refresh_for_start,
            &workspace_for_start,
            TerminalRefreshOutcome {
                process_changed: true,
                transcript_changed: true,
            },
        );
    });

    let db_for_resize = database_path.clone();
    let workspace_for_resize = workspace_name.to_owned();
    let buffer_for_resize = transcript.buffer();
    let transcript_for_resize = transcript.clone();
    resize_btn.connect_clicked(move |_| {
        let running_shell = latest_running_runtime_shell(&db_for_resize, &workspace_for_resize);
        let Some(record) = running_shell else {
            append_text(
                &buffer_for_resize,
                "\n[terminal] no running runtime shell session to resize\n",
            );
            return;
        };
        let (rows, cols) = terminal_size_from_pixels(
            transcript_for_resize.allocated_width(),
            transcript_for_resize.allocated_height(),
        );
        crate::archcar_async::spawn_archcar_request(
            archductor_core::paths::AppPaths::from_env(),
            ArchcarRequest::ResizeSession {
                session_id: record.id,
                rows,
                cols,
            },
        );
        append_text(
            &buffer_for_resize,
            &format!(
                "\n[terminal] shell resize requested through archcar for #{}: {}x{}\n",
                record.id, cols, rows
            ),
        );
    });

    let db_for_stop = database_path.clone();
    let workspace_for_stop = workspace_name.to_owned();
    let buffer_for_stop = transcript.buffer();
    let refresh_for_stop = refresh_hub.clone();
    stop_btn.connect_clicked(move |_| {
        let running_shell = latest_running_runtime_shell(&db_for_stop, &workspace_for_stop);
        let Some(record) = running_shell else {
            append_text(
                &buffer_for_stop,
                "\n[terminal] no running runtime shell session\n",
            );
            return;
        };
        let request = ArchcarRequest::KillSession {
            session_id: record.id,
        };
        crate::archcar_async::spawn_archcar_request(
            archductor_core::paths::AppPaths::from_env(),
            request,
        );
        append_text(
            &buffer_for_stop,
            &format!(
                "\n[terminal] shell stop requested through archcar for #{}\n",
                record.id
            ),
        );
        notify_terminal_refresh(
            &refresh_for_stop,
            &workspace_for_stop,
            TerminalRefreshOutcome {
                process_changed: true,
                transcript_changed: true,
            },
        );
    });

    let presets = terminal_action_row();
    for preset in command_presets {
        let button = terminal_flat_button(&preset.label);
        button.set_tooltip_text(Some(&preset.command));
        let db = database_path.clone();
        let workspace = workspace_name.to_owned();
        let buffer = transcript.buffer();
        let command = preset.command.clone();
        button.connect_clicked(move |_| {
            run_terminal_command(
                db.clone(),
                workspace.clone(),
                command.clone(),
                buffer.clone(),
            );
        });
        presets.append(&button);
    }
    root.append(&presets);

    let command_row = terminal_action_row();
    let entry = Entry::new();
    entry.set_placeholder_text(Some("workspace command"));
    entry.set_hexpand(true);
    let run_btn = text_button("Run");
    run_btn.add_css_class("suggested-action");
    command_row.append(&entry);
    command_row.append(&run_btn);
    root.append(&command_row);

    let run_command = Rc::new({
        let db = database_path.clone();
        let workspace = workspace_name.to_owned();
        let buffer = transcript.buffer();
        let entry = entry.clone();
        move || {
            let command = entry.text().trim().to_owned();
            if command.is_empty() {
                return;
            }
            run_terminal_command(db.clone(), workspace.clone(), command, buffer.clone());
            entry.set_text("");
        }
    });
    let run_for_click = run_command.clone();
    run_btn.connect_clicked(move |_| run_for_click());
    let run_for_activate = run_command.clone();
    entry.connect_activate(move |_| run_for_activate());

    root
}

fn clear_search_results(history_browser: &ListBox) {
    while let Some(child) = history_browser.first_child() {
        history_browser.remove(&child);
    }

    let empty = Label::new(Some("No terminal transcript matches."));
    empty.set_xalign(0.0);
    history_browser.append(&empty);
}

fn terminal_runtime_record_is_shell(record: &ProcessRecord) -> bool {
    let command = record.command.trim();
    !(command == "codex"
        || command.starts_with("codex ")
        || command == "claude"
        || command.starts_with("claude "))
}

fn latest_running_runtime_shell(
    database_path: &Path,
    workspace_name: &str,
) -> Option<ProcessRecord> {
    WorkspaceStore::open_app(database_path)
        .and_then(|store| store.list_sessions(workspace_name))
        .ok()
        .and_then(|records| {
            records.into_iter().find(|record| {
                record.status == ProcessStatus::Running && terminal_runtime_record_is_shell(record)
            })
        })
}

fn terminal_runtime_error_text(err: &anyhow::Error) -> String {
    format!("\n[terminal runtime error]\n{err:#}\n")
}

fn toast_if_terminal_error(toast_manager: &ToastManager, message: &str) {
    if message.contains("[terminal ") && message.contains(" error]") || message.contains("[error]")
    {
        toast_manager.error(message.trim().to_owned());
    }
}

fn load_terminal_tab_transcript(
    database_path: PathBuf,
    workspace_name: String,
    process_id: i64,
    buffer: TextBuffer,
    toast_manager: ToastManager,
) {
    match WorkspaceStore::open_app(database_path.clone()).and_then(|store| {
        store
            .list_terminals(&workspace_name)?
            .into_iter()
            .find(|record| record.id == process_id)
            .with_context(|| {
                format!("terminal session {process_id} not found for workspace {workspace_name}")
            })
    }) {
        Ok(record) => run_terminal_transcript_load(
            database_path,
            workspace_name,
            record,
            buffer,
            toast_manager,
        ),
        Err(err) => {
            let message = format!(
                "\n[terminal transcript error]\nCould not load terminal session {process_id}: {err:#}\n"
            );
            toast_manager.error(message.trim().to_owned());
            append_text(&buffer, &message);
        }
    }
}

fn run_terminal_command(
    database_path: PathBuf,
    workspace_name: String,
    command: String,
    buffer: TextBuffer,
) {
    let Some(record) = latest_running_runtime_shell(&database_path, &workspace_name) else {
        append_text(
            &buffer,
            "\n[terminal] no running runtime shell session; start shell first\n",
        );
        return;
    };
    crate::archcar_async::spawn_archcar_request(
        archductor_core::paths::AppPaths::from_env(),
        ArchcarRequest::SendInput {
            session_id: record.id,
            input: command.clone(),
            visible_input: None,
            kind: ArchcarInputKind::ControlCommand,
            delivery: archductor_core::archcar::protocol::ArchcarInputDelivery::Auto,
        },
    );
    append_text(
        &buffer,
        &format!(
            "\n$ {command}\n[terminal] sent to archcar shell #{}\n",
            record.id
        ),
    );
}

fn run_terminal_log_search(
    database_path: PathBuf,
    workspace_name: String,
    query: String,
    buffer: TextBuffer,
    history_browser: ListBox,
    browser_database_path: PathBuf,
    browser_workspace_name: String,
    jump_history_pages: Rc<RefCell<HashMap<i64, usize>>>,
    toast_manager: ToastManager,
) {
    append_text(
        &buffer,
        &format!("\n[terminal search] {query}\n[searching]\n"),
    );
    let query_for_thread = query.clone();
    let buffer_for_ui = buffer.clone();
    let buffer_for_disconnect = buffer.clone();
    let toast_for_result = toast_manager.clone();
    let toast_for_disconnect = toast_manager.clone();
    run_terminal_worker(
        move || {
            WorkspaceStore::open_app(database_path)
                .and_then(|store| store.search_terminal_logs(&workspace_name, &query_for_thread))
        },
        move |result| match result {
            Ok(matches) => {
                append_text(
                    &buffer_for_ui,
                    &format_terminal_search_results(&query, &matches),
                );
                set_terminal_search_results_browser(
                    &history_browser,
                    &matches,
                    browser_database_path.clone(),
                    browser_workspace_name.clone(),
                    buffer_for_ui.clone(),
                    jump_history_pages.clone(),
                    toast_for_result.clone(),
                );
            }
            Err(err) => {
                let message = format!("[terminal search error]\n{err:#}\n");
                toast_for_result.error(message.trim().to_owned());
                append_text(&buffer_for_ui, &message);
            }
        },
        move || {
            let message = "[error]\nterminal search worker disconnected\n";
            toast_for_disconnect.error(message.trim().to_owned());
            append_text(&buffer_for_disconnect, message);
        },
    );
}

fn run_terminal_match_transcript(
    database_path: PathBuf,
    workspace_name: String,
    process_id: i64,
    line_number: usize,
    buffer: TextBuffer,
    toast_manager: ToastManager,
) {
    append_text(
        &buffer,
        &format!(
            "\n[terminal match #{}] loading matching context around line {}\n",
            process_id, line_number
        ),
    );
    let buffer_for_disconnect = buffer.clone();
    let toast_for_message = toast_manager.clone();
    let toast_for_disconnect = toast_manager.clone();
    run_terminal_worker(
        move || {
            WorkspaceStore::open_app(database_path)
                .and_then(|store| {
                    let record = store
                        .list_terminals(&workspace_name)?
                        .into_iter()
                        .find(|record| record.id == process_id)
                        .with_context(|| {
                            format!(
                            "terminal session {process_id} not found for workspace {workspace_name}"
                        )
                        })?;
                    let transcript = store.read_terminal_log(&workspace_name, process_id)?;
                    let excerpt = terminal_log_excerpt(
                        &transcript,
                        line_number,
                        TERMINAL_SEARCH_CONTEXT_LINES,
                    );
                    Ok(format_terminal_match_transcript(
                        &record,
                        line_number,
                        &excerpt,
                    ))
                })
                .unwrap_or_else(|err| format!("[terminal match error]\n{err:#}\n"))
        },
        move |message| {
            toast_if_terminal_error(&toast_for_message, &message);
            buffer.set_text(&message);
        },
        move || {
            let message = "[error]\nterminal match worker disconnected\n";
            toast_for_disconnect.error(message.trim().to_owned());
            append_text(&buffer_for_disconnect, message);
        },
    );
}

fn run_terminal_line_transcript(
    database_path: PathBuf,
    workspace_name: String,
    process_id: i64,
    line_number: usize,
    context_lines: usize,
    buffer: TextBuffer,
    toast_manager: ToastManager,
) {
    append_text(
        &buffer,
        &format!(
            "\n[terminal session #{}] loading around line {}\n",
            process_id, line_number
        ),
    );
    let buffer_for_disconnect = buffer.clone();
    let toast_for_message = toast_manager.clone();
    let toast_for_disconnect = toast_manager.clone();
    run_terminal_worker(
        move || {
            WorkspaceStore::open_app(database_path)
                .and_then(|store| {
                    let record = store
                        .list_terminals(&workspace_name)?
                        .into_iter()
                        .find(|record| record.id == process_id)
                        .with_context(|| {
                            format!(
                            "terminal session {process_id} not found for workspace {workspace_name}"
                        )
                        })?;
                    let transcript = store.read_terminal_log(&workspace_name, process_id)?;
                    let excerpt = terminal_log_excerpt(&transcript, line_number, context_lines);
                    Ok(format_terminal_line_transcript(
                        &record,
                        line_number,
                        context_lines,
                        &excerpt,
                    ))
                })
                .unwrap_or_else(|err| {
                    format!("[terminal line jump error for session #{process_id}]\n{err:#}\n")
                })
        },
        move |message| {
            toast_if_terminal_error(&toast_for_message, &message);
            buffer.set_text(&message);
        },
        move || {
            let message = "[error]\nterminal line-jump worker disconnected\n";
            toast_for_disconnect.error(message.trim().to_owned());
            append_text(&buffer_for_disconnect, message);
        },
    );
}

fn terminal_line_jump_target(current_line: usize, delta: isize) -> usize {
    if delta == 0 {
        return current_line;
    }

    let magnitude = delta.unsigned_abs();
    if delta > 0 {
        current_line.saturating_add(magnitude)
    } else {
        current_line.saturating_sub(magnitude).max(1)
    }
}

fn terminal_positive_line_number(line_text: &str) -> Option<usize> {
    line_text
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
}

fn run_terminal_tail_transcript(
    database_path: PathBuf,
    workspace_name: String,
    process_id: i64,
    buffer: TextBuffer,
    toast_manager: ToastManager,
) {
    append_text(
        &buffer,
        &format!("\n[terminal session #{process_id}] loading tail output...\n"),
    );
    let database_path_for_thread = database_path.clone();
    let workspace_name_for_thread = workspace_name.clone();
    let buffer_for_disconnect = buffer.clone();
    let toast_for_message = toast_manager.clone();
    let toast_for_disconnect = toast_manager.clone();
    run_terminal_worker(
        move || {
            WorkspaceStore::open_app(database_path_for_thread.clone())
            .and_then(|store| {
                let record = store
                    .list_terminals(&workspace_name_for_thread)?
                    .into_iter()
                    .find(|record| record.id == process_id)
                    .with_context(|| {
                        format!(
                            "terminal session {process_id} not found for workspace {workspace_name_for_thread}"
                        )
                    })?;
                store
                    .read_terminal_log(&workspace_name_for_thread, process_id)
                    .map(|transcript| {
                        let tail = terminal_log_tail(&transcript, TERMINAL_TAIL_PREVIEW_LINES);
                        format_terminal_tail_transcript(&record, &tail)
                    })
            })
            .unwrap_or_else(|err| {
                format!("[terminal tail error for session #{process_id}]\n{err:#}\n")
            })
        },
        move |message| {
            toast_if_terminal_error(&toast_for_message, &message);
            buffer.set_text(&message);
        },
        move || {
            let message = "[error]\nterminal tail worker disconnected\n";
            toast_for_disconnect.error(message.trim().to_owned());
            append_text(&buffer_for_disconnect, message);
        },
    );
}

fn run_terminal_head_transcript(
    database_path: PathBuf,
    workspace_name: String,
    process_id: i64,
    buffer: TextBuffer,
    toast_manager: ToastManager,
) {
    append_text(
        &buffer,
        &format!("\n[terminal session #{process_id}] loading head output...\n"),
    );
    let database_path_for_thread = database_path.clone();
    let workspace_name_for_thread = workspace_name.clone();
    let buffer_for_disconnect = buffer.clone();
    let toast_for_message = toast_manager.clone();
    let toast_for_disconnect = toast_manager.clone();
    run_terminal_worker(
        move || {
            WorkspaceStore::open_app(database_path_for_thread.clone())
            .and_then(|store| {
                let record = store
                    .list_terminals(&workspace_name_for_thread)?
                    .into_iter()
                    .find(|record| record.id == process_id)
                    .with_context(|| {
                        format!(
                            "terminal session {process_id} not found for workspace {workspace_name_for_thread}"
                        )
                    })?;
                store
                    .read_terminal_log(&workspace_name_for_thread, process_id)
                    .map(|transcript| {
                        let head = terminal_log_head(&transcript, TERMINAL_HEAD_PREVIEW_LINES);
                        format_terminal_head_transcript(&record, &head)
                    })
            })
            .unwrap_or_else(|err| {
                format!("[terminal head error for session #{process_id}]\n{err:#}\n")
            })
        },
        move |message| {
            toast_if_terminal_error(&toast_for_message, &message);
            buffer.set_text(&message);
        },
        move || {
            let message = "[error]\nterminal head worker disconnected\n";
            toast_for_disconnect.error(message.trim().to_owned());
            append_text(&buffer_for_disconnect, message);
        },
    );
}

fn terminal_log_excerpt(transcript: &str, line_number: usize, context: usize) -> String {
    let lines = transcript.lines().collect::<Vec<_>>();
    if line_number == 0 || line_number > lines.len() {
        return "[no matching line in this transcript]\n".to_owned();
    }

    let start = line_number.saturating_sub(context + 1);
    let end = (line_number + context).min(lines.len());
    let mut excerpt = String::new();
    for (index, line) in lines.iter().enumerate().take(end).skip(start) {
        let number = index + 1;
        let marker = if number == line_number { ">>" } else { " " };
        excerpt.push_str(&format!("{number:>6} {marker} {line}\n"));
    }

    if excerpt.is_empty() {
        "[no matching line in this transcript]\n".to_owned()
    } else {
        terminal_display_text(&excerpt)
    }
}

fn terminal_log_tail(transcript: &str, line_count: usize) -> String {
    if line_count == 0 {
        return "[terminal session output tail empty]\n".to_owned();
    }

    let lines = transcript.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return "[terminal session output tail empty]\n".to_owned();
    }

    let start = lines.len().saturating_sub(line_count);
    let mut tail = String::new();
    for (index, line) in lines.iter().enumerate().skip(start) {
        tail.push_str(&format!("{:>6}  {}\n", index + 1, line));
    }
    terminal_display_text(&tail)
}

fn terminal_log_head(transcript: &str, line_count: usize) -> String {
    if line_count == 0 {
        return "[terminal session output head empty]\n".to_owned();
    }

    let lines = transcript.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return "[terminal session output head empty]\n".to_owned();
    }

    let end = line_count.min(lines.len());
    let mut head = String::new();
    for (index, line) in lines.iter().enumerate().take(end) {
        head.push_str(&format!("{:>6}  {}\n", index + 1, line));
    }
    terminal_display_text(&head)
}

fn format_terminal_match_transcript(
    record: &ProcessRecord,
    line_number: usize,
    excerpt: &str,
) -> String {
    format!(
        "[terminal match #{}]\n[around line {}]\nstatus={} pid={} exit={} started={}\ncommand: {}\n\n{}",
        record.id,
        line_number,
        record.status.as_str(),
        record.pid,
        terminal_exit_label(record.exit_code),
        record.started_at,
        record.command,
        excerpt,
    )
}

fn format_terminal_line_transcript(
    record: &ProcessRecord,
    line_number: usize,
    context_lines: usize,
    excerpt: &str,
) -> String {
    format!(
        "[terminal session #{0}] around line {1}\nstatus={2} pid={3} exit={4} started={5}\ncommand: {6}\ncontext={7} lines\n\n{8}",
        record.id,
        line_number,
        record.status.as_str(),
        record.pid,
        terminal_exit_label(record.exit_code),
        record.started_at,
        record.command,
        context_lines,
        excerpt,
    )
}

fn format_terminal_tail_transcript(record: &ProcessRecord, excerpt: &str) -> String {
    format!(
        "[terminal session #{0}] tail (last {1} lines)\nstatus={2} pid={3} exit={4} started={5}\ncommand: {6}\n\n{7}",
        record.id,
        TERMINAL_TAIL_PREVIEW_LINES,
        record.status.as_str(),
        record.pid,
        terminal_exit_label(record.exit_code),
        record.started_at,
        record.command,
        excerpt,
    )
}

fn format_terminal_head_transcript(record: &ProcessRecord, excerpt: &str) -> String {
    format!(
        "[terminal session #{0}] head (first {1} lines)\nstatus={2} pid={3} exit={4} started={5}\ncommand: {6}\n\n{7}",
        record.id,
        TERMINAL_HEAD_PREVIEW_LINES,
        record.status.as_str(),
        record.pid,
        terminal_exit_label(record.exit_code),
        record.started_at,
        record.command,
        excerpt,
    )
}

fn run_terminal_history(
    database_path: PathBuf,
    workspace_name: String,
    buffer: TextBuffer,
    history_combo: ComboBoxText,
    history_records: Rc<RefCell<Vec<TerminalSessionSummary>>>,
    history_records_all: Rc<RefCell<Vec<TerminalSessionSummary>>>,
    history_filter: ComboBoxText,
    history_session_filter: Entry,
    history_browser: ListBox,
    browser_database_path: PathBuf,
    browser_workspace_name: String,
    browser_buffer: TextBuffer,
    jump_history_pages: Rc<RefCell<HashMap<i64, usize>>>,
    toast_manager: ToastManager,
) {
    append_text(&buffer, "\n[terminal history]\n[loading]\n");
    let preserved_selection = history_combo
        .active_id()
        .and_then(|id| id.as_str().parse::<i64>().ok());
    let buffer_for_disconnect = buffer.clone();
    let toast_for_result = toast_manager.clone();
    let toast_for_disconnect = toast_manager.clone();
    run_terminal_worker(
        move || {
            WorkspaceStore::open_app(database_path)
                .and_then(|store| store.list_terminal_summaries(&workspace_name))
        },
        move |result| match result {
            Ok(summaries) => {
                let display_summaries = terminal_history_summaries_for_display(&summaries);
                let filter = terminal_history_filter_status(&history_filter);
                let query = history_session_filter.text().to_string();
                let filtered = terminal_history_summaries_for_filter(&display_summaries, filter);
                let filtered = terminal_history_summaries_for_filter_with_query(
                    &filtered,
                    filter,
                    Some(query.as_str()),
                );
                set_terminal_history_combo(&history_combo, &filtered, preserved_selection);
                set_terminal_history_browser(
                    &history_browser,
                    &history_combo,
                    &filtered,
                    browser_database_path.clone(),
                    browser_workspace_name.clone(),
                    browser_buffer.clone(),
                    jump_history_pages.clone(),
                    toast_for_result.clone(),
                );
                *history_records.borrow_mut() = filtered;
                *history_records_all.borrow_mut() = display_summaries;
                let displayed_records = history_records.borrow();
                append_text(&buffer, &format_terminal_history(&displayed_records));
            }
            Err(err) => {
                let message = format!("[terminal history error]\n{err:#}\n");
                toast_for_result.error(message.trim().to_owned());
                append_text(&buffer, &message);
            }
        },
        move || {
            let message = "[error]\nterminal history worker disconnected\n";
            toast_for_disconnect.error(message.trim().to_owned());
            append_text(&buffer_for_disconnect, message);
        },
    );
}

fn run_terminal_transcript_load(
    database_path: PathBuf,
    workspace_name: String,
    record: ProcessRecord,
    buffer: TextBuffer,
    toast_manager: ToastManager,
) {
    append_text(
        &buffer,
        &format!("\n[terminal transcript #{}]\n[loading]\n", record.id),
    );
    let buffer_for_disconnect = buffer.clone();
    let toast_for_message = toast_manager.clone();
    let toast_for_disconnect = toast_manager.clone();
    run_terminal_worker(
        move || match WorkspaceStore::open_app(database_path)
            .and_then(|store| store.read_terminal_log(&workspace_name, record.id))
        {
            Ok(transcript) => format_selected_terminal_transcript(&record, &transcript),
            Err(err) => format!("[terminal transcript error]\n{err:#}\n"),
        },
        move |message| {
            toast_if_terminal_error(&toast_for_message, &message);
            buffer.set_text(&message);
        },
        move || {
            let message = "[error]\nterminal transcript worker disconnected\n";
            toast_for_disconnect.error(message.trim().to_owned());
            append_text(&buffer_for_disconnect, message);
        },
    );
}

fn format_terminal_search_results(query: &str, matches: &[TerminalLogMatch]) -> String {
    let mut text = format!(
        "\n[terminal search] {query}\n{n} match(es)\n",
        n = matches.len()
    );
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
        if !item.context_before.is_empty() || !item.context_after.is_empty() {
            for line in &item.context_before {
                text.push_str(&format!("  before: {line}\n"));
            }
            for line in &item.context_after {
                text.push_str(&format!("  after: {line}\n"));
            }
        }
    }
    text
}

fn format_terminal_search_result_summary(matches: &[TerminalLogMatch]) -> String {
    let process_count = matches
        .iter()
        .map(|item| item.process_id)
        .collect::<HashSet<_>>()
        .len();
    format!(
        "Terminal matches: {} across {} processes",
        matches.len(),
        process_count
    )
}

fn set_terminal_search_results_browser(
    history_browser: &ListBox,
    matches: &[TerminalLogMatch],
    database_path: PathBuf,
    workspace_name: String,
    buffer: TextBuffer,
    jump_history_pages: Rc<RefCell<HashMap<i64, usize>>>,
    toast_manager: ToastManager,
) {
    while let Some(child) = history_browser.first_child() {
        history_browser.remove(&child);
    }

    if matches.is_empty() {
        let empty = Label::new(Some("No terminal transcript matches."));
        empty.set_xalign(0.0);
        history_browser.append(&empty);
        return;
    }

    let summary_label = Label::new(Some(&format_terminal_search_result_summary(matches)));
    summary_label.set_xalign(0.0);
    summary_label.set_hexpand(true);
    let summary_row = GBox::new(Orientation::Horizontal, 8);
    summary_row.append(&summary_label);
    history_browser.append(&summary_row);

    let mut grouped_matches: Vec<(i64, Vec<&TerminalLogMatch>)> = Vec::new();
    for item in matches {
        match grouped_matches
            .iter_mut()
            .find(|(pid, _)| *pid == item.process_id)
        {
            Some((_, grouped_items)) => grouped_items.push(item),
            None => grouped_matches.push((item.process_id, vec![item])),
        }
    }

    for (process_id, grouped_items) in grouped_matches {
        let first = grouped_items
            .first()
            .expect("grouped terminal matches should never be empty");
        let latest_match_line = grouped_items
            .iter()
            .map(|item| item.line_number)
            .max()
            .unwrap_or(1);
        let file_name = first
            .log_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("terminal.log");
        let header_label = Label::new(Some(&format!(
            "#{} {} [{}] · {} match(es)",
            process_id,
            first.command.trim(),
            file_name,
            grouped_items.len()
        )));
        header_label.set_xalign(0.0);
        header_label.set_hexpand(true);
        let header_row = GBox::new(Orientation::Horizontal, 8);
        header_row.append(&header_label);

        let open_transcript_btn = text_button("Open transcript");
        open_transcript_btn.set_tooltip_text(Some("Open full transcript for this process"));
        let open_transcript_btn_db = database_path.clone();
        let open_transcript_btn_workspace = workspace_name.clone();
        let open_transcript_btn_buffer = buffer.clone();
        let open_tail_btn_db = database_path.clone();
        let open_tail_btn_workspace = workspace_name.clone();
        let open_tail_btn_buffer = buffer.clone();
        let open_transcript_btn_jump_pages = jump_history_pages.clone();
        let open_transcript_btn_toast = toast_manager.clone();
        open_transcript_btn.connect_clicked(move |_| {
            open_transcript_btn_jump_pages
                .borrow_mut()
                .insert(process_id, latest_match_line);
            load_terminal_tab_transcript(
                open_transcript_btn_db.clone(),
                open_transcript_btn_workspace.clone(),
                process_id,
                open_transcript_btn_buffer.clone(),
                open_transcript_btn_toast.clone(),
            );
        });
        header_row.append(&open_transcript_btn);

        let tail_transcript_btn = text_button("Tail output");
        tail_transcript_btn.set_tooltip_text(Some("Load only tail output for this process"));
        let tail_transcript_btn_jump_pages = jump_history_pages.clone();
        let tail_transcript_btn_toast = toast_manager.clone();
        tail_transcript_btn.connect_clicked(move |_| {
            tail_transcript_btn_jump_pages
                .borrow_mut()
                .insert(process_id, latest_match_line);
            run_terminal_tail_transcript(
                open_tail_btn_db.clone(),
                open_tail_btn_workspace.clone(),
                process_id,
                open_tail_btn_buffer.clone(),
                tail_transcript_btn_toast.clone(),
            );
        });
        header_row.append(&tail_transcript_btn);
        history_browser.append(&header_row);

        for item in grouped_items {
            let file_name = item
                .log_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("terminal.log");
            let context = format!(
                "before={} after={}",
                item.context_before.len(),
                item.context_after.len()
            );
            let snippet = terminal_search_match_snippet(item);
            let label = format!(
                "#{} {} [{}]\n{}\n{}",
                item.process_id, item.command, context, file_name, snippet
            );
            let row = GBox::new(Orientation::Horizontal, 8);
            let match_row = text_button(&label);
            match_row.set_hexpand(true);
            let row_db = database_path.clone();
            let row_workspace = workspace_name.clone();
            let row_buffer = buffer.clone();
            let row_toast = toast_manager.clone();
            let process_id = item.process_id;
            let line_hint = item.line_number;
            let command = item.command.clone();
            let file_name_for_tooltip = file_name.to_owned();
            let tooltip = format!("{command} (line {line_hint} in {file_name_for_tooltip})");
            let match_row_jump_pages = jump_history_pages.clone();
            match_row.set_tooltip_text(Some(&tooltip));
            match_row.connect_clicked(move |_| {
                match_row_jump_pages
                    .borrow_mut()
                    .insert(process_id, line_hint);
                append_text(
                    &row_buffer,
                    &format!(
                        "\n[terminal match #{}] loading matching line {}\n",
                        process_id, line_hint
                    ),
                );
                run_terminal_match_transcript(
                    row_db.clone(),
                    row_workspace.clone(),
                    process_id,
                    line_hint,
                    row_buffer.clone(),
                    row_toast.clone(),
                );
            });

            let open_btn = text_button("Open session");
            open_btn.set_tooltip_text(Some("Load full session transcript"));
            let open_btn_db = database_path.clone();
            let open_btn_workspace = workspace_name.to_owned();
            let open_btn_buffer = buffer.clone();
            let open_btn_process = item.process_id;
            let open_btn_jump_pages = jump_history_pages.clone();
            let open_btn_toast = toast_manager.clone();
            open_btn.connect_clicked(move |_| {
                open_btn_jump_pages
                    .borrow_mut()
                    .insert(open_btn_process, line_hint);
                load_terminal_tab_transcript(
                    open_btn_db.clone(),
                    open_btn_workspace.clone(),
                    open_btn_process,
                    open_btn_buffer.clone(),
                    open_btn_toast.clone(),
                );
            });

            let tail_btn = text_button("Tail session");
            tail_btn.set_tooltip_text(Some("Load latest lines from this session"));
            let tail_btn_db = database_path.clone();
            let tail_btn_workspace = workspace_name.to_owned();
            let tail_btn_buffer = buffer.clone();
            let tail_btn_process = item.process_id;
            let tail_btn_jump_pages = jump_history_pages.clone();
            let tail_btn_toast = toast_manager.clone();
            tail_btn.connect_clicked(move |_| {
                tail_btn_jump_pages
                    .borrow_mut()
                    .insert(tail_btn_process, line_hint);
                run_terminal_tail_transcript(
                    tail_btn_db.clone(),
                    tail_btn_workspace.clone(),
                    tail_btn_process,
                    tail_btn_buffer.clone(),
                    tail_btn_toast.clone(),
                );
            });

            row.append(&match_row);
            row.append(&open_btn);
            row.append(&tail_btn);
            history_browser.append(&row);
        }
    }
}

fn terminal_search_match_snippet(match_record: &TerminalLogMatch) -> String {
    let mut snippet = String::new();
    if !match_record.context_before.is_empty() {
        snippet.push_str("...\n");
        let before_start = match_record
            .line_number
            .saturating_sub(match_record.context_before.len());
        for (offset, line) in match_record.context_before.iter().enumerate() {
            let line_number = before_start + offset + 1;
            snippet.push_str(&format!("{line_number:>6}  {line}\n"));
        }
    }
    snippet.push_str(&format!(
        "{:>6}> {}\n",
        match_record.line_number, match_record.line
    ));
    if !match_record.context_after.is_empty() {
        let after_start = match_record.line_number.saturating_add(1);
        for (offset, line) in match_record.context_after.iter().enumerate() {
            let line_number = after_start + offset;
            snippet.push_str(&format!("{line_number:>6}   {line}\n"));
        }
        snippet.push_str("...\n");
    }
    truncate_text_for_display(&snippet, 220)
}

fn format_terminal_history(summaries: &[TerminalSessionSummary]) -> String {
    let mut text = "\n[terminal history]\n".to_owned();
    if summaries.is_empty() {
        text.push_str("No terminal shells recorded.\n");
        return text;
    }

    let running = summaries
        .iter()
        .filter(|summary| summary.process.status == ProcessStatus::Running)
        .count();
    let stopped = summaries
        .iter()
        .filter(|summary| summary.process.status == ProcessStatus::Stopped)
        .count();
    let exited = summaries
        .iter()
        .filter(|summary| summary.process.status == ProcessStatus::Exited)
        .count();
    text.push_str(&format!(
        "{} sessions: {} running, {} stopped, {} exited\n",
        summaries.len(),
        running,
        stopped,
        exited
    ));

    let sorted_summaries = terminal_history_summaries_for_display(summaries);

    for summary in &sorted_summaries {
        let record = &summary.process;
        let file_name = record
            .log_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("terminal.log");
        text.push_str(&format!(
            "#{} {} pid={} exit={} log={} started={}\n{} lines, {} bytes\npreview: {}\n{}\n",
            record.id,
            record.status.as_str(),
            record.pid,
            terminal_exit_label(record.exit_code),
            file_name,
            record.started_at,
            summary.line_count,
            summary.byte_count,
            summary.preview,
            record.command
        ));
    }
    text
}

fn terminal_history_summaries_for_display(
    summaries: &[TerminalSessionSummary],
) -> Vec<TerminalSessionSummary> {
    let mut sorted_summaries = summaries.to_vec();
    sorted_summaries.sort_by(|left, right| right.process.started_at.cmp(&left.process.started_at));
    sorted_summaries
}

fn terminal_history_option_label(summary: &TerminalSessionSummary) -> String {
    let record = &summary.process;
    let file_name = record
        .log_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("terminal.log");
    format!(
        "#{} {} pid={} {} lines {}",
        record.id,
        record.status.as_str(),
        record.pid,
        summary.line_count,
        file_name
    )
}

fn terminal_history_browser_row_label(summary: &TerminalSessionSummary) -> String {
    let record = &summary.process;
    let preview = truncate_text_for_display(&summary.preview, 120);
    format!(
        "#{} {} pid={} exit={} · {} lines · {} bytes · {} · {}",
        record.id,
        record.status.as_str(),
        record.pid,
        terminal_exit_label(record.exit_code),
        summary.line_count,
        summary.byte_count,
        record.started_at,
        preview
    )
}

fn truncate_text_for_display(text: &str, max_chars: usize) -> String {
    let mut output = text.chars().take(max_chars).collect::<String>();
    if text.chars().count() > max_chars {
        output.push('…');
    }
    output
}

fn set_terminal_history_browser(
    history_browser: &ListBox,
    history_combo: &ComboBoxText,
    summaries: &[TerminalSessionSummary],
    database_path: PathBuf,
    workspace_name: String,
    buffer: TextBuffer,
    jump_history_pages: Rc<RefCell<HashMap<i64, usize>>>,
    toast_manager: ToastManager,
) {
    while let Some(child) = history_browser.first_child() {
        history_browser.remove(&child);
    }

    if summaries.is_empty() {
        let empty = Label::new(Some("No terminal sessions match the current filter."));
        empty.set_xalign(0.0);
        history_browser.append(&empty);
        return;
    }

    let total_label = Label::new(Some(&format!("Terminal sessions: {}", summaries.len())));
    total_label.set_xalign(0.0);
    total_label.set_hexpand(true);
    let total_row = GBox::new(Orientation::Horizontal, 8);
    total_row.append(&total_label);
    history_browser.append(&total_row);

    let mut grouped_summaries: Vec<(ProcessStatus, Vec<&TerminalSessionSummary>)> = vec![
        (ProcessStatus::Running, Vec::new()),
        (ProcessStatus::Stopped, Vec::new()),
        (ProcessStatus::Exited, Vec::new()),
    ];
    for summary in summaries {
        if let Some((_, grouped)) = grouped_summaries
            .iter_mut()
            .find(|(status, _)| *status == summary.process.status)
        {
            grouped.push(summary);
        }
    }

    for (status, grouped) in grouped_summaries {
        if grouped.is_empty() {
            continue;
        }

        let section_label = Label::new(Some(&format!(
            "{} sessions · {}",
            status.as_str(),
            grouped.len()
        )));
        section_label.set_xalign(0.0);
        section_label.set_hexpand(true);
        let section_row = GBox::new(Orientation::Horizontal, 8);
        section_row.append(&section_label);
        history_browser.append(&section_row);

        for summary in grouped {
            let file_name = summary
                .process
                .log_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("terminal.log");
            let meta = format!(
                "{} · {}",
                terminal_history_browser_row_label(summary),
                file_name
            );
            let command = truncate_text_for_display(
                summary.process.command.trim().replace('\n', " ").as_str(),
                120,
            );
            let row_label = if command.is_empty() {
                meta
            } else {
                format!("{meta}\n{command}")
            };
            let record = summary.process.clone();
            let row = GBox::new(Orientation::Horizontal, 8);
            let latest_line = summary.line_count.max(1);

            let row_button = text_button(&row_label);
            row_button.set_hexpand(true);
            row_button.set_tooltip_text(Some(&record.command));
            let row_combo = history_combo.clone();
            let row_combo_for_row_button = row_combo.clone();
            let row_combo_for_open_btn = row_combo.clone();
            let row_combo_for_tail_btn = row_combo.clone();
            let row_combo_for_head_btn = row_combo.clone();
            let row_combo_for_jump_prev_btn = row_combo.clone();
            let row_combo_for_jump_next_btn = row_combo.clone();
            let row_combo_for_jump_latest_btn = row_combo.clone();
            let row_db = database_path.clone();
            let row_workspace = workspace_name.clone();
            let row_buffer = buffer.clone();
            let row_record = record.clone();
            let row_toast = toast_manager.clone();
            let row_button_db = row_db.clone();
            let row_button_workspace = row_workspace.clone();
            let row_button_buffer = row_buffer.clone();
            let row_button_record = row_record.clone();
            let row_button_jump_pages = jump_history_pages.clone();
            let row_button_toast = row_toast.clone();
            row_button.connect_clicked(move |_| {
                row_combo_for_row_button.set_active_id(Some(&row_button_record.id.to_string()));
                row_button_jump_pages
                    .borrow_mut()
                    .insert(row_button_record.id, latest_line);
                run_terminal_transcript_load(
                    row_button_db.clone(),
                    row_button_workspace.clone(),
                    row_button_record.clone(),
                    row_button_buffer.clone(),
                    row_button_toast.clone(),
                );
            });

            let open_btn = text_button("Load transcript");
            open_btn.set_tooltip_text(Some("Load selected session transcript"));
            let open_btn_db = row_db.clone();
            let open_btn_workspace = row_workspace.clone();
            let open_btn_buffer = row_buffer.clone();
            let open_btn_record = row_record.clone();
            let open_btn_combo = row_combo_for_open_btn.clone();
            let open_btn_latest_line = latest_line;
            let open_btn_jump_pages = jump_history_pages.clone();
            let open_btn_toast = row_toast.clone();
            let tail_btn_db = row_db.clone();
            let tail_btn_workspace = row_workspace.clone();
            let tail_btn_buffer = row_buffer.clone();
            let tail_btn_record = row_record.clone();
            let tail_btn_toast = row_toast.clone();
            open_btn.connect_clicked(move |_| {
                open_btn_combo.set_active_id(Some(&open_btn_record.id.to_string()));
                open_btn_jump_pages
                    .borrow_mut()
                    .insert(open_btn_record.id, open_btn_latest_line);
                run_terminal_transcript_load(
                    open_btn_db.clone(),
                    open_btn_workspace.clone(),
                    open_btn_record.clone(),
                    open_btn_buffer.clone(),
                    open_btn_toast.clone(),
                );
            });

            let tail_btn = text_button("Tail output");
            tail_btn.set_tooltip_text(Some("Load latest lines only"));
            let tail_btn_latest_line = latest_line;
            let tail_btn_jump_pages = jump_history_pages.clone();
            tail_btn.connect_clicked(move |_| {
                row_combo_for_tail_btn.set_active_id(Some(&tail_btn_record.id.to_string()));
                tail_btn_jump_pages
                    .borrow_mut()
                    .insert(tail_btn_record.id, tail_btn_latest_line);
                run_terminal_tail_transcript(
                    tail_btn_db.clone(),
                    tail_btn_workspace.clone(),
                    tail_btn_record.id,
                    tail_btn_buffer.clone(),
                    tail_btn_toast.clone(),
                );
            });

            let head_btn = text_button("Head output");
            head_btn.set_tooltip_text(Some("Load first lines only"));
            let head_btn_db = row_db.clone();
            let head_btn_workspace = row_workspace.clone();
            let head_btn_buffer = row_buffer.clone();
            let head_btn_record = row_record.clone();
            let head_btn_combo = row_combo_for_head_btn.clone();
            let head_btn_latest_line = latest_line;
            let head_btn_jump_pages = jump_history_pages.clone();
            let head_btn_toast = row_toast.clone();
            head_btn.connect_clicked(move |_| {
                head_btn_combo.set_active_id(Some(&head_btn_record.id.to_string()));
                head_btn_jump_pages
                    .borrow_mut()
                    .insert(head_btn_record.id, head_btn_latest_line);
                run_terminal_head_transcript(
                    head_btn_db.clone(),
                    head_btn_workspace.clone(),
                    head_btn_record.id,
                    head_btn_buffer.clone(),
                    head_btn_toast.clone(),
                );
            });

            let jump_latest_btn = text_button("Jump latest line");
            jump_latest_btn.set_tooltip_text(Some("Load around the latest transcript line"));
            let jump_latest_btn_db = row_db.clone();
            let jump_latest_btn_workspace = row_workspace.clone();
            let jump_latest_btn_buffer = row_buffer.clone();
            let jump_latest_btn_record = row_record.clone();
            let jump_latest_btn_combo = row_combo_for_jump_latest_btn.clone();
            let jump_latest_btn_line = latest_line;
            let jump_latest_pages = jump_history_pages.clone();
            let jump_latest_btn_toast = row_toast.clone();
            jump_latest_btn.connect_clicked(move |_| {
                jump_latest_btn_combo.set_active_id(Some(&jump_latest_btn_record.id.to_string()));
                jump_latest_pages
                    .borrow_mut()
                    .insert(jump_latest_btn_record.id, jump_latest_btn_line);
                run_terminal_line_transcript(
                    jump_latest_btn_db.clone(),
                    jump_latest_btn_workspace.clone(),
                    jump_latest_btn_record.id,
                    jump_latest_btn_line,
                    TERMINAL_LINE_JUMP_CONTEXT,
                    jump_latest_btn_buffer.clone(),
                    jump_latest_btn_toast.clone(),
                );
            });

            let jump_prev_btn = text_button(&format!("Prev {TERMINAL_LINE_JUMP_PAGE_SIZE} lines"));
            jump_prev_btn.set_tooltip_text(Some("Jump back through transcript pages"));
            let jump_prev_btn_db = row_db.clone();
            let jump_prev_btn_workspace = row_workspace.clone();
            let jump_prev_btn_buffer = row_buffer.clone();
            let jump_prev_btn_record = row_record.clone();
            let jump_prev_btn_combo = row_combo_for_jump_prev_btn.clone();
            let jump_prev_pages = jump_history_pages.clone();
            let jump_prev_line = latest_line;
            let jump_prev_btn_toast = row_toast.clone();
            jump_prev_btn.connect_clicked(move |_| {
                jump_prev_btn_combo.set_active_id(Some(&jump_prev_btn_record.id.to_string()));
                let existing_line = jump_prev_pages
                    .borrow()
                    .get(&jump_prev_btn_record.id)
                    .copied()
                    .unwrap_or(jump_prev_line);
                let line_number = terminal_line_jump_target(
                    existing_line,
                    -(TERMINAL_LINE_JUMP_PAGE_SIZE as isize),
                );
                jump_prev_pages
                    .borrow_mut()
                    .insert(jump_prev_btn_record.id, line_number);
                run_terminal_line_transcript(
                    jump_prev_btn_db.clone(),
                    jump_prev_btn_workspace.clone(),
                    jump_prev_btn_record.id,
                    line_number,
                    TERMINAL_LINE_JUMP_CONTEXT,
                    jump_prev_btn_buffer.clone(),
                    jump_prev_btn_toast.clone(),
                );
            });

            let jump_next_btn = text_button(&format!("Next {TERMINAL_LINE_JUMP_PAGE_SIZE} lines"));
            jump_next_btn.set_tooltip_text(Some("Jump forward through transcript pages"));
            let jump_next_btn_db = row_db.clone();
            let jump_next_btn_workspace = row_workspace.clone();
            let jump_next_btn_buffer = row_buffer.clone();
            let jump_next_btn_record = row_record.clone();
            let jump_next_btn_combo = row_combo_for_jump_next_btn.clone();
            let jump_next_pages = jump_history_pages.clone();
            let jump_next_line = latest_line;
            let jump_next_btn_toast = row_toast.clone();
            jump_next_btn.connect_clicked(move |_| {
                jump_next_btn_combo.set_active_id(Some(&jump_next_btn_record.id.to_string()));
                let existing_line = jump_next_pages
                    .borrow()
                    .get(&jump_next_btn_record.id)
                    .copied()
                    .unwrap_or(jump_next_line);
                let line_number =
                    terminal_line_jump_target(existing_line, TERMINAL_LINE_JUMP_PAGE_SIZE as isize);
                jump_next_pages
                    .borrow_mut()
                    .insert(jump_next_btn_record.id, line_number);
                run_terminal_line_transcript(
                    jump_next_btn_db.clone(),
                    jump_next_btn_workspace.clone(),
                    jump_next_btn_record.id,
                    line_number,
                    TERMINAL_LINE_JUMP_CONTEXT,
                    jump_next_btn_buffer.clone(),
                    jump_next_btn_toast.clone(),
                );
            });

            row.append(&row_button);
            row.append(&open_btn);
            row.append(&tail_btn);
            row.append(&head_btn);
            row.append(&jump_latest_btn);
            row.append(&jump_prev_btn);
            row.append(&jump_next_btn);
            history_browser.append(&row);
        }
    }
}

fn terminal_history_filter_status(history_filter: &ComboBoxText) -> Option<ProcessStatus> {
    match history_filter.active_id().as_deref() {
        Some("running") => Some(ProcessStatus::Running),
        Some("stopped") => Some(ProcessStatus::Stopped),
        Some("exited") => Some(ProcessStatus::Exited),
        _ => None,
    }
}

fn terminal_history_summaries_for_filter(
    summaries: &[TerminalSessionSummary],
    status: Option<ProcessStatus>,
) -> Vec<TerminalSessionSummary> {
    terminal_history_summaries_for_filter_with_query(summaries, status, None)
}

fn terminal_history_summaries_for_filter_with_query(
    summaries: &[TerminalSessionSummary],
    status: Option<ProcessStatus>,
    query: Option<&str>,
) -> Vec<TerminalSessionSummary> {
    let query = query.unwrap_or("").trim().to_lowercase();
    let has_query = !query.is_empty();

    summaries
        .iter()
        .filter(|summary| match status {
            Some(status) => summary.process.status == status,
            None => true,
        })
        .filter(|summary| {
            if !has_query {
                return true;
            }
            let haystack = format!(
                "{} {} {} {} {} {}",
                summary.process.id,
                summary.process.command,
                summary.process.status.as_str(),
                summary.process.pid,
                summary.process.exit_code.unwrap_or(-1),
                summary.preview,
            )
            .to_lowercase();
            haystack.contains(&query)
        })
        .cloned()
        .collect()
}

fn set_terminal_history_combo(
    history_combo: &ComboBoxText,
    summaries: &[TerminalSessionSummary],
    preserved_selection: Option<i64>,
) {
    history_combo.remove_all();
    for summary in summaries {
        history_combo.append(
            Some(&summary.process.id.to_string()),
            &terminal_history_option_label(summary),
        );
    }
    if !summaries.is_empty() {
        if let Some(preserved_selection) = preserved_selection {
            if let Some(index) = summaries
                .iter()
                .position(|summary| summary.process.id == preserved_selection)
            {
                history_combo.set_active(Some(index as u32));
                return;
            }
        }
        history_combo.set_active(Some(0));
    }
}

fn active_terminal_session_option_label(_index: usize, process_id: Option<i64>) -> String {
    match process_id {
        Some(process_id) => format!("Terminal #{}", process_id),
        None => "Terminal".to_owned(),
    }
}

fn active_terminal_tab_label(
    _index: usize,
    _process_id: Option<i64>,
    _status: ProcessStatus,
    _attached: bool,
) -> String {
    "Terminal".to_owned()
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

fn rebuild_terminal_tabs(
    terminal_tabs: &GBox,
    active_pty_combo: &ComboBoxText,
    tab_buttons: &Rc<RefCell<Vec<Button>>>,
    terminal_tab_states: &Rc<RefCell<Vec<TerminalTabState>>>,
    database_path: PathBuf,
    workspace_name: String,
    transcript_buffer: TextBuffer,
    active_index: Option<usize>,
    toast_manager: ToastManager,
) {
    while let Some(child) = terminal_tabs.first_child() {
        terminal_tabs.remove(&child);
    }

    active_pty_combo.remove_all();
    {
        let mut tabs = tab_buttons.borrow_mut();
        tabs.clear();
    }

    let Ok(states_ref) = terminal_tab_states.try_borrow() else {
        return;
    };
    let states = states_ref.clone();
    let effective_active = active_index.filter(|index| *index < states.len());

    for (index, state) in states.iter().enumerate() {
        active_pty_combo.append(
            Some(&index.to_string()),
            &active_terminal_session_option_label(index, Some(state.process_id)),
        );
        let tab = text_button(&active_terminal_tab_label(
            index,
            Some(state.process_id),
            state.status,
            state.attached,
        ));
        tab.add_css_class("flat");
        let active_pty_combo_for_tab = active_pty_combo.clone();
        let tab_buttons_for_tab = tab_buttons.clone();
        let database_path_for_tab = database_path.clone();
        let workspace_for_tab = workspace_name.clone();
        let buffer_for_tab = transcript_buffer.clone();
        let terminal_tab_states_for_tab = terminal_tab_states.clone();
        let index_for_tab = index;
        let toast_for_tab = toast_manager.clone();
        tab.connect_clicked(move |_| {
            active_pty_combo_for_tab.set_active(Some(index_for_tab as u32));
            set_terminal_tab_active(&tab_buttons_for_tab.borrow(), Some(index_for_tab));
            if terminal_tab_states_for_tab
                .borrow()
                .get(index_for_tab)
                .is_some_and(|state| state.is_running() && state.attached)
            {
                return;
            }
            if let Some(state) = terminal_tab_states_for_tab.borrow().get(index_for_tab) {
                load_terminal_tab_transcript(
                    database_path_for_tab.clone(),
                    workspace_for_tab.clone(),
                    state.process_id,
                    buffer_for_tab.clone(),
                    toast_for_tab.clone(),
                );
            }
        });

        terminal_tabs.append(&tab);
        tab_buttons.borrow_mut().push(tab);
    }

    if let Some(active_index) = effective_active {
        active_pty_combo.set_active(Some(active_index as u32));
    } else {
        active_pty_combo.set_active(None);
    }
    set_terminal_tab_active(&tab_buttons.borrow(), effective_active);
}

fn next_active_terminal_tab(current_index: usize, running_tabs: &[bool]) -> Option<usize> {
    if running_tabs.is_empty() {
        return None;
    }

    running_tabs
        .iter()
        .enumerate()
        .skip(current_index.saturating_add(1))
        .find_map(|(index, running)| (*running).then_some(index))
        .or_else(|| {
            running_tabs
                .iter()
                .enumerate()
                .take(current_index)
                .find_map(|(index, running)| (*running).then_some(index))
        })
        .or_else(|| Some(current_index.min(running_tabs.len() - 1)))
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

pub(crate) fn terminal_refresh_event(
    workspace: &str,
    outcome: TerminalRefreshOutcome,
) -> Option<RefreshEvent> {
    outcome
        .process_changed
        .then(|| RefreshEvent::TerminalChanged {
            workspace: workspace.to_owned(),
        })
}

fn notify_terminal_refresh(
    refresh_hub: &RefreshHub,
    workspace: &str,
    outcome: TerminalRefreshOutcome,
) {
    if let Some(event) = terminal_refresh_event(workspace, outcome) {
        refresh_hub.refresh_event(event);
    }
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
    preferences: &TerminalPreferences,
) -> String {
    let restored = WorkspaceStore::open_app(database_path)
        .and_then(|store| store.read_latest_terminal_log(workspace_name))
        .ok()
        .filter(|log| !log.trim().is_empty());
    format_initial_terminal_text(
        workspace_name,
        workspace_path,
        restored.as_deref(),
        preferences,
    )
}

fn format_initial_terminal_text(
    workspace_name: &str,
    workspace_path: &Path,
    restored_transcript: Option<&str>,
    preferences: &TerminalPreferences,
) -> String {
    let mut text = format!(
        "Workspace terminal\nworkspace: {}\npath: {}\n{}\n\nCommands run here execute inside the workspace with ARCHDUCTOR_* environment variables.",
        workspace_name,
        workspace_path.display(),
        preferences.summary()
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
    trim_terminal_buffer(buffer, terminal_buffer_scrollback(buffer));
}

fn append_terminal_text(existing: &str, incoming: &str, max_lines: usize) -> String {
    trim_terminal_scrollback(
        &format!("{existing}{}", terminal_display_text(incoming)),
        max_lines,
    )
}

fn set_terminal_buffer_scrollback(buffer: &TextBuffer, max_lines: usize) {
    TERMINAL_BUFFER_SCROLLBACK.with(|limits| {
        limits
            .borrow_mut()
            .insert(terminal_buffer_key(buffer), max_lines);
    });
}

fn terminal_buffer_scrollback(buffer: &TextBuffer) -> usize {
    TERMINAL_BUFFER_SCROLLBACK.with(|limits| {
        limits
            .borrow()
            .get(&terminal_buffer_key(buffer))
            .copied()
            .unwrap_or(TERMINAL_SCROLLBACK_LINES)
    })
}

fn terminal_buffer_key(buffer: &TextBuffer) -> usize {
    buffer.as_ptr() as usize
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

fn apply_terminal_preferences(transcript: &TextView, preferences: &TerminalPreferences) {
    transcript.set_tooltip_text(Some(&preferences.summary()));
    let Some(font) = preferences.font.as_deref() else {
        return;
    };
    let class_name = terminal_font_class(font);
    transcript.add_css_class(&class_name);
    let css = format!(
        "textview.{class_name} {{ {} }}",
        terminal_font_css_declarations(font)
    );
    let provider = CssProvider::new();
    provider.load_from_data(&css);
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn terminal_font_class(font: &str) -> String {
    let mut hasher = DefaultHasher::new();
    font.hash(&mut hasher);
    format!("terminal-font-{:x}", hasher.finish())
}

fn terminal_font_css_declarations(font: &str) -> String {
    let trimmed = font.trim();
    let mut parts = trimmed.rsplitn(2, ' ');
    let last = parts.next().unwrap_or(trimmed);
    let maybe_family = parts.next();
    if let (Some(family), Ok(size)) = (maybe_family, last.parse::<u16>()) {
        if !family.trim().is_empty() {
            return format!(
                "font-family: \"{}\"; font-size: {size}pt;",
                css_string_escape(family.trim())
            );
        }
    }
    format!("font-family: \"{}\";", css_string_escape(trimmed))
}

fn css_string_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

pub(crate) fn terminal_display_text(text: &str) -> String {
    let mut rendered = Vec::new();
    let mut cursor = None;
    let mut saved_cursor = None;
    let mut normal_screen = None;
    let mut absolute_position_cursor = false;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                cursor = Some(line_start(&rendered));
                continue;
            }
            '\u{7f}' => {
                cursor = Some(move_terminal_display_cursor_left(
                    &rendered,
                    cursor.unwrap_or(rendered.len()),
                    1,
                ));
                continue;
            }
            '\u{8}' => {
                cursor = Some(move_terminal_display_cursor_left(
                    &rendered,
                    cursor.unwrap_or(rendered.len()),
                    1,
                ));
                continue;
            }
            '\u{0b}' => {
                cursor = Some(line_start(&rendered));
                continue;
            }
            '\u{0c}' => {
                rendered.clear();
                cursor = Some(0);
                continue;
            }
            '\t' => {
                let cursor_position = cursor.unwrap_or(rendered.len());
                let line_start = line_start_before(&rendered, cursor_position);
                let target = line_start + ((cursor_position - line_start) / 8 + 1) * 8;
                let count = target.saturating_sub(cursor_position);
                if cursor.is_some() {
                    for _ in 0..count {
                        push_terminal_display_char(&mut rendered, &mut cursor, ' ');
                    }
                } else if count > 0 {
                    rendered.resize(cursor_position + count, ' ');
                }
                continue;
            }
            '\n' => {
                if let Some(position) = cursor {
                    if absolute_position_cursor {
                        let line_end = line_end_after(&rendered, position);
                        if position < line_end {
                            rendered.drain(position..position + 1);
                        }
                    }
                    absolute_position_cursor = false;
                    if rendered.get(position) == Some(&'\n') {
                        cursor = None;
                        continue;
                    }
                    if rendered[position.min(rendered.len())..].contains(&'\n') {
                        cursor = None;
                        continue;
                    }
                }
                cursor = None;
                rendered.push(ch);
                continue;
            }
            '\u{7}' => continue,
            '\u{1b}' => {}
            _ if ch.is_control() => continue,
            _ => {}
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
                    Some('B') | Some('e') => {
                        cursor = Some(move_terminal_display_cursor_down(
                            &rendered,
                            cursor.unwrap_or(rendered.len()),
                            csi_first_number(&sequence, 1),
                        ));
                    }
                    Some('C') | Some('a') => {
                        cursor = Some(move_terminal_display_cursor_right(
                            &rendered,
                            cursor.unwrap_or(rendered.len()),
                            csi_first_number(&sequence, 1),
                        ));
                    }
                    Some('D') => {
                        cursor = Some(move_terminal_display_cursor_left(
                            &rendered,
                            cursor.unwrap_or(rendered.len()),
                            csi_first_number(&sequence, 1),
                        ));
                    }
                    Some('E') => {
                        cursor = Some(move_terminal_display_cursor_next_line(
                            &rendered,
                            cursor.unwrap_or(rendered.len()),
                            csi_first_number(&sequence, 1),
                        ));
                    }
                    Some('F') => {
                        cursor = Some(move_terminal_display_cursor_previous_line(
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
                    Some('d') => {
                        cursor = Some(move_terminal_display_cursor_to_line(
                            &rendered,
                            cursor.unwrap_or(rendered.len()),
                            csi_first_number(&sequence, 1),
                        ));
                        absolute_position_cursor = true;
                    }
                    Some('H') => {
                        cursor = Some(move_terminal_display_cursor_to_position(
                            &rendered,
                            csi_numbers(&sequence),
                        ));
                        absolute_position_cursor = true;
                    }
                    Some('f') => {
                        cursor = Some(move_terminal_display_cursor_to_position(
                            &rendered,
                            csi_numbers(&sequence),
                        ));
                        absolute_position_cursor = true;
                    }
                    Some('J') => {
                        let cursor_position = cursor.unwrap_or(rendered.len());
                        match csi_first_number(&sequence, 0) {
                            0 => {
                                rendered.truncate(cursor_position);
                            }
                            1 => {
                                rendered.drain(0..cursor_position.min(rendered.len()));
                                cursor = Some(0);
                            }
                            _ => {
                                rendered.clear();
                                cursor = Some(0);
                            }
                        }
                    }
                    Some('K') => {
                        let cursor_position = cursor.unwrap_or(rendered.len());
                        let mode = csi_first_number_with_zero(&sequence, 0);
                        cursor = Some(clear_terminal_display_line(
                            &mut rendered,
                            cursor_position,
                            mode,
                        ));
                    }
                    Some('P') => {
                        absolute_position_cursor = false;
                        let cursor_position = cursor.unwrap_or(rendered.len());
                        let count = csi_first_number_with_zero(&sequence, 1);
                        cursor = Some(delete_terminal_display_chars(
                            &mut rendered,
                            cursor_position,
                            count,
                        ));
                    }
                    Some('@') => {
                        absolute_position_cursor = false;
                        let cursor_position = cursor.unwrap_or(rendered.len());
                        let count = csi_first_number_with_zero(&sequence, 1);
                        cursor = Some(insert_terminal_display_chars(
                            &mut rendered,
                            cursor_position,
                            count,
                        ));
                    }
                    Some('X') => {
                        absolute_position_cursor = false;
                        let cursor_position = cursor.unwrap_or(rendered.len());
                        let count = csi_first_number_with_zero(&sequence, 1);
                        erase_terminal_display_chars(&mut rendered, cursor_position, count);
                    }
                    Some('L') => {
                        absolute_position_cursor = false;
                        let cursor_position = cursor.unwrap_or(rendered.len());
                        let count = csi_first_number_with_zero(&sequence, 1);
                        cursor = Some(insert_terminal_display_lines(
                            &mut rendered,
                            cursor_position,
                            count,
                        ));
                    }
                    Some('M') => {
                        absolute_position_cursor = false;
                        let cursor_position = cursor.unwrap_or(rendered.len());
                        let count = csi_first_number_with_zero(&sequence, 1);
                        cursor = Some(delete_terminal_display_lines(
                            &mut rendered,
                            cursor_position,
                            count,
                        ));
                    }
                    Some('S') => {
                        let cursor_position = cursor.unwrap_or(rendered.len());
                        let count = csi_first_number_with_zero(&sequence, 1);
                        cursor = Some(scroll_terminal_display_lines_up(
                            &mut rendered,
                            cursor_position,
                            count,
                        ));
                    }
                    Some('T') => {
                        let cursor_position = cursor.unwrap_or(rendered.len());
                        let count = csi_first_number_with_zero(&sequence, 1);
                        cursor = Some(scroll_terminal_display_lines_down(
                            &mut rendered,
                            cursor_position,
                            count,
                        ));
                    }
                    Some('m') => {}
                    Some('h') if csi_private_mode_enabled(&sequence, &["47", "1047", "1049"]) => {
                        if normal_screen.is_none() {
                            normal_screen = Some((
                                rendered.clone(),
                                cursor,
                                saved_cursor,
                                absolute_position_cursor,
                            ));
                        }
                        rendered.clear();
                        cursor = Some(0);
                        saved_cursor = None;
                        absolute_position_cursor = false;
                    }
                    Some('l') if csi_private_mode_enabled(&sequence, &["47", "1047", "1049"]) => {
                        if let Some((
                            saved_rendered,
                            saved_position,
                            saved_saved_cursor,
                            saved_absolute,
                        )) = normal_screen.take()
                        {
                            rendered = saved_rendered;
                            cursor = saved_position;
                            saved_cursor = saved_saved_cursor;
                            absolute_position_cursor = saved_absolute;
                        }
                    }
                    Some('s') => {
                        saved_cursor = Some(cursor.unwrap_or(rendered.len()));
                    }
                    Some('u') => {
                        cursor = Some(saved_cursor.unwrap_or(rendered.len()));
                    }
                    Some('7') => {
                        saved_cursor = Some(cursor.unwrap_or(rendered.len()));
                    }
                    Some('8') => {
                        cursor = Some(saved_cursor.unwrap_or(rendered.len()));
                    }
                    Some('n') => {}
                    _ => {}
                }
            }
            Some(']') => {
                chars.next();
                skip_osc_sequence(&mut chars);
            }
            Some('7') => {
                chars.next();
                saved_cursor = Some(cursor.unwrap_or(rendered.len()));
            }
            Some('8') => {
                chars.next();
                cursor = Some(saved_cursor.unwrap_or(rendered.len()));
            }
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    normalize_terminal_display_lines(rendered.into_iter().collect())
}

fn normalize_terminal_display_lines(text: String) -> String {
    let trailing_newlines = text.chars().rev().take_while(|ch| *ch == '\n').count();
    let mut lines = text
        .split('\n')
        .enumerate()
        .map(|(index, line)| {
            if index == 0 {
                line.to_owned()
            } else {
                line.trim_end_matches(' ').to_owned()
            }
        })
        .collect::<Vec<_>>();
    while lines.len() > 1 && lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    let mut normalized = lines.join("\n");
    for _ in 0..trailing_newlines {
        normalized.push('\n');
    }
    normalized
}

fn skip_osc_sequence(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while let Some(code) = chars.next() {
        if code == '\u{7}' {
            break;
        }
        if code == '\u{1b}' {
            if matches!(chars.peek(), Some('\\')) {
                chars.next();
            }
            break;
        }
        if code == '\u{9c}' {
            break;
        }
    }
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

fn clear_terminal_display_line(rendered: &mut Vec<char>, cursor: usize, mode: usize) -> usize {
    let line_start = line_start_before(rendered, cursor);
    let line_end = line_end_after(rendered, cursor);
    let start = match mode {
        1 | 2 => line_start,
        _ => cursor.min(rendered.len()),
    };
    let end = if mode == 1 {
        if cursor == line_start {
            line_end
        } else {
            cursor.saturating_add(1).min(line_end)
        }
    } else {
        line_end
    };
    if start >= end {
        return cursor.min(rendered.len());
    }
    if (mode == 0 && line_start == 0) || mode == 2 {
        let spaces = vec![' '; end.saturating_sub(start)];
        rendered.splice(start..end, spaces);
    } else {
        rendered.drain(start..end);
    }
    match mode {
        1 | 2 => line_start,
        _ => start,
    }
}

fn insert_terminal_display_chars(rendered: &mut Vec<char>, cursor: usize, count: usize) -> usize {
    if count == 0 {
        return cursor.min(rendered.len());
    }
    rendered.splice(
        cursor.min(rendered.len())..cursor.min(rendered.len()),
        std::iter::repeat_n(' ', count),
    );
    cursor.min(rendered.len())
}

fn delete_terminal_display_chars(rendered: &mut Vec<char>, cursor: usize, count: usize) -> usize {
    if count == 0 {
        return cursor.min(rendered.len());
    }
    let start = cursor.min(rendered.len());
    let end = (start.saturating_add(count)).min(line_end_after(rendered, start));
    if start < end {
        rendered.drain(start..end);
    } else if start == rendered.len() {
        rendered.push(' ');
    }
    start.min(rendered.len())
}

fn erase_terminal_display_chars(rendered: &mut Vec<char>, cursor: usize, count: usize) {
    if count == 0 {
        return;
    }
    let start = cursor.min(rendered.len());
    let end = (start.saturating_add(count)).min(line_end_after(rendered, start));
    let spaces = vec![' '; end.saturating_sub(start)];
    rendered.splice(start..end, spaces);
}

fn insert_terminal_display_lines(rendered: &mut Vec<char>, cursor: usize, count: usize) -> usize {
    if count == 0 {
        return cursor.min(rendered.len());
    }
    let mut inserted = 0;
    while inserted < count {
        let line_end = line_end_after(rendered, cursor.min(rendered.len()));
        let insertion_point = if line_end < rendered.len() {
            line_end.saturating_add(1)
        } else {
            rendered.len()
        };
        rendered.insert(insertion_point, '\n');
        inserted += 1;
    }
    cursor.min(rendered.len())
}

fn delete_terminal_display_lines(rendered: &mut Vec<char>, cursor: usize, count: usize) -> usize {
    if count == 0 || rendered.is_empty() {
        return cursor.min(rendered.len());
    }
    let mut deleted = 0;
    let lines_to_delete = if cursor == 0 {
        count.saturating_add(1)
    } else {
        count
    };
    while deleted < lines_to_delete && !rendered.is_empty() {
        let safe_cursor = cursor.min(rendered.len());
        let line_start = line_start_before(rendered, safe_cursor);
        if line_start >= rendered.len() {
            break;
        }
        let line_end = line_end_after(rendered, line_start);
        if line_end < rendered.len() {
            rendered.drain(line_start..=line_end);
        } else {
            rendered.drain(line_start..rendered.len());
        }
        deleted += 1;
    }
    cursor.min(rendered.len())
}

fn scroll_terminal_display_lines_up(
    rendered: &mut Vec<char>,
    cursor: usize,
    count: usize,
) -> usize {
    if count == 0 || rendered.is_empty() {
        return cursor.min(rendered.len());
    }
    let mut scrolled = 0;
    while scrolled < count && !rendered.is_empty() {
        let first_newline = rendered.iter().position(|ch| *ch == '\n');
        if let Some(newline_index) = first_newline {
            rendered.drain(0..=newline_index);
        } else {
            rendered.clear();
            rendered.push('\n');
            break;
        }
        rendered.push('\n');
        scrolled += 1;
    }
    cursor.min(rendered.len())
}

fn scroll_terminal_display_lines_down(
    rendered: &mut Vec<char>,
    cursor: usize,
    count: usize,
) -> usize {
    if count == 0 || rendered.is_empty() {
        return cursor.min(rendered.len());
    }
    let mut scrolled = 0;
    while scrolled < count && !rendered.is_empty() {
        let last_newline = rendered.iter().rposition(|ch| *ch == '\n');
        if let Some(newline_index) = last_newline {
            let remove_start = if newline_index > 0 {
                line_start_before(rendered, newline_index)
            } else {
                0
            };
            if remove_start <= newline_index {
                rendered.drain(remove_start..=newline_index);
            } else {
                rendered.clear();
            }
            rendered.insert(0, '\n');
        } else {
            rendered.clear();
            rendered.push('\n');
            break;
        }
        scrolled += 1;
    }
    cursor.min(rendered.len())
}

fn line_end_after(rendered: &[char], start: usize) -> usize {
    rendered[start.min(rendered.len())..]
        .iter()
        .position(|ch| *ch == '\n')
        .map(|offset| start + offset)
        .unwrap_or(rendered.len())
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

fn move_terminal_display_cursor_down(rendered: &[char], cursor: usize, lines: usize) -> usize {
    let column = cursor.saturating_sub(line_start_before(rendered, cursor));
    let mut start = line_start_before(rendered, cursor);
    for _ in 0..lines {
        let current_end = line_end_after(rendered, start);
        if current_end >= rendered.len() {
            return rendered.len();
        }
        start = current_end.saturating_add(1).min(rendered.len());
    }
    (start + column).min(line_end_after(rendered, start))
}

fn move_terminal_display_cursor_right(rendered: &[char], cursor: usize, columns: usize) -> usize {
    (cursor + columns).min(line_end_after(rendered, cursor))
}

fn move_terminal_display_cursor_left(rendered: &[char], cursor: usize, columns: usize) -> usize {
    cursor
        .saturating_sub(columns)
        .max(line_start_before(rendered, cursor))
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

fn move_terminal_display_cursor_to_line(rendered: &[char], cursor: usize, row: usize) -> usize {
    let column = cursor.saturating_sub(line_start_before(rendered, cursor));
    let mut start = 0;
    for _ in 1..row.max(1) {
        start = match rendered[start.min(rendered.len())..]
            .iter()
            .position(|ch| *ch == '\n')
        {
            Some(offset) => start + offset + 1,
            None => rendered.len(),
        };
    }
    (start + column).min(line_end_after(rendered, start))
}

fn move_terminal_display_cursor_next_line(rendered: &[char], cursor: usize, lines: usize) -> usize {
    let mut start = line_start_before(rendered, cursor);
    for _ in 0..lines {
        let current_end = line_end_after(rendered, start);
        if current_end >= rendered.len() {
            return cursor;
        }
        start = current_end.saturating_add(1).min(rendered.len());
    }
    move_terminal_display_cursor_to_column(rendered, start, 1)
}

fn move_terminal_display_cursor_previous_line(
    rendered: &[char],
    cursor: usize,
    lines: usize,
) -> usize {
    if rendered.is_empty() {
        return cursor;
    }
    let mut start = line_start_before(rendered, cursor);
    for _ in 0..lines {
        if start == 0 {
            return cursor;
        }
        start = line_start_before(rendered, start.saturating_sub(1));
    }
    move_terminal_display_cursor_to_column(rendered, start, 1)
}

fn csi_first_number(sequence: &str, default: usize) -> usize {
    sequence
        .split(';')
        .next()
        .and_then(|part| part.parse::<usize>().ok())
        .filter(|number| *number > 0)
        .unwrap_or(default)
}

fn csi_first_number_with_zero(sequence: &str, default: usize) -> usize {
    sequence
        .split(';')
        .next()
        .and_then(|part| part.parse::<usize>().ok())
        .unwrap_or(default)
}

fn csi_numbers(sequence: &str) -> Vec<usize> {
    sequence
        .split(';')
        .filter_map(|part| part.parse::<usize>().ok())
        .collect()
}

fn csi_private_mode_enabled(sequence: &str, modes: &[&str]) -> bool {
    let Some(private_modes) = sequence.strip_prefix('?') else {
        return false;
    };
    private_modes.split(';').any(|mode| modes.contains(&mode))
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

pub(crate) fn terminal_command_presets(configured: &[String]) -> Vec<TerminalCommandPreset> {
    let presets = if configured.is_empty() {
        default_terminal_command_presets()
    } else {
        configured
            .iter()
            .filter_map(|entry| terminal_command_preset_from_config(entry))
            .collect()
    };
    if presets.is_empty() {
        default_terminal_command_presets()
    } else {
        presets
    }
}

fn default_terminal_command_presets() -> Vec<TerminalCommandPreset> {
    [
        ("Env", "env | sort | grep '^ARCHDUCTOR_'"),
        ("Git Status", "git status --short --branch"),
        ("Git Diff", "git diff --stat && git diff -- ."),
        (
            "Files",
            "find . -maxdepth 2 -type f | sort | sed 's#^./##' | head -80",
        ),
    ]
    .into_iter()
    .map(|(label, command)| terminal_command_preset(label, command))
    .collect()
}

fn terminal_command_preset_from_config(entry: &str) -> Option<TerminalCommandPreset> {
    let trimmed = entry.trim();
    if trimmed.is_empty() {
        return None;
    }
    match normalize_terminal_preset_alias(trimmed).as_str() {
        "test" => return Some(terminal_command_preset("Test", "pnpm test")),
        "lint" => return Some(terminal_command_preset("Lint", "pnpm lint")),
        "build" => return Some(terminal_command_preset("Build", "pnpm build")),
        "typecheck" | "type" => {
            return Some(terminal_command_preset("Typecheck", "pnpm typecheck"));
        }
        "ci" => {
            return Some(terminal_command_preset(
                "CI",
                "pnpm test && pnpm lint && pnpm build",
            ));
        }
        "status" | "gitstatus" => {
            return Some(terminal_command_preset(
                "Git Status",
                "git status --short --branch",
            ));
        }
        "diff" | "gitdiff" => {
            return Some(terminal_command_preset(
                "Git Diff",
                "git diff --stat && git diff -- .",
            ));
        }
        "env" => {
            return Some(terminal_command_preset(
                "Env",
                "env | sort | grep '^ARCHDUCTOR_'",
            ))
        }
        "files" => {
            return Some(terminal_command_preset(
                "Files",
                "find . -maxdepth 2 -type f | sort | sed 's#^./##' | head -80",
            ));
        }
        _ => {}
    }

    let (label, command) = trimmed
        .split_once('=')
        .or_else(|| trimmed.split_once(':'))?;
    let label = label.trim();
    let command = command.trim();
    if label.is_empty() || command.is_empty() {
        return None;
    }
    Some(terminal_command_preset(label, command))
}

fn terminal_command_preset(label: &str, command: &str) -> TerminalCommandPreset {
    TerminalCommandPreset {
        label: label.to_owned(),
        command: command.to_owned(),
    }
}

fn normalize_terminal_preset_alias(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
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
    use archductor_core::workspace::{ProcessKind, ProcessStatus};

    fn terminal_summary(
        id: i64,
        command: &str,
        status: ProcessStatus,
        started_at: &str,
        line_count: usize,
        byte_count: usize,
        preview: &str,
    ) -> TerminalSessionSummary {
        TerminalSessionSummary {
            process: ProcessRecord {
                id,
                workspace_id: 1,
                chat_thread_id: None,
                kind: ProcessKind::Terminal,
                command: command.to_owned(),
                pid: 4_000 + id as u32,
                log_path: PathBuf::from(format!("/tmp/logs/terminal-{id}.log")),
                status,
                started_at: started_at.to_owned(),
                exit_code: (status != ProcessStatus::Running).then_some(0),
                ended_at: (status != ProcessStatus::Running)
                    .then(|| "2026-06-18T04:00:00Z".to_owned()),
                session_harness_metadata: None,
                session_resume_id: None,
            },
            line_count,
            byte_count,
            preview: preview.to_owned(),
        }
    }

    #[test]
    fn terminal_search_results_render_process_line_and_empty_state() {
        let matches = vec![TerminalLogMatch {
            process_id: 42,
            command: "/bin/sh".to_owned(),
            log_path: PathBuf::from("/tmp/logs/terminal.log"),
            line_number: 3,
            line: "needle found".to_owned(),
            context_before: vec!["before line".to_owned()],
            context_after: vec!["after line".to_owned()],
        }];

        let rendered = format_terminal_search_results("needle", &matches);

        assert!(rendered.contains("[terminal search] needle"));
        assert!(rendered.contains("1 match(es)"));
        assert!(rendered.contains("#42 /bin/sh"));
        assert!(rendered.contains("terminal.log:3"));
        assert!(rendered.contains("needle found"));
        assert!(rendered.contains("before: before line"));
        assert!(rendered.contains("after: after line"));
        assert_eq!(
            format_terminal_search_results("missing", &[]),
            "\n[terminal search] missing\n0 match(es)\nNo terminal transcript matches.\n"
        );
    }

    #[test]
    fn terminal_runtime_error_text_is_visible_in_transcript() {
        let err = anyhow::anyhow!("database is locked");

        assert_eq!(
            terminal_runtime_error_text(&err),
            "\n[terminal runtime error]\ndatabase is locked\n"
        );
    }

    #[test]
    fn terminal_search_summary_collects_unique_processes() {
        let matches = vec![
            TerminalLogMatch {
                process_id: 1,
                command: "bash".to_owned(),
                log_path: PathBuf::from("/tmp/term-1.log"),
                line_number: 1,
                line: "a".to_owned(),
                context_before: vec![],
                context_after: vec![],
            },
            TerminalLogMatch {
                process_id: 1,
                command: "bash".to_owned(),
                log_path: PathBuf::from("/tmp/term-1.log"),
                line_number: 2,
                line: "b".to_owned(),
                context_before: vec![],
                context_after: vec![],
            },
            TerminalLogMatch {
                process_id: 2,
                command: "zsh".to_owned(),
                log_path: PathBuf::from("/tmp/term-2.log"),
                line_number: 1,
                line: "c".to_owned(),
                context_before: vec![],
                context_after: vec![],
            },
        ];

        assert_eq!(
            format_terminal_search_result_summary(&matches),
            "Terminal matches: 3 across 2 processes"
        );
    }

    #[test]
    fn restored_terminal_transcript_is_included_in_initial_text() {
        let preferences = TerminalPreferences::default();
        let text = format_initial_terminal_text(
            "berlin",
            Path::new("/tmp/workspaces/berlin"),
            Some("last shell output\n"),
            &preferences,
        );

        assert!(text.contains("workspace: berlin"));
        assert!(text.contains("font: system monospace"));
        assert!(text.contains("[restored latest terminal transcript]"));
        assert!(text.contains("last shell output"));
    }

    #[test]
    fn terminal_display_text_strips_common_ansi_escape_sequences() {
        let rendered = terminal_display_text("\u{1b}[32mok\u{1b}[0m \u{1b}]0;title\u{7}done\n");

        assert_eq!(rendered, "ok done\n");
    }

    #[test]
    fn terminal_display_text_ignores_osc_title_with_string_terminator() {
        let rendered = terminal_display_text("log\u{1b}]0;title\u{1b}\\done\n");

        assert_eq!(rendered, "logdone\n");
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
    fn terminal_display_text_supports_cursor_down() {
        let rendered = terminal_display_text("12345\nabcde\nfghij\u{1b}[2;3HY\u{1b}[1BZ");

        assert_eq!(rendered, "12345\nabYde\nfghZj");
    }

    #[test]
    fn terminal_display_text_applies_clear_screen_default() {
        let rendered = terminal_display_text("old line\n\u{1b}[Jfresh");

        assert_eq!(rendered, "old line\nfresh");
    }

    #[test]
    fn terminal_display_text_supports_next_and_previous_lines() {
        let next = terminal_display_text("12345\nabcde\nfghij\u{1b}[1;5HA\u{1b}[1EX");
        let previous = terminal_display_text("12345\nabcde\nfghij\u{1b}[2;4HX\u{1b}[1FY");

        assert_eq!(next, "1234A\nXbcde\nfghij");
        assert_eq!(previous, "Y2345\nabcXe\nfghij");
    }

    #[test]
    fn terminal_display_text_supports_cursor_insert_delete_and_erase_chars() {
        let inserted = terminal_display_text("abcde\u{1b}[2D\u{1b}[2@Z\n");
        let deleted = terminal_display_text("abcde\u{1b}[2D\u{1b}[2PZ\n");
        let erased = terminal_display_text("abcde\u{1b}[2D\u{1b}[2XZ\n");

        assert_eq!(inserted, "abcZ de\n");
        assert_eq!(deleted, "abcZ\n");
        assert_eq!(erased, "abcZ \n");
    }

    #[test]
    fn terminal_display_text_supports_tabs() {
        let rendered = terminal_display_text("123\tX");

        assert_eq!(rendered, "123     X");
    }

    #[test]
    fn terminal_display_text_applies_insert_lines() {
        let rendered = terminal_display_text("a\nbc\ndef\n\u{1b}[2;1H\u{1b}[1L");

        assert_eq!(rendered, "a\nbc\n\ndef\n");
    }

    #[test]
    fn terminal_display_text_applies_delete_lines() {
        let rendered = terminal_display_text("a\nbc\ndef\n\u{1b}[2;1H\u{1b}[1M");

        assert_eq!(rendered, "a\ndef\n");
    }

    #[test]
    fn terminal_display_text_applies_insert_lines_at_buffer_end() {
        let rendered = terminal_display_text("a\nbcdef\u{1b}[2;1H\u{1b}[2L");

        assert_eq!(rendered, "a\nbcdef\n\n");
    }

    #[test]
    fn terminal_display_text_applies_delete_multiple_lines() {
        let rendered = terminal_display_text("a\nbc\ndef\nghi\n\u{1b}[1;1H\u{1b}[2M");

        assert_eq!(rendered, "ghi\n");
    }

    #[test]
    fn terminal_display_text_applies_scroll_up() {
        let rendered = terminal_display_text("a\nbc\ndef\n\u{1b}[1S");

        assert_eq!(rendered, "bc\ndef\n\n");
    }

    #[test]
    fn terminal_display_text_applies_scroll_down() {
        let rendered = terminal_display_text("a\nbc\ndef\n\u{1b}[1T");

        assert_eq!(rendered, "\na\nbc\n");
    }

    #[test]
    fn terminal_display_text_scroll_up_keeps_single_line_when_unterminated() {
        let rendered = terminal_display_text("abc\u{1b}[1S");

        assert_eq!(rendered, "\n");
    }

    #[test]
    fn terminal_display_text_scroll_down_keeps_single_line_when_unterminated() {
        let rendered = terminal_display_text("abc\u{1b}[1T");

        assert_eq!(rendered, "\n");
    }

    #[test]
    fn terminal_display_text_applies_erase_line_modes() {
        let rendered =
            terminal_display_text("abcdef\u{1b}[3G\u{1b}[1KXYZ\nold line\u{1b}[2Kdone\n");

        assert_eq!(rendered, "XYZ\ndone\n");
    }

    #[test]
    fn terminal_display_text_applies_erase_entire_line_mode() {
        let rendered = terminal_display_text("abcde\u{1b}[3G\u{1b}[2KXY\n");

        assert_eq!(rendered, "XY   \n");
    }

    #[test]
    fn terminal_display_text_keeps_cursor_when_erasing_to_line_end() {
        let rendered = terminal_display_text("abCD\u{1b}[2D\u{1b}[KXY\n");

        assert_eq!(rendered, "abXY\n");
    }

    #[test]
    fn terminal_display_text_supports_carriage_return_after_newline() {
        let rendered = terminal_display_text("line1\nabcde\rX\n");

        assert_eq!(rendered, "line1\nXbcde\n");
    }

    #[test]
    fn terminal_display_text_supports_carriage_return_with_insert_then_erase() {
        let rendered = terminal_display_text("task...\r\u{1b}[1Kdone\u{1b}[3P\n");

        assert_eq!(rendered, "done \n");
    }

    #[test]
    fn terminal_display_text_supports_carriage_return_with_clear_line() {
        let rendered = terminal_display_text("loading...\r\u{1b}[Kdone\n");

        assert_eq!(rendered, "done      \n");
    }

    #[test]
    fn terminal_display_text_supports_multiline_carriage_return_update() {
        let rendered = terminal_display_text("first\nprogress...\r\u{1b}[Kdone\nsecond\n");

        assert_eq!(rendered, "first\ndone\nsecond\n");
    }

    #[test]
    fn terminal_display_text_supports_insert_and_delete_on_multiple_lines() {
        let inserted = terminal_display_text("abc\ndef\n\u{1b}[2;2H\u{1b}[1@Z\n");
        let deleted = terminal_display_text("abc\ndef\n\u{1b}[2;2H\u{1b}[2P\n");

        assert_eq!(inserted, "abc\ndZef\n");
        assert_eq!(deleted, "abc\nd\n");
    }

    #[test]
    fn terminal_display_text_applies_clear_screen_and_cursor_home() {
        let rendered = terminal_display_text("old line\n\u{1b}[2J\u{1b}[Hfresh\n");

        assert_eq!(rendered, "fresh\n");
    }

    #[test]
    fn terminal_display_text_applies_cursor_left_and_right_overwrites() {
        let rendered = terminal_display_text("abcd\u{1b}[2DXY\u{1b}[1CZ\n");

        assert_eq!(rendered, "abXYZ\n");
    }

    #[test]
    fn terminal_display_text_applies_backspace_overwrites() {
        let rendered = terminal_display_text("spinner -\u{8}\\\u{8}|\n");

        assert_eq!(rendered, "spinner |\n");
    }

    #[test]
    fn terminal_display_text_treats_del_as_backspace() {
        let rendered = terminal_display_text("spinner -\u{7f}|\n");

        assert_eq!(rendered, "spinner |\n");
    }

    #[test]
    fn terminal_display_text_supports_cursor_position_alias_f() {
        let rendered = terminal_display_text("top line\nanother line\n\u{1b}[2;4fZZ\n");

        assert_eq!(rendered, "top line\nanoZZr line\n");
    }

    #[test]
    fn terminal_display_text_supports_cursor_position_csi_variants() {
        let forward = terminal_display_text("abcde\u{1b}[2D\u{1b}[1aZ\n");
        let vertical_absolute = terminal_display_text("one\ntwo\nthree\u{1b}[2dX\n");
        let vertical_relative = terminal_display_text("12345\nabcde\nfghij\u{1b}[2;3HY\u{1b}[1eZ");

        assert_eq!(forward, "abcdZ\n");
        assert_eq!(vertical_absolute, "one\ntwoX\nthree");
        assert_eq!(vertical_relative, "12345\nabYde\nfghZj");
    }

    #[test]
    fn terminal_display_text_applies_saved_cursor_restore() {
        let rendered = terminal_display_text("hello\u{1b}[2D\u{1b}[sXY\u{1b}[uZ\n");

        assert_eq!(rendered, "helZY\n");
    }

    #[test]
    fn terminal_display_text_applies_form_feed() {
        let rendered = terminal_display_text("old output\n\u{0c}new output\n");

        assert_eq!(rendered, "new output\n");
    }

    #[test]
    fn terminal_display_text_ignores_shift_state_controls() {
        let rendered = terminal_display_text("start\u{0e}mid\u{0f}end\n");

        assert_eq!(rendered, "startmidend\n");
    }

    #[test]
    fn terminal_display_text_ignores_vertical_tab_as_cursor_to_line_start() {
        let rendered = terminal_display_text("line1\u{0b}line2\n");

        assert_eq!(rendered, "line2\n");
    }

    #[test]
    fn terminal_display_text_supports_saved_cursor_aliases() {
        let rendered = terminal_display_text("hello\u{1b}[2D\u{1b}7XY\u{1b}8Z\n");

        assert_eq!(rendered, "helZY\n");
    }

    #[test]
    fn terminal_display_text_restores_after_alternate_screen() {
        let rendered =
            terminal_display_text("before\n\u{1b}[?1049hfull screen ui\n\u{1b}[?1049lafter\n");

        assert_eq!(rendered, "before\nafter\n");
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
    fn terminal_preferences_normalize_font_and_scrollback() {
        let preferences =
            TerminalPreferences::from_config(Some("  JetBrains Mono 13  "), Some(25_000));

        assert_eq!(preferences.font.as_deref(), Some("JetBrains Mono 13"));
        assert_eq!(preferences.scrollback_lines, 20_000);
    }

    #[test]
    fn terminal_preferences_ignore_blank_font_and_keep_default_scrollback() {
        let preferences = TerminalPreferences::from_config(Some("  "), Some(4));

        assert_eq!(preferences.font, None);
        assert_eq!(preferences.scrollback_lines, TERMINAL_SCROLLBACK_LINES);
    }

    #[test]
    fn initial_terminal_text_includes_active_terminal_preferences() {
        let preferences = TerminalPreferences::from_config(Some("Iosevka Term 12"), Some(5000));
        let text = format_initial_terminal_text(
            "berlin",
            Path::new("/tmp/berlin"),
            Some("restored\n"),
            &preferences,
        );

        assert!(text.contains("font: Iosevka Term 12"));
        assert!(text.contains("scrollback: 5000 lines"));
        assert!(text.contains("[restored latest terminal transcript]"));
    }

    #[test]
    fn append_terminal_text_uses_configured_scrollback_limit() {
        let rendered = append_terminal_text("one\ntwo\n", "three\nfour\n", 3);

        assert_eq!(
            rendered,
            "[terminal scrollback trimmed]\ntwo\nthree\nfour\n"
        );
    }

    #[test]
    fn terminal_command_presets_use_builtins_when_config_is_empty() {
        let presets = terminal_command_presets(&[]);

        assert!(presets.iter().any(|preset| preset.label == "Git Status"));
        assert!(presets.iter().any(|preset| preset.label == "Git Diff"));
    }

    #[test]
    fn terminal_command_presets_parse_aliases_and_label_commands() {
        let presets = terminal_command_presets(&[
            "test".to_owned(),
            "Preview=pnpm dev".to_owned(),
            "Ship: pnpm build && pnpm test".to_owned(),
            "bad entry".to_owned(),
        ]);

        assert!(presets
            .iter()
            .any(|preset| preset.label == "Test" && preset.command == "pnpm test"));
        assert!(presets
            .iter()
            .any(|preset| preset.label == "Preview" && preset.command == "pnpm dev"));
        assert!(presets
            .iter()
            .any(|preset| preset.label == "Ship" && preset.command == "pnpm build && pnpm test"));
        assert!(!presets.iter().any(|preset| preset.label == "bad entry"));
    }

    #[test]
    fn terminal_log_excerpt_includes_requested_line_with_context() {
        let transcript = "one\ntwo\nthree\nfour\nfive\n";
        let excerpt = terminal_log_excerpt(transcript, 3, 1);

        assert!(excerpt.contains("     2   two"));
        assert!(excerpt.contains(">>"));
        assert!(excerpt.contains("three"));
        assert!(excerpt.contains("     4   four"));
    }

    #[test]
    fn terminal_search_match_snippet_includes_context_lines() {
        let match_record = TerminalLogMatch {
            process_id: 7,
            command: "shell".to_owned(),
            log_path: std::path::PathBuf::from("terminal-7.log"),
            line_number: 42,
            line: "needle".to_owned(),
            context_before: vec!["alpha".to_owned(), "beta".to_owned()],
            context_after: vec!["gamma".to_owned(), "delta".to_owned()],
        };
        let snippet = terminal_search_match_snippet(&match_record);

        assert!(snippet.contains("...\n    41  alpha"));
        assert!(snippet.contains("    42> needle"));
        assert!(snippet.contains("    43   gamma"));
        assert!(snippet.contains("    44   delta"));
        assert!(snippet.contains("...\n"));
    }

    #[test]
    fn terminal_log_tail_shows_recent_lines_with_numbers() {
        let excerpt = terminal_log_tail("first\nsecond\nthird\nfourth\n", 2);

        assert_eq!(excerpt, "     3  third\n     4  fourth\n");
    }

    #[test]
    fn terminal_log_head_shows_first_lines_with_numbers() {
        let excerpt = terminal_log_head("first\nsecond\nthird\nfourth\n", 2);

        assert_eq!(excerpt, "     1  first\n     2  second\n");
    }

    #[test]
    fn terminal_tail_transcript_format_distinguishes_tail() {
        let record = ProcessRecord {
            id: 19,
            workspace_id: 1,
            chat_thread_id: None,
            kind: ProcessKind::Terminal,
            command: "/bin/bash".to_owned(),
            pid: 4040,
            log_path: PathBuf::from("/tmp/logs/terminal-19.log"),
            status: ProcessStatus::Running,
            started_at: "2026-06-18T02:00:00Z".to_owned(),
            exit_code: None,
            ended_at: None,
            session_harness_metadata: None,
            session_resume_id: None,
        };

        let formatted = format_terminal_tail_transcript(&record, "tail line\n");

        assert!(formatted.contains("tail (last "));
        assert!(formatted.contains("status=running"));
        assert!(formatted.contains("tail line"));
    }

    #[test]
    fn terminal_head_transcript_format_distinguishes_head() {
        let record = ProcessRecord {
            id: 33,
            workspace_id: 1,
            chat_thread_id: None,
            kind: ProcessKind::Terminal,
            command: "/bin/bash".to_owned(),
            pid: 4040,
            log_path: PathBuf::from("/tmp/logs/terminal-33.log"),
            status: ProcessStatus::Running,
            started_at: "2026-06-18T02:00:00Z".to_owned(),
            exit_code: None,
            ended_at: None,
            session_harness_metadata: None,
            session_resume_id: None,
        };

        let formatted = format_terminal_head_transcript(&record, "first line\n");

        assert!(formatted.contains("head (first "));
        assert!(formatted.contains("status=running"));
        assert!(formatted.contains("first line"));
    }

    #[test]
    fn terminal_line_transcript_format_includes_context_and_line_number() {
        let record = ProcessRecord {
            id: 88,
            workspace_id: 1,
            chat_thread_id: None,
            kind: ProcessKind::Terminal,
            command: "/bin/bash".to_owned(),
            pid: 4040,
            log_path: PathBuf::from("/tmp/logs/terminal-88.log"),
            status: ProcessStatus::Running,
            started_at: "2026-06-18T02:00:00Z".to_owned(),
            exit_code: None,
            ended_at: None,
            session_harness_metadata: None,
            session_resume_id: None,
        };

        let formatted = format_terminal_line_transcript(&record, 17, 5, "ctx\n");

        assert!(formatted.contains("[terminal session #88] around line 17"));
        assert!(formatted.contains("context=5 lines"));
        assert!(formatted.contains("ctx"));
    }

    #[test]
    fn terminal_line_jump_target_pages_clamp_to_positive_minimum() {
        assert_eq!(
            terminal_line_jump_target(40, -(TERMINAL_LINE_JUMP_PAGE_SIZE as isize)),
            1
        );
        assert_eq!(
            terminal_line_jump_target(40, -(TERMINAL_LINE_JUMP_PAGE_SIZE as isize * 2)),
            1
        );
        assert_eq!(
            terminal_line_jump_target(40, TERMINAL_LINE_JUMP_PAGE_SIZE as isize),
            40 + TERMINAL_LINE_JUMP_PAGE_SIZE
        );
    }

    #[test]
    fn terminal_line_jump_target_keeps_positive_when_no_delta() {
        assert_eq!(terminal_line_jump_target(7, 0), 7);
    }

    #[test]
    fn terminal_positive_line_number_parses_only_positive_integers() {
        assert_eq!(terminal_positive_line_number(""), None);
        assert_eq!(terminal_positive_line_number("  12 "), Some(12));
        assert_eq!(terminal_positive_line_number("0"), None);
        assert_eq!(terminal_positive_line_number("-1"), None);
        assert_eq!(terminal_positive_line_number("abc"), None);
    }

    #[test]
    fn terminal_history_summary_lists_terminal_records() {
        let summaries = vec![terminal_summary(
            7,
            "/bin/bash",
            ProcessStatus::Exited,
            "2026-06-18T02:00:00Z",
            2,
            21,
            "build finished",
        )];

        let rendered = format_terminal_history(&summaries);

        assert!(rendered.contains("[terminal history]"));
        assert!(rendered.contains("#7 exited pid=4007 exit=0"));
        assert!(rendered.contains("2 lines, 21 bytes"));
        assert!(rendered.contains("preview: build finished"));
        assert!(rendered.contains("terminal-7.log"));
        assert!(rendered.contains("/bin/bash"));
    }

    #[test]
    fn terminal_history_summaries_can_filter_by_status() {
        let summaries = vec![
            terminal_summary(
                7,
                "/bin/bash",
                ProcessStatus::Exited,
                "2026-06-18T02:00:00Z",
                1,
                7,
                "bash",
            ),
            terminal_summary(
                9,
                "/bin/zsh",
                ProcessStatus::Running,
                "2026-06-18T03:00:00Z",
                1,
                6,
                "zsh",
            ),
            terminal_summary(
                8,
                "/bin/fish",
                ProcessStatus::Stopped,
                "2026-06-18T02:30:00Z",
                1,
                7,
                "fish",
            ),
        ];

        let running =
            terminal_history_summaries_for_filter(&summaries, Some(ProcessStatus::Running));
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].process.id, 9);
    }

    #[test]
    fn terminal_history_summaries_can_filter_by_query() {
        let summaries = vec![
            terminal_summary(
                7,
                "/bin/bash --noprofile",
                ProcessStatus::Exited,
                "2026-06-18T02:00:00Z",
                1,
                7,
                "build artifact",
            ),
            terminal_summary(
                9,
                "/bin/zsh",
                ProcessStatus::Running,
                "2026-06-18T03:00:00Z",
                1,
                6,
                "shell for work",
            ),
            terminal_summary(
                8,
                "/bin/fish-shell",
                ProcessStatus::Stopped,
                "2026-06-18T02:30:00Z",
                1,
                7,
                "fish preview",
            ),
        ];

        let filtered =
            terminal_history_summaries_for_filter_with_query(&summaries, None, Some("shell"));
        let filtered_ids = filtered
            .iter()
            .map(|summary| summary.process.id)
            .collect::<Vec<_>>();
        assert_eq!(filtered_ids, vec![9, 8]);
    }

    #[test]
    fn terminal_history_summary_counts_statuses_and_lists_newest_first() {
        let summaries = vec![
            terminal_summary(
                7,
                "/bin/bash",
                ProcessStatus::Exited,
                "2026-06-18T02:00:00Z",
                1,
                7,
                "bash",
            ),
            terminal_summary(
                9,
                "/bin/zsh",
                ProcessStatus::Running,
                "2026-06-18T03:00:00Z",
                1,
                6,
                "zsh",
            ),
            terminal_summary(
                8,
                "/bin/fish",
                ProcessStatus::Stopped,
                "2026-06-18T02:30:00Z",
                1,
                7,
                "fish",
            ),
        ];

        let rendered = format_terminal_history(&summaries);

        assert!(rendered.contains("3 sessions: 1 running, 1 stopped, 1 exited"));
        let zsh = rendered.find("#9 running").unwrap();
        let fish = rendered.find("#8 stopped").unwrap();
        let bash = rendered.find("#7 exited").unwrap();
        assert!(zsh < fish);
        assert!(fish < bash);
    }

    #[test]
    fn terminal_history_summaries_for_display_are_newest_first() {
        let summaries = vec![
            terminal_summary(
                7,
                "/bin/bash",
                ProcessStatus::Exited,
                "2026-06-18T02:00:00Z",
                1,
                7,
                "bash",
            ),
            terminal_summary(
                9,
                "/bin/zsh",
                ProcessStatus::Running,
                "2026-06-18T03:00:00Z",
                1,
                6,
                "zsh",
            ),
        ];

        let sorted = terminal_history_summaries_for_display(&summaries);

        assert_eq!(
            sorted
                .iter()
                .map(|summary| summary.process.id)
                .collect::<Vec<_>>(),
            vec![9, 7]
        );
    }

    #[test]
    fn terminal_history_browser_row_label_truncates_preview() {
        let preview_text = "a".repeat(300);
        let summary = terminal_summary(
            12,
            "/bin/bash",
            ProcessStatus::Exited,
            "2026-06-18T04:00:00Z",
            2,
            22,
            &preview_text,
        );
        let label = terminal_history_browser_row_label(&summary);
        let preview = truncate_text_for_display(&summary.preview, 120);
        assert!(label.contains(&preview));
        assert!(preview.ends_with('…'));
        assert!(preview.chars().count() <= 141);
    }

    #[test]
    fn active_terminal_session_option_labels_include_process_id() {
        assert_eq!(
            active_terminal_session_option_label(1, Some(42)),
            "Terminal #42"
        );
        assert_eq!(active_terminal_session_option_label(0, None), "Terminal");
    }

    #[test]
    fn active_terminal_tab_labels_include_state() {
        assert_eq!(
            active_terminal_tab_label(1, Some(42), ProcessStatus::Running, true),
            "Terminal"
        );
        assert_eq!(
            active_terminal_tab_label(0, Some(7), ProcessStatus::Stopped, false),
            "Terminal"
        );
        assert_eq!(
            active_terminal_tab_label(0, Some(7), ProcessStatus::Running, false),
            "Terminal"
        );
        assert_eq!(
            active_terminal_tab_label(0, Some(7), ProcessStatus::Exited, false),
            "Terminal"
        );
    }

    #[test]
    fn next_active_terminal_tab_prefers_running_shell_after_stopped_tab() {
        assert_eq!(next_active_terminal_tab(1, &[true, false, true]), Some(2));
        assert_eq!(next_active_terminal_tab(2, &[true, false, false]), Some(0));
        assert_eq!(next_active_terminal_tab(0, &[false, false]), Some(0));
    }

    #[test]
    fn selected_terminal_transcript_renders_session_header() {
        let record = ProcessRecord {
            id: 7,
            workspace_id: 1,
            chat_thread_id: None,
            kind: ProcessKind::Terminal,
            command: "/bin/bash".to_owned(),
            pid: 4242,
            log_path: PathBuf::from("/tmp/logs/terminal-4242.log"),
            status: ProcessStatus::Exited,
            started_at: "2026-06-18T02:00:00Z".to_owned(),
            exit_code: Some(0),
            ended_at: Some("2026-06-18T02:05:00Z".to_owned()),
            session_harness_metadata: None,
            session_resume_id: None,
        };

        let rendered = format_selected_terminal_transcript(&record, "hello\n");

        assert!(rendered.contains("[terminal transcript #7]"));
        assert!(rendered.contains("status=exited pid=4242 exit=0"));
        assert!(rendered.contains("/bin/bash"));
        assert!(rendered.contains("hello"));
    }

    #[test]
    fn terminal_refresh_event_only_emits_for_process_changes() {
        assert_eq!(
            terminal_refresh_event(
                "berlin",
                TerminalRefreshOutcome {
                    process_changed: true,
                    transcript_changed: false,
                },
            ),
            Some(RefreshEvent::TerminalChanged {
                workspace: "berlin".to_owned(),
            })
        );
        assert_eq!(
            terminal_refresh_event(
                "berlin",
                TerminalRefreshOutcome {
                    process_changed: false,
                    transcript_changed: true,
                },
            ),
            None
        );
    }

    #[test]
    #[cfg_attr(
        target_os = "macos",
        ignore = "GTK init must run on main thread on macOS"
    )]
    fn rebuild_terminal_tabs_does_not_panic_on_transient_state_borrow() {
        let _ = gtk::init();
        let toast_overlay = adw::ToastOverlay::new();
        let terminal_tabs = GBox::new(Orientation::Horizontal, 6);
        let active_pty_combo = ComboBoxText::new();
        let tab_buttons = Rc::new(RefCell::new(Vec::<Button>::new()));
        let terminal_tab_states = Rc::new(RefCell::new(vec![TerminalTabState {
            process_id: 7,
            attached: true,
            status: ProcessStatus::Running,
        }]));
        let borrow = terminal_tab_states.borrow_mut();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rebuild_terminal_tabs(
                &terminal_tabs,
                &active_pty_combo,
                &tab_buttons,
                &terminal_tab_states,
                PathBuf::from("/tmp/state.db"),
                "berlin".to_owned(),
                TextBuffer::new(None),
                Some(0),
                ToastManager::new(&toast_overlay),
            );
        }));
        drop(borrow);

        assert!(result.is_ok());
    }
}
