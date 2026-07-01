use adw::{Toast, ToastOverlay};
use gtk::prelude::*;
use gtk::{
    Align, Box as GBox, Button, CheckButton, ComboBoxText, Entry, Label, ListBox, ListBoxRow,
    Orientation, Paned, PolicyType, ScrolledWindow, Separator, Stack, StackSwitcher, TextTag,
    TextView, WrapMode,
};
use linux_archductor_core::pty::PtySession;
use linux_archductor_core::workspace::{
    ChatThreadRecord, DiffFileSummary, PullRequest, PullRequestReviewThread, ReviewComment,
    SessionKind, Workspace, WorkspaceStore,
};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use tracing::error;

const WORKSPACE_SPLIT_START_WEIGHT: i32 = 5;
const WORKSPACE_SPLIT_END_WEIGHT: i32 = 3;
const WORKSPACE_SPLIT_MIN_START: i32 = 360;
const WORKSPACE_SPLIT_MIN_END: i32 = 280;
type WorkspaceTabSelector = Rc<dyn Fn(&str)>;

use crate::refresh::{RefreshHub, RefreshScope};
use crate::state::{AppState, WorkspaceTab};
use crate::{
    buttons::text_button, cli_binary, detail_row, history, session_surface, shell_quote,
    spawn_terminal_command, terminal, title_case_workspace,
};

fn workspace_repository_name(store: &WorkspaceStore, workspace_name: &str) -> String {
    store
        .list_status()
        .ok()
        .and_then(|lines| {
            lines
                .into_iter()
                .find(|line| line.workspace.name == workspace_name)
                .map(|line| line.repository_name)
        })
        .unwrap_or_else(|| workspace_name.to_owned())
}

fn workspace_repository_name_from_db(db_path: &Path, workspace_name: &str) -> String {
    WorkspaceStore::open(db_path)
        .ok()
        .map(|store| workspace_repository_name(&store, workspace_name))
        .unwrap_or_else(|| workspace_name.to_owned())
}

#[derive(Clone)]
struct FileTreeRow {
    row: ListBoxRow,
    path: String,
    is_dir: bool,
}

pub(crate) fn build_workspace_command_center(
    app_state: &AppState,
    refresh_hub: RefreshHub,
    toast_overlay: ToastOverlay,
    collapse_sidebar: Rc<dyn Fn()>,
) -> (GBox, impl Fn() + Clone + 'static) {
    let root = GBox::new(Orientation::Vertical, 0);
    root.set_vexpand(true);
    root.set_hexpand(true);

    // body is what gets swapped on refresh
    let body = GBox::new(Orientation::Vertical, 0);
    body.set_vexpand(true);
    body.set_hexpand(true);
    root.append(&body);

    let db_path = app_state.workspace_database_path();
    let state = app_state.clone();
    let run_console_states: RunConsoleStateStore = Rc::new(RefCell::new(HashMap::new()));
    let run_console_terminals: RunConsoleTerminalStore = Rc::new(RefCell::new(HashMap::new()));
    let refresh = move || {
        while let Some(child) = body.first_child() {
            body.remove(&child);
        }

        let Some(name) = state.selected_workspace() else {
            let empty = Label::new(Some("Select a workspace from the sidebar."));
            empty.add_css_class("workspace-empty-label");
            empty.set_valign(Align::Center);
            empty.set_halign(Align::Center);
            empty.set_vexpand(true);
            body.append(&empty);
            return;
        };
        let Ok(store) = WorkspaceStore::open(db_path.clone()) else {
            return;
        };
        let Ok(Some(line)) = store
            .list_status()
            .map(|lines| lines.into_iter().find(|l| l.workspace.name == name))
        else {
            return;
        };

        body.append(&simple_workspace_shell(
            &db_path,
            &store,
            &line.workspace,
            &state,
            run_console_states.clone(),
            run_console_terminals.clone(),
            refresh_hub.clone(),
            toast_overlay.clone(),
            collapse_sidebar.clone(),
        ));
    };
    refresh();
    (root, refresh)
}

fn simple_workspace_shell(
    db_path: &Path,
    store: &WorkspaceStore,
    ws: &Workspace,
    state: &AppState,
    run_console_states: RunConsoleStateStore,
    run_console_terminals: RunConsoleTerminalStore,
    refresh_hub: RefreshHub,
    toast_overlay: ToastOverlay,
    collapse_sidebar: Rc<dyn Fn()>,
) -> GBox {
    let shell = GBox::new(Orientation::Vertical, 0);
    shell.set_vexpand(true);
    shell.set_hexpand(true);
    shell.set_overflow(gtk::Overflow::Hidden);

    // Horizontal split: center (flex) + right (fixed 300px)
    let split = Paned::new(Orientation::Horizontal);
    split.set_wide_handle(false);
    split.set_resize_start_child(true);
    split.set_resize_end_child(true);
    split.set_shrink_start_child(true);
    split.set_shrink_end_child(true);
    split.set_position(split_position_for_ratio(
        1280,
        WORKSPACE_SPLIT_START_WEIGHT,
        WORKSPACE_SPLIT_END_WEIGHT,
        WORKSPACE_SPLIT_MIN_START,
        WORKSPACE_SPLIT_MIN_END,
    ));
    split.set_vexpand(true);

    // Center: custom tab bar + chat/terminal/file content
    let (center, open_file) = ws_center_panel(
        db_path,
        store,
        ws,
        state,
        refresh_hub.clone(),
        collapse_sidebar.clone(),
    );
    split.set_start_child(Some(&center));

    // Right: file list + run console
    let right = ws_right_panel(
        db_path,
        store,
        ws,
        state,
        run_console_states,
        run_console_terminals,
        refresh_hub,
        toast_overlay,
        open_file,
        collapse_sidebar,
    );
    split.set_end_child(Some(&right));

    let split_for_resize = split.clone();
    let last_width = Rc::new(RefCell::new(0));
    let last_width_for_resize = last_width.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        if split_for_resize.root().is_none() {
            return glib::ControlFlow::Break;
        }

        let width = split_for_resize.allocated_width();
        if width > 0 && *last_width_for_resize.borrow() != width {
            split_for_resize.set_position(split_position_for_ratio(
                width,
                WORKSPACE_SPLIT_START_WEIGHT,
                WORKSPACE_SPLIT_END_WEIGHT,
                WORKSPACE_SPLIT_MIN_START,
                WORKSPACE_SPLIT_MIN_END,
            ));
            *last_width_for_resize.borrow_mut() = width;
        }

        glib::ControlFlow::Continue
    });

    shell.append(&split);
    shell
}

fn split_position_for_ratio(
    total_width: i32,
    start_weight: i32,
    end_weight: i32,
    min_start: i32,
    min_end: i32,
) -> i32 {
    if total_width <= 0 {
        return min_start.max(0);
    }

    let total_weight = (start_weight + end_weight).max(1);
    let preferred = total_width.saturating_mul(start_weight) / total_weight;
    let max_start = (total_width - min_end).max(min_start);
    preferred.clamp(min_start, max_start)
}

fn make_action_row() -> GBox {
    let row = GBox::new(Orientation::Horizontal, 8);
    row.add_css_class("action-row");
    row
}

type RunConsoleStateStore = Rc<RefCell<HashMap<String, WorkspaceRunConsoleState>>>;
type RunConsoleTerminalStore = Rc<RefCell<HashMap<String, WorkspaceRunConsoleTerminalConnection>>>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceRunConsoleTerminalState {
    id: usize,
    process_id: Option<i64>,
    pid: Option<u32>,
    draft: String,
    transcript: String,
}

impl WorkspaceRunConsoleTerminalState {
    fn new(id: usize) -> Self {
        Self {
            id,
            process_id: None,
            pid: None,
            draft: String::new(),
            transcript: String::new(),
        }
    }

    fn tab_name(&self) -> String {
        format!("terminal-{}", self.id)
    }

    fn label(&self) -> String {
        match self.id {
            1 => "Terminal".to_owned(),
            id => format!("Terminal {id}"),
        }
    }

    fn display_text(&self) -> &str {
        if self.transcript.is_empty() {
            "Enter a command above."
        } else {
            &self.transcript
        }
    }

    fn append_command(&mut self, command: &str) {
        self.transcript
            .push_str(&format!("\n$ {command}\n[running]\n"));
    }

    fn append_result(&mut self, result: &str) {
        self.transcript.push_str(result);
    }

    fn clear_transcript(&mut self) {
        self.transcript.clear();
    }
}

enum WorkspaceRunConsoleTerminalConnection {
    Live(PtySession),
    Reattached {
        write: std::fs::File,
        output: Arc<Mutex<String>>,
        read_cursor: usize,
        pid: u32,
    },
}

impl WorkspaceRunConsoleTerminalConnection {
    fn try_reattach_running(pid: u32) -> anyhow::Result<Self> {
        let path = workspace_terminal_device_path_for_pid(pid)?;
        let mut reader = OpenOptions::new().read(true).open(&path)?;
        let write = OpenOptions::new().write(true).open(&path)?;
        let output = Arc::new(Mutex::new(String::new()));
        let reader_output = Arc::clone(&output);
        std::thread::spawn(move || {
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

    fn write(&mut self, input: &str) -> anyhow::Result<()> {
        match self {
            Self::Live(session) => session.write(input),
            Self::Reattached { write, .. } => {
                write.write_all(input.as_bytes())?;
                write.flush()?;
                Ok(())
            }
        }
    }

    fn has_exited(&mut self) -> anyhow::Result<bool> {
        match self {
            Self::Live(session) => session.has_exited(),
            Self::Reattached { pid, .. } => Ok(!workspace_terminal_process_alive(*pid)),
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceRunConsoleState {
    active_tab: String,
    next_terminal_id: usize,
    terminals: Vec<WorkspaceRunConsoleTerminalState>,
}

impl Default for WorkspaceRunConsoleState {
    fn default() -> Self {
        Self {
            active_tab: "setup".to_owned(),
            next_terminal_id: 2,
            terminals: vec![WorkspaceRunConsoleTerminalState::new(1)],
        }
    }
}

impl WorkspaceRunConsoleState {
    fn active_tab_name(&self) -> String {
        if self.active_tab == "setup" || self.active_tab == "run" {
            return self.active_tab.clone();
        }

        if self
            .terminals
            .iter()
            .any(|terminal| terminal.tab_name() == self.active_tab)
        {
            return self.active_tab.clone();
        }

        "setup".to_owned()
    }

    fn add_terminal_tab(&mut self) -> String {
        let id = self.next_terminal_id;
        self.next_terminal_id += 1;
        let terminal = WorkspaceRunConsoleTerminalState::new(id);
        let name = terminal.tab_name();
        self.terminals.push(terminal);
        self.active_tab = name.clone();
        name
    }

    fn terminal_by_name(&self, tab_name: &str) -> Option<&WorkspaceRunConsoleTerminalState> {
        self.terminals
            .iter()
            .find(|terminal| terminal.tab_name() == tab_name)
    }

    fn terminal_by_name_mut(
        &mut self,
        tab_name: &str,
    ) -> Option<&mut WorkspaceRunConsoleTerminalState> {
        self.terminals
            .iter_mut()
            .find(|terminal| terminal.tab_name() == tab_name)
    }
}

// ── Center panel (chat + terminal + file tabs) ───────────────────

fn ws_center_panel(
    db_path: &Path,
    store: &WorkspaceStore,
    ws: &Workspace,
    state: &AppState,
    refresh_hub: RefreshHub,
    collapse_sidebar: Rc<dyn Fn()>,
) -> (GBox, WorkspaceTabSelector) {
    let panel = GBox::new(Orientation::Vertical, 0);
    panel.add_css_class("ws-center");
    panel.set_hexpand(true);
    panel.set_vexpand(true);

    panel.append(&session_surface::session_header_row(
        &workspace_repository_name(store, &ws.name),
        &ws.branch,
        collapse_sidebar.clone(),
    ));

    let tab_bar = GBox::new(Orientation::Horizontal, 8);
    tab_bar.add_css_class("ws-tab-bar");
    let chat_tabs = GBox::new(Orientation::Horizontal, 6);
    let spacer = GBox::new(Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    let file_tabs = GBox::new(Orientation::Horizontal, 6);
    tab_bar.append(&chat_tabs);
    tab_bar.append(&spacer);
    tab_bar.append(&file_tabs);
    panel.append(&tab_bar);

    // Separator below tab bar
    let tab_sep = Separator::new(Orientation::Horizontal);
    tab_sep.add_css_class("ws-tab-sep");
    panel.append(&tab_sep);

    // Content stack
    let content = Stack::new();
    content.set_vexpand(true);
    content.set_overflow(gtk::Overflow::Hidden);
    content.set_hexpand(true);

    let known_threads = Rc::new(RefCell::new(
        store.list_chat_threads(&ws.name).unwrap_or_default(),
    ));
    let selected_thread =
        Rc::new(RefCell::new(state.selected_chat_thread().or_else(|| {
            known_threads.borrow().first().map(|thread| thread.id)
        })));
    if state.selected_chat_thread().is_none() {
        state.set_selected_chat_thread(*selected_thread.borrow());
    }
    let external_thread_selection: session_surface::ExternalThreadSelectionController =
        Rc::new(RefCell::new(None));

    let refresh_sessions = refresh_hub.clone();
    let on_threads_changed: Rc<dyn Fn(Vec<ChatThreadRecord>, Option<i64>)> = {
        let chat_tabs = chat_tabs.clone();
        let known_threads = known_threads.clone();
        let selected_thread = selected_thread.clone();
        let state = state.clone();
        let external_thread_selection = external_thread_selection.clone();
        let content = content.clone();
        Rc::new(move |threads, selected| {
            *known_threads.borrow_mut() = threads.clone();
            *selected_thread.borrow_mut() = selected;
            state.set_selected_chat_thread(selected);
            while let Some(child) = chat_tabs.first_child() {
                chat_tabs.remove(&child);
            }
            for thread in threads.iter().take(10) {
                let button = ws_tab_button(&workspace_chat_tab_label(thread));
                if Some(thread.id) == selected {
                    button.add_css_class("ws-tab-active");
                }
                let controller = external_thread_selection.clone();
                let content = content.clone();
                let selected_thread = selected_thread.clone();
                let state = state.clone();
                let thread_id = thread.id;
                button.connect_clicked(move |_| {
                    *selected_thread.borrow_mut() = Some(thread_id);
                    state.set_selected_chat_thread(Some(thread_id));
                    content.set_visible_child_name("chat");
                    if let Some(select_thread) = controller.borrow().as_ref().cloned() {
                        select_thread(Some(thread_id));
                    }
                });
                chat_tabs.append(&button);
            }
        })
    };
    let chat_widget = session_surface::agent_session_panel(
        db_path.to_path_buf(),
        &ws.name,
        &workspace_repository_name(store, &ws.name),
        &ws.branch,
        collapse_sidebar.clone(),
        state.clone(),
        move || {
            refresh_sessions.refresh(RefreshScope::Sidebar);
            refresh_sessions.refresh(RefreshScope::Dashboard);
            refresh_sessions.refresh(RefreshScope::History);
        },
        false,
        Some(session_surface::ExternalChatTabs {
            on_threads_changed: on_threads_changed.clone(),
            selection_controller: external_thread_selection.clone(),
        }),
    );
    content.add_named(&chat_widget, Some("chat"));
    let add_tab_btn = text_button("+");
    add_tab_btn.add_css_class("ws-tab-add-btn");
    {
        let db_path = db_path.to_path_buf();
        let workspace_name = ws.name.clone();
        let known_threads = known_threads.clone();
        let selected_thread = selected_thread.clone();
        let state = state.clone();
        let external_thread_selection = external_thread_selection.clone();
        let on_threads_changed = on_threads_changed.clone();
        let content = content.clone();
        add_tab_btn.connect_clicked(move |_| {
            let existing = { known_threads.borrow().clone() };
            if existing.len() >= 10 {
                return;
            }
            let active_thread = *selected_thread.borrow();
            let provider = existing
                .iter()
                .find(|thread| Some(thread.id) == active_thread)
                .map(|thread| thread.provider.clone())
                .unwrap_or_else(|| "codex".to_owned());
            let title = workspace_chat_default_title(&existing);
            let Ok(store) = WorkspaceStore::open(db_path.clone()) else {
                return;
            };
            let Ok(thread) = store.create_chat_thread(&workspace_name, &provider, &title, None)
            else {
                return;
            };
            let mut threads = store.list_chat_threads(&workspace_name).unwrap_or_default();
            if !threads.iter().any(|item| item.id == thread.id) {
                threads.insert(0, thread.clone());
            }
            *selected_thread.borrow_mut() = Some(thread.id);
            state.set_selected_chat_thread(Some(thread.id));
            content.set_visible_child_name("chat");
            (on_threads_changed)(threads, Some(thread.id));
            if let Some(select_thread) = external_thread_selection.borrow().as_ref().cloned() {
                select_thread(Some(thread.id));
            }
        });
    }
    tab_bar.append(&add_tab_btn);

    // Sync active tab state
    let state_tabs = state.clone();
    content.connect_visible_child_name_notify(move |stack| {
        stack.visible_child_name().as_deref();
        state_tabs.set_active_workspace_tab(WorkspaceTab::Chats);
    });

    content.set_visible_child_name(match state.snapshot().active_workspace_tab {
        WorkspaceTab::Terminal => "terminal",
        _ => "chat",
    });

    panel.append(&content);

    // Open-file closure: reads file from disk, opens as a new tab
    let ws_path = ws.path.clone();
    let content_ref = content.clone();
    let file_tabs_ref = file_tabs.clone();

    let open_file: Rc<dyn Fn(&str)> = Rc::new(move |rel_path: &str| {
        let tab_key = format!("file:{rel_path}");

        if content_ref.child_by_name(&tab_key).is_none() {
            // File pane with Edit / Diff mode switcher
            let file_pane = GBox::new(Orientation::Vertical, 0);
            file_pane.set_vexpand(true);

            let mode_tabs = Stack::new();
            mode_tabs.set_vexpand(true);
            let mode_sw = StackSwitcher::new();
            mode_sw.set_stack(Some(&mode_tabs));
            mode_sw.add_css_class("ws-mode-switcher");
            file_pane.append(&mode_sw);

            let full_path = ws_path.join(rel_path);
            let file_content = fs::read_to_string(&full_path)
                .unwrap_or_else(|e| format!("# Error reading file\n{e}"));

            let edit_view = TextView::new();
            edit_view.set_monospace(true);
            edit_view.set_vexpand(true);
            edit_view.buffer().set_text(&file_content);
            let edit_scroll = ScrolledWindow::new();
            edit_scroll.set_vexpand(true);
            edit_scroll.set_child(Some(&edit_view));
            mode_tabs.add_titled(&edit_scroll, Some("edit"), "Edit");

            let diff_view = TextView::new();
            diff_view.set_editable(false);
            diff_view.set_monospace(true);
            diff_view.set_vexpand(true);
            let diff_scroll = ScrolledWindow::new();
            diff_scroll.set_vexpand(true);
            diff_scroll.set_child(Some(&diff_view));
            mode_tabs.add_titled(&diff_scroll, Some("diff"), "Diff");

            let preview_view = TextView::new();
            preview_view.set_editable(false);
            preview_view.set_wrap_mode(WrapMode::WordChar);
            preview_view.set_vexpand(true);
            preview_view.buffer().set_text(&file_content);
            let preview_scroll = ScrolledWindow::new();
            preview_scroll.set_vexpand(true);
            preview_scroll.set_child(Some(&preview_view));
            mode_tabs.add_titled(&preview_scroll, Some("preview"), "Preview");

            mode_tabs.set_visible_child_name("edit");
            file_pane.append(&mode_tabs);
            content_ref.add_named(&file_pane, Some(&tab_key));

            // Tab button for this file
            let short_name = std::path::Path::new(rel_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(rel_path);
            let file_btn = ws_tab_button(short_name);
            let cr = content_ref.clone();
            let tk = tab_key.clone();
            file_btn.connect_clicked(move |_| {
                cr.set_visible_child_name(&tk);
            });
            file_tabs_ref.append(&file_btn);
        }
        content_ref.set_visible_child_name(&tab_key);
    });

    let initial_threads = known_threads.borrow().clone();
    let initial_selected_thread = *selected_thread.borrow();
    (on_threads_changed)(initial_threads, initial_selected_thread);

    (panel, open_file)
}

fn ws_tab_button(label: &str) -> Button {
    let btn = text_button(label);
    btn.add_css_class("ws-tab-btn");
    btn
}

fn workspace_chat_default_title(threads: &[ChatThreadRecord]) -> String {
    let next = threads.len() + 1;
    if next == 1 {
        "New Chat".to_owned()
    } else {
        format!("New Chat {next}")
    }
}

fn workspace_chat_tab_label(thread: &ChatThreadRecord) -> String {
    let title = thread.title.trim();
    if title.is_empty() {
        "New Chat".to_owned()
    } else {
        title.to_owned()
    }
}

fn guarded_gtk_callback<T, F>(fallback: T, callback: F) -> T
where
    F: FnOnce() -> T,
{
    match catch_unwind(AssertUnwindSafe(callback)) {
        Ok(value) => value,
        Err(_) => {
            error!("recovered panic inside workspace command center GTK callback");
            fallback
        }
    }
}

// ── Right panel (file list + run console) ───────────────────────

fn ws_right_panel(
    db_path: &Path,
    store: &WorkspaceStore,
    ws: &Workspace,
    state: &AppState,
    run_console_states: RunConsoleStateStore,
    run_console_terminals: RunConsoleTerminalStore,
    refresh_hub: RefreshHub,
    toast_overlay: ToastOverlay,
    open_file: Rc<dyn Fn(&str)>,
    collapse_sidebar: Rc<dyn Fn()>,
) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 0);
    panel.add_css_class("ws-right-panel");
    panel.set_vexpand(true);
    panel.set_hexpand(true);
    panel.set_overflow(gtk::Overflow::Hidden);

    panel.append(&workspace_checks_panel(
        db_path,
        store,
        &ws.name,
        state.clone(),
        refresh_hub.clone(),
        toast_overlay.clone(),
    ));
    panel.append(&Separator::new(Orientation::Horizontal));
    let tab_strip = GBox::new(Orientation::Horizontal, 4);
    tab_strip.add_css_class("command-center-strip");
    tab_strip.set_spacing(6);
    let content = Stack::new();
    content.set_vexpand(true);

    let all_btn = text_button("All files");
    all_btn.add_css_class("nav-button");
    all_btn.add_css_class("nav-button-active");
    let files_widget = ws_simple_file_list(db_path, ws, open_file.clone());
    content.add_named(&files_widget, Some("files"));

    let changes_btn = text_button("Changes");
    changes_btn.add_css_class("nav-button");
    let changes_widget = workspace_changes_panel(
        db_path,
        store,
        &ws.name,
        refresh_hub.clone(),
        toast_overlay.clone(),
    );
    content.add_named(&changes_widget, Some("changes"));
    {
        let c = content.clone();
        let all_btn_for_click = all_btn.clone();
        let changes_btn_for_click = changes_btn.clone();
        changes_btn.connect_clicked(move |_| {
            c.set_visible_child_name("changes");
            changes_btn_for_click.add_css_class("nav-button-active");
            all_btn_for_click.remove_css_class("nav-button-active");
        });
    }
    {
        let c = content.clone();
        let all_btn_for_click = all_btn.clone();
        let changes_btn_for_click = changes_btn.clone();
        all_btn.connect_clicked(move |_| {
            c.set_visible_child_name("files");
            all_btn_for_click.add_css_class("nav-button-active");
            changes_btn_for_click.remove_css_class("nav-button-active");
        });
    }
    tab_strip.append(&all_btn);
    tab_strip.append(&changes_btn);

    content.set_visible_child_name("files");
    panel.append(&tab_strip);
    panel.append(&Separator::new(Orientation::Horizontal));
    panel.append(&content);

    panel.append(&Separator::new(Orientation::Horizontal));
    panel.append(&ws_run_console(
        db_path,
        store,
        ws,
        state,
        run_console_states,
        run_console_terminals,
        refresh_hub,
        toast_overlay,
        collapse_sidebar,
    ));

    panel
}

fn ws_simple_file_list(_db_path: &Path, ws: &Workspace, open_file: Rc<dyn Fn(&str)>) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 0);
    panel.set_vexpand(true);
    panel.set_overflow(gtk::Overflow::Hidden);

    let list = ListBox::new();
    list.add_css_class("ws-file-list");
    list.set_selection_mode(gtk::SelectionMode::Single);

    let mut files = list_workspace_files(&ws.path);
    files.sort();

    let rows: Rc<RefCell<Vec<FileTreeRow>>> = Rc::new(RefCell::new(Vec::new()));
    let collapsed_dirs: Rc<RefCell<HashSet<String>>> = Rc::new(RefCell::new(HashSet::new()));
    let mut dir_children: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut file_children: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for path in &files {
        let path_ref = std::path::Path::new(path);
        let mut parent_key = String::new();
        if let Some(parent) = path_ref.parent() {
            for component in parent.components() {
                let name = component.as_os_str().to_string_lossy().into_owned();
                let next_key = tree_join_path(&parent_key, &name);
                dir_children
                    .entry(parent_key.clone())
                    .or_default()
                    .insert(name);
                parent_key = next_key;
            }
        }
        let file_name = path_ref
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path.as_str())
            .to_owned();
        file_children
            .entry(parent_key)
            .or_default()
            .insert(file_name);
    }

    append_file_tree_rows(
        &list,
        &rows,
        &collapsed_dirs,
        &dir_children,
        &file_children,
        "",
        0,
    );

    let rows_for_select = rows.clone();
    list.connect_row_selected(move |_, row| {
        guarded_gtk_callback((), || {
            if let Some(r) = row {
                let idx = r.index() as usize;
                let path = rows_for_select
                    .borrow()
                    .get(idx)
                    .and_then(|entry| (!entry.is_dir).then(|| entry.path.clone()));
                if let Some(path) = path {
                    open_file(path.as_str());
                }
            }
        })
    });

    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    scroll.set_vexpand(true);
    scroll.set_propagate_natural_width(false);
    scroll.set_child(Some(&list));
    panel.append(&scroll);

    panel
}

fn tree_join_path(parent: &str, child: &str) -> String {
    if parent.is_empty() {
        child.to_owned()
    } else {
        format!("{parent}/{child}")
    }
}

fn tree_row_visible(path: &str, _is_dir: bool, collapsed_dirs: &HashSet<String>) -> bool {
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    if parts.is_empty() {
        return true;
    }
    let limit = parts.len().saturating_sub(1);
    let mut prefix = String::new();
    for (index, part) in parts.iter().enumerate() {
        if index >= limit {
            break;
        }
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(part);
        if collapsed_dirs.contains(&prefix) {
            return false;
        }
    }
    true
}

fn update_tree_visibility(
    rows: &Rc<RefCell<Vec<FileTreeRow>>>,
    collapsed_dirs: &Rc<RefCell<HashSet<String>>>,
) {
    let collapsed_dirs = collapsed_dirs.borrow();
    for entry in rows.borrow().iter() {
        entry
            .row
            .set_visible(tree_row_visible(&entry.path, entry.is_dir, &collapsed_dirs));
    }
}

fn append_file_tree_rows(
    list: &ListBox,
    rows: &Rc<RefCell<Vec<FileTreeRow>>>,
    collapsed_dirs: &Rc<RefCell<HashSet<String>>>,
    dir_children: &BTreeMap<String, BTreeSet<String>>,
    file_children: &BTreeMap<String, BTreeSet<String>>,
    parent_key: &str,
    depth: usize,
) {
    if let Some(children) = dir_children.get(parent_key) {
        for child in children {
            let dir_path = tree_join_path(parent_key, child);
            let row_box = GBox::new(Orientation::Horizontal, 6);
            row_box.add_css_class("ws-dir-row");
            row_box.set_margin_start((depth as i32) * 16);
            let toggle = text_button("▾");
            toggle.add_css_class("ws-folder-toggle");
            let name_lbl = Label::new(Some(child));
            name_lbl.add_css_class("ws-folder-name");
            name_lbl.set_xalign(0.0);
            name_lbl.set_hexpand(true);
            row_box.append(&toggle);
            row_box.append(&name_lbl);
            let row = ListBoxRow::builder().child(&row_box).build();
            row.set_selectable(false);
            list.append(&row);
            rows.borrow_mut().push(FileTreeRow {
                row: row.clone(),
                path: dir_path.clone(),
                is_dir: true,
            });

            let rows_for_toggle = rows.clone();
            let collapsed_for_toggle = collapsed_dirs.clone();
            let dir_path_for_toggle = dir_path.clone();
            toggle.connect_clicked(move |btn| {
                let collapsed_now = {
                    let mut collapsed = collapsed_for_toggle.borrow_mut();
                    if !collapsed.remove(&dir_path_for_toggle) {
                        collapsed.insert(dir_path_for_toggle.clone());
                        true
                    } else {
                        false
                    }
                };
                btn.set_label(if collapsed_now { "▸" } else { "▾" });
                update_tree_visibility(&rows_for_toggle, &collapsed_for_toggle);
            });

            append_file_tree_rows(
                list,
                rows,
                collapsed_dirs,
                dir_children,
                file_children,
                &dir_path,
                depth + 1,
            );
        }
    }

    if let Some(files) = file_children.get(parent_key) {
        for file in files {
            let file_path = tree_join_path(parent_key, file);
            let row_box = GBox::new(Orientation::Horizontal, 6);
            row_box.add_css_class("ws-file-row");
            row_box.set_margin_start((depth as i32) * 16);

            let ext_lbl = Label::new(Some(file_type_badge(&file_path)));
            ext_lbl.add_css_class("ws-file-badge");
            row_box.append(&ext_lbl);

            let name_lbl = Label::new(Some(file));
            name_lbl.add_css_class("ws-file-name");
            name_lbl.set_xalign(0.0);
            name_lbl.set_hexpand(true);
            row_box.append(&name_lbl);

            let row = ListBoxRow::builder().child(&row_box).build();
            row.set_selectable(true);
            list.append(&row);
            rows.borrow_mut().push(FileTreeRow {
                row,
                path: file_path,
                is_dir: false,
            });
        }
    }

    if parent_key.is_empty() {
        update_tree_visibility(rows, collapsed_dirs);
    }
}

fn file_type_badge(path: &str) -> &'static str {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match ext {
        "rs" => "rs",
        "ts" | "tsx" => "ts",
        "js" | "jsx" => "js",
        "py" => "py",
        "md" => "md",
        "toml" => "ml",
        "yaml" | "yml" => "yl",
        "json" => "{}",
        "css" | "scss" => "cs",
        "html" => "ht",
        "sh" | "bash" => "sh",
        "go" => "go",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" => "c+",
        _ => "  ",
    }
}

fn ws_run_console(
    db_path: &Path,
    _store: &WorkspaceStore,
    ws: &Workspace,
    _state: &AppState,
    run_console_states: RunConsoleStateStore,
    run_console_terminals: RunConsoleTerminalStore,
    _refresh_hub: RefreshHub,
    _toast_overlay: ToastOverlay,
    _collapse_sidebar: Rc<dyn Fn()>,
) -> GBox {
    {
        let mut states = run_console_states.borrow_mut();
        states.entry(ws.name.clone()).or_default();
    }
    let section = GBox::new(Orientation::Vertical, 0);
    section.add_css_class("ws-run-section");
    section.set_overflow(gtk::Overflow::Hidden);

    let header = GBox::new(Orientation::Horizontal, 4);
    header.add_css_class("ws-run-tab-bar");
    header.set_spacing(6);

    let body = GBox::new(Orientation::Vertical, 8);
    body.add_css_class("ws-run-body");

    let tab_bar = GBox::new(Orientation::Horizontal, 4);
    tab_bar.add_css_class("ws-run-tab-bar");
    tab_bar.set_spacing(6);
    let tabs_row = GBox::new(Orientation::Horizontal, 4);
    tabs_row.set_spacing(6);
    tabs_row.set_hexpand(true);
    let content = Stack::new();
    content.set_vexpand(true);

    let tabs: Rc<RefCell<Vec<(String, Button)>>> = Rc::new(RefCell::new(Vec::new()));
    let initial_active = run_console_states
        .borrow()
        .get(&ws.name)
        .map(|state| state.active_tab_name())
        .unwrap_or_else(|| "setup".to_owned());
    let active_name: Rc<RefCell<String>> = Rc::new(RefCell::new(initial_active));
    let set_active = {
        let content = content.clone();
        let tabs = tabs.clone();
        let active_name = active_name.clone();
        let run_console_states = run_console_states.clone();
        let workspace_name = ws.name.clone();
        Rc::new(move |name: &str| {
            content.set_visible_child_name(name);
            *active_name.borrow_mut() = name.to_owned();
            if let Some(state) = run_console_states.borrow_mut().get_mut(&workspace_name) {
                state.active_tab = name.to_owned();
            }
            for (tab_name, button) in tabs.borrow().iter() {
                if tab_name == name {
                    button.add_css_class("ws-run-tab-active");
                } else {
                    button.remove_css_class("ws-run-tab-active");
                }
            }
        })
    };
    let register_tab = {
        let tabs = tabs.clone();
        let set_active = set_active.clone();
        Rc::new(move |name: &str, button: &Button| {
            tabs.borrow_mut().push((name.to_owned(), button.clone()));
            let set_active = set_active.clone();
            let name = name.to_owned();
            button.connect_clicked(move |_| set_active(&name));
        })
    };

    let setup_view = workspace_prompt_tab_view(
        "Build Setup Script",
        "Setup Prompt",
        &workspace_setup_prompt(db_path, &ws.name),
        _state.clone(),
    );
    content.add_named(&setup_view, Some("setup"));
    let setup_btn = text_button("Setup");
    setup_btn.add_css_class("ws-run-tab-btn");
    setup_btn.add_css_class("ws-run-tab-active");
    register_tab("setup", &setup_btn);
    tabs_row.append(&setup_btn);

    let run_view = workspace_prompt_tab_view(
        "Build Run Script",
        "Run Prompt",
        &workspace_run_prompt(db_path, &ws.name),
        _state.clone(),
    );
    content.add_named(&run_view, Some("run"));
    let run_btn = text_button("Run");
    run_btn.add_css_class("ws-run-tab-btn");
    register_tab("run", &run_btn);
    tabs_row.append(&run_btn);

    let terminal_tabs = run_console_states
        .borrow()
        .get(&ws.name)
        .map(|state| state.terminals.clone())
        .unwrap_or_default();
    for terminal_state in terminal_tabs {
        let name = terminal_state.tab_name();
        let label = terminal_state.label();
        let tab = workspace_terminal_tab_view(
            &ws.name,
            &name,
            &label,
            db_path,
            run_console_states.clone(),
            run_console_terminals.clone(),
        );
        content.add_named(&tab, Some(&name));
        let button = text_button(&label);
        button.add_css_class("ws-run-tab-btn");
        register_tab(&name, &button);
        tabs_row.append(&button);
    }

    let add_terminal_tab = {
        let db_path = db_path.to_path_buf();
        let workspace_name = ws.name.clone();
        let content = content.clone();
        let tabs_row = tabs_row.clone();
        let set_active = set_active.clone();
        let register_tab = register_tab.clone();
        let run_console_states = run_console_states.clone();
        Rc::new(move || {
            if tabs.borrow().len() >= 7 {
                return;
            }
            let (name, label) = {
                let mut states = run_console_states.borrow_mut();
                let state = states.entry(workspace_name.clone()).or_default();
                let name = state.add_terminal_tab();
                let label = state
                    .terminal_by_name(&name)
                    .map(|terminal| terminal.label())
                    .unwrap_or_else(|| "Terminal".to_owned());
                (name, label)
            };
            let tab = workspace_terminal_tab_view(
                &workspace_name,
                &name,
                &label,
                &db_path,
                run_console_states.clone(),
                run_console_terminals.clone(),
            );
            content.add_named(&tab, Some(&name));
            let button = text_button(&label);
            button.add_css_class("ws-run-tab-btn");
            register_tab(&name, &button);
            tabs_row.append(&button);
            set_active(&name);
        })
    };

    let add_btn = text_button("+");
    add_btn.add_css_class("ws-run-tab-add-btn");
    let collapse_btn = text_button("▾");
    collapse_btn.add_css_class("ws-run-collapse-btn");
    {
        let add_terminal_tab = add_terminal_tab.clone();
        add_btn.connect_clicked(move |_| add_terminal_tab());
    }
    let spacer = GBox::new(Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    tab_bar.append(&tabs_row);
    tab_bar.append(&spacer);
    tab_bar.append(&add_btn);
    tab_bar.append(&collapse_btn);
    header.append(&tab_bar);
    section.append(&header);

    body.set_visible(true);
    body.append(&content);
    section.append(&body);
    let active_tab = run_console_states
        .borrow()
        .get(&ws.name)
        .map(|state| state.active_tab_name())
        .unwrap_or_else(|| "setup".to_owned());
    content.set_visible_child_name(&active_tab);

    let expanded = Rc::new(RefCell::new(true));
    {
        let expanded = expanded.clone();
        let body = body.clone();
        let section = section.clone();
        let collapse_btn_for_toggle = collapse_btn.clone();
        collapse_btn_for_toggle.clone().connect_clicked(move |_| {
            let next = !*expanded.borrow();
            *expanded.borrow_mut() = next;
            body.set_visible(next);
            section.queue_resize();
            collapse_btn_for_toggle.set_label(if next { "▾" } else { "▴" });
        });
    }

    set_active(&active_tab);
    section
}

fn workspace_prompt_tab_view(
    modal_title: &str,
    title: &str,
    prompt: &str,
    app_state: AppState,
) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 8);
    panel.add_css_class("ws-run-panel");

    let heading = Label::new(Some(title));
    heading.add_css_class("detail-label");
    heading.set_xalign(0.0);
    panel.append(&heading);

    let prompt_view = TextView::new();
    prompt_view.set_editable(false);
    prompt_view.set_monospace(true);
    prompt_view.set_wrap_mode(WrapMode::WordChar);
    prompt_view.set_vexpand(true);
    prompt_view.add_css_class("ws-run-output");
    prompt_view
        .buffer()
        .set_text(&format!("$ cat <<'PROMPT'\n{}\nPROMPT\n", prompt.trim()));
    let prompt_scroll = ScrolledWindow::new();
    prompt_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    prompt_scroll.set_vexpand(true);
    prompt_scroll.set_child(Some(&prompt_view));
    let prompt_overlay = gtk::Overlay::new();
    prompt_overlay.set_child(Some(&prompt_scroll));

    let modal = GBox::new(Orientation::Vertical, 6);
    modal.add_css_class("ws-prompt-modal");
    modal.set_halign(Align::Center);
    modal.set_valign(Align::Center);
    let modal_title_label = Label::new(Some(modal_title));
    modal_title_label.add_css_class("detail-label");
    modal_title_label.set_xalign(0.5);
    let stage_btn = text_button(match title {
        "Setup Prompt" => "Queue Bootstrap Draft",
        "Run Prompt" => "Queue Launch Draft",
        _ => "Queue Prompt",
    });
    stage_btn.add_css_class("suggested-action");
    let prompt_text = prompt.to_owned();
    stage_btn.connect_clicked(move |_| {
        app_state.set_staged_review_prompt(Some(prompt_text.clone()));
    });
    modal.append(&modal_title_label);
    modal.append(&stage_btn);
    prompt_overlay.add_overlay(&modal);
    panel.append(&prompt_overlay);

    panel
}

fn workspace_terminal_tab_view(
    workspace_name: &str,
    tab_name: &str,
    label: &str,
    db_path: &Path,
    run_console_states: RunConsoleStateStore,
    run_console_terminals: RunConsoleTerminalStore,
) -> GBox {
    ensure_workspace_terminal_session(
        db_path,
        workspace_name,
        tab_name,
        &run_console_states,
        &run_console_terminals,
    );
    let panel = GBox::new(Orientation::Vertical, 8);
    panel.add_css_class("ws-run-panel");

    let heading = Label::new(Some(label));
    heading.add_css_class("detail-label");
    heading.set_xalign(0.0);
    panel.append(&heading);

    let command_row = make_action_row();
    let command_entry = Entry::new();
    command_entry.set_placeholder_text(Some("Run terminal command"));
    command_entry.set_hexpand(true);
    let run_btn = text_button("Run");
    run_btn.add_css_class("suggested-action");
    let clear_btn = text_button("Clear");
    clear_btn.add_css_class("secondary-action");
    command_row.append(&command_entry);
    command_row.append(&run_btn);
    command_row.append(&clear_btn);
    panel.append(&command_row);

    let output_view = TextView::new();
    output_view.set_editable(false);
    output_view.set_monospace(true);
    output_view.set_vexpand(true);
    output_view.add_css_class("ws-run-output");
    let initial_terminal_state = run_console_states
        .borrow()
        .get(workspace_name)
        .and_then(|state| state.terminal_by_name(tab_name).cloned())
        .unwrap_or_else(|| WorkspaceRunConsoleTerminalState::new(1));
    command_entry.set_text(&initial_terminal_state.draft);
    output_view
        .buffer()
        .set_text(initial_terminal_state.display_text());
    let output_scroll = ScrolledWindow::new();
    output_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    output_scroll.set_vexpand(true);
    output_scroll.set_child(Some(&output_view));
    panel.append(&output_scroll);

    let buffer = output_view.buffer();
    let panel_for_poll = panel.clone();
    let state_for_poll = run_console_states.clone();
    let terminals_for_poll = run_console_terminals.clone();
    let workspace_for_poll = workspace_name.to_owned();
    let tab_for_poll = tab_name.to_owned();
    let buffer_for_poll = buffer.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        if panel_for_poll.root().is_none() {
            return glib::ControlFlow::Break;
        }
        let key = run_console_terminal_key(&workspace_for_poll, &tab_for_poll);
        let mut remove_connection = false;
        let output = {
            let mut terminals = terminals_for_poll.borrow_mut();
            let Some(connection) = terminals.get_mut(&key) else {
                return glib::ControlFlow::Continue;
            };
            match connection.has_exited() {
                Ok(true) => {
                    remove_connection = true;
                    connection.read_available()
                }
                Ok(false) => connection.read_available(),
                Err(_) => String::new(),
            }
        };
        if !output.is_empty() {
            let transcript = {
                let mut states = state_for_poll.borrow_mut();
                let Some(state) = states.get_mut(&workspace_for_poll) else {
                    return glib::ControlFlow::Continue;
                };
                let Some(terminal) = state.terminal_by_name_mut(&tab_for_poll) else {
                    return glib::ControlFlow::Continue;
                };
                terminal.append_result(&output);
                terminal.display_text().to_owned()
            };
            buffer_for_poll.set_text(&transcript);
        }
        if remove_connection {
            terminals_for_poll.borrow_mut().remove(&key);
            let transcript = {
                let mut states = state_for_poll.borrow_mut();
                let Some(state) = states.get_mut(&workspace_for_poll) else {
                    return glib::ControlFlow::Break;
                };
                let Some(terminal) = state.terminal_by_name_mut(&tab_for_poll) else {
                    return glib::ControlFlow::Break;
                };
                terminal.append_result("\n[terminal exited]\n");
                terminal.display_text().to_owned()
            };
            buffer_for_poll.set_text(&transcript);
        }
        glib::ControlFlow::Continue
    });
    let state_for_change = run_console_states.clone();
    let workspace_for_change = workspace_name.to_owned();
    let tab_for_change = tab_name.to_owned();
    command_entry.connect_changed(move |entry| {
        if let Some(state) = state_for_change.borrow_mut().get_mut(&workspace_for_change) {
            if let Some(terminal) = state.terminal_by_name_mut(&tab_for_change) {
                terminal.draft = entry.text().to_string();
            }
        }
    });
    let db_for_run = db_path.to_path_buf();
    let workspace_for_run = workspace_name.to_owned();
    let buffer_for_run = buffer.clone();
    let tab_for_run = tab_name.to_owned();
    let state_for_run = run_console_states.clone();
    let terminals_for_run = run_console_terminals.clone();
    let command_entry_for_run = command_entry.clone();
    let run_command = Rc::new(move || {
        let command = command_entry_for_run.text().trim().to_owned();
        if command.is_empty() {
            return;
        }
        let transcript = {
            let mut states = state_for_run.borrow_mut();
            let Some(state) = states.get_mut(&workspace_for_run) else {
                return;
            };
            let Some(terminal) = state.terminal_by_name_mut(&tab_for_run) else {
                return;
            };
            terminal.draft.clear();
            terminal.display_text().to_owned()
        };
        command_entry_for_run.set_text("");
        let key = run_console_terminal_key(&workspace_for_run, &tab_for_run);
        if !terminals_for_run.borrow().contains_key(&key) {
            ensure_workspace_terminal_session(
                &db_for_run,
                &workspace_for_run,
                &tab_for_run,
                &state_for_run,
                &terminals_for_run,
            );
        }
        let mut terminals = terminals_for_run.borrow_mut();
        let Some(connection) = terminals.get_mut(&key) else {
            return;
        };
        let _ = connection.write(&(command + "\n"));
        buffer_for_run.set_text(&transcript);
    });
    let run_btn_handler = run_command.clone();
    run_btn.connect_clicked(move |_| {
        run_btn_handler();
    });
    let run_activate = run_command.clone();
    command_entry.connect_activate(move |_| {
        run_activate();
    });

    let buffer_for_clear = buffer.clone();
    let state_for_clear = run_console_states;
    let workspace_for_clear = workspace_name.to_owned();
    let tab_for_clear = tab_name.to_owned();
    clear_btn.connect_clicked(move |_| {
        if let Some(state) = state_for_clear.borrow_mut().get_mut(&workspace_for_clear) {
            if let Some(terminal) = state.terminal_by_name_mut(&tab_for_clear) {
                terminal.clear_transcript();
            }
        }
        buffer_for_clear.set_text("Enter a command above.");
    });

    panel
}

fn run_console_terminal_key(workspace_name: &str, tab_name: &str) -> String {
    format!("{workspace_name}::{tab_name}")
}

fn ensure_workspace_terminal_session(
    db_path: &Path,
    workspace_name: &str,
    tab_name: &str,
    run_console_states: &RunConsoleStateStore,
    run_console_terminals: &RunConsoleTerminalStore,
) {
    let key = run_console_terminal_key(workspace_name, tab_name);
    if run_console_terminals.borrow().contains_key(&key) {
        return;
    }

    let existing_pid = run_console_states
        .borrow()
        .get(workspace_name)
        .and_then(|state| state.terminal_by_name(tab_name))
        .and_then(|terminal| terminal.pid);
    if let Some(pid) = existing_pid {
        if let Ok(connection) = WorkspaceRunConsoleTerminalConnection::try_reattach_running(pid) {
            run_console_terminals.borrow_mut().insert(key, connection);
            return;
        }
    }

    let Ok((process_id, pid, connection)) =
        spawn_workspace_terminal_session(db_path, workspace_name)
    else {
        return;
    };
    run_console_terminals.borrow_mut().insert(key, connection);
    if let Some(state) = run_console_states.borrow_mut().get_mut(workspace_name) {
        if let Some(terminal) = state.terminal_by_name_mut(tab_name) {
            terminal.process_id = Some(process_id);
            terminal.pid = Some(pid);
            if terminal.transcript.is_empty() {
                terminal
                    .transcript
                    .push_str(&format!("[terminal started #{process_id} pid={pid}]\n"));
            }
        }
    }
}

fn spawn_workspace_terminal_session(
    db_path: &Path,
    workspace_name: &str,
) -> anyhow::Result<(i64, u32, WorkspaceRunConsoleTerminalConnection)> {
    let store = WorkspaceStore::open(db_path)?;
    let launch = store.session_launch(workspace_name, SessionKind::Shell)?;
    let command = format!("{}", launch.program.display());
    let session = PtySession::spawn(launch.program, launch.args, &launch.cwd, launch.env, 24, 80)?;
    let pid = session
        .process_id()
        .ok_or_else(|| anyhow::anyhow!("terminal PTY did not report a process id"))?;
    let process = store.record_terminal_process(workspace_name, &command, pid)?;
    Ok((
        process.id,
        pid,
        WorkspaceRunConsoleTerminalConnection::Live(session),
    ))
}

fn workspace_terminal_process_alive(process_id: u32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(process_id.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn workspace_terminal_device_path_for_pid(process_id: u32) -> anyhow::Result<PathBuf> {
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

fn append_terminal_buffer_text(buffer: &gtk::TextBuffer, text: &str) {
    let mut end = buffer.end_iter();
    buffer.insert(&mut end, text);
}

fn workspace_files_panel(
    db_path: &Path,
    ws: &Workspace,
    refresh_hub: RefreshHub,
    toast_overlay: ToastOverlay,
) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 8);
    panel.set_vexpand(true);

    let selected_file = Rc::new(RefCell::new(None::<String>));
    let current_file = Label::new(Some("Select a file."));
    current_file.add_css_class("card-meta");
    current_file.set_xalign(0.0);
    current_file.set_wrap(true);
    panel.append(&current_file);

    let mode_stack = Stack::new();
    let mode_switcher = StackSwitcher::new();
    mode_switcher.set_stack(Some(&mode_stack));
    mode_switcher.add_css_class("panel-switcher");
    panel.append(&mode_switcher);

    let edit_view = TextView::new();
    edit_view.set_monospace(true);
    edit_view.set_vexpand(true);
    let edit_scroll = ScrolledWindow::new();
    edit_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    edit_scroll.set_vexpand(true);
    edit_scroll.set_child(Some(&edit_view));
    mode_stack.add_titled(&edit_scroll, Some("edit"), "Edit");

    let diff_view = TextView::new();
    diff_view.set_editable(false);
    diff_view.set_monospace(true);
    diff_view.set_vexpand(true);
    let diff_scroll = ScrolledWindow::new();
    diff_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    diff_scroll.set_vexpand(true);
    diff_scroll.set_child(Some(&diff_view));
    mode_stack.add_titled(&diff_scroll, Some("diff"), "Diff");

    let preview_view = TextView::new();
    preview_view.set_editable(false);
    preview_view.set_wrap_mode(WrapMode::WordChar);
    preview_view.set_vexpand(true);
    let preview_scroll = ScrolledWindow::new();
    preview_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    preview_scroll.set_vexpand(true);
    preview_scroll.set_child(Some(&preview_view));
    mode_stack.add_titled(&preview_scroll, Some("preview"), "Preview");

    let feedback = Label::new(Some("No file action run yet."));
    feedback.add_css_class("card-meta");
    feedback.set_xalign(0.0);
    feedback.set_wrap(true);

    let action_row = make_action_row();
    let reload_btn = secondary_button("Reload");
    let save_btn = text_button("Save");
    save_btn.add_css_class("suggested-action");
    action_row.append(&reload_btn);
    action_row.append(&save_btn);
    panel.append(&action_row);

    let file_list = GBox::new(Orientation::Vertical, 4);
    let files = list_workspace_files(&ws.path);
    if files.is_empty() {
        file_list.append(&detail_row("Files", "No visible files."));
    } else {
        for relative in files {
            let open_btn = flat_button(&relative);
            open_btn.set_hexpand(true);
            open_btn.set_halign(Align::Fill);
            let current_file_open = current_file.clone();
            let selected_file_open = selected_file.clone();
            let edit_buffer = edit_view.buffer();
            let diff_buffer = diff_view.buffer();
            let preview_buffer = preview_view.buffer();
            let workspace_path = ws.path.clone();
            let db_path_open = db_path.to_path_buf();
            let workspace_name = ws.name.clone();
            let relative_path = relative.clone();
            let mode_stack_open = mode_stack.clone();
            open_btn.connect_clicked(move |_| {
                *selected_file_open.borrow_mut() = Some(relative_path.clone());
                current_file_open.set_text(&relative_path);
                let file_path = workspace_path.join(&relative_path);
                let contents = fs::read_to_string(&file_path)
                    .unwrap_or_else(|_| "[binary or unreadable file]".to_owned());
                edit_buffer.set_text(&contents);
                preview_buffer.set_text(&contents);
                diff_buffer.set_text(&workspace_diff_text_for_path(
                    &db_path_open,
                    &workspace_name,
                    Some(&relative_path),
                ));
                if relative_path.ends_with(".md") {
                    mode_stack_open.set_visible_child_name("preview");
                } else {
                    mode_stack_open.set_visible_child_name("edit");
                }
            });
            file_list.append(&open_btn);
        }
    }

    let file_scroll = ScrolledWindow::new();
    file_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    file_scroll.set_min_content_height(160);
    file_scroll.set_child(Some(&file_list));
    panel.append(&file_scroll);
    panel.append(&mode_stack);
    panel.append(&feedback);

    let selected_file_reload = selected_file.clone();
    let current_file_reload = current_file.clone();
    let edit_buffer_reload = edit_view.buffer();
    let diff_buffer_reload = diff_view.buffer();
    let preview_buffer_reload = preview_view.buffer();
    let workspace_path_reload = ws.path.clone();
    let db_path_reload = db_path.to_path_buf();
    let workspace_name_reload = ws.name.clone();
    reload_btn.connect_clicked(move |_| {
        let Some(relative_path) = selected_file_reload.borrow().clone() else {
            current_file_reload.set_text("Select a file.");
            return;
        };
        let file_path = workspace_path_reload.join(&relative_path);
        let contents = fs::read_to_string(&file_path)
            .unwrap_or_else(|_| "[binary or unreadable file]".to_owned());
        edit_buffer_reload.set_text(&contents);
        preview_buffer_reload.set_text(&contents);
        diff_buffer_reload.set_text(&workspace_diff_text_for_path(
            &db_path_reload,
            &workspace_name_reload,
            Some(&relative_path),
        ));
    });

    let selected_file_save = selected_file;
    let edit_buffer_save = edit_view.buffer();
    let workspace_path_save = ws.path.clone();
    let feedback_save = feedback.clone();
    let toast_save = toast_overlay;
    save_btn.connect_clicked(move |_| {
        let Some(relative_path) = selected_file_save.borrow().clone() else {
            apply_action_feedback(
                &feedback_save,
                &toast_save,
                "Select a file before saving.",
                true,
            );
            return;
        };
        let file_path = workspace_path_save.join(&relative_path);
        match fs::write(&file_path, text_buffer_contents(&edit_buffer_save)) {
            Ok(()) => {
                apply_action_feedback(
                    &feedback_save,
                    &toast_save,
                    &format!("Saved {}.", relative_path),
                    true,
                );
                refresh_hub.refresh(RefreshScope::Workspace);
            }
            Err(err) => apply_action_feedback(
                &feedback_save,
                &toast_save,
                &format!("Could not save {}: {err}", relative_path),
                true,
            ),
        }
    });

    panel
}

fn list_workspace_files(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    list_workspace_files_recursive(root, root, &mut files);
    files.sort();
    files.truncate(400);
    files
}

fn list_workspace_files_recursive(root: &Path, current: &Path, files: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(current) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if matches!(name.as_ref(), ".git" | "target" | "node_modules") {
            continue;
        }
        if path.is_dir() {
            // ponytail: skip deep vendor/build trees; add a real tree model only if users hit this ceiling.
            list_workspace_files_recursive(root, &path, files);
            continue;
        }
        if let Ok(relative) = path.strip_prefix(root) {
            files.push(relative.to_string_lossy().to_string());
        }
    }
}

fn text_buffer_contents(buffer: &gtk::TextBuffer) -> String {
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .to_string()
}

fn make_action_stack() -> GBox {
    let stack = GBox::new(Orientation::Vertical, 8);
    stack.add_css_class("action-stack");
    stack
}

fn secondary_button(label: &str) -> Button {
    let button = text_button(label);
    button.add_css_class("secondary-action");
    button
}

fn flat_button(label: &str) -> Button {
    let button = text_button(label);
    button.add_css_class("flat-action");
    button
}

fn destructive_button(label: &str) -> Button {
    let button = text_button(label);
    button.add_css_class("destructive-action");
    button
}

fn toolbar_label(text: &str) -> Label {
    let label = Label::new(Some(text));
    label.add_css_class("toolbar-label");
    label.set_xalign(0.0);
    label
}

fn workspace_status_strip(
    ws: &Workspace,
    checks: Option<&linux_archductor_core::workspace::ChecksSummary>,
) -> GBox {
    let strip = GBox::new(Orientation::Horizontal, 10);
    strip.add_css_class("command-center-strip");
    strip.add_css_class("workspace-summary-strip");
    strip.append(&metric_card("Status", &ws.status));
    strip.append(&metric_card("Port", &ws.port_base.to_string()));
    strip.append(&metric_card(
        "Files",
        &checks
            .map(|summary| summary.changed_files.to_string())
            .unwrap_or_else(|| "-".to_owned()),
    ));
    strip.append(&metric_card(
        "Todos",
        &checks
            .map(|summary| format!("{} open", summary.open_todos))
            .unwrap_or_else(|| "-".to_owned()),
    ));
    strip.append(&metric_card(
        "Review",
        &checks
            .map(|summary| format!("{} open", summary.open_review_comments))
            .unwrap_or_else(|| "-".to_owned()),
    ));
    strip.append(&metric_card(
        "Sessions",
        &checks
            .map(|summary| summary.active_sessions.to_string())
            .unwrap_or_else(|| "-".to_owned()),
    ));
    strip
}

fn metric_card(label: &str, value: &str) -> GBox {
    let card = GBox::new(Orientation::Vertical, 4);
    card.add_css_class("metric-card");
    card.set_hexpand(true);
    let label_widget = Label::new(Some(label));
    label_widget.add_css_class("detail-label");
    label_widget.set_xalign(0.0);
    let value_widget = Label::new(Some(value));
    value_widget.add_css_class("metric-value");
    value_widget.set_xalign(0.0);
    value_widget.set_ellipsize(gtk::pango::EllipsizeMode::End);
    card.append(&label_widget);
    card.append(&value_widget);
    card
}

fn agents_panel(
    db_path: &Path,
    ws: &Workspace,
    app_state: &AppState,
    refresh_hub: RefreshHub,
    collapse_sidebar: Rc<dyn Fn()>,
) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 10);
    panel.add_css_class("command-panel");
    panel.add_css_class("session-tool-surface");
    panel.set_hexpand(true);
    panel.append(&section_title("Agents"));

    // Profile selector: populated from configured agent profiles
    let profile_row = GBox::new(Orientation::Horizontal, 8);
    let profile_label = Label::new(Some("Profile:"));
    profile_label.add_css_class("detail-label");
    let profile_select = ComboBoxText::new();
    profile_select.append(Some("default"), "Default");
    let profile_names = WorkspaceStore::open(db_path)
        .and_then(|store| store.workspace_view_defaults(&ws.name))
        .map(|defaults| defaults.agent_profile_names)
        .unwrap_or_default();
    for name in &profile_names {
        profile_select.append(Some(name.as_str()), name.as_str());
    }
    profile_select.set_active_id(Some("default"));
    if profile_names.is_empty() {
        profile_row.set_visible(false);
    }
    profile_row.append(&profile_label);
    profile_row.append(&profile_select);
    panel.append(&profile_row);

    let actions = GBox::new(Orientation::Horizontal, 8);
    for (label, kind) in [("Shell", "shell"), ("Codex", "codex"), ("Claude", "claude")] {
        let button = text_button(label);
        let workspace = ws.name.clone();
        let db_for_launch = db_path.to_path_buf();
        let profile_select_for_launch = profile_select.clone();
        button.connect_clicked(move |_| {
            let profile = profile_select_for_launch
                .active_id()
                .filter(|id| id != "default")
                .map(|id| id.to_string());
            let general_prompt = WorkspaceStore::open(db_for_launch.clone())
                .and_then(|store| store.workspace_repo_settings(&workspace))
                .ok()
                .and_then(|settings| settings.prompts.and_then(|p| p.general))
                .filter(|p| !p.is_empty());
            let launch_cmd = build_session_open_command(&workspace, kind, profile.as_deref());
            if let Some(prompt) = general_prompt {
                show_prompt_preview(&prompt, &launch_cmd);
            } else {
                spawn_terminal_command(&launch_cmd);
            }
        });
        actions.append(&button);
    }
    panel.append(&actions);

    let session_box = GBox::new(Orientation::Vertical, 8);
    let db_for_sessions = db_path.to_path_buf();
    let workspace_for_sessions = ws.name.clone();
    let refresh_sessions = refresh_hub.clone();
    session_box.append(&session_surface::agent_session_panel(
        db_for_sessions,
        &workspace_for_sessions,
        &workspace_repository_name_from_db(db_path, &workspace_for_sessions),
        &ws.branch,
        collapse_sidebar.clone(),
        app_state.clone(),
        move || {
            refresh_sessions.refresh(RefreshScope::Sidebar);
            refresh_sessions.refresh(RefreshScope::Dashboard);
            refresh_sessions.refresh(RefreshScope::History);
        },
        false,
        None,
    ));
    panel.append(&session_box);
    panel
}

fn build_session_open_command(workspace: &str, kind: &str, profile: Option<&str>) -> String {
    let mut cmd = format!(
        "{} session open {} --kind {}",
        cli_binary().display(),
        shell_quote(workspace),
        kind
    );
    if let Some(profile) = profile {
        cmd.push_str(&format!(" --profile {}", shell_quote(profile)));
    }
    cmd
}

fn show_prompt_preview(prompt: &str, launch_cmd: &str) {
    let dialog = gtk::Window::builder()
        .title("Prompt Preview")
        .modal(true)
        .default_width(520)
        .default_height(320)
        .build();
    let body = GBox::new(Orientation::Vertical, 10);
    body.add_css_class("modal-body");
    body.set_margin_top(14);
    body.set_margin_bottom(14);
    body.set_margin_start(14);
    body.set_margin_end(14);
    let title = Label::new(Some("General agent prompt"));
    title.add_css_class("section-title");
    title.set_xalign(0.0);
    body.append(&title);
    let hint = Label::new(Some(
        "This prompt will be injected when the session starts.",
    ));
    hint.add_css_class("card-meta");
    hint.set_xalign(0.0);
    hint.set_wrap(true);
    body.append(&hint);
    let text_view = TextView::new();
    text_view.set_editable(false);
    text_view.set_monospace(true);
    text_view.set_wrap_mode(WrapMode::WordChar);
    text_view.buffer().set_text(prompt);
    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    scroll.set_vexpand(true);
    scroll.set_min_content_height(140);
    scroll.set_child(Some(&text_view));
    body.append(&scroll);
    let buttons = GBox::new(Orientation::Horizontal, 8);
    buttons.set_halign(Align::End);
    let cancel_btn = text_button("Cancel");
    let launch_btn = text_button("Launch");
    launch_btn.add_css_class("suggested-action");
    let dialog_for_cancel = dialog.clone();
    cancel_btn.connect_clicked(move |_| {
        dialog_for_cancel.close();
    });
    let dialog_for_launch = dialog.clone();
    let cmd = launch_cmd.to_owned();
    launch_btn.connect_clicked(move |_| {
        spawn_terminal_command(&cmd);
        dialog_for_launch.close();
    });
    buttons.append(&cancel_btn);
    buttons.append(&launch_btn);
    body.append(&buttons);
    dialog.set_child(Some(&body));
    dialog.present();
}

fn runtime_panel(
    db_path: &Path,
    ws: &Workspace,
    store: &WorkspaceStore,
    refresh_hub: RefreshHub,
    toast_overlay: ToastOverlay,
) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 10);
    panel.add_css_class("command-panel");
    panel.set_hexpand(true);
    panel.append(&section_title("Runtime"));

    let actions = make_action_stack();
    let setup_btn = secondary_button("Setup");
    let run_btn = text_button("Run");
    run_btn.add_css_class("suggested-action");
    let stop_btn = destructive_button("Stop");
    let spotlight_on_btn = text_button("Spotlight On");
    spotlight_on_btn.add_css_class("suggested-action");
    let spotlight_sync_btn = secondary_button("Sync Spotlight");
    let spotlight_repair_btn = destructive_button("Repair Spotlight");
    let spotlight_off_btn = flat_button("Spotlight Off");
    let folder_btn = flat_button("Open Folder");
    let status = Label::new(None);
    status.add_css_class("card-meta");
    status.set_xalign(0.0);
    status.set_wrap(true);

    let autosync_workspace = ws.name.clone();
    let autosync_db_path = db_path.to_path_buf();
    let autosync_status = status.clone();
    let autosync_refresh = refresh_hub.clone();
    let autosync_panel = panel.clone();
    glib::timeout_add_local(std::time::Duration::from_secs(3), move || {
        if autosync_panel.root().is_none() {
            return glib::ControlFlow::Break;
        }
        match WorkspaceStore::open(autosync_db_path.clone())
            .and_then(|store| store.spotlight_sync_if_changed(&autosync_workspace))
        {
            Ok(Some(session)) => {
                autosync_status.set_text(&format!(
                    "Spotlight auto-synced for {}",
                    session.workspace_name
                ));
                autosync_refresh.refresh(RefreshScope::All);
            }
            Ok(None) => {}
            Err(err) => {
                autosync_status.set_text(&format!("Spotlight auto-sync paused: {err:#}"));
            }
        }
        glib::ControlFlow::Continue
    });

    let setup_workspace = ws.name.clone();
    let db_path_setup = db_path.to_path_buf();
    let refresh_setup = refresh_hub.clone();
    let status_setup = status.clone();
    let toast_setup = toast_overlay.clone();
    setup_btn.connect_clicked(move |_| {
        status_setup.set_text("Starting setup...");
        match WorkspaceStore::open(db_path_setup.clone())
            .and_then(|store| store.setup_workspace(&setup_workspace))
        {
            Ok(record) => {
                let message = format!("Setup started: pid {}", record.pid);
                apply_action_feedback(&status_setup, &toast_setup, &message, true);
            }
            Err(err) => apply_runtime_action_feedback(
                &status_setup,
                &toast_setup,
                runtime_action_failure_feedback("Setup", &err),
            ),
        }
        refresh_setup.refresh(RefreshScope::All);
    });

    let run_workspace = ws.name.clone();
    let db_path_run = db_path.to_path_buf();
    let refresh_run = refresh_hub.clone();
    let status_run = status.clone();
    let toast_run = toast_overlay.clone();
    run_btn.connect_clicked(move |_| {
        status_run.set_text("Starting run...");
        match WorkspaceStore::open(db_path_run.clone())
            .and_then(|store| store.run_workspace(&run_workspace))
        {
            Ok(record) => {
                let message = format!("Run started: pid {}", record.pid);
                apply_action_feedback(&status_run, &toast_run, &message, true);
            }
            Err(err) => apply_runtime_action_feedback(
                &status_run,
                &toast_run,
                runtime_action_failure_feedback("Run", &err),
            ),
        }
        refresh_run.refresh(RefreshScope::All);
    });

    let stop_workspace = ws.name.clone();
    let db_path_stop = db_path.to_path_buf();
    let refresh_stop = refresh_hub.clone();
    let status_stop = status.clone();
    let toast_stop = toast_overlay.clone();
    stop_btn.connect_clicked(move |_| {
        status_stop.set_text("Stopping run...");
        match WorkspaceStore::open(db_path_stop.clone())
            .and_then(|store| store.stop_workspace(&stop_workspace))
        {
            Ok(record) => {
                let message = format!("Stopped pid {}", record.pid);
                apply_action_feedback(&status_stop, &toast_stop, &message, true);
            }
            Err(err) => apply_runtime_action_feedback(
                &status_stop,
                &toast_stop,
                runtime_action_failure_feedback("Stop", &err),
            ),
        }
        refresh_stop.refresh(RefreshScope::All);
    });

    let spotlight_workspace = ws.name.clone();
    let db_path_spotlight_on = db_path.to_path_buf();
    let refresh_spotlight_on = refresh_hub.clone();
    let status_spotlight_on = status.clone();
    let toast_spotlight_on = toast_overlay.clone();
    spotlight_on_btn.connect_clicked(move |_| {
        status_spotlight_on.set_text("Starting Spotlight...");
        match WorkspaceStore::open(db_path_spotlight_on.clone())
            .and_then(|store| store.spotlight_start(&spotlight_workspace))
        {
            Ok(session) => {
                let message = format!("Spotlight active for {}", session.workspace_name);
                apply_action_feedback(&status_spotlight_on, &toast_spotlight_on, &message, true);
            }
            Err(err) => apply_runtime_action_feedback(
                &status_spotlight_on,
                &toast_spotlight_on,
                runtime_action_failure_feedback("Spotlight", &err),
            ),
        }
        refresh_spotlight_on.refresh(RefreshScope::All);
    });

    let spotlight_sync_workspace = ws.name.clone();
    let db_path_spotlight_sync = db_path.to_path_buf();
    let refresh_spotlight_sync = refresh_hub.clone();
    let status_spotlight_sync = status.clone();
    let toast_spotlight_sync = toast_overlay.clone();
    spotlight_sync_btn.connect_clicked(move |_| {
        status_spotlight_sync.set_text("Syncing Spotlight...");
        match WorkspaceStore::open(db_path_spotlight_sync.clone())
            .and_then(|store| store.spotlight_sync(&spotlight_sync_workspace))
        {
            Ok(session) => {
                let message = format!("Spotlight synced for {}", session.workspace_name);
                apply_action_feedback(
                    &status_spotlight_sync,
                    &toast_spotlight_sync,
                    &message,
                    true,
                );
            }
            Err(err) => apply_runtime_action_feedback(
                &status_spotlight_sync,
                &toast_spotlight_sync,
                runtime_action_failure_feedback("Spotlight sync", &err),
            ),
        }
        refresh_spotlight_sync.refresh(RefreshScope::All);
    });

    let spotlight_repair_workspace = ws.name.clone();
    let db_path_spotlight_repair = db_path.to_path_buf();
    let refresh_spotlight_repair = refresh_hub.clone();
    let status_spotlight_repair = status.clone();
    let toast_spotlight_repair = toast_overlay.clone();
    spotlight_repair_btn.connect_clicked(move |_| {
        status_spotlight_repair.set_text("Repairing Spotlight root: discarding root-only edits...");
        match WorkspaceStore::open(db_path_spotlight_repair.clone())
            .and_then(|store| store.spotlight_repair_root(&spotlight_repair_workspace))
        {
            Ok(session) => {
                let message = format!("Spotlight root repaired for {}", session.workspace_name);
                apply_action_feedback(
                    &status_spotlight_repair,
                    &toast_spotlight_repair,
                    &message,
                    true,
                );
            }
            Err(err) => apply_runtime_action_feedback(
                &status_spotlight_repair,
                &toast_spotlight_repair,
                runtime_action_failure_feedback("Spotlight repair", &err),
            ),
        }
        refresh_spotlight_repair.refresh(RefreshScope::All);
    });

    let spotlight_stop_workspace = ws.name.clone();
    let db_path_spotlight_off = db_path.to_path_buf();
    let refresh_spotlight_off = refresh_hub.clone();
    let status_spotlight_off = status.clone();
    let toast_spotlight_off = toast_overlay;
    spotlight_off_btn.connect_clicked(move |_| {
        status_spotlight_off.set_text("Stopping Spotlight...");
        match WorkspaceStore::open(db_path_spotlight_off.clone())
            .and_then(|store| store.spotlight_stop(&spotlight_stop_workspace))
        {
            Ok(session) => {
                let message = format!("Spotlight stopped for {}", session.workspace_name);
                apply_action_feedback(&status_spotlight_off, &toast_spotlight_off, &message, true);
            }
            Err(err) => apply_runtime_action_feedback(
                &status_spotlight_off,
                &toast_spotlight_off,
                runtime_action_failure_feedback("Spotlight stop", &err),
            ),
        }
        refresh_spotlight_off.refresh(RefreshScope::All);
    });

    let path = ws.path.clone();
    folder_btn.connect_clicked(move |_| {
        let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
    });

    let launch_row = make_action_row();
    launch_row.append(&setup_btn);
    launch_row.append(&run_btn);
    launch_row.append(&stop_btn);
    let spotlight_row = make_action_row();
    spotlight_row.append(&spotlight_on_btn);
    spotlight_row.append(&spotlight_sync_btn);
    spotlight_row.append(&spotlight_repair_btn);
    spotlight_row.append(&spotlight_off_btn);
    let utility_row = make_action_row();
    utility_row.append(&folder_btn);
    actions.append(&launch_row);
    actions.append(&spotlight_row);
    actions.append(&utility_row);
    panel.append(&actions);
    panel.append(&detail_row("Setup", &latest_setup_line(store, &ws.name)));
    panel.append(&detail_row("Latest", &latest_runtime_line(store, &ws.name)));
    panel.append(&detail_row("Spotlight", &spotlight_line(store, &ws.name)));
    panel.append(&detail_row(
        "Setup Log",
        &latest_setup_log_line(store, &ws.name),
    ));
    panel.append(&detail_row(
        "Run Log",
        &latest_run_log_line(store, &ws.name),
    ));
    panel.append(&status);
    panel
}

fn lifecycle_panel(
    db_path: &Path,
    ws: &Workspace,
    state: &AppState,
    refresh_hub: RefreshHub,
    toast_overlay: ToastOverlay,
) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 10);
    panel.add_css_class("command-panel");
    panel.append(&section_title("Workspace Actions"));

    let row = make_action_stack();
    let rename_entry = Entry::new();
    rename_entry.set_placeholder_text(Some("new workspace name"));
    rename_entry.set_text(&ws.name);
    let rename_btn = secondary_button("Rename");
    let confirm = CheckButton::with_label("Confirm archive/discard");
    let archive_btn = secondary_button("Archive");
    let restore_btn = flat_button("Restore");
    let discard_btn = destructive_button("Discard");
    let progress = Label::new(None);
    progress.add_css_class("card-meta");
    progress.set_xalign(0.0);
    progress.set_wrap(true);

    let db_rename = db_path.to_path_buf();
    let current_name = ws.name.clone();
    let state_after_rename = state.clone();
    let refresh_after_rename = refresh_hub.clone();
    let progress_rename = progress.clone();
    let toast_rename = toast_overlay.clone();
    let rename_entry_clone = rename_entry.clone();
    rename_btn.connect_clicked(move |_| {
        let new_name = rename_entry_clone.text().trim().to_owned();
        if new_name.is_empty() || new_name == current_name {
            progress_rename.set_text("Enter a different workspace name.");
            return;
        }
        progress_rename.set_text("Renaming...");
        match WorkspaceStore::open(db_rename.clone())
            .and_then(|store| store.rename(&current_name, &new_name))
        {
            Ok(workspace) => {
                state_after_rename.set_selected_workspace(Some(workspace.name.clone()));
                progress_rename.set_text(&format!("Renamed to {}", workspace.name));
            }
            Err(err) => apply_runtime_action_feedback(
                &progress_rename,
                &toast_rename,
                lifecycle_action_failure_feedback("Rename", &err),
            ),
        }
        refresh_after_rename.refresh(RefreshScope::All);
    });

    for (button, action) in [
        (archive_btn.clone(), "archive"),
        (restore_btn.clone(), "restore"),
        (discard_btn.clone(), "discard"),
    ] {
        let workspace = ws.name.clone();
        let db_action = db_path.to_path_buf();
        let refresh_after_action = refresh_hub.clone();
        let confirm_action = confirm.clone();
        let progress_action = progress.clone();
        let toast_action = toast_overlay.clone();
        button.connect_clicked(move |_| {
            if matches!(action, "archive" | "discard") && !confirm_action.is_active() {
                progress_action.set_text("Check confirm before archive/discard.");
                return;
            }
            progress_action.set_text(&format!("{action} in progress..."));
            let result = WorkspaceStore::open(db_action.clone()).and_then(|store| match action {
                "archive" => store.archive(&workspace, false),
                "restore" => store.restore(&workspace),
                "discard" => store.discard(&workspace),
                _ => unreachable!(),
            });
            match result {
                Ok(workspace) => progress_action.set_text(&format!(
                    "{} complete: {}",
                    title_case_workspace(action),
                    workspace.name
                )),
                Err(err) => apply_runtime_action_feedback(
                    &progress_action,
                    &toast_action,
                    lifecycle_action_failure_feedback(&title_case_workspace(action), &err),
                ),
            }
            refresh_after_action.refresh(RefreshScope::All);
        });
    }

    let rename_row = make_action_row();
    rename_row.append(&rename_entry);
    rename_row.append(&rename_btn);
    let lifecycle_row = make_action_row();
    lifecycle_row.append(&confirm);
    lifecycle_row.append(&archive_btn);
    lifecycle_row.append(&restore_btn);
    lifecycle_row.append(&discard_btn);
    row.append(&rename_row);
    row.append(&lifecycle_row);
    panel.append(&row);
    panel.append(&progress);
    panel
}

fn work_tabs(
    db_path: &Path,
    store: &WorkspaceStore,
    ws: &Workspace,
    state: &AppState,
    refresh_hub: RefreshHub,
    toast_overlay: ToastOverlay,
    collapse_sidebar: Rc<dyn Fn()>,
) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 8);
    let tabs = Stack::new();
    tabs.set_vexpand(true);
    let switcher = StackSwitcher::new();
    switcher.set_stack(Some(&tabs));
    switcher.add_css_class("panel-switcher");
    panel.append(&switcher);
    let terminal_preferences = store
        .workspace_view_defaults(&ws.name)
        .map(|defaults| {
            terminal::TerminalPreferences::from_config(
                defaults.terminal_font.as_deref(),
                defaults.terminal_scrollback,
            )
        })
        .unwrap_or_default();
    let terminal_command_presets = store
        .workspace_view_defaults(&ws.name)
        .map(|defaults| terminal::terminal_command_presets(&defaults.command_palette_presets))
        .unwrap_or_else(|_| terminal::terminal_command_presets(&[]));

    tabs.add_titled(
        &changes_checks_review_tabs(
            db_path,
            store,
            &ws.name,
            state.clone(),
            refresh_hub.clone(),
            toast_overlay.clone(),
        ),
        Some("work"),
        "Changes",
    );
    tabs.add_titled(
        &parallel_agents_panel(
            db_path,
            ws,
            state.clone(),
            refresh_hub.clone(),
            collapse_sidebar.clone(),
        ),
        Some("chat-terminal"),
        "Chat",
    );
    tabs.add_titled(
        &terminal::embedded_terminal_panel(
            db_path.to_path_buf(),
            &ws.name,
            &ws.path,
            true,
            refresh_hub.clone(),
            terminal_preferences,
            terminal_command_presets,
        ),
        Some("terminal"),
        "Terminal",
    );
    tabs.add_titled(
        &workspace_todos_panel(store, &ws.name),
        Some("todos"),
        "Todos",
    );
    tabs.add_titled(
        &workspace_checkpoint_panel(
            db_path,
            &ws.name,
            refresh_hub.clone(),
            toast_overlay.clone(),
        ),
        Some("checkpoints"),
        "Checkpoints",
    );
    tabs.add_titled(
        &text_panel(&workspace_processes_text(store, &ws.name)),
        Some("processes"),
        "Processes",
    );
    tabs.set_visible_child_name(workspace_tab_stack_name(
        &state.snapshot().active_workspace_tab,
    ));

    let state_tabs = state.clone();
    tabs.connect_visible_child_name_notify(move |stack| {
        match stack.visible_child_name().as_deref() {
            Some("work") => {
                if !matches!(
                    state_tabs.snapshot().active_workspace_tab,
                    WorkspaceTab::Checks | WorkspaceTab::Review
                ) {
                    state_tabs.set_active_workspace_tab(WorkspaceTab::Changes);
                }
            }
            Some("todos") => state_tabs.set_active_workspace_tab(WorkspaceTab::Todos),
            Some("processes") => state_tabs.set_active_workspace_tab(WorkspaceTab::Processes),
            Some("terminal") => state_tabs.set_active_workspace_tab(WorkspaceTab::Terminal),
            Some("chat-terminal") => state_tabs.set_active_workspace_tab(WorkspaceTab::Chats),
            Some("checkpoints") => state_tabs.set_active_workspace_tab(WorkspaceTab::Checkpoints),
            _ => state_tabs.set_active_workspace_tab(WorkspaceTab::Chats),
        }
    });
    panel.append(&tabs);
    panel
}

fn workspace_tab_stack_name(tab: &WorkspaceTab) -> &'static str {
    match tab {
        WorkspaceTab::Chats => "chat-terminal",
        WorkspaceTab::Changes | WorkspaceTab::Checks | WorkspaceTab::Review => "work",
        WorkspaceTab::Checkpoints => "checkpoints",
        WorkspaceTab::Todos => "todos",
        WorkspaceTab::Processes => "processes",
        WorkspaceTab::Terminal => "terminal",
    }
}

fn workspace_checkpoint_panel(
    db_path: &Path,
    name: &str,
    refresh_hub: RefreshHub,
    toast_overlay: ToastOverlay,
) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 8);
    panel.add_css_class("command-panel");
    panel.append(&section_title("Checkpoints"));

    let create_row = make_action_row();
    let message = Entry::new();
    message.set_placeholder_text(Some("Checkpoint message"));
    message.set_hexpand(true);
    let create_btn = text_button("Create");
    create_btn.add_css_class("suggested-action");
    let feedback = Label::new(None);
    feedback.add_css_class("card-meta");
    feedback.set_xalign(0.0);
    feedback.set_wrap(true);

    let db_for_create = db_path.to_path_buf();
    let workspace_for_create = name.to_owned();
    let refresh_after_create = refresh_hub.clone();
    let feedback_for_create = feedback.clone();
    let toast_for_create = toast_overlay.clone();
    let message_for_create = message.clone();
    create_btn.connect_clicked(move |_| {
        let message = message_for_create.text().trim().to_owned();
        if message.is_empty() {
            apply_action_feedback(
                &feedback_for_create,
                &toast_for_create,
                "Checkpoint message required.",
                true,
            );
            return;
        }
        match WorkspaceStore::open(db_for_create.clone())
            .and_then(|store| store.checkpoint_create(&workspace_for_create, &message, None))
        {
            Ok(cp) => {
                apply_action_feedback(
                    &feedback_for_create,
                    &toast_for_create,
                    &format!("Created checkpoint #{}", cp.id),
                    true,
                );
                message_for_create.set_text("");
                refresh_after_create.refresh(RefreshScope::Workspace);
            }
            Err(err) => apply_action_feedback(
                &feedback_for_create,
                &toast_for_create,
                &format!("Create checkpoint failed: {err:#}"),
                true,
            ),
        }
    });
    create_row.append(&message);
    create_row.append(&create_btn);
    panel.append(&create_row);
    panel.append(&feedback);

    let mut checkpoints_loaded = Vec::new();
    let mut list_error = None;
    match WorkspaceStore::open(db_path).and_then(|store| store.checkpoint_list(name)) {
        Ok(checkpoints) => {
            checkpoints_loaded = checkpoints;
        }
        Err(err) => {
            list_error = Some(err.to_string());
        }
    }

    if let Some(err) = list_error {
        panel.append(&detail_row(
            "Checkpoint list",
            &format!("Could not load checkpoints: {err}"),
        ));
        return panel;
    }

    if checkpoints_loaded.is_empty() {
        panel.append(&detail_row("Checkpoints", "No checkpoints yet."));
        return panel;
    }

    let header = GBox::new(Orientation::Horizontal, 8);
    header.append(&detail_row("ID", "Status"));
    panel.append(&header);

    for checkpoint in checkpoints_loaded {
        let row = make_action_row();
        let label = Label::new(Some(&format!(
            "#{} {} - {}",
            checkpoint.id, checkpoint.created_at, checkpoint.message
        )));
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.set_hexpand(true);
        let restore_btn = secondary_button("Restore");
        let checkpoint_id = checkpoint.id;
        let workspace_for_restore = name.to_owned();
        let db_for_restore = db_path.to_path_buf();
        let refresh_after_restore = refresh_hub.clone();
        let feedback_for_restore = feedback.clone();
        let toast_for_restore = toast_overlay.clone();
        restore_btn.connect_clicked(move |_| {
            match WorkspaceStore::open(db_for_restore.clone())
                .and_then(|store| store.checkpoint_restore(&workspace_for_restore, checkpoint_id))
            {
                Ok(cp) => {
                    apply_action_feedback(
                        &feedback_for_restore,
                        &toast_for_restore,
                        &format!("Restored checkpoint #{}", cp.id),
                        true,
                    );
                    refresh_after_restore.refresh(RefreshScope::Workspace);
                }
                Err(err) => {
                    apply_action_feedback(
                        &feedback_for_restore,
                        &toast_for_restore,
                        &format!("Restore checkpoint failed: {err:#}"),
                        true,
                    );
                }
            }
        });
        row.append(&label);
        row.append(&restore_btn);
        panel.append(&row);
    }

    panel
}

fn changes_checks_review_tabs(
    db_path: &Path,
    store: &WorkspaceStore,
    name: &str,
    app_state: AppState,
    refresh_hub: RefreshHub,
    toast_overlay: ToastOverlay,
) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 8);
    let tabs = Stack::new();
    tabs.set_vexpand(true);
    let switcher = StackSwitcher::new();
    switcher.set_stack(Some(&tabs));
    switcher.add_css_class("panel-switcher");
    panel.append(&switcher);
    tabs.add_titled(
        &workspace_changes_panel(
            db_path,
            store,
            name,
            refresh_hub.clone(),
            toast_overlay.clone(),
        ),
        Some("changes"),
        "Changes",
    );
    tabs.add_titled(
        &workspace_checks_panel(
            db_path,
            store,
            name,
            app_state.clone(),
            refresh_hub.clone(),
            toast_overlay.clone(),
        ),
        Some("checks"),
        "Checks",
    );
    tabs.add_titled(
        &workspace_review_panel(
            db_path,
            store,
            name,
            app_state.clone(),
            refresh_hub,
            toast_overlay,
        ),
        Some("review"),
        "Review",
    );
    tabs.set_visible_child_name(changes_checks_review_tab_stack_name(
        &app_state.snapshot().active_workspace_tab,
    ));
    let state_tabs = app_state.clone();
    tabs.connect_visible_child_name_notify(move |stack| {
        match stack.visible_child_name().as_deref() {
            Some("checks") => state_tabs.set_active_workspace_tab(WorkspaceTab::Checks),
            Some("review") => state_tabs.set_active_workspace_tab(WorkspaceTab::Review),
            Some("changes") => state_tabs.set_active_workspace_tab(WorkspaceTab::Changes),
            _ => {}
        }
    });
    panel.append(&tabs);
    panel
}

fn changes_checks_review_tab_stack_name(tab: &WorkspaceTab) -> &'static str {
    match tab {
        WorkspaceTab::Checks => "checks",
        WorkspaceTab::Review => "review",
        _ => "changes",
    }
}

fn chat_terminal_split(
    db_path: &Path,
    ws: &Workspace,
    app_state: AppState,
    refresh_hub: RefreshHub,
    terminal_preferences: terminal::TerminalPreferences,
    terminal_command_presets: Vec<terminal::TerminalCommandPreset>,
    collapse_sidebar: Rc<dyn Fn()>,
) -> Paned {
    let split = Paned::new(Orientation::Horizontal);
    split.set_wide_handle(true);
    split.set_position(520);
    split.set_shrink_start_child(true);
    split.set_shrink_end_child(true);

    let chat_box = GBox::new(Orientation::Vertical, 8);
    chat_box.add_css_class("command-panel");
    chat_box.add_css_class("session-tool-surface");
    chat_box.append(&section_title("Chat"));
    let db_for_sessions = db_path.to_path_buf();
    let workspace_for_sessions = ws.name.clone();
    let refresh_sessions = refresh_hub.clone();
    chat_box.append(&session_surface::agent_session_panel(
        db_for_sessions,
        &workspace_for_sessions,
        &workspace_repository_name_from_db(db_path, &workspace_for_sessions),
        &ws.branch,
        collapse_sidebar.clone(),
        app_state,
        move || {
            refresh_sessions.refresh(RefreshScope::Sidebar);
            refresh_sessions.refresh(RefreshScope::Dashboard);
            refresh_sessions.refresh(RefreshScope::History);
        },
        false,
        None,
    ));
    for chat in history::sessions_for_workspace_path(db_path, &ws.path)
        .into_iter()
        .take(8)
    {
        chat_box.append(&history::session_summary_row(&chat));
    }
    chat_box.append(&linked_directories_panel(
        db_path,
        &ws.name,
        refresh_hub.clone(),
    ));

    let terminal_box = GBox::new(Orientation::Vertical, 8);
    terminal_box.add_css_class("command-panel");
    terminal_box.add_css_class("session-tool-surface");
    terminal_box.append(&section_title("Terminal"));
    terminal_box.append(&terminal::embedded_terminal_panel(
        db_path.to_path_buf(),
        &ws.name,
        &ws.path,
        false,
        refresh_hub,
        terminal_preferences,
        terminal_command_presets,
    ));

    split.set_start_child(Some(&chat_box));
    split.set_end_child(Some(&terminal_box));
    split
}

fn parallel_agents_panel(
    db_path: &Path,
    ws: &Workspace,
    app_state: AppState,
    refresh_hub: RefreshHub,
    collapse_sidebar: Rc<dyn Fn()>,
) -> Paned {
    let split = Paned::new(Orientation::Horizontal);
    split.set_wide_handle(true);
    split.set_position(580);
    split.set_shrink_start_child(true);
    split.set_shrink_end_child(true);

    let chat_box = GBox::new(Orientation::Vertical, 8);
    chat_box.add_css_class("command-panel");
    chat_box.add_css_class("session-tool-surface");
    chat_box.append(&section_title("Chat"));
    let refresh_chat = refresh_hub.clone();
    chat_box.append(&session_surface::agent_session_panel(
        db_path.to_path_buf(),
        &ws.name,
        &WorkspaceStore::open(db_path)
            .ok()
            .map(|store| workspace_repository_name(&store, &ws.name))
            .unwrap_or_else(|| ws.name.clone()),
        &ws.branch,
        collapse_sidebar.clone(),
        app_state,
        move || {
            refresh_chat.refresh(RefreshScope::Sidebar);
            refresh_chat.refresh(RefreshScope::Dashboard);
            refresh_chat.refresh(RefreshScope::History);
        },
        false,
        None,
    ));
    for chat in history::sessions_for_workspace_path(db_path, &ws.path)
        .into_iter()
        .take(8)
    {
        chat_box.append(&history::session_summary_row(&chat));
    }
    chat_box.append(&linked_directories_panel(db_path, &ws.name, refresh_hub));

    let right = GBox::new(Orientation::Vertical, 0);
    right.add_css_class("command-panel");
    right.add_css_class("session-tool-surface");

    let file_tabs = Stack::new();
    file_tabs.set_vexpand(true);
    let file_switcher = StackSwitcher::new();
    file_switcher.set_stack(Some(&file_tabs));
    file_switcher.add_css_class("panel-switcher");

    let changes_text = WorkspaceStore::open(db_path)
        .and_then(|store| store.diff_file_summaries(&ws.name))
        .map(|summaries| format_diff_file_summary(&summaries))
        .unwrap_or_else(|_| "No changes yet.\n".to_owned());
    file_tabs.add_titled(&text_panel(&changes_text), Some("changes"), "Changes");

    let checks_text = WorkspaceStore::open(db_path)
        .map(|store| workspace_checks_text(&store, &ws.name))
        .unwrap_or_else(|_| "No checks yet.\n".to_owned());
    file_tabs.add_titled(&text_panel(&checks_text), Some("checks"), "Checks");

    right.append(&file_switcher);
    right.append(&file_tabs);

    let run_label = section_title("Run");
    run_label.set_margin_top(8);
    run_label.set_margin_start(8);
    right.append(&run_label);

    let run_text = WorkspaceStore::open(db_path)
        .map(|store| latest_run_log_line(&store, &ws.name))
        .unwrap_or_else(|_| "No run log yet.\n".to_owned());
    let run_view = TextView::new();
    run_view.set_editable(false);
    run_view.set_monospace(true);
    run_view.add_css_class("history-view");
    run_view.buffer().set_text(&run_text);
    let run_scroll = ScrolledWindow::new();
    run_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    run_scroll.set_min_content_height(120);
    run_scroll.set_child(Some(&run_view));
    right.append(&run_scroll);

    split.set_start_child(Some(&chat_box));
    split.set_end_child(Some(&right));
    split
}

fn linked_directories_panel(db_path: &Path, name: &str, refresh_hub: RefreshHub) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 6);
    panel.append(&section_title("Linked Directories"));

    let links_view = TextView::new();
    links_view.set_editable(false);
    links_view.set_monospace(true);
    links_view.add_css_class("history-view");
    links_view
        .buffer()
        .set_text(&linked_directories_text(db_path, name));
    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    scroll.set_min_content_height(86);
    scroll.set_child(Some(&links_view));
    panel.append(&scroll);

    let row = GBox::new(Orientation::Horizontal, 8);
    let target_entry = Entry::new();
    target_entry.set_placeholder_text(Some("Target workspace name"));
    target_entry.set_hexpand(true);
    let link_btn = text_button("Link");
    link_btn.add_css_class("suggested-action");
    let unlink_btn = destructive_button("Unlink");
    row.append(&target_entry);
    row.append(&link_btn);
    row.append(&unlink_btn);
    panel.append(&row);

    let db_for_link = db_path.to_path_buf();
    let workspace_for_link = name.to_owned();
    let target_for_link = target_entry.clone();
    let buffer_for_link = links_view.buffer();
    let hub_for_link = refresh_hub.clone();
    link_btn.connect_clicked(move |_| {
        let target = target_for_link.text().trim().to_owned();
        if target.is_empty() {
            buffer_for_link.set_text("Enter a target workspace name to link.\n");
            return;
        }
        match WorkspaceStore::open(db_for_link.clone())
            .and_then(|store| store.link_workspace_directory(&workspace_for_link, &target))
        {
            Ok(_) => {
                buffer_for_link
                    .set_text(&linked_directories_text(&db_for_link, &workspace_for_link));
                hub_for_link.refresh(RefreshScope::Workspace);
            }
            Err(err) => buffer_for_link.set_text(&format!("Could not link directory: {err:#}\n")),
        }
    });

    let db_for_unlink = db_path.to_path_buf();
    let workspace_for_unlink = name.to_owned();
    let target_for_unlink = target_entry;
    let buffer_for_unlink = links_view.buffer();
    let hub_for_unlink = refresh_hub;
    unlink_btn.connect_clicked(move |_| {
        let target = target_for_unlink.text().trim().to_owned();
        if target.is_empty() {
            buffer_for_unlink.set_text("Enter a target workspace name to unlink.\n");
            return;
        }
        match WorkspaceStore::open(db_for_unlink.clone())
            .and_then(|store| store.unlink_workspace_directory(&workspace_for_unlink, &target))
        {
            Ok(_) => {
                buffer_for_unlink.set_text(&linked_directories_text(
                    &db_for_unlink,
                    &workspace_for_unlink,
                ));
                hub_for_unlink.refresh(RefreshScope::Workspace);
            }
            Err(err) => {
                buffer_for_unlink.set_text(&format!("Could not unlink directory: {err:#}\n"))
            }
        }
    });

    panel
}

fn linked_directories_text(db_path: &Path, name: &str) -> String {
    match WorkspaceStore::open(db_path).and_then(|store| store.list_linked_directories(name)) {
        Ok(links) if links.is_empty() => "No linked directories.\n".to_owned(),
        Ok(links) => links
            .into_iter()
            .map(|link| {
                format!(
                    "{} -> {}\nlink: {}\n",
                    link.target_workspace_name,
                    link.target_workspace_path.display(),
                    link.link_path.display()
                )
            })
            .collect(),
        Err(err) => format!("Could not read linked directories: {err:#}\n"),
    }
}

fn section_title(text: &str) -> Label {
    let label = Label::new(Some(text));
    label.add_css_class("section-title");
    label.set_xalign(0.0);
    label
}

fn text_panel(text: &str) -> ScrolledWindow {
    let view = TextView::new();
    view.set_editable(false);
    view.set_monospace(true);
    view.add_css_class("history-view");
    view.buffer().set_text(text);
    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    scroll.set_vexpand(true);
    scroll.set_child(Some(&view));
    scroll
}

fn apply_diff_tags(buffer: &gtk::TextBuffer) {
    {
        let s = buffer.start_iter();
        let e = buffer.end_iter();
        buffer.remove_all_tags(&s, &e);
    }
    let text = {
        let s = buffer.start_iter();
        let e = buffer.end_iter();
        buffer.text(&s, &e, false).to_string()
    };

    let table = buffer.tag_table();

    macro_rules! ensure_tag {
        ($name:expr, $bg:expr, $fg:expr) => {{
            if let Some(t) = table.lookup($name) {
                t
            } else {
                let t = TextTag::new(Some($name));
                t.set_property("paragraph-background", $bg);
                t.set_property("paragraph-background-set", true);
                t.set_property("foreground", $fg);
                t.set_property("foreground-set", true);
                table.add(&t);
                t
            }
        }};
    }

    let add_tag = ensure_tag!("diff-add", "#0d2612", "#8fcf9f");
    let del_tag = ensure_tag!("diff-del", "#2d0d0d", "#cf8f8f");
    let hunk_tag = ensure_tag!("diff-hunk", "#0f1a2d", "#7fa0bf");
    let header_tag = ensure_tag!("diff-header", "#1a1a1a", "#909090");

    let mut iter = buffer.start_iter();
    for line in text.split('\n') {
        let line_start = iter;
        let mut line_end = iter;
        line_end.forward_to_line_end();

        if line.starts_with('+') && !line.starts_with("+++") {
            buffer.apply_tag(&add_tag, &line_start, &line_end);
        } else if line.starts_with('-') && !line.starts_with("---") {
            buffer.apply_tag(&del_tag, &line_start, &line_end);
        } else if line.starts_with("@@") {
            buffer.apply_tag(&hunk_tag, &line_start, &line_end);
        } else if line.starts_with("diff ")
            || line.starts_with("index ")
            || line.starts_with("--- ")
            || line.starts_with("+++ ")
        {
            buffer.apply_tag(&header_tag, &line_start, &line_end);
        }

        iter.forward_line();
    }
}

fn workspace_changes_panel(
    db_path: &Path,
    store: &WorkspaceStore,
    name: &str,
    _refresh_hub: RefreshHub,
    _toast_overlay: ToastOverlay,
) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 8);
    let header_row = GBox::new(Orientation::Horizontal, 8);
    header_row.add_css_class("ws-changes-header");
    let title = Label::new(Some("Changes"));
    title.add_css_class("section-title");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    let scope_label = Label::new(Some("Total changes"));
    scope_label.add_css_class("detail-label");
    let menu_btn = text_button("⋯");
    menu_btn.add_css_class("ws-changes-menu-btn");
    header_row.append(&title);
    header_row.append(&scope_label);
    header_row.append(&menu_btn);
    panel.append(&header_row);

    let status = Label::new(Some(&format!(
        "Branch: {}",
        workspace_branch_state_text(store, name)
    )));
    status.add_css_class("card-meta");
    status.set_xalign(0.0);
    status.set_wrap(true);
    panel.append(&status);

    let body_stack = Stack::new();
    body_stack.set_vexpand(true);
    body_stack.set_hexpand(true);

    let total_view = workspace_changes_text_view(store, name);
    body_stack.add_named(&total_view, Some("total"));

    let untracked_view = workspace_untracked_changes_view(db_path, name);
    body_stack.add_named(&untracked_view, Some("untracked"));

    let last_turn_view = workspace_last_turn_changes_view(db_path, name);
    body_stack.add_named(&last_turn_view, Some("last_turn"));

    let checks_view = workspace_checks_text_view(store, name);
    body_stack.add_named(&checks_view, Some("checks"));

    let commits_view = workspace_commit_browser_view(db_path, name);
    body_stack.add_named(&commits_view, Some("commits"));

    body_stack.set_visible_child_name("total");
    panel.append(&body_stack);

    let popover = gtk::Popover::new();
    popover.set_parent(&menu_btn);
    let menu = GBox::new(Orientation::Vertical, 4);
    menu.add_css_class("chat-menu-list");

    for (label, target, scope_text) in [
        ("Total changes", "total", "Total changes"),
        ("Untracked changes", "untracked", "Untracked changes"),
        ("Last turn changes", "last_turn", "Last turn changes"),
        ("By commit", "commits", "Changes by commit"),
        ("Checks", "checks", "Checks"),
    ] {
        let item = text_button(label);
        item.add_css_class("chat-menu-item");
        let body_stack_for_item = body_stack.clone();
        let scope_label_for_item = scope_label.clone();
        let popover_for_item = popover.clone();
        item.connect_clicked(move |_| {
            body_stack_for_item.set_visible_child_name(target);
            scope_label_for_item.set_text(scope_text);
            popover_for_item.popdown();
        });
        menu.append(&item);
    }
    popover.set_child(Some(&menu));
    menu_btn.connect_clicked(move |_| {
        popover.popup();
    });

    let feedback = Label::new(Some("Use the menu for diffs, commit views, and PR checks."));
    feedback.add_css_class("card-meta");
    feedback.set_xalign(0.0);
    feedback.set_wrap(true);
    panel.append(&feedback);

    panel
}

fn workspace_changes_text(store: &WorkspaceStore, name: &str) -> String {
    let mut out = String::new();
    out.push_str("Recent commits\n");
    out.push_str(
        &store
            .git_log_oneline(name, 12)
            .unwrap_or_else(|err| format!("Could not read log: {err:#}\n")),
    );
    out.push_str("\n\nStatus\n");
    out.push_str(
        &store
            .git_status_short(name)
            .unwrap_or_else(|err| format!("Could not read status: {err:#}\n")),
    );
    out.push_str("\n\n");
    match store.diff_file_summaries(name) {
        Ok(summaries) => out.push_str(&format_diff_file_summary(&summaries)),
        Err(err) => out.push_str(&format!(
            "Files changed\nCould not read diff summary: {err:#}\n"
        )),
    }
    out.push_str("\n\nDiff\n");
    out.push_str(
        &store
            .unified_diff(name, None)
            .unwrap_or_else(|err| format!("Could not read diff: {err:#}\n")),
    );
    out
}

fn workspace_branch_state_text(store: &WorkspaceStore, name: &str) -> String {
    match store.checks_summary(name) {
        Ok(summary) => summary
            .branch_push_state
            .map(|state| {
                if state.has_upstream {
                    format!("ahead {} / behind {}", state.ahead, state.behind)
                } else {
                    "no upstream".to_owned()
                }
            })
            .unwrap_or_else(|| "unavailable".to_owned()),
        Err(err) => format!("Could not read branch state: {err:#}"),
    }
}

fn workspace_diff_text(store: &WorkspaceStore, name: &str, path: Option<&str>) -> String {
    match path {
        Some(path) => store
            .unified_diff(name, Some(Path::new(path)))
            .unwrap_or_else(|err| format!("Could not read diff for {path}: {err:#}\n")),
        None => store
            .unified_diff(name, None)
            .unwrap_or_else(|err| format!("Could not read diff: {err:#}\n")),
    }
}

fn workspace_diff_text_for_path(db_path: &Path, name: &str, path: Option<&str>) -> String {
    WorkspaceStore::open(db_path)
        .map(|store| workspace_diff_text(&store, name, path))
        .unwrap_or_else(|err| format!("Could not open workspace database: {err:#}\n"))
}

fn workspace_changes_text_view(store: &WorkspaceStore, name: &str) -> ScrolledWindow {
    let view = TextView::new();
    view.set_editable(false);
    view.set_monospace(true);
    view.set_vexpand(true);
    view.add_css_class("ws-diff-view");
    view.set_left_margin(6);
    view.set_right_margin(6);
    view.set_top_margin(4);
    view.buffer().set_text(&workspace_changes_text(store, name));
    apply_diff_tags(&view.buffer());
    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    scroll.set_child(Some(&view));
    scroll
}

fn workspace_untracked_changes_view(db_path: &Path, name: &str) -> ScrolledWindow {
    let view = TextView::new();
    view.set_editable(false);
    view.set_monospace(true);
    view.set_vexpand(true);
    view.add_css_class("ws-diff-view");
    view.set_left_margin(6);
    view.set_right_margin(6);
    view.set_top_margin(4);
    view.buffer()
        .set_text(&workspace_untracked_changes_text(db_path, name));
    apply_diff_tags(&view.buffer());
    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    scroll.set_child(Some(&view));
    scroll
}

fn workspace_last_turn_changes_view(db_path: &Path, name: &str) -> ScrolledWindow {
    let view = TextView::new();
    view.set_editable(false);
    view.set_monospace(true);
    view.set_vexpand(true);
    view.add_css_class("ws-diff-view");
    view.set_left_margin(6);
    view.set_right_margin(6);
    view.set_top_margin(4);
    view.buffer()
        .set_text(&workspace_last_turn_changes_text(db_path, name));
    apply_diff_tags(&view.buffer());
    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    scroll.set_child(Some(&view));
    scroll
}

fn workspace_checks_text_view(store: &WorkspaceStore, name: &str) -> ScrolledWindow {
    let view = TextView::new();
    view.set_editable(false);
    view.set_monospace(true);
    view.set_vexpand(true);
    view.add_css_class("ws-diff-view");
    view.set_left_margin(6);
    view.set_right_margin(6);
    view.set_top_margin(4);
    view.buffer()
        .set_text(&workspace_checks_and_todos_text(store, name));
    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    scroll.set_child(Some(&view));
    scroll
}

fn workspace_commit_browser_view(db_path: &Path, name: &str) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 8);
    panel.set_vexpand(true);
    let commit_status = Label::new(Some("Select a commit to view its diff."));
    commit_status.add_css_class("card-meta");
    commit_status.set_xalign(0.0);
    commit_status.set_wrap(true);
    panel.append(&commit_status);

    let split = Paned::new(Orientation::Horizontal);
    split.set_wide_handle(true);
    split.set_resize_start_child(true);
    split.set_shrink_end_child(false);
    let commits = recent_commit_summaries(db_path, name, 8);
    let list = GBox::new(Orientation::Vertical, 4);
    let diff_view = TextView::new();
    diff_view.set_editable(false);
    diff_view.set_monospace(true);
    diff_view.set_vexpand(true);
    diff_view.add_css_class("ws-diff-view");
    diff_view.set_left_margin(6);
    diff_view.set_right_margin(6);
    diff_view.set_top_margin(4);
    diff_view
        .buffer()
        .set_text("Select a commit on the left to inspect its diff.");
    apply_diff_tags(&diff_view.buffer());
    let diff_scroll = ScrolledWindow::new();
    diff_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    diff_scroll.set_child(Some(&diff_view));

    if commits.is_empty() {
        list.append(&detail_row("Commits", "No commits found."));
    } else {
        for commit in commits {
            let row = make_action_row();
            let btn = flat_button(&commit.label);
            btn.set_hexpand(true);
            btn.set_halign(Align::Fill);
            let db_for_commit = db_path.to_path_buf();
            let workspace_for_commit = name.to_owned();
            let diff_buffer = diff_view.buffer();
            let status_for_commit = commit_status.clone();
            let commit_hash = commit.hash.clone();
            let commit_summary = commit.summary.clone();
            btn.connect_clicked(move |_| {
                let text = WorkspaceStore::open(db_for_commit.clone())
                    .and_then(|store| store.git_show_commit(&workspace_for_commit, &commit_hash))
                    .unwrap_or_else(|err| format!("Could not read commit {commit_hash}: {err:#}"));
                diff_buffer.set_text(&text);
                apply_diff_tags(&diff_buffer);
                status_for_commit.set_text(&format!("{commit_hash} {commit_summary}"));
            });
            row.append(&btn);
            list.append(&row);
        }
    }

    let list_scroll = ScrolledWindow::new();
    list_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    list_scroll.set_min_content_width(180);
    list_scroll.set_child(Some(&list));
    split.set_start_child(Some(&list_scroll));
    split.set_end_child(Some(&diff_scroll));

    panel.append(&split);
    panel
}

#[derive(Clone)]
struct CommitSummary {
    hash: String,
    summary: String,
    label: String,
}

fn recent_commit_summaries(db_path: &Path, name: &str, limit: usize) -> Vec<CommitSummary> {
    let Ok(store) = WorkspaceStore::open(db_path) else {
        return Vec::new();
    };
    let Ok(log) = store.git_log_oneline(name, limit) else {
        return Vec::new();
    };
    log.lines()
        .filter_map(|line| {
            let (hash, summary) = line.split_once(' ')?;
            Some(CommitSummary {
                hash: hash.to_owned(),
                summary: summary.to_owned(),
                label: format!("{hash} {summary}"),
            })
        })
        .collect()
}

fn workspace_untracked_changes_text(db_path: &Path, name: &str) -> String {
    match WorkspaceStore::open(db_path).and_then(|store| store.untracked_files(name)) {
        Ok(files) if files.is_empty() => "Untracked changes\nNo untracked files.\n".to_owned(),
        Ok(files) => {
            let mut out = "Untracked changes\n".to_owned();
            for path in files {
                out.push_str(&format!("{path}\n"));
            }
            out
        }
        Err(err) => format!("Untracked changes\nCould not read files: {err:#}\n"),
    }
}

fn workspace_last_turn_changes_text(db_path: &Path, name: &str) -> String {
    WorkspaceStore::open(db_path)
        .and_then(|store| store.git_show_commit(name, "HEAD"))
        .unwrap_or_else(|err| format!("Last turn changes\nCould not read HEAD commit: {err:#}\n"))
}

fn workspace_checks_and_todos_text(store: &WorkspaceStore, name: &str) -> String {
    let mut out = workspace_checks_text(store, name);
    out.push_str("\n\nTodos\n");
    out.push_str(&workspace_todos_text(store, name));
    out
}

fn workspace_todos_text(store: &WorkspaceStore, name: &str) -> String {
    match store.list_todos(name) {
        Ok(todos) if todos.is_empty() => "No todos.\n".to_owned(),
        Ok(todos) => {
            let mut out = String::new();
            for todo in todos {
                out.push_str(&format!("#{} [{}] {}\n", todo.id, todo.status, todo.text));
            }
            out
        }
        Err(err) => format!("Could not read todos: {err:#}\n"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PullRequestStateKind {
    Open,
    Ready,
    Failed,
    Merged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PullRequestStatusSummary {
    pub label: String,
    pub css_class: &'static str,
    pub kind: PullRequestStateKind,
}

impl PullRequestStatusSummary {
    pub(crate) fn attention_label(&self) -> Option<&str> {
        match self.kind {
            PullRequestStateKind::Open => None,
            PullRequestStateKind::Ready
            | PullRequestStateKind::Failed
            | PullRequestStateKind::Merged => Some(self.label.as_str()),
        }
    }

    pub(crate) fn attention_css_class(&self) -> Option<&'static str> {
        match self.kind {
            PullRequestStateKind::Open => None,
            PullRequestStateKind::Ready
            | PullRequestStateKind::Failed
            | PullRequestStateKind::Merged => Some(self.css_class),
        }
    }
}

pub(crate) fn pull_request_status_summary(
    pr: &PullRequest,
    readiness: Option<&linux_archductor_core::workspace::PullRequestReadiness>,
    summary: &linux_archductor_core::workspace::ChecksSummary,
) -> PullRequestStatusSummary {
    if pr.state.eq_ignore_ascii_case("merged") {
        return PullRequestStatusSummary {
            label: "merged".to_owned(),
            css_class: "ws-pr-status-merged",
            kind: PullRequestStateKind::Merged,
        };
    }

    if let Some(readiness) = readiness {
        if pull_request_is_failed(readiness) {
            return PullRequestStatusSummary {
                label: "checks failed".to_owned(),
                css_class: "ws-pr-status-failed",
                kind: PullRequestStateKind::Failed,
            };
        }
        if pull_request_is_ready(readiness, summary) {
            return PullRequestStatusSummary {
                label: "ready to merge".to_owned(),
                css_class: "ws-pr-status-ready",
                kind: PullRequestStateKind::Ready,
            };
        }
    }

    PullRequestStatusSummary {
        label: pr.state.to_owned(),
        css_class: "ws-pr-status-muted",
        kind: PullRequestStateKind::Open,
    }
}

pub(crate) fn workspace_pull_request_status_summary(
    store: &WorkspaceStore,
    workspace_name: &str,
    pr: &PullRequest,
) -> PullRequestStatusSummary {
    let readiness = store
        .pull_request_panel_state(workspace_name)
        .ok()
        .and_then(|panel| panel.readiness);
    let summary = store.checks_summary(workspace_name).ok();

    match summary.as_ref() {
        Some(summary) => pull_request_status_summary(pr, readiness.as_ref(), summary),
        None => pull_request_status_summary_without_checks_summary(pr, readiness.as_ref()),
    }
}

fn pull_request_status_summary_without_checks_summary(
    pr: &PullRequest,
    readiness: Option<&linux_archductor_core::workspace::PullRequestReadiness>,
) -> PullRequestStatusSummary {
    if pr.state.eq_ignore_ascii_case("merged") {
        return PullRequestStatusSummary {
            label: "merged".to_owned(),
            css_class: "ws-pr-status-merged",
            kind: PullRequestStateKind::Merged,
        };
    }

    if let Some(readiness) = readiness {
        if pull_request_is_failed(readiness) {
            return PullRequestStatusSummary {
                label: "checks failed".to_owned(),
                css_class: "ws-pr-status-failed",
                kind: PullRequestStateKind::Failed,
            };
        }
    }

    PullRequestStatusSummary {
        label: pr.state.to_owned(),
        css_class: "ws-pr-status-muted",
        kind: PullRequestStateKind::Open,
    }
}

fn pull_request_is_failed(
    readiness: &linux_archductor_core::workspace::PullRequestReadiness,
) -> bool {
    readiness.review_decision.as_deref() == Some("CHANGES_REQUESTED")
        || readiness.checks.iter().any(|check| check.is_failure())
        || readiness
            .deployments
            .iter()
            .any(|deployment| deployment.is_failure())
}

fn pull_request_is_ready(
    readiness: &linux_archductor_core::workspace::PullRequestReadiness,
    summary: &linux_archductor_core::workspace::ChecksSummary,
) -> bool {
    readiness.review_decision.as_deref() == Some("APPROVED")
        && readiness
            .checks
            .iter()
            .all(|check| !check.is_failure() && !check.is_pending())
        && readiness
            .deployments
            .iter()
            .all(|deployment| !deployment.is_failure() && !deployment.is_pending())
        && summary.open_todos == 0
        && summary.open_review_comments == 0
        && summary.conflicting_workspaces.is_empty()
}

fn workspace_setup_prompt(db_path: &Path, name: &str) -> String {
    workspace_script_prompt(db_path, name, "setup", "Setup")
}

fn workspace_run_prompt(db_path: &Path, name: &str) -> String {
    workspace_script_prompt(db_path, name, "run", "Run")
}

fn workspace_continue_prompt(db_path: &Path, name: &str) -> String {
    WorkspaceStore::open(db_path)
        .and_then(|store| store.workspace_repo_settings(name))
        .ok()
        .and_then(|settings| settings.prompts.and_then(|prompts| prompts.create_pr))
        .filter(|prompt| !prompt.trim().is_empty())
        .map(|prompt| format!("Continue after the merged PR for workspace {name}.\n\n{prompt}"))
        .unwrap_or_else(|| {
            format!(
                "Continue after the merged PR for workspace {name}.\n\
                 Check remaining todos, decide the next branch or follow-up PR, and keep going."
            )
        })
}

fn workspace_script_prompt(db_path: &Path, name: &str, script_key: &str, label: &str) -> String {
    let current = WorkspaceStore::open(db_path)
        .and_then(|store| store.workspace_repo_settings(name))
        .ok()
        .and_then(|settings| match script_key {
            "setup" => settings.scripts.setup,
            "run" => settings.scripts.run,
            _ => None,
        });
    match current {
        Some(script) if !script.trim().is_empty() => format!(
            "Create or update .archductor/settings.toml for workspace {name}.\n\
             Set scripts.{script_key} to this multiline shell block of successive commands:\n\n{script}\n"
        ),
        _ => format!(
            "Create or update .archductor/settings.toml for workspace {name}.\n\
             Define scripts.{script_key} as a multiline shell block so the {label} tab can run successive commands in order.\n\
             Keep the commands short, reliable, and checked into the repo."
        ),
    }
}

fn workspace_file_comments_text(db_path: &Path, name: &str, path: &str) -> String {
    WorkspaceStore::open(db_path)
        .and_then(|store| store.list_review_comments(name))
        .map(|comments| file_inline_comments_text(&comments, path))
        .unwrap_or_else(|err| format!("Could not read comments for {path}: {err:#}\n"))
}

fn diff_summary_label(summary: &DiffFileSummary) -> String {
    let counts = match (summary.additions, summary.deletions) {
        (Some(additions), Some(deletions)) => format!("+{additions} -{deletions}"),
        _ => "binary".to_owned(),
    };
    format!("{} {}", summary.path, counts)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DiffTreeRow {
    Directory(String),
    File(DiffFileSummary),
}

fn diff_tree_rows(summaries: &[DiffFileSummary]) -> Vec<DiffTreeRow> {
    let mut rows = Vec::new();
    let mut seen_directories = std::collections::BTreeSet::new();
    for summary in summaries {
        let parts = summary.path.split('/').collect::<Vec<_>>();
        if parts.len() > 1 {
            let mut prefix = String::new();
            for (depth, part) in parts[..parts.len() - 1].iter().enumerate() {
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(part);
                if seen_directories.insert(prefix.clone()) {
                    rows.push(DiffTreeRow::Directory(format!(
                        "{}{}/",
                        "  ".repeat(depth),
                        part
                    )));
                }
            }
        }
        rows.push(DiffTreeRow::File(summary.clone()));
    }
    rows
}

fn diff_tree_file_label(summary: &DiffFileSummary) -> String {
    let depth = summary.path.matches('/').count();
    format!("{}{}", "  ".repeat(depth), diff_summary_label(summary))
}

fn file_inline_comments_text(comments: &[ReviewComment], path: &str) -> String {
    let filtered = comments
        .iter()
        .filter(|comment| comment.file_path == path)
        .collect::<Vec<_>>();
    if filtered.is_empty() {
        return format!("No inline comments for {path}.");
    }
    let mut out = format!("Inline comments for {path}\n");
    for comment in filtered {
        let line = comment
            .line_number
            .map(|line| format!(":{line}"))
            .unwrap_or_default();
        out.push_str(&format!(
            "#{} [{}] {}{} - {}\n",
            comment.id, comment.status, comment.file_path, line, comment.body
        ));
    }
    out
}

fn format_diff_file_summary(summaries: &[DiffFileSummary]) -> String {
    let mut out = "Files changed\n".to_owned();
    if summaries.is_empty() {
        out.push_str("No unstaged file changes.\n");
        return out;
    }
    for summary in summaries {
        let counts = match (summary.additions, summary.deletions) {
            (Some(additions), Some(deletions)) => format!("+{additions} -{deletions}"),
            _ => "binary".to_owned(),
        };
        out.push_str(&format!("{} {}\n", summary.path, counts));
    }
    out
}

fn workspace_checks_text(store: &WorkspaceStore, name: &str) -> String {
    match store.checks_summary(name) {
        Ok(summary) => {
            let push = summary
                .branch_push_state
                .as_ref()
                .map(|state| {
                    if state.has_upstream {
                        format!("ahead {} / behind {}", state.ahead, state.behind)
                    } else {
                        "no upstream".to_owned()
                    }
                })
                .unwrap_or_else(|| "unavailable".to_owned());
            let pr = summary
                .pull_request
                .as_ref()
                .map(|pr| format!("#{} {} {}", pr.number, pr.state, pr.url))
                .unwrap_or_else(|| "none".to_owned());
            let conflicts = if summary.conflicting_workspaces.is_empty() {
                "none".to_owned()
            } else {
                summary
                    .conflicting_workspaces
                    .iter()
                    .map(|(workspace, files)| format!("{workspace}: {}", files.join(", ")))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let blockers = merge_blockers_text(
                summary.open_todos,
                summary.open_review_comments,
                summary.conflicting_workspaces.len(),
            );
            format!(
                "Changed files: {}\nRun: {}\nSessions: {}\nPR: {}\n{}\nTodos: {} open / {} total\nReview comments: {} open\nBranch: {}\nConflicts:\n{}",
                summary.changed_files,
                summary
                    .run_status
                    .map(|status| status.as_str().to_owned())
                    .unwrap_or_else(|| "none".to_owned()),
                summary.active_sessions,
                pr,
                blockers,
                summary.open_todos,
                summary.total_todos,
                summary.open_review_comments,
                push,
                conflicts
            )
        }
        Err(err) => format!("Could not read checks: {err:#}"),
    }
}

fn ws_checks_visual_header(store: &WorkspaceStore, name: &str) -> GBox {
    let container = GBox::new(Orientation::Vertical, 0);
    container.add_css_class("ws-check-summary");

    let text = workspace_checks_text(store, name);
    for line in text.lines() {
        if let Some((key, val)) = line.split_once(": ") {
            let row = GBox::new(Orientation::Horizontal, 0);
            row.add_css_class("ws-check-row");

            let (icon, icon_class) = check_status_icon(key, val);
            let icon_lbl = Label::new(Some(icon));
            icon_lbl.add_css_class("ws-check-icon");
            icon_lbl.add_css_class(icon_class);
            row.append(&icon_lbl);

            let key_lbl = Label::new(Some(key));
            key_lbl.add_css_class("ws-check-key");
            key_lbl.set_xalign(0.0);
            row.append(&key_lbl);

            let val_lbl = Label::new(Some(val));
            val_lbl.add_css_class("ws-check-val");
            val_lbl.set_xalign(0.0);
            val_lbl.set_hexpand(true);
            val_lbl.set_ellipsize(gtk::pango::EllipsizeMode::End);
            row.append(&val_lbl);

            container.append(&row);
        } else if !line.trim().is_empty() {
            let lbl = Label::new(Some(line));
            lbl.add_css_class("ws-check-sub");
            lbl.set_xalign(0.0);
            lbl.set_margin_start(42);
            container.append(&lbl);
        }
    }

    container
}

fn check_status_icon(key: &str, val: &str) -> (&'static str, &'static str) {
    let v = val.to_lowercase();
    if v == "none" || v == "0" || v.contains("pass") || v.contains("no upstream") {
        ("◈ ", "ws-check-icon-muted")
    } else if v.contains("fail") || v.contains("error") || v.contains("conflict") {
        ("✗ ", "ws-check-icon-fail")
    } else if v.contains("running") || v.contains("pending") || v.contains("behind") {
        ("● ", "ws-check-icon-active")
    } else if key == "PR" && !v.contains("none") {
        ("◉ ", "ws-check-icon-active")
    } else {
        ("◈ ", "ws-check-icon-muted")
    }
}

fn workspace_checks_panel(
    db_path: &Path,
    store: &WorkspaceStore,
    name: &str,
    app_state: AppState,
    refresh_hub: RefreshHub,
    toast_overlay: ToastOverlay,
) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 8);
    let summary = match store.checks_summary(name) {
        Ok(summary) => summary,
        Err(err) => {
            panel.append(&detail_row(
                "Pull request",
                &format!("Could not read PR state: {err:#}"),
            ));
            return panel;
        }
    };

    let panel_state_result = store.pull_request_panel_state(name);
    let panel_state_error = panel_state_result
        .as_ref()
        .err()
        .map(|err| format!("{err:#}"));
    let panel_state = panel_state_result.ok();
    let pr = panel_state
        .as_ref()
        .and_then(|state| state.pull_request.clone())
        .or_else(|| summary.pull_request.clone());
    let has_pull_request = pr.is_some();
    let checks_text = workspace_checks_text(store, name);
    let readiness_text = panel_state
        .as_ref()
        .map(|state| state.readiness_text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .or_else(|| {
            panel_state_error
                .as_ref()
                .filter(|_| has_pull_request)
                .map(|err| format!("Could not read PR readiness: {err}"))
        })
        .unwrap_or_else(|| "No pull request yet.".to_owned());
    let review_text = panel_state
        .as_ref()
        .and_then(|state| state.review_text.as_deref())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            panel_state_error
                .as_ref()
                .filter(|_| has_pull_request)
                .map(|err| format!("Could not read PR comments/reviews: {err}"))
        })
        .unwrap_or_else(|| {
            if has_pull_request {
                "No PR comments/reviews yet.".to_owned()
            } else {
                "No pull request yet.".to_owned()
            }
        });

    let header = GBox::new(Orientation::Vertical, 8);
    header.add_css_class("ws-pr-nav");
    let feedback = Label::new(None);
    feedback.add_css_class("card-meta");
    feedback.set_xalign(0.0);
    feedback.set_wrap(true);

    match pr {
        Some(pr) => {
            let status_summary = pull_request_status_summary(
                &pr,
                panel_state
                    .as_ref()
                    .and_then(|state| state.readiness.as_ref()),
                &summary,
            );
            let action_labels = pull_request_action_labels(Some(status_summary.kind));

            let status_row = GBox::new(Orientation::Horizontal, 8);
            status_row.add_css_class("ws-pr-nav");
            status_row.set_hexpand(true);
            let number = Label::new(Some(&format!("#{}", pr.number)));
            number.add_css_class("ws-pr-number");
            number.set_xalign(0.0);
            let status = Label::new(Some(&status_summary.label));
            status.add_css_class("ws-pr-status");
            status.add_css_class(status_summary.css_class);
            status.set_xalign(0.0);
            status.set_hexpand(true);
            status_row.append(&number);
            status_row.append(&status);
            header.append(&status_row);

            let actions = make_action_stack();
            match status_summary.kind {
                PullRequestStateKind::Open => {
                    let top_row = make_action_row();
                    let summary_btn = secondary_button(action_labels[0]);
                    let review_btn = secondary_button(action_labels[1]);
                    let refresh_btn = secondary_button(action_labels[2]);

                    let db_for_stage = db_path.to_path_buf();
                    let workspace_for_stage = name.to_owned();
                    let app_state_for_stage = app_state.clone();
                    let feedback_for_stage = feedback.clone();
                    let toast_for_stage = toast_overlay.clone();
                    summary_btn.connect_clicked(move |_| {
                        match WorkspaceStore::open(db_for_stage.clone()).and_then(|store| {
                            store.pull_request_readiness_agent_prompt(&workspace_for_stage)
                        }) {
                            Ok(prompt) => {
                                app_state_for_stage.set_staged_review_prompt(Some(prompt));
                                apply_action_feedback(
                                    &feedback_for_stage,
                                    &toast_for_stage,
                                    "PR summary prompt ready for the selected agent session.",
                                    true,
                                );
                            }
                            Err(err) => apply_action_feedback(
                                &feedback_for_stage,
                                &toast_for_stage,
                                &format!("Could not create PR summary prompt: {err:#}"),
                                true,
                            ),
                        }
                    });

                    let db_for_review = db_path.to_path_buf();
                    let workspace_for_review = name.to_owned();
                    let app_state_for_review = app_state.clone();
                    let feedback_for_review = feedback.clone();
                    let toast_for_review = toast_overlay.clone();
                    review_btn.connect_clicked(move |_| {
                        match WorkspaceStore::open(db_for_review.clone()).and_then(|store| {
                            store.pull_request_review_agent_prompt(&workspace_for_review)
                        }) {
                            Ok(prompt) => {
                                app_state_for_review.set_staged_review_prompt(Some(prompt));
                                apply_action_feedback(
                                    &feedback_for_review,
                                    &toast_for_review,
                                    "PR review prompt ready for the selected agent session.",
                                    true,
                                );
                            }
                            Err(err) => apply_action_feedback(
                                &feedback_for_review,
                                &toast_for_review,
                                &format!("Could not create PR review prompt: {err:#}"),
                                true,
                            ),
                        }
                    });

                    let db_for_refresh = db_path.to_path_buf();
                    let workspace_for_refresh = name.to_owned();
                    let refresh_after = refresh_hub.clone();
                    let feedback_for_refresh = feedback.clone();
                    let toast_for_refresh = toast_overlay.clone();
                    refresh_btn.connect_clicked(move |_| {
                        let result =
                            WorkspaceStore::open(db_for_refresh.clone()).and_then(|store| {
                                store.refresh_pull_request_state(&workspace_for_refresh)
                            });
                        let message = pull_request_refresh_feedback(result);
                        apply_action_feedback(
                            &feedback_for_refresh,
                            &toast_for_refresh,
                            &message,
                            true,
                        );
                        refresh_after.refresh(RefreshScope::All);
                    });

                    top_row.append(&summary_btn);
                    top_row.append(&review_btn);
                    top_row.append(&refresh_btn);
                    actions.append(&top_row);
                }
                PullRequestStateKind::Ready => {
                    let merge_row = make_action_row();
                    let top_row = make_action_row();
                    let merge_method = ComboBoxText::new();
                    merge_method.append(Some("squash"), "Squash");
                    merge_method.append(Some("merge"), "Merge");
                    merge_method.append(Some("rebase"), "Rebase");
                    merge_method.set_active_id(Some("squash"));
                    let merge_btn = text_button(action_labels[0]);
                    merge_btn.add_css_class("suggested-action");
                    let summary_btn = secondary_button(action_labels[1]);
                    let refresh_btn = secondary_button(action_labels[2]);

                    let db_for_merge = db_path.to_path_buf();
                    let workspace_for_merge = name.to_owned();
                    let refresh_after_merge = refresh_hub.clone();
                    let feedback_for_merge = feedback.clone();
                    let toast_for_merge = toast_overlay.clone();
                    let merge_method_for_merge = merge_method.clone();
                    merge_btn.connect_clicked(move |_| {
                        let method = merge_method_for_merge
                            .active_id()
                            .map(|method| method.to_string())
                            .unwrap_or_else(|| "squash".to_owned());
                        let result = WorkspaceStore::open(db_for_merge.clone()).and_then(|store| {
                            store.merge_and_maybe_archive_pull_request(
                                &workspace_for_merge,
                                Some(&method),
                            )
                        });
                        apply_action_feedback(
                            &feedback_for_merge,
                            &toast_for_merge,
                            &pull_request_merge_and_archive_feedback(result),
                            true,
                        );
                        refresh_after_merge.refresh(RefreshScope::All);
                    });

                    let db_for_stage = db_path.to_path_buf();
                    let workspace_for_stage = name.to_owned();
                    let app_state_for_stage = app_state.clone();
                    let feedback_for_stage = feedback.clone();
                    let toast_for_stage = toast_overlay.clone();
                    summary_btn.connect_clicked(move |_| {
                        match WorkspaceStore::open(db_for_stage.clone()).and_then(|store| {
                            store.pull_request_readiness_agent_prompt(&workspace_for_stage)
                        }) {
                            Ok(prompt) => {
                                app_state_for_stage.set_staged_review_prompt(Some(prompt));
                                apply_action_feedback(
                                    &feedback_for_stage,
                                    &toast_for_stage,
                                    "PR summary prompt ready for the selected agent session.",
                                    true,
                                );
                            }
                            Err(err) => apply_action_feedback(
                                &feedback_for_stage,
                                &toast_for_stage,
                                &format!("Could not create PR summary prompt: {err:#}"),
                                true,
                            ),
                        }
                    });

                    let db_for_refresh = db_path.to_path_buf();
                    let workspace_for_refresh = name.to_owned();
                    let refresh_after = refresh_hub.clone();
                    let feedback_for_refresh = feedback.clone();
                    let toast_for_refresh = toast_overlay.clone();
                    refresh_btn.connect_clicked(move |_| {
                        let result =
                            WorkspaceStore::open(db_for_refresh.clone()).and_then(|store| {
                                store.refresh_pull_request_state(&workspace_for_refresh)
                            });
                        let message = pull_request_refresh_feedback(result);
                        apply_action_feedback(
                            &feedback_for_refresh,
                            &toast_for_refresh,
                            &message,
                            true,
                        );
                        refresh_after.refresh(RefreshScope::All);
                    });

                    merge_row.append(&merge_method);
                    merge_row.append(&merge_btn);
                    top_row.append(&summary_btn);
                    top_row.append(&refresh_btn);
                    actions.append(&merge_row);
                    actions.append(&top_row);
                }
                PullRequestStateKind::Failed => {
                    let top_row = make_action_row();
                    let fix_btn = text_button(action_labels[0]);
                    fix_btn.add_css_class("suggested-action");
                    let summary_btn = secondary_button(action_labels[1]);
                    let refresh_btn = secondary_button(action_labels[2]);

                    let db_for_fix = db_path.to_path_buf();
                    let workspace_for_fix = name.to_owned();
                    let app_state_for_fix = app_state.clone();
                    let feedback_for_fix = feedback.clone();
                    let toast_for_fix = toast_overlay.clone();
                    fix_btn.connect_clicked(move |_| {
                        match WorkspaceStore::open(db_for_fix.clone()).and_then(|store| {
                            store.pull_request_checks_agent_prompt(&workspace_for_fix)
                        }) {
                            Ok(prompt) => {
                                app_state_for_fix.set_staged_review_prompt(Some(prompt));
                                apply_action_feedback(
                                    &feedback_for_fix,
                                    &toast_for_fix,
                                    "Failing checks prompt ready for the selected agent session.",
                                    true,
                                );
                            }
                            Err(err) => apply_action_feedback(
                                &feedback_for_fix,
                                &toast_for_fix,
                                &format!("Could not stage failing checks: {err:#}"),
                                true,
                            ),
                        }
                    });

                    let db_for_stage = db_path.to_path_buf();
                    let workspace_for_stage = name.to_owned();
                    let app_state_for_stage = app_state.clone();
                    let feedback_for_stage = feedback.clone();
                    let toast_for_stage = toast_overlay.clone();
                    summary_btn.connect_clicked(move |_| {
                        match WorkspaceStore::open(db_for_stage.clone()).and_then(|store| {
                            store.pull_request_readiness_agent_prompt(&workspace_for_stage)
                        }) {
                            Ok(prompt) => {
                                app_state_for_stage.set_staged_review_prompt(Some(prompt));
                                apply_action_feedback(
                                    &feedback_for_stage,
                                    &toast_for_stage,
                                    "PR summary prompt ready for the selected agent session.",
                                    true,
                                );
                            }
                            Err(err) => apply_action_feedback(
                                &feedback_for_stage,
                                &toast_for_stage,
                                &format!("Could not create PR summary prompt: {err:#}"),
                                true,
                            ),
                        }
                    });

                    let db_for_refresh = db_path.to_path_buf();
                    let workspace_for_refresh = name.to_owned();
                    let refresh_after = refresh_hub.clone();
                    let feedback_for_refresh = feedback.clone();
                    let toast_for_refresh = toast_overlay.clone();
                    refresh_btn.connect_clicked(move |_| {
                        let result =
                            WorkspaceStore::open(db_for_refresh.clone()).and_then(|store| {
                                store.refresh_pull_request_state(&workspace_for_refresh)
                            });
                        let message = pull_request_refresh_feedback(result);
                        apply_action_feedback(
                            &feedback_for_refresh,
                            &toast_for_refresh,
                            &message,
                            true,
                        );
                        refresh_after.refresh(RefreshScope::All);
                    });

                    top_row.append(&fix_btn);
                    top_row.append(&summary_btn);
                    top_row.append(&refresh_btn);
                    actions.append(&top_row);
                }
                PullRequestStateKind::Merged => {
                    let top_row = make_action_row();
                    let continue_btn = text_button(action_labels[0]);
                    continue_btn.add_css_class("suggested-action");
                    let archive_btn = destructive_button(action_labels[1]);

                    let db_for_continue = db_path.to_path_buf();
                    let workspace_for_continue = name.to_owned();
                    let app_state_for_continue = app_state.clone();
                    continue_btn.connect_clicked(move |_| {
                        let prompt =
                            workspace_continue_prompt(&db_for_continue, &workspace_for_continue);
                        app_state_for_continue.set_staged_review_prompt(Some(prompt));
                    });

                    let db_for_archive = db_path.to_path_buf();
                    let workspace_for_archive = name.to_owned();
                    let refresh_after_archive = refresh_hub.clone();
                    let feedback_for_archive = feedback.clone();
                    let toast_for_archive = toast_overlay.clone();
                    archive_btn.connect_clicked(move |_| {
                        let result = WorkspaceStore::open(db_for_archive.clone())
                            .and_then(|store| store.archive(&workspace_for_archive, false));
                        apply_action_feedback(
                            &feedback_for_archive,
                            &toast_for_archive,
                            &pull_request_archive_feedback(result),
                            true,
                        );
                        refresh_after_archive.refresh(RefreshScope::All);
                    });

                    top_row.append(&continue_btn);
                    top_row.append(&archive_btn);
                    actions.append(&top_row);
                }
            }
            header.append(&actions);
        }
        None => {
            let action_labels = pull_request_action_labels(None);
            let status_row = GBox::new(Orientation::Horizontal, 8);
            status_row.add_css_class("ws-pr-nav");
            status_row.set_hexpand(true);
            let empty = Label::new(Some("No pull request yet."));
            empty.add_css_class("card-meta");
            empty.set_xalign(0.0);
            empty.set_wrap(true);
            empty.set_hexpand(true);
            status_row.append(&empty);
            header.append(&status_row);

            let title_row = make_action_row();
            let title_entry = Entry::new();
            title_entry.set_placeholder_text(Some("PR title"));
            title_entry.set_hexpand(true);
            title_row.append(&title_entry);
            header.append(&title_row);

            let body_label = toolbar_label("PR body");
            header.append(&body_label);
            let body_view = TextView::new();
            body_view.set_wrap_mode(WrapMode::WordChar);
            body_view.set_top_margin(8);
            body_view.set_bottom_margin(8);
            body_view.set_left_margin(8);
            body_view.set_right_margin(8);
            body_view.buffer().set_text(
                &store
                    .read_context_brief(name)
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
            );
            let body_scroll = ScrolledWindow::new();
            body_scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
            body_scroll.set_min_content_height(72);
            body_scroll.set_max_content_height(120);
            body_scroll.set_child(Some(&body_view));
            header.append(&body_scroll);

            let button_row = make_action_row();
            let draft_check = CheckButton::with_label("Draft");
            let push_btn = secondary_button(action_labels[0]);
            let create_btn = text_button(action_labels[1]);
            create_btn.add_css_class("suggested-action");
            let body_buffer = body_view.buffer();

            let db_for_push = db_path.to_path_buf();
            let workspace_for_push = name.to_owned();
            let feedback_for_push = feedback.clone();
            let toast_for_push = toast_overlay.clone();
            push_btn.connect_clicked(move |_| {
                match WorkspaceStore::open(db_for_push.clone())
                    .and_then(|store| store.push_branch(&workspace_for_push))
                {
                    Ok(output) => {
                        let message = output
                            .lines()
                            .map(str::trim)
                            .find(|line| !line.is_empty())
                            .map(|line| format!("Pushed branch: {line}"))
                            .unwrap_or_else(|| "Pushed branch.".to_owned());
                        apply_action_feedback(&feedback_for_push, &toast_for_push, &message, true);
                    }
                    Err(err) => apply_action_feedback(
                        &feedback_for_push,
                        &toast_for_push,
                        &format!("Push branch failed: {err:#}"),
                        true,
                    ),
                }
            });

            let db_for_create = db_path.to_path_buf();
            let workspace_for_create = name.to_owned();
            let feedback_for_create = feedback.clone();
            let toast_for_create = toast_overlay.clone();
            let refresh_after_create = refresh_hub.clone();
            let title_entry_for_create = title_entry.clone();
            let draft_check_for_create = draft_check.clone();
            create_btn.connect_clicked(move |_| {
                let title = optional_entry_text(&title_entry_for_create);
                let body = optional_text_buffer_text(&body_buffer);
                let result = WorkspaceStore::open(db_for_create.clone()).and_then(|store| {
                    store.create_pull_request(
                        &workspace_for_create,
                        title.as_deref(),
                        body.as_deref(),
                        draft_check_for_create.is_active(),
                    )
                });
                let should_refresh = result.is_ok();
                apply_action_feedback(
                    &feedback_for_create,
                    &toast_for_create,
                    &pull_request_create_feedback(result),
                    true,
                );
                if should_refresh {
                    refresh_after_create.refresh(RefreshScope::All);
                }
            });

            button_row.append(&draft_check);
            button_row.append(&push_btn);
            button_row.append(&create_btn);
            header.append(&button_row);
        }
    }

    panel.append(&header);
    panel.append(&feedback);
    panel.append(&pull_request_text_section("Checks Summary", &checks_text));
    panel.append(&pull_request_text_section("PR Readiness", &readiness_text));
    panel.append(&pull_request_text_section(
        "PR Comments / Reviews",
        &review_text,
    ));

    panel
}

fn optional_entry_text(entry: &Entry) -> Option<String> {
    let value = entry.text().trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn optional_text_buffer_text(buffer: &gtk::TextBuffer) -> Option<String> {
    let value = text_buffer_contents(buffer).trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn pull_request_action_labels(kind: Option<PullRequestStateKind>) -> Vec<&'static str> {
    match kind {
        None => vec!["Push branch", "Create PR"],
        Some(PullRequestStateKind::Open) => vec!["PR summary", "Reviews", "Refresh"],
        Some(PullRequestStateKind::Ready) => vec!["Merge", "PR summary", "Refresh"],
        Some(PullRequestStateKind::Failed) => vec!["Fix checks", "PR summary", "Refresh"],
        Some(PullRequestStateKind::Merged) => vec!["Continue", "Archive"],
    }
}

fn pull_request_text_section(title: &str, text: &str) -> GBox {
    let section = GBox::new(Orientation::Vertical, 4);
    section.append(&section_title(title));

    let view = TextView::new();
    view.set_editable(false);
    view.set_monospace(true);
    view.set_wrap_mode(WrapMode::WordChar);
    view.add_css_class("history-view");
    view.set_left_margin(6);
    view.set_right_margin(6);
    view.set_top_margin(6);
    view.set_bottom_margin(6);
    view.buffer().set_text(text);

    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    scroll.set_min_content_height(56);
    scroll.set_max_content_height(88);
    scroll.set_child(Some(&view));
    section.append(&scroll);

    section
}

fn workspace_conflict_resolution_panel(
    db_path: &Path,
    store: &WorkspaceStore,
    name: &str,
    app_state: AppState,
    refresh_hub: RefreshHub,
) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 8);
    panel.add_css_class("command-panel");
    panel.append(&section_title("Conflict Resolution"));

    let summary = match store.checks_summary(name) {
        Ok(summary) => summary,
        Err(err) => {
            panel.append(&detail_row(
                "Conflict resolution",
                &format!("Could not load conflicts: {err:#}"),
            ));
            return panel;
        }
    };

    if summary.conflicting_workspaces.is_empty() {
        panel.append(&detail_row("Conflicts", "No sibling workspace conflicts."));
        return panel;
    }

    let diff_preview = TextView::new();
    diff_preview.set_editable(false);
    diff_preview.set_monospace(true);
    diff_preview.set_hexpand(true);
    diff_preview.set_vexpand(true);
    diff_preview
        .buffer()
        .set_text("Select a conflict file to preview its diff.");
    let conflict_feedback = Label::new(None);
    conflict_feedback.add_css_class("card-meta");
    conflict_feedback.set_xalign(0.0);
    conflict_feedback.set_wrap(true);
    conflict_feedback.set_text("No conflict action run yet.");
    let diff_container = ScrolledWindow::new();
    diff_container.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    diff_container.set_min_content_height(180);
    diff_container.set_child(Some(&diff_preview));

    for (conflict_workspace, files) in summary.conflicting_workspaces {
        let workspace_group = GBox::new(Orientation::Vertical, 6);
        let title = Label::new(Some(&format!(
            "{} ({})",
            conflict_workspace,
            if files.len() == 1 {
                "1 file".to_owned()
            } else {
                format!("{} files", files.len())
            }
        )));
        title.set_xalign(0.0);
        title.add_css_class("detail-label");

        let open_workspace_btn = flat_button("Open workspace");
        let app_state_for_open = app_state.clone();
        let refresh_for_open = refresh_hub.clone();
        let conflict_workspace_for_open = conflict_workspace.clone();
        open_workspace_btn.connect_clicked(move |_| {
            app_state_for_open.set_selected_workspace(Some(conflict_workspace_for_open.clone()));
            refresh_for_open.refresh(RefreshScope::All);
        });

        let action_row = make_action_row();
        action_row.append(&title);
        action_row.append(&open_workspace_btn);
        let diff_all_btn = flat_button("View all diffs");
        let files_for_diff_all = files.clone();
        let source_workspace_for_diff_all = conflict_workspace.clone();
        let feedback_for_diff_all = conflict_feedback.clone();
        let diff_buffer_for_diff_all = diff_preview.buffer();
        let db_for_diff_all = db_path.to_path_buf();
        diff_all_btn.connect_clicked(move |_| {
            let mut sections = Vec::new();
            for file in &files_for_diff_all {
                let file_path = Path::new(file).to_path_buf();
                match WorkspaceStore::open(db_for_diff_all.clone()).and_then(|store| {
                    store.unified_diff(&source_workspace_for_diff_all, Some(file_path.as_path()))
                }) {
                    Ok(output) => {
                        sections.push(format!(
                            "# {}:{}\n{}\n",
                            source_workspace_for_diff_all,
                            file_path.display(),
                            output
                        ));
                    }
                    Err(err) => {
                        feedback_for_diff_all
                            .set_text(&format!("Could not read diff for {file}: {err:#}"));
                        return;
                    }
                }
            }
            if sections.is_empty() {
                diff_buffer_for_diff_all.set_text("No conflicting files to diff.");
            } else {
                diff_buffer_for_diff_all.set_text(&sections.join("\n"));
            }
        });
        let copy_all_btn = secondary_button("Copy all from sibling");
        let files_for_copy_all = files.clone();
        let source_workspace = conflict_workspace.clone();
        let destination_workspace = name.to_owned();
        let db_for_copy_all = db_path.to_path_buf();
        let feedback_for_copy_all = conflict_feedback.clone();
        let refresh_after_copy_all = refresh_hub.clone();
        copy_all_btn.connect_clicked(move |_| {
            let mut copied = 0usize;
            let mut failures = Vec::new();
            for file in &files_for_copy_all {
                let result = WorkspaceStore::open(db_for_copy_all.clone()).and_then(|store| {
                    store.copy_conflict_file_from_workspace(
                        &destination_workspace,
                        &source_workspace,
                        file,
                    )
                });
                match result {
                    Ok(()) => copied += 1,
                    Err(err) => failures.push(format!("{file}: {err:#}")),
                }
            }
            match (copied, failures.is_empty()) {
                (0, true) => {
                    feedback_for_copy_all.set_text(&format!(
                        "No files available to copy from {source_workspace}."
                    ));
                }
                (0, false) => {
                    feedback_for_copy_all.set_text(&format!(
                        "Failed to copy files from {source_workspace}: {}",
                        failures.join("; ")
                    ));
                }
                (_, true) => {
                    refresh_after_copy_all.refresh(RefreshScope::Workspace);
                    feedback_for_copy_all.set_text(&format!(
                        "Copied {} conflicting file(s) from {source_workspace}.",
                        copied
                    ));
                }
                (_, false) => {
                    refresh_after_copy_all.refresh(RefreshScope::Workspace);
                    feedback_for_copy_all.set_text(&format!(
                        "Copied {copied} file(s) from {source_workspace}, but {} failed: {}",
                        failures.len(),
                        failures.join("; ")
                    ));
                }
            }
        });

        action_row.append(&copy_all_btn);
        action_row.append(&diff_all_btn);
        workspace_group.append(&action_row);

        for file in files {
            let file_row = make_action_row();
            let file_label = Label::new(Some(&file));
            file_label.set_xalign(0.0);
            file_label.set_wrap(true);
            file_label.set_hexpand(true);

            let diff_btn = flat_button("View diff");
            let db_for_diff = db_path.to_path_buf();
            let source_workspace_for_diff = conflict_workspace.clone();
            let file_for_diff = Path::new(&file).to_path_buf();
            let diff_buffer = diff_preview.buffer();
            diff_btn.connect_clicked(move |_| {
                let output = WorkspaceStore::open(db_for_diff.clone())
                    .and_then(|store| {
                        store
                            .unified_diff(&source_workspace_for_diff, Some(file_for_diff.as_path()))
                    })
                    .unwrap_or_else(|err| {
                        format!(
                            "Could not read diff for {}: {err:#}",
                            file_for_diff.display()
                        )
                    });
                let formatted = format!(
                    "# {}:{}\n{}",
                    source_workspace_for_diff,
                    file_for_diff.display(),
                    output
                );
                diff_buffer.set_text(&formatted);
            });

            let copy_btn = secondary_button("Copy from sibling");
            let file_for_copy = file.clone();
            let db_for_copy = db_path.to_path_buf();
            let destination_workspace = name.to_owned();
            let source_workspace = conflict_workspace.clone();
            let feedback_for_copy = conflict_feedback.clone();
            let refresh_after_copy = refresh_hub.clone();
            copy_btn.connect_clicked(move |_| {
                let result = WorkspaceStore::open(db_for_copy.clone()).and_then(|store| {
                    store.copy_conflict_file_from_workspace(
                        &destination_workspace,
                        &source_workspace,
                        &file_for_copy,
                    )
                });
                match result {
                    Ok(()) => {
                        feedback_for_copy.set_text(&format!(
                            "Copied {file_for_copy} from {source_workspace} into {destination_workspace}"
                        ));
                        refresh_after_copy.refresh(RefreshScope::Workspace);
                    }
                    Err(err) => {
                        feedback_for_copy
                            .set_text(&format!("Could not copy {file_for_copy}: {err:#}"));
                    }
                }
            });

            file_row.append(&file_label);
            file_row.append(&diff_btn);
            file_row.append(&copy_btn);
            workspace_group.append(&file_row);
        }

        panel.append(&workspace_group);
    }

    panel.append(&conflict_feedback);
    panel.append(&diff_container);
    panel
}

fn pull_request_create_feedback(result: anyhow::Result<String>) -> String {
    match result {
        Ok(output) => output
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| line.starts_with("https://"))
            .map(|url| format!("Created PR: {url}"))
            .unwrap_or_else(|| "Created PR.".to_owned()),
        Err(err) => format!("Create PR failed: {err:#}"),
    }
}

fn pull_request_merge_feedback(result: anyhow::Result<String>) -> String {
    match result {
        Ok(output) => output
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(|line| format!("Merged PR: {line}"))
            .unwrap_or_else(|| "Merged PR.".to_owned()),
        Err(err) => format!("Merge PR failed: {err:#}"),
    }
}

fn pull_request_merge_and_archive_feedback(
    result: anyhow::Result<linux_archductor_core::workspace::MergePullRequestResult>,
) -> String {
    match result {
        Ok(result) => {
            let mut text = pull_request_merge_feedback(Ok(result.merge_output));
            if let Some(workspace) = result.archived_workspace {
                text.push_str(&format!("; archived workspace {}.", workspace.name));
            }
            text
        }
        Err(err) => format!("Merge PR failed: {err:#}"),
    }
}

fn pull_request_refresh_feedback(result: anyhow::Result<Option<PullRequest>>) -> String {
    match result {
        Ok(Some(pr)) => format!("PR #{} state: {}", pr.number, pr.state),
        Ok(None) => "No PR recorded for this workspace.".to_owned(),
        Err(err) => format!("Refresh PR failed: {err:#}"),
    }
}

fn pull_request_checks_feedback(result: anyhow::Result<String>) -> String {
    match result {
        Ok(output) => {
            let output = output.trim();
            if output.is_empty() {
                "PR checks returned no output.".to_owned()
            } else {
                format!("PR checks:\n{output}")
            }
        }
        Err(err) => format!("View checks failed: {err:#}"),
    }
}

fn pull_request_review_feedback(result: anyhow::Result<String>) -> String {
    match result {
        Ok(output) => {
            let output = output.trim();
            if output.is_empty() {
                "PR comments/reviews returned no output.".to_owned()
            } else {
                format!("PR comments/reviews:\n{output}")
            }
        }
        Err(err) => format!("View PR comments/reviews failed: {err:#}"),
    }
}

fn pull_request_readiness_feedback(result: anyhow::Result<String>) -> String {
    match result {
        Ok(output) => {
            let output = output.trim();
            if output.is_empty() {
                "PR readiness summary returned no output.".to_owned()
            } else {
                output.to_owned()
            }
        }
        Err(err) => format!("View PR summary failed: {err:#}"),
    }
}

fn pull_request_review_thread_action_feedback(
    action: &str,
    result: anyhow::Result<PullRequestReviewThread>,
) -> String {
    match result {
        Ok(thread) => {
            let id = thread.id.as_deref().unwrap_or("unknown thread");
            let state = if thread.resolved {
                "resolved"
            } else {
                "unresolved"
            };
            let location = match (thread.path.as_deref(), thread.line) {
                (Some(path), Some(line)) => format!("{path}:{line}"),
                (Some(path), None) => path.to_owned(),
                (None, Some(line)) => format!("line {line}"),
                (None, None) => "unknown location".to_owned(),
            };
            format!("{action} review thread {id}: {state} at {location}.")
        }
        Err(err) => format!("{action} review thread failed: {err:#}"),
    }
}

fn pull_request_archive_feedback(result: anyhow::Result<Workspace>) -> String {
    match result {
        Ok(workspace) => format!("Archived workspace {}.", workspace.name),
        Err(err) => format!("Archive failed: {err:#}"),
    }
}

fn merge_blockers_text(
    open_todos: usize,
    open_review_comments: usize,
    conflicting_workspaces: usize,
) -> String {
    let mut blockers = Vec::new();
    if open_todos > 0 {
        blockers.push(pluralize(open_todos, "open todo"));
    }
    if open_review_comments > 0 {
        blockers.push(pluralize(open_review_comments, "open review comment"));
    }
    if conflicting_workspaces > 0 {
        blockers.push(pluralize(conflicting_workspaces, "conflicting workspace"));
    }
    if blockers.is_empty() {
        "Merge blockers: none".to_owned()
    } else {
        format!("Merge blockers: {}", blockers.join(", "))
    }
}

fn pluralize(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

fn workspace_review_panel(
    db_path: &Path,
    store: &WorkspaceStore,
    name: &str,
    app_state: AppState,
    refresh_hub: RefreshHub,
    toast_overlay: ToastOverlay,
) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 8);
    let feedback = Label::new(None);
    feedback.add_css_class("card-meta");
    feedback.set_xalign(0.0);
    feedback.set_wrap(true);
    let github_threads_state = match store.pull_request(name) {
        Ok(Some(_)) => match store.pull_request_readiness(name) {
            Ok(readiness) => Ok(readiness.review_threads),
            Err(err) => Err(format!("Could not read GitHub review threads: {err:#}")),
        },
        Ok(None) => Ok(Vec::new()),
        Err(err) => Err(format!("Could not read pull request state: {err:#}")),
    };
    let show_github_threads_section = match &github_threads_state {
        Ok(threads) => !threads.is_empty(),
        Err(_) => true,
    };
    let section_titles = review_tab_section_titles(show_github_threads_section);

    if show_github_threads_section {
        panel.append(&section_title(section_titles[0]));
        match github_threads_state {
            Ok(threads) => {
                for thread in threads {
                    let thread_box = GBox::new(Orientation::Vertical, 4);
                    let row = make_action_row();
                    let summary = Label::new(Some(&format_review_thread_row(&thread)));
                    summary.set_xalign(0.0);
                    summary.set_wrap(true);
                    summary.set_hexpand(true);
                    row.append(&summary);

                    if let Some(thread_id) = thread.id.clone() {
                        let button =
                            secondary_button(if thread.resolved { "Reopen" } else { "Resolve" });
                        let db_for_resolution = db_path.to_path_buf();
                        let workspace_for_resolution = name.to_owned();
                        let refresh_after_resolution = refresh_hub.clone();
                        let feedback_for_resolution = feedback.clone();
                        let toast_for_resolution = toast_overlay.clone();
                        let action = if thread.resolved { "Reopen" } else { "Resolve" };
                        let resolved = !thread.resolved;
                        button.connect_clicked(move |_| {
                            let result =
                                WorkspaceStore::open(db_for_resolution.clone()).and_then(|store| {
                                    store.set_pull_request_review_thread_resolution(
                                        &workspace_for_resolution,
                                        &thread_id,
                                        resolved,
                                    )
                                });
                            let should_refresh = result.is_ok();
                            let message =
                                pull_request_review_thread_action_feedback(action, result);
                            apply_action_feedback(
                                &feedback_for_resolution,
                                &toast_for_resolution,
                                &message,
                                true,
                            );
                            if should_refresh {
                                refresh_after_resolution.refresh(RefreshScope::All);
                            }
                        });
                        row.append(&button);
                    }

                    thread_box.append(&row);
                    if let Some(comment) = thread.comments.last() {
                        let preview = Label::new(Some(&format!(
                            "{}: {}",
                            comment.author,
                            comment.body.replace('\n', " ")
                        )));
                        preview.add_css_class("card-meta");
                        preview.set_xalign(0.0);
                        preview.set_wrap(true);
                        thread_box.append(&preview);
                    }
                    panel.append(&thread_box);
                }
            }
            Err(message) => panel.append(&detail_row("GitHub", &message)),
        }
        panel.append(&Separator::new(Orientation::Horizontal));
    }

    panel.append(&section_title(
        section_titles
            .last()
            .copied()
            .unwrap_or("Local review comments"),
    ));

    let form = make_action_row();
    let file_entry = Entry::new();
    file_entry.set_placeholder_text(Some("file path"));
    file_entry.set_hexpand(true);
    let line_entry = Entry::new();
    line_entry.set_placeholder_text(Some("line"));
    let body_entry = Entry::new();
    body_entry.set_placeholder_text(Some("comment"));
    body_entry.set_hexpand(true);
    let add_btn = text_button("Add Comment");
    add_btn.add_css_class("suggested-action");
    let db_for_add = db_path.to_path_buf();
    let workspace_for_add = name.to_owned();
    let refresh_after_add = refresh_hub.clone();
    let file_for_add = file_entry.clone();
    let line_for_add = line_entry.clone();
    let body_for_add = body_entry.clone();
    let feedback_for_add = feedback.clone();
    let toast_for_add = toast_overlay.clone();
    add_btn.connect_clicked(move |_| {
        let file = file_for_add.text().trim().to_owned();
        let body = body_for_add.text().trim().to_owned();
        if file.is_empty() || body.is_empty() {
            apply_action_feedback(
                &feedback_for_add,
                &toast_for_add,
                "File and comment are required.",
                true,
            );
            return;
        }
        let line = match parse_review_comment_line(line_for_add.text().as_ref()) {
            Ok(line) => line,
            Err(err) => {
                apply_action_feedback(&feedback_for_add, &toast_for_add, err, true);
                return;
            }
        };
        match WorkspaceStore::open(db_for_add.clone())
            .and_then(|store| store.add_review_comment(&workspace_for_add, &file, line, &body))
        {
            Ok(comment) => {
                apply_action_feedback(
                    &feedback_for_add,
                    &toast_for_add,
                    &format!("Added review comment #{}", comment.id),
                    true,
                );
                file_for_add.set_text("");
                line_for_add.set_text("");
                body_for_add.set_text("");
                refresh_after_add.refresh(RefreshScope::All);
            }
            Err(err) => apply_action_feedback(
                &feedback_for_add,
                &toast_for_add,
                &format!("Could not add comment: {err:#}"),
                true,
            ),
        }
    });
    form.append(&file_entry);
    form.append(&line_entry);
    form.append(&body_entry);
    form.append(&add_btn);
    panel.append(&form);

    let stage_btn = text_button("Queue Open Comments");
    stage_btn.add_css_class("suggested-action");
    let stage_feedback = feedback.clone();
    let stage_toast = toast_overlay.clone();
    let db_for_stage = db_path.to_path_buf();
    let workspace_for_stage = name.to_owned();
    stage_btn.connect_clicked(move |_| {
        match WorkspaceStore::open(db_for_stage.clone()).and_then(|store| {
            let prompt = store.review_comments_agent_prompt(&workspace_for_stage)?;
            if prompt.contains("No open review comments.") {
                anyhow::bail!("No open review comments to stage");
            }
            Ok(prompt)
        }) {
            Ok(prompt) => {
                app_state.set_staged_review_prompt(Some(prompt));
                apply_action_feedback(
                    &stage_feedback,
                    &stage_toast,
                    "Staged open review comments for the selected agent session.",
                    true,
                );
            }
            Err(err) => apply_action_feedback(
                &stage_feedback,
                &stage_toast,
                &format!("Could not stage review prompt: {err:#}"),
                true,
            ),
        }
    });
    panel.append(&stage_btn);
    panel.append(&feedback);

    match store.list_review_comments(name) {
        Ok(comments) if comments.is_empty() => {
            panel.append(&detail_row("Local review", "No local comments"))
        }
        Ok(comments) => {
            for comment in comments {
                let row = make_action_row();
                let summary = Label::new(Some(&review_comment_row_summary(&comment)));
                summary.set_xalign(0.0);
                summary.set_wrap(true);
                summary.set_hexpand(true);
                row.append(&summary);
                if review_comment_can_resolve(&comment) {
                    let button = secondary_button("Resolve");
                    let db_for_resolve = db_path.to_path_buf();
                    let refresh_after_resolve = refresh_hub.clone();
                    let comment_id = comment.id;
                    let feedback_for_resolve = feedback.clone();
                    let toast_for_resolve = toast_overlay.clone();
                    button.connect_clicked(move |_| {
                        let result = WorkspaceStore::open(db_for_resolve.clone())
                            .and_then(|store| store.resolve_review_comment(comment_id));
                        let message = match result {
                            Ok(ref comment) => {
                                format!("Resolved review comment #{}", comment.id)
                            }
                            Err(ref err) => format!("Could not resolve comment: {err:#}"),
                        };
                        apply_action_feedback(
                            &feedback_for_resolve,
                            &toast_for_resolve,
                            &message,
                            true,
                        );
                        if result.is_ok() {
                            refresh_after_resolve.refresh(RefreshScope::All);
                        }
                    });
                    row.append(&button);
                }
                panel.append(&row);
            }
        }
        Err(err) => panel.append(&detail_row(
            "Local review",
            &format!("Could not read local review comments: {err:#}"),
        )),
    }
    panel
}

fn review_tab_section_titles(has_threads: bool) -> Vec<&'static str> {
    if has_threads {
        vec!["GitHub review threads", "Local review comments"]
    } else {
        vec!["Local review comments"]
    }
}

fn format_review_thread_row(thread: &PullRequestReviewThread) -> String {
    let state = if thread.resolved { "resolved" } else { "open" };
    let location = match (thread.path.as_deref(), thread.line) {
        (Some(path), Some(line)) => format!("{path}:{line}"),
        (Some(path), None) => path.to_owned(),
        (None, Some(line)) => format!("line {line}"),
        (None, None) => "unknown location".to_owned(),
    };
    format!(
        "{} [{}] {} · {}",
        thread.id.as_deref().unwrap_or("thread"),
        state,
        location,
        pluralize(thread.comments.len(), "comment")
    )
}

fn review_comment_row_summary(comment: &ReviewComment) -> String {
    let line = comment
        .line_number
        .map(|line| format!(":{line}"))
        .unwrap_or_default();
    format!(
        "#{} [{}] {}{} - {}",
        comment.id, comment.status, comment.file_path, line, comment.body
    )
}

fn review_comment_can_resolve(comment: &ReviewComment) -> bool {
    comment.status == "open"
}

fn parse_review_comment_line(value: &str) -> Result<Option<i64>, &'static str> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let line = value
        .parse::<i64>()
        .map_err(|_| "line must be a positive number")?;
    if line <= 0 {
        return Err("line must be greater than zero");
    }
    Ok(Some(line))
}

fn workspace_todos_panel(store: &WorkspaceStore, name: &str) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 8);
    match store.list_todos(name) {
        Ok(todos) if todos.is_empty() => panel.append(&detail_row("Todos", "No todos")),
        Ok(todos) => {
            for todo in todos {
                panel.append(&detail_row(
                    &format!("#{} {}", todo.id, todo.status),
                    &todo.text,
                ));
            }
        }
        Err(err) => panel.append(&detail_row(
            "Todos",
            &format!("Could not read todos: {err:#}"),
        )),
    }
    let entry_row = make_action_row();
    let entry = Entry::new();
    entry.set_placeholder_text(Some("Add todo..."));
    entry.set_hexpand(true);
    let add_btn = text_button("Add Todo");
    add_btn.add_css_class("suggested-action");
    let db_path = linux_archductor_core::paths::AppPaths::from_env().database_path;
    let workspace = name.to_owned();
    let entry_clone = entry.clone();
    add_btn.connect_clicked(move |_| {
        let text = entry_clone.text().trim().to_owned();
        if text.is_empty() {
            return;
        }
        if let Ok(store) = WorkspaceStore::open(db_path.clone()) {
            let _ = store.add_todo(&workspace, &text);
            entry_clone.set_text("");
        }
    });
    entry_row.append(&entry);
    entry_row.append(&add_btn);
    panel.append(&entry_row);
    panel
}

fn workspace_processes_text(store: &WorkspaceStore, name: &str) -> String {
    let mut out = String::new();
    out.push_str("Setups\n");
    match store.list_setups(name) {
        Ok(records) if records.is_empty() => out.push_str("No setup runs recorded.\n"),
        Ok(records) => {
            for record in records {
                out.push_str(&format!(
                    "#{} {} pid={} exit={} started={} log={}\n",
                    record.id,
                    record.status.as_str(),
                    record.pid,
                    exit_code_label(record.exit_code),
                    record.started_at,
                    record.log_path.display()
                ));
            }
        }
        Err(err) => out.push_str(&format!("Could not read setup runs: {err:#}\n")),
    }
    out.push('\n');
    out.push_str("Runs\n");
    match store.list_runs(name) {
        Ok(records) if records.is_empty() => out.push_str("No runs recorded.\n"),
        Ok(records) => {
            for record in records {
                out.push_str(&format!(
                    "#{} {} pid={} exit={} started={} log={}\n",
                    record.id,
                    record.status.as_str(),
                    record.pid,
                    exit_code_label(record.exit_code),
                    record.started_at,
                    record.log_path.display()
                ));
            }
        }
        Err(err) => out.push_str(&format!("Could not read runs: {err:#}\n")),
    }
    out.push_str("\nSessions\n");
    match store.list_sessions(name) {
        Ok(records) if records.is_empty() => out.push_str("No sessions recorded.\n"),
        Ok(records) => {
            for record in records {
                out.push_str(&format!(
                    "#{} {} {} pid={} exit={} started={} log={}\n",
                    record.id,
                    record.command,
                    record.status.as_str(),
                    record.pid,
                    exit_code_label(record.exit_code),
                    record.started_at,
                    record.log_path.display()
                ));
            }
        }
        Err(err) => out.push_str(&format!("Could not read sessions: {err:#}\n")),
    }
    out.push_str("\nTerminals\n");
    match store.list_terminals(name) {
        Ok(records) if records.is_empty() => out.push_str("No terminal shells recorded.\n"),
        Ok(records) => {
            for record in records {
                out.push_str(&format!(
                    "#{} {} {} pid={} exit={} started={} log={}\n",
                    record.id,
                    record.command,
                    record.status.as_str(),
                    record.pid,
                    exit_code_label(record.exit_code),
                    record.started_at,
                    record.log_path.display()
                ));
            }
        }
        Err(err) => out.push_str(&format!("Could not read terminals: {err:#}\n")),
    }
    out
}

fn latest_setup_line(store: &WorkspaceStore, name: &str) -> String {
    match store.list_setups(name) {
        Ok(records) => records
            .into_iter()
            .next()
            .map(|record| {
                format!(
                    "{} pid={} exit={} log={}",
                    record.status.as_str(),
                    record.pid,
                    exit_code_label(record.exit_code),
                    record.log_path.display()
                )
            })
            .unwrap_or_else(|| "No setup runs recorded.".to_owned()),
        Err(err) => format!("Could not read setup runtime: {err:#}"),
    }
}

fn latest_runtime_line(store: &WorkspaceStore, name: &str) -> String {
    match store.list_runs(name) {
        Ok(records) => records
            .into_iter()
            .next()
            .map(|record| {
                format!(
                    "{} pid={} exit={} log={}",
                    record.status.as_str(),
                    record.pid,
                    exit_code_label(record.exit_code),
                    record.log_path.display()
                )
            })
            .unwrap_or_else(|| "No runs recorded.".to_owned()),
        Err(err) => format!("Could not read runtime: {err:#}"),
    }
}

fn spotlight_line(store: &WorkspaceStore, name: &str) -> String {
    match store.spotlight_status(name) {
        Ok(Some(session)) => {
            let root_status = match store.spotlight_root_conflict_paths(name) {
                Ok(paths) => spotlight_root_conflict_status(&paths),
                Err(err) => format!("root check failed: {err:#}"),
            };
            format!(
                "{} since {} patch={}\n{}",
                session.status,
                session.started_at,
                session.patch_path.display(),
                root_status
            )
        }
        Ok(None) => "Inactive".to_owned(),
        Err(err) => format!("Could not read Spotlight status: {err:#}"),
    }
}

fn spotlight_root_conflict_status(paths: &[String]) -> String {
    if paths.is_empty() {
        return "root clean".to_owned();
    }
    format!("root extra edits: {}", paths.join(", "))
}

fn latest_setup_log_line(store: &WorkspaceStore, name: &str) -> String {
    match store.read_latest_setup_log(name) {
        Ok(log) => tail_lines(&log, 12),
        Err(_) => "No setup log yet.".to_owned(),
    }
}

fn latest_run_log_line(store: &WorkspaceStore, name: &str) -> String {
    match store.read_latest_run_log(name) {
        Ok(log) => tail_lines(&log, 12),
        Err(_) => "No run log yet.".to_owned(),
    }
}

fn tail_lines(text: &str, max_lines: usize) -> String {
    let lines = text.lines().collect::<Vec<_>>();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

fn exit_code_label(exit_code: Option<i32>) -> String {
    exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "-".to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeActionFeedback {
    status_text: String,
    toast_text: Option<String>,
}

fn runtime_action_failure_feedback(action: &str, err: &anyhow::Error) -> RuntimeActionFeedback {
    if is_spotlight_dirty_root_error(err) {
        let detail = spotlight_dirty_root_paths(err)
            .map(|paths| {
                let list = paths
                    .into_iter()
                    .map(|path| format!("\n- {path}"))
                    .collect::<String>();
                format!("\nConflicting root edits:{list}")
            })
            .unwrap_or_default();
        return RuntimeActionFeedback {
            status_text: format!(
                "{action} blocked: repository root has extra edits outside the active Spotlight patch.{detail}\nRepair Spotlight discards root-only edits and reapplies the active patch. Clean/save root changes manually if you need to keep them."
            ),
            toast_text: Some(
                "Spotlight root has extra edits. Use Repair Spotlight or clean/save root changes."
                    .to_owned(),
            ),
        };
    }
    let text = format!("{action} failed: {err:#}");
    RuntimeActionFeedback {
        status_text: text.clone(),
        toast_text: Some(text),
    }
}

fn is_spotlight_dirty_root_error(err: &anyhow::Error) -> bool {
    err.to_string()
        .contains("repository root has changes outside the active Spotlight patch")
}

fn spotlight_dirty_root_detail(err: &anyhow::Error) -> Option<String> {
    let message = err.to_string();
    let detail = message.split("changed root paths: ").nth(1)?;
    let detail = detail
        .split("; clean or save root changes")
        .next()
        .unwrap_or(detail)
        .trim();
    (!detail.is_empty()).then(|| detail.to_owned())
}

fn spotlight_dirty_root_paths(err: &anyhow::Error) -> Option<Vec<String>> {
    let detail = spotlight_dirty_root_detail(err)?;
    let paths = detail
        .split(',')
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    (!paths.is_empty()).then_some(paths)
}

fn lifecycle_action_failure_feedback(action: &str, err: &anyhow::Error) -> RuntimeActionFeedback {
    let text = format!("{action} failed: {err:#}");
    RuntimeActionFeedback {
        status_text: text.clone(),
        toast_text: Some(text),
    }
}

fn apply_runtime_action_feedback(
    status: &Label,
    toast_overlay: &ToastOverlay,
    feedback: RuntimeActionFeedback,
) {
    status.set_text(&feedback.status_text);
    if let Some(toast_text) = feedback.toast_text {
        toast_overlay.add_toast(Toast::new(&toast_text));
    }
}

fn apply_action_feedback(
    status: &Label,
    toast_overlay: &ToastOverlay,
    text: &str,
    show_toast: bool,
) {
    status.set_text(text);
    if show_toast {
        toast_overlay.add_toast(Toast::new(text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_tab_stack_name_maps_palette_targets_to_tabs() {
        assert_eq!(
            workspace_tab_stack_name(&WorkspaceTab::Chats),
            "chat-terminal"
        );
        assert_eq!(workspace_tab_stack_name(&WorkspaceTab::Changes), "work");
        assert_eq!(workspace_tab_stack_name(&WorkspaceTab::Checks), "work");
        assert_eq!(
            workspace_tab_stack_name(&WorkspaceTab::Terminal),
            "terminal"
        );
        assert_eq!(workspace_tab_stack_name(&WorkspaceTab::Todos), "todos");
        assert_eq!(
            workspace_tab_stack_name(&WorkspaceTab::Checkpoints),
            "checkpoints"
        );
        assert_eq!(
            workspace_tab_stack_name(&WorkspaceTab::Processes),
            "processes"
        );
    }

    #[test]
    fn runtime_action_failure_feedback_includes_status_and_toast() {
        let feedback = runtime_action_failure_feedback("Setup", &anyhow::anyhow!("missing setup"));

        assert_eq!(feedback.status_text, "Setup failed: missing setup");
        assert_eq!(
            feedback.toast_text.as_deref(),
            Some("Setup failed: missing setup")
        );
    }

    #[test]
    fn spotlight_conflict_feedback_points_to_repair_action() {
        let feedback = runtime_action_failure_feedback(
            "Spotlight sync",
            &anyhow::anyhow!(
                "repository root has changes outside the active Spotlight patch; changed root paths: root-only.txt"
            ),
        );

        assert!(feedback.status_text.contains("Spotlight sync blocked"));
        assert!(feedback.status_text.contains("Repair Spotlight"));
        assert!(feedback.status_text.contains("root-only.txt"));
        assert_eq!(
            feedback.toast_text.as_deref(),
            Some(
                "Spotlight root has extra edits. Use Repair Spotlight or clean/save root changes."
            )
        );
    }

    #[test]
    fn spotlight_conflict_feedback_lists_paths_and_warns_repair_discards_root_edits() {
        let feedback = runtime_action_failure_feedback(
            "Spotlight stop",
            &anyhow::anyhow!(
                "repository root has changes outside the active Spotlight patch; changed root paths: root-only.txt, config/local.env; clean or save root changes before changing Spotlight state"
            ),
        );

        assert!(feedback.status_text.contains("Conflicting root edits:"));
        assert!(feedback.status_text.contains("- root-only.txt"));
        assert!(feedback.status_text.contains("- config/local.env"));
        assert!(feedback.status_text.contains("Repair Spotlight discards"));
    }

    #[test]
    fn spotlight_root_conflict_status_summarizes_clean_and_dirty_roots() {
        assert_eq!(spotlight_root_conflict_status(&[]), "root clean");
        assert_eq!(
            spotlight_root_conflict_status(&[
                "root-only.txt".to_owned(),
                "config/local.env".to_owned()
            ]),
            "root extra edits: root-only.txt, config/local.env"
        );
    }

    #[test]
    fn diff_file_summary_renders_review_scan_rows() {
        let summaries = vec![
            linux_archductor_core::workspace::DiffFileSummary {
                path: "README.md".to_owned(),
                additions: Some(2),
                deletions: Some(1),
            },
            linux_archductor_core::workspace::DiffFileSummary {
                path: "assets/logo.png".to_owned(),
                additions: None,
                deletions: None,
            },
        ];

        let rendered = format_diff_file_summary(&summaries);

        assert!(rendered.contains("Files changed"));
        assert!(rendered.contains("README.md +2 -1"));
        assert!(rendered.contains("assets/logo.png binary"));
    }

    #[test]
    fn review_comment_summary_marks_open_comments_resolvable() {
        let comment = linux_archductor_core::workspace::ReviewComment {
            id: 7,
            workspace_id: 1,
            file_path: "src/lib.rs".to_owned(),
            line_number: Some(42),
            body: "handle empty input".to_owned(),
            status: "open".to_owned(),
            github_thread_id: None,
            created_at: "2026-06-19T00:00:00Z".to_owned(),
            updated_at: "2026-06-19T00:00:00Z".to_owned(),
        };

        let summary = review_comment_row_summary(&comment);

        assert_eq!(summary, "#7 [open] src/lib.rs:42 - handle empty input");
        assert!(review_comment_can_resolve(&comment));
    }

    #[test]
    fn file_inline_comments_text_filters_to_selected_file() {
        let comments = vec![
            linux_archductor_core::workspace::ReviewComment {
                id: 7,
                workspace_id: 1,
                file_path: "src/lib.rs".to_owned(),
                line_number: Some(42),
                body: "handle empty input".to_owned(),
                status: "open".to_owned(),
                github_thread_id: None,
                created_at: "2026-06-19T00:00:00Z".to_owned(),
                updated_at: "2026-06-19T00:00:00Z".to_owned(),
            },
            linux_archductor_core::workspace::ReviewComment {
                id: 8,
                workspace_id: 1,
                file_path: "README.md".to_owned(),
                line_number: None,
                body: "clarify setup".to_owned(),
                status: "resolved".to_owned(),
                github_thread_id: None,
                created_at: "2026-06-19T00:00:00Z".to_owned(),
                updated_at: "2026-06-19T00:00:00Z".to_owned(),
            },
        ];

        let rendered = file_inline_comments_text(&comments, "src/lib.rs");

        assert!(rendered.contains("Inline comments for src/lib.rs"));
        assert!(rendered.contains("#7 [open] src/lib.rs:42 - handle empty input"));
        assert!(!rendered.contains("clarify setup"));
    }

    #[test]
    fn diff_tree_rows_insert_directory_headers_once() {
        let summaries = vec![
            linux_archductor_core::workspace::DiffFileSummary {
                path: "src/lib.rs".to_owned(),
                additions: Some(1),
                deletions: Some(0),
            },
            linux_archductor_core::workspace::DiffFileSummary {
                path: "src/ui/panel.rs".to_owned(),
                additions: Some(3),
                deletions: Some(1),
            },
        ];

        let rows = diff_tree_rows(&summaries);

        assert_eq!(
            rows,
            vec![
                DiffTreeRow::Directory("src/".to_owned()),
                DiffTreeRow::File(summaries[0].clone()),
                DiffTreeRow::Directory("  ui/".to_owned()),
                DiffTreeRow::File(summaries[1].clone()),
            ]
        );
        assert_eq!(
            diff_tree_file_label(&summaries[1]),
            "    src/ui/panel.rs +3 -1"
        );
    }

    #[test]
    fn review_comment_line_input_allows_blank_or_positive_numbers() {
        assert_eq!(parse_review_comment_line(""), Ok(None));
        assert_eq!(parse_review_comment_line(" 42 "), Ok(Some(42)));
        assert_eq!(
            parse_review_comment_line("zero").unwrap_err(),
            "line must be a positive number"
        );
        assert_eq!(
            parse_review_comment_line("0").unwrap_err(),
            "line must be greater than zero"
        );
    }

    #[test]
    fn run_console_state_seeds_terminal_tab_and_defaults_to_setup() {
        let state = WorkspaceRunConsoleState::default();

        assert_eq!(state.active_tab_name(), "setup");
        assert_eq!(state.terminals.len(), 1);
        assert_eq!(state.terminals[0].tab_name(), "terminal-1");
    }

    #[test]
    fn split_position_for_ratio_prefers_five_to_three_layout_and_clamps() {
        assert_eq!(split_position_for_ratio(1280, 5, 3, 360, 280), 800);
        assert_eq!(split_position_for_ratio(700, 5, 3, 360, 280), 420);
        assert_eq!(split_position_for_ratio(500, 5, 3, 360, 280), 360);
    }

    #[test]
    fn run_console_state_keeps_active_terminal_when_it_exists() {
        let mut state = WorkspaceRunConsoleState::default();
        let active = state.add_terminal_tab();
        state.active_tab = active.clone();

        assert_eq!(state.active_tab_name(), active);
    }

    #[test]
    fn run_console_state_falls_back_to_setup_when_active_terminal_is_missing() {
        let state = WorkspaceRunConsoleState {
            active_tab: "terminal-99".to_owned(),
            ..Default::default()
        };

        assert_eq!(state.active_tab_name(), "setup");
    }

    #[test]
    fn workspace_chat_default_title_counts_from_existing_threads() {
        assert_eq!(workspace_chat_default_title(&[]), "New Chat");
        assert_eq!(
            workspace_chat_default_title(&[ChatThreadRecord {
                id: 1,
                workspace_id: 2,
                provider: "codex".to_owned(),
                title: "New Chat".to_owned(),
                status: "active".to_owned(),
                native_thread_id: None,
                harness_metadata: None,
                created_at: "now".to_owned(),
                updated_at: "now".to_owned(),
                archived_at: None,
            }]),
            "New Chat 2"
        );
    }

    #[test]
    fn run_console_terminal_state_appends_command_output() {
        let mut terminal = WorkspaceRunConsoleTerminalState::new(3);
        terminal.append_command("cargo test");
        terminal.append_result("[exit 0]\n");

        assert!(terminal.transcript.contains("$ cargo test"));
        assert!(terminal.transcript.contains("[running]"));
        assert!(terminal.transcript.contains("[exit 0]"));
    }

    #[test]
    fn review_thread_row_format_includes_state_location_and_comment_count() {
        let thread = linux_archductor_core::workspace::PullRequestReviewThread {
            id: Some("PRRT_fake".to_owned()),
            path: Some("src/lib.rs".to_owned()),
            line: Some(42),
            resolved: false,
            comments: vec![linux_archductor_core::workspace::PullRequestThreadComment {
                author: "alice".to_owned(),
                body: "Need a test.".to_owned(),
                url: None,
                created_at: None,
            }],
        };

        let summary = format_review_thread_row(&thread);

        assert_eq!(summary, "PRRT_fake [open] src/lib.rs:42 · 1 comment");
    }

    #[test]
    fn review_tab_sections_keep_github_threads_separate_from_local_comments() {
        assert_eq!(
            review_tab_section_titles(true),
            vec!["GitHub review threads", "Local review comments"]
        );
        assert_eq!(
            review_tab_section_titles(false),
            vec!["Local review comments"]
        );
    }

    #[test]
    fn pull_request_create_feedback_summarizes_output() {
        let success = pull_request_create_feedback(Ok(
            "Creating pull request\nhttps://github.com/example/demo/pull/42\n".to_owned(),
        ));
        assert_eq!(
            success,
            "Created PR: https://github.com/example/demo/pull/42"
        );

        let failure = pull_request_create_feedback(Err(anyhow::anyhow!(
            "workspace berlin has no changed files"
        )));
        assert_eq!(
            failure,
            "Create PR failed: workspace berlin has no changed files"
        );
    }

    #[test]
    fn pull_request_actions_include_create_controls_without_pr() {
        let actions = pull_request_action_labels(None);

        assert_eq!(actions, vec!["Push branch", "Create PR"]);
    }

    #[test]
    fn pull_request_actions_include_summary_review_and_refresh_for_open_prs() {
        let actions = pull_request_action_labels(Some(PullRequestStateKind::Open));

        assert_eq!(actions, vec!["PR summary", "Reviews", "Refresh"]);
    }

    #[test]
    fn pull_request_actions_include_merge_summary_and_refresh_for_ready_prs() {
        let actions = pull_request_action_labels(Some(PullRequestStateKind::Ready));

        assert_eq!(actions, vec!["Merge", "PR summary", "Refresh"]);
    }

    #[test]
    fn pull_request_actions_include_fix_summary_and_refresh_for_failed_prs() {
        let actions = pull_request_action_labels(Some(PullRequestStateKind::Failed));

        assert_eq!(actions, vec!["Fix checks", "PR summary", "Refresh"]);
    }

    #[test]
    fn pull_request_actions_include_continue_and_archive_for_merged_prs() {
        let actions = pull_request_action_labels(Some(PullRequestStateKind::Merged));

        assert_eq!(actions, vec!["Continue", "Archive"]);
    }

    #[test]
    fn pull_request_merge_feedback_summarizes_output() {
        let success = pull_request_merge_feedback(Ok(
            "Merged pull request #42\nDeleted branch lc/berlin\n".to_owned(),
        ));
        assert_eq!(success, "Merged PR: Merged pull request #42");

        let failure = pull_request_merge_feedback(Err(anyhow::anyhow!(
            "2 open todo(s) remain in workspace berlin"
        )));
        assert_eq!(
            failure,
            "Merge PR failed: 2 open todo(s) remain in workspace berlin"
        );
    }

    #[test]
    fn pull_request_merge_and_archive_feedback_reports_archive_state() {
        let success = pull_request_merge_and_archive_feedback(Ok(
            linux_archductor_core::workspace::MergePullRequestResult {
                merge_output: "Merged pull request #42\n".to_owned(),
                archived_workspace: Some(Workspace {
                    id: 1,
                    repository_id: 2,
                    name: "berlin".to_owned(),
                    path: std::path::PathBuf::from("/tmp/berlin"),
                    branch: "lc/berlin".to_owned(),
                    base_ref: "main".to_owned(),
                    port_base: 4200,
                    status: "archived".to_owned(),
                    archived_at: Some("now".to_owned()),
                    created_at: "then".to_owned(),
                    updated_at: "now".to_owned(),
                }),
            },
        ));
        assert_eq!(
            success,
            "Merged PR: Merged pull request #42; archived workspace berlin."
        );

        let no_archive = pull_request_merge_and_archive_feedback(Ok(
            linux_archductor_core::workspace::MergePullRequestResult {
                merge_output: "Merged pull request #42\n".to_owned(),
                archived_workspace: None,
            },
        ));
        assert_eq!(no_archive, "Merged PR: Merged pull request #42");
    }

    #[test]
    fn merge_blockers_text_lists_blocking_review_state() {
        let text = merge_blockers_text(2, 1, 1);

        assert_eq!(
            text,
            "Merge blockers: 2 open todos, 1 open review comment, 1 conflicting workspace"
        );
        assert_eq!(merge_blockers_text(0, 0, 0), "Merge blockers: none");
    }

    #[test]
    fn pull_request_refresh_feedback_summarizes_state() {
        let success = pull_request_refresh_feedback(Ok(Some(
            linux_archductor_core::workspace::PullRequest {
                id: 1,
                workspace_id: 2,
                provider: "github".to_owned(),
                number: 42,
                url: "https://github.com/example/demo/pull/42".to_owned(),
                state: "MERGED".to_owned(),
                created_at: "now".to_owned(),
                updated_at: "now".to_owned(),
            },
        )));
        assert_eq!(success, "PR #42 state: MERGED");

        let missing = pull_request_refresh_feedback(Ok(None));
        assert_eq!(missing, "No PR recorded for this workspace.");

        let failure = pull_request_refresh_feedback(Err(anyhow::anyhow!("gh auth required")));
        assert_eq!(failure, "Refresh PR failed: gh auth required");
    }

    #[test]
    fn pull_request_checks_feedback_keeps_raw_check_output() {
        let success = pull_request_checks_feedback(Ok(
            "build\tpass\t1m\thttps://github.com/example/demo/actions/runs/1\n".to_owned(),
        ));
        assert_eq!(
            success,
            "PR checks:\nbuild\tpass\t1m\thttps://github.com/example/demo/actions/runs/1"
        );

        let empty = pull_request_checks_feedback(Ok(String::new()));
        assert_eq!(empty, "PR checks returned no output.");

        let failure = pull_request_checks_feedback(Err(anyhow::anyhow!("no pull requests found")));
        assert_eq!(failure, "View checks failed: no pull requests found");
    }

    #[test]
    fn pull_request_review_feedback_keeps_raw_review_output() {
        let success = pull_request_review_feedback(Ok(
            "Reviewers: changes requested\nalice: add a test\n".to_owned(),
        ));
        assert_eq!(
            success,
            "PR comments/reviews:\nReviewers: changes requested\nalice: add a test"
        );

        let empty = pull_request_review_feedback(Ok(String::new()));
        assert_eq!(empty, "PR comments/reviews returned no output.");

        let failure = pull_request_review_feedback(Err(anyhow::anyhow!("gh auth required")));
        assert_eq!(failure, "View PR comments/reviews failed: gh auth required");
    }

    #[test]
    fn pull_request_readiness_feedback_summarizes_structured_pr_state() {
        let success = pull_request_readiness_feedback(Ok(
            "PR readiness for workspace berlin.\nReview decision: CHANGES_REQUESTED\n".to_owned(),
        ));
        assert_eq!(
            success,
            "PR readiness for workspace berlin.\nReview decision: CHANGES_REQUESTED"
        );

        let empty = pull_request_readiness_feedback(Ok(String::new()));
        assert_eq!(empty, "PR readiness summary returned no output.");

        let failure = pull_request_readiness_feedback(Err(anyhow::anyhow!("gh auth required")));
        assert_eq!(failure, "View PR summary failed: gh auth required");
    }

    #[test]
    fn pull_request_status_summary_prefers_failed_checks() {
        let status = pull_request_status_summary(
            &linux_archductor_core::workspace::PullRequest {
                id: 1,
                workspace_id: 2,
                provider: "github".to_owned(),
                number: 42,
                url: "https://github.com/example/demo/pull/42".to_owned(),
                state: "OPEN".to_owned(),
                created_at: "then".to_owned(),
                updated_at: "now".to_owned(),
            },
            Some(&linux_archductor_core::workspace::PullRequestReadiness {
                review_decision: Some("CHANGES_REQUESTED".to_owned()),
                latest_reviews: Vec::new(),
                comments: Vec::new(),
                review_threads: Vec::new(),
                checks: Vec::new(),
                deployments: Vec::new(),
            }),
            &linux_archductor_core::workspace::ChecksSummary {
                workspace: Workspace {
                    id: 1,
                    repository_id: 2,
                    name: "berlin".to_owned(),
                    path: std::path::PathBuf::from("/tmp/berlin"),
                    branch: "lc/berlin".to_owned(),
                    base_ref: "main".to_owned(),
                    port_base: 4200,
                    status: "active".to_owned(),
                    archived_at: None,
                    created_at: "then".to_owned(),
                    updated_at: "now".to_owned(),
                },
                changed_files: 0,
                run_status: None,
                session_status: None,
                active_sessions: 0,
                pull_request: None,
                open_todos: 0,
                total_todos: 0,
                branch_push_state: None,
                open_review_comments: 0,
                conflicting_workspaces: Vec::new(),
            },
        );

        assert_eq!(status.label, "checks failed");
        assert_eq!(status.css_class, "ws-pr-status-failed");
        assert_eq!(status.kind, PullRequestStateKind::Failed);
    }

    #[test]
    fn pull_request_status_summary_open_state_is_not_attention() {
        let status = pull_request_status_summary(
            &linux_archductor_core::workspace::PullRequest {
                id: 1,
                workspace_id: 2,
                provider: "github".to_owned(),
                number: 42,
                url: "https://github.com/example/demo/pull/42".to_owned(),
                state: "OPEN".to_owned(),
                created_at: "then".to_owned(),
                updated_at: "now".to_owned(),
            },
            None,
            &linux_archductor_core::workspace::ChecksSummary {
                workspace: Workspace {
                    id: 1,
                    repository_id: 2,
                    name: "berlin".to_owned(),
                    path: std::path::PathBuf::from("/tmp/berlin"),
                    branch: "lc/berlin".to_owned(),
                    base_ref: "main".to_owned(),
                    port_base: 4200,
                    status: "active".to_owned(),
                    archived_at: None,
                    created_at: "then".to_owned(),
                    updated_at: "now".to_owned(),
                },
                changed_files: 0,
                run_status: None,
                session_status: None,
                active_sessions: 0,
                pull_request: None,
                open_todos: 0,
                total_todos: 0,
                branch_push_state: None,
                open_review_comments: 0,
                conflicting_workspaces: Vec::new(),
            },
        );

        assert_eq!(status.label, "OPEN");
        assert_eq!(status.kind, PullRequestStateKind::Open);
        assert_eq!(status.attention_label(), None);
        assert_eq!(status.attention_css_class(), None);
    }

    #[test]
    fn pull_request_status_summary_keeps_failed_attention_without_checks_summary() {
        let pr = linux_archductor_core::workspace::PullRequest {
            id: 1,
            workspace_id: 2,
            provider: "github".to_owned(),
            number: 42,
            url: "https://github.com/example/demo/pull/42".to_owned(),
            state: "OPEN".to_owned(),
            created_at: "then".to_owned(),
            updated_at: "now".to_owned(),
        };
        let readiness = linux_archductor_core::workspace::PullRequestReadiness {
            review_decision: Some("CHANGES_REQUESTED".to_owned()),
            latest_reviews: Vec::new(),
            comments: Vec::new(),
            review_threads: Vec::new(),
            checks: Vec::new(),
            deployments: Vec::new(),
        };

        let status = pull_request_status_summary_without_checks_summary(&pr, Some(&readiness));

        assert_eq!(status.label, "checks failed");
        assert_eq!(status.css_class, "ws-pr-status-failed");
        assert_eq!(status.kind, PullRequestStateKind::Failed);
        assert_eq!(status.attention_label(), Some("checks failed"));
    }

    #[test]
    fn pull_request_review_thread_action_feedback_reports_state_and_id() {
        let success = pull_request_review_thread_action_feedback(
            "Resolve",
            Ok(linux_archductor_core::workspace::PullRequestReviewThread {
                id: Some("PRRT_fake".to_owned()),
                path: Some("src/lib.rs".to_owned()),
                line: Some(42),
                resolved: true,
                comments: Vec::new(),
            }),
        );
        assert_eq!(
            success,
            "Resolve review thread PRRT_fake: resolved at src/lib.rs:42."
        );

        let failure =
            pull_request_review_thread_action_feedback("Reopen", Err(anyhow::anyhow!("bad id")));
        assert_eq!(failure, "Reopen review thread failed: bad id");
    }

    #[test]
    fn pull_request_archive_feedback_summarizes_workspace_status() {
        let success = pull_request_archive_feedback(Ok(Workspace {
            id: 1,
            repository_id: 2,
            name: "berlin".to_owned(),
            path: std::path::PathBuf::from("/tmp/berlin"),
            branch: "lc/berlin".to_owned(),
            base_ref: "main".to_owned(),
            port_base: 4200,
            status: "archived".to_owned(),
            archived_at: Some("now".to_owned()),
            created_at: "then".to_owned(),
            updated_at: "now".to_owned(),
        }));
        assert_eq!(success, "Archived workspace berlin.");

        let failure = pull_request_archive_feedback(Err(anyhow::anyhow!("archive script failed")));
        assert_eq!(failure, "Archive failed: archive script failed");
    }

    #[test]
    fn lifecycle_action_failure_feedback_includes_status_and_toast() {
        let feedback = lifecycle_action_failure_feedback("Rename", &anyhow::anyhow!("bad name"));

        assert_eq!(feedback.status_text, "Rename failed: bad name");
        assert_eq!(
            feedback.toast_text.as_deref(),
            Some("Rename failed: bad name")
        );
    }
}
