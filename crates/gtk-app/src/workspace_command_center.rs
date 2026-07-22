use crate::file_component::OpenWorkspaceFile;
use adw::ToastOverlay;
use archductor_core::agent_tools::launchable_provider_key;
use archductor_core::archcar::protocol::{ArchcarInputKind, ArchcarRequest};
use archductor_core::doctor::SetupReadiness;
use archductor_core::paths::AppPaths;
use archductor_core::settings::PromptKind;
use archductor_core::workspace::{
    ChatThreadRecord, DiffFileSummary, ProcessRecord, ProcessStatus, PullRequest,
    PullRequestCheckRun, PullRequestReviewThread, ReviewComment, SessionKind, Workspace,
    WorkspaceStore, WorkspaceTimelineEvent,
};
use gtk::prelude::*;
use gtk::{
    Align, Box as GBox, Button, CheckButton, ComboBoxText, Entry, EventControllerScroll,
    EventControllerScrollFlags, GestureClick, Image, Label, ListBox, ListBoxRow, Orientation,
    Paned, PolicyType, Popover, ScrolledWindow, Separator, Spinner, Stack, StackSwitcher, TextTag,
    TextView, Widget, WrapMode,
};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use tracing::error;

const WORKSPACE_SPLIT_MIN_START: i32 = 280;
const WORKSPACE_SPLIT_MIN_END: i32 = 260;
const WORKSPACE_SPLIT_DEFAULT_CONTENT_WIDTH: i32 = 1280;
const WORKSPACE_RIGHT_PANEL_DEFAULT_WIDTH: i32 = 340;
const WS_CHAT_TAB_LIMIT: usize = 10;
const DIFF_RENDER_LIMIT_BYTES: usize = 200_000;
const WORKSPACE_COMMIT_CHANGE_LIMIT: usize = 25;
type WorkspaceTabSelector = Rc<dyn Fn(&str)>;
type ContextMenuItem = (&'static str, Rc<dyn Fn()>);

struct WorkspaceChatTabSnapshot {
    workspace_name: String,
    selected: Option<i64>,
    threads: Vec<ChatThreadRecord>,
    nav_items_by_thread: HashMap<i64, crate::background_sync::WorkspaceChatNavItem>,
}

struct WorkspaceFileSnapshot {
    relative_path: String,
    contents: String,
    diff_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WorkspaceChatTabStateInput {
    selected: bool,
    generating: bool,
    finished_unread: bool,
    composer_dirty: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkspaceChatTabVisualState {
    Generating,
    FinishedGenerating,
    Read,
    Selected,
    SelectedGenerating,
    Editing,
}

fn reduce_workspace_chat_tab_state(
    input: WorkspaceChatTabStateInput,
) -> WorkspaceChatTabVisualState {
    if input.selected && input.generating {
        WorkspaceChatTabVisualState::SelectedGenerating
    } else if input.selected && input.composer_dirty {
        WorkspaceChatTabVisualState::Editing
    } else if input.selected {
        WorkspaceChatTabVisualState::Selected
    } else if input.generating {
        WorkspaceChatTabVisualState::Generating
    } else if input.finished_unread {
        WorkspaceChatTabVisualState::FinishedGenerating
    } else {
        WorkspaceChatTabVisualState::Read
    }
}

fn chat_outcome_requires_nav_refresh(outcome: &session_surface::ChatRefreshOutcome) -> bool {
    outcome.requires_nav_refresh()
}

fn chat_message_event_matches_selected_workspace(
    event_workspace: &str,
    selected_workspace: Option<&str>,
) -> bool {
    selected_workspace == Some(event_workspace)
}

fn workspace_phase_status_text(phase: Option<WorkspaceUiPhase>) -> Option<String> {
    match phase {
        Some(WorkspaceUiPhase::Creating { detail }) => Some(detail),
        Some(WorkspaceUiPhase::StartingAgent { detail }) => Some(detail),
        Some(WorkspaceUiPhase::Failed { message }) => Some(message),
        Some(WorkspaceUiPhase::Ready) | None => None,
    }
}

fn apply_workspace_phase_status(label: &Label, phase: Option<WorkspaceUiPhase>) {
    if let Some(text) = workspace_phase_status_text(phase) {
        label.set_text(&text);
        label.set_visible(true);
    } else {
        label.set_text("");
        label.set_visible(false);
    }
}

fn clone_external_thread_selection_controller(
    controller: &session_surface::ExternalThreadSelectionController,
) -> Option<Rc<dyn Fn(Option<i64>)>> {
    controller.borrow().as_ref().cloned()
}

use crate::refresh::{RefreshEvent, RefreshHub, RefreshScope};
use crate::state::{
    AppState, AppStateEvent, ChatUiPhase, ChatUiTarget, WorkspaceRightPanelTab, WorkspaceTab,
    WorkspaceUiPhase,
};
use crate::toast::{show_toast as emit_toast, surface_label_error, ToastManager, ToastMessage};
use crate::{
    archcar_async::{spawn_archcar_request, spawn_background_job},
    buttons::{menu_text_button, text_button},
    cli_binary, detail_row, history, session_surface, shell_quote, spawn_terminal_command,
    terminal, title_case_workspace,
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
    WorkspaceStore::open_app(db_path)
        .ok()
        .map(|store| workspace_repository_name(&store, workspace_name))
        .unwrap_or_else(|| workspace_name.to_owned())
}

fn load_workspace_chat_tab_snapshot(
    db_path: PathBuf,
    workspace_name: String,
    selected: Option<i64>,
) -> Result<WorkspaceChatTabSnapshot, String> {
    WorkspaceStore::open_app(db_path)
        .and_then(|store| {
            let threads = store.list_chat_threads(&workspace_name)?;
            let nav_items_by_thread =
                crate::background_sync::load_workspace_chat_nav(&store, &workspace_name, selected)?
                    .into_iter()
                    .map(|item| (item.thread_id, item))
                    .collect::<HashMap<_, _>>();
            Ok(WorkspaceChatTabSnapshot {
                workspace_name,
                selected,
                threads,
                nav_items_by_thread,
            })
        })
        .map_err(|err| format!("{err:#}"))
}

fn load_workspace_file_snapshot(
    workspace_path: PathBuf,
    db_path: PathBuf,
    workspace_name: String,
    relative_path: String,
) -> WorkspaceFileSnapshot {
    let file_path = workspace_path.join(&relative_path);
    let contents =
        fs::read_to_string(&file_path).unwrap_or_else(|_| "[binary or unreadable file]".to_owned());
    let diff_text = workspace_diff_text_for_path(&db_path, &workspace_name, Some(&relative_path));
    WorkspaceFileSnapshot {
        relative_path,
        contents,
        diff_text,
    }
}

#[derive(Clone)]
struct FileTreeRow {
    row: ListBoxRow,
    path: String,
    is_dir: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceChangesScopeItem {
    label: String,
    stack_key: String,
    menu_label: String,
    persisted_scope: String,
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
        let Ok(store) = WorkspaceStore::open_app(db_path.clone()) else {
            return;
        };
        let Ok(Some(line)) = store
            .list_status()
            .map(|lines| lines.into_iter().find(|l| l.workspace.name == name))
        else {
            return;
        };

        if matches!(line.workspace.status.as_str(), "creating" | "failed") {
            body.append(&workspace_creation_status_shell(
                &db_path,
                &line.workspace,
                &state,
                refresh_hub.clone(),
                toast_overlay.clone(),
            ));
            return;
        }

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

fn workspace_creation_status_shell(
    db_path: &Path,
    ws: &Workspace,
    state: &AppState,
    refresh_hub: RefreshHub,
    toast_overlay: ToastOverlay,
) -> GBox {
    let shell = GBox::new(Orientation::Vertical, 0);
    shell.set_vexpand(true);
    shell.set_hexpand(true);
    shell.add_css_class("chat-surface");

    let header = GBox::new(Orientation::Horizontal, 8);
    header.add_css_class("session-header-row");
    header.set_margin_top(14);
    header.set_margin_bottom(14);
    header.set_margin_start(14);
    header.set_margin_end(14);

    if ws.status == "creating" {
        let spinner = Spinner::new();
        spinner.start();
        header.append(&spinner);
    }

    let title_box = GBox::new(Orientation::Vertical, 2);
    title_box.set_hexpand(true);
    let title = Label::new(Some(&title_case_workspace(&ws.name)));
    title.add_css_class("session-title");
    title.set_xalign(0.0);
    title_box.append(&title);
    let meta = Label::new(Some(&format!("{} · {}", ws.branch, ws.status)));
    meta.add_css_class("workspace-meta");
    meta.set_xalign(0.0);
    title_box.append(&meta);
    header.append(&title_box);
    shell.append(&header);

    let body = GBox::new(Orientation::Vertical, 8);
    body.set_vexpand(true);
    body.set_valign(Align::Center);
    body.set_halign(Align::Center);
    let status = if ws.status == "failed" {
        "Workspace creation failed."
    } else {
        "Creating workspace..."
    };
    let label = Label::new(Some(status));
    label.add_css_class(if ws.status == "failed" {
        "status-error"
    } else {
        "workspace-empty-label"
    });
    body.append(&label);
    if workspace_creation_status_allows_delete(&ws.status) {
        let confirm = CheckButton::with_label("Confirm delete");
        let delete_btn = destructive_button("Delete");
        let progress = Label::new(None);
        progress.add_css_class("card-meta");
        progress.set_wrap(true);
        progress.set_xalign(0.0);

        let db_delete = db_path.to_path_buf();
        let workspace_delete = ws.name.clone();
        let refresh_after_delete = refresh_hub.clone();
        let progress_after_delete = progress.clone();
        let toast_after_delete = toast_overlay.clone();
        let state_after_delete = state.clone();
        let confirm_delete = confirm.clone();
        delete_btn.connect_clicked(move |_| {
            if !confirm_delete.is_active() {
                progress_after_delete.set_text("Check confirm before delete.");
                return;
            }
            progress_after_delete.set_text("Deleting in background...");
            let db_delete_job = db_delete.clone();
            let workspace_delete_job = workspace_delete.clone();
            let refresh_done = refresh_after_delete.clone();
            let progress_done = progress_after_delete.clone();
            let toast_done = toast_after_delete.clone();
            let state_done = state_after_delete.clone();
            spawn_background_job(
                move || {
                    WorkspaceStore::open_app(db_delete_job)
                        .and_then(|store| {
                            let result =
                                store.delete_lifecycle_job(&workspace_delete_job, true, false)?;
                            if let Some(err) = result.cleanup_error {
                                error!(
                                    workspace = %result.workspace.name,
                                    error = %err,
                                    "workspace artifact cleanup failed after metadata delete"
                                );
                            }
                            Ok(result.workspace)
                        })
                        .map_err(|err| format!("{err:#}"))
                },
                move |result| {
                    match result {
                        Ok(deleted) => {
                            progress_done.set_text(&workspace_delete_feedback(Ok(deleted.clone())));
                            apply_workspace_delete_navigation_result(&state_done, &Ok(deleted));
                        }
                        Err(err) => {
                            let err = anyhow::anyhow!(err);
                            apply_runtime_action_feedback(
                                &progress_done,
                                &toast_done,
                                lifecycle_action_failure_feedback("Delete", &err),
                            );
                        }
                    }
                    refresh_done.refresh_event(RefreshEvent::WorkspaceInventoryChanged);
                },
            );
        });

        let actions = GBox::new(Orientation::Horizontal, 8);
        actions.set_halign(Align::Center);
        actions.append(&confirm);
        actions.append(&delete_btn);
        body.append(&actions);
        body.append(&progress);
    }
    shell.append(&body);

    shell
}

fn workspace_creation_status_allows_delete(status: &str) -> bool {
    status == "failed"
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

    // Horizontal split: center (flex) + right review/context panel.
    let main_split = Paned::new(Orientation::Horizontal);
    main_split.set_wide_handle(false);
    main_split.set_resize_start_child(true);
    main_split.set_resize_end_child(false);
    main_split.set_shrink_start_child(true);
    main_split.set_shrink_end_child(true);
    main_split.set_position(workspace_split_position_for_width(
        WORKSPACE_SPLIT_DEFAULT_CONTENT_WIDTH,
    ));
    install_workspace_split_ratio(&main_split);
    main_split.set_vexpand(true);
    let right_panel_handle = Rc::new(RefCell::new(None::<GBox>));
    let collapse_right_panel: Rc<dyn Fn()> = {
        let right_panel_handle = right_panel_handle.clone();
        Rc::new(move || {
            if let Some(panel) = right_panel_handle.borrow().as_ref() {
                panel.set_visible(!panel.is_visible());
            }
        })
    };

    // Center: custom tab bar + chat/terminal/file content
    let (center, open_file) = ws_center_panel(
        db_path,
        store,
        ws,
        state,
        refresh_hub.clone(),
        collapse_right_panel,
        ToastManager::new(&toast_overlay),
    );
    main_split.set_start_child(Some(&center));

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
    *right_panel_handle.borrow_mut() = Some(right.clone());
    main_split.set_end_child(Some(&right));

    shell.append(&main_split);
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

fn workspace_split_position_for_width(total_width: i32) -> i32 {
    split_position_for_ratio(
        total_width,
        5,
        3,
        WORKSPACE_SPLIT_MIN_START,
        WORKSPACE_SPLIT_MIN_END,
    )
}

fn install_workspace_split_ratio(split: &Paned) {
    // Keep the workspace center and detail pane at the product's fixed 5:3
    // ratio whenever the allocated width changes.
    let last_width = Rc::new(RefCell::new(0));
    split.add_tick_callback(move |paned, _| {
        let width = paned.allocated_width();
        if width > 0 && *last_width.borrow() != width {
            paned.set_position(workspace_split_position_for_width(width));
            *last_width.borrow_mut() = width;
        }
        gtk::glib::ControlFlow::Continue
    });
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

#[derive(Debug, Clone)]
struct WorkspaceRunConsoleTerminalConnection {
    database_path: PathBuf,
    workspace_name: String,
}

impl WorkspaceRunConsoleTerminalConnection {
    fn runtime(database_path: PathBuf, workspace_name: String) -> Self {
        Self {
            database_path,
            workspace_name,
        }
    }

    fn write(&mut self, input: &str) -> anyhow::Result<()> {
        let input = input.trim_end_matches(['\r', '\n']).trim();
        if input.is_empty() {
            return Ok(());
        }
        let Some(record) = latest_running_runtime_shell(&self.database_path, &self.workspace_name)
        else {
            return Err(anyhow::anyhow!(
                "no running runtime shell session; start shell first"
            ));
        };
        crate::archcar_async::spawn_archcar_request(
            archductor_core::paths::AppPaths::from_env(),
            ArchcarRequest::SendInput {
                session_id: record.id,
                input: input.to_owned(),
                visible_input: None,
                kind: ArchcarInputKind::ControlCommand,
                delivery: archductor_core::archcar::protocol::ArchcarInputDelivery::Auto,
            },
        );
        Ok(())
    }
}

fn runtime_record_is_shell(record: &ProcessRecord) -> bool {
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
                record.status == ProcessStatus::Running && runtime_record_is_shell(record)
            })
        })
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
    toast_manager: ToastManager,
) -> (GBox, WorkspaceTabSelector) {
    let panel = GBox::new(Orientation::Vertical, 0);
    panel.add_css_class("ws-center");
    panel.set_hexpand(true);
    panel.set_vexpand(true);

    let (session_header, branch_label) = session_surface::session_header_row_with_branch_label(
        &workspace_repository_name(store, &ws.name),
        &ws.branch,
        collapse_sidebar.clone(),
    );
    panel.append(&session_header);
    let workspace_phase_label = Label::new(None);
    workspace_phase_label.add_css_class("surface-note");
    workspace_phase_label.set_xalign(0.0);
    workspace_phase_label.set_wrap(true);
    workspace_phase_label.set_margin_start(12);
    workspace_phase_label.set_margin_end(12);
    workspace_phase_label.set_margin_bottom(6);
    apply_workspace_phase_status(&workspace_phase_label, state.workspace_phase(&ws.name));
    panel.append(&workspace_phase_label);
    {
        let workspace_name = ws.name.clone();
        let state_for_phase = state.clone();
        let label_for_phase = workspace_phase_label.clone();
        let workspace_phase_subscription = state.subscribe(move |event, _| {
            if let AppStateEvent::WorkspacePhaseChanged { workspace } = event {
                if workspace == &workspace_name {
                    apply_workspace_phase_status(
                        &label_for_phase,
                        state_for_phase.workspace_phase(&workspace_name),
                    );
                }
            }
        });
        workspace_phase_label.connect_destroy(move |_| {
            let _keep_subscription_alive = &workspace_phase_subscription;
        });
    }

    let tab_bar = GBox::new(Orientation::Horizontal, 8);
    tab_bar.add_css_class("ws-tab-bar");
    tab_bar.set_hexpand(true);
    let chat_tabs = GBox::new(Orientation::Horizontal, 6);
    chat_tabs.add_css_class("ws-chat-tabs");
    chat_tabs.set_hexpand(true);
    let chat_tabs_scroll = ScrolledWindow::new();
    chat_tabs_scroll.add_css_class("ws-chat-tabs-scroll");
    chat_tabs_scroll.set_policy(PolicyType::Automatic, PolicyType::Never);
    chat_tabs_scroll.set_hexpand(true);
    chat_tabs_scroll.set_propagate_natural_width(false);
    chat_tabs_scroll.set_child(Some(&chat_tabs));
    install_horizontal_wheel_scroll(&chat_tabs_scroll);
    let file_tabs = GBox::new(Orientation::Horizontal, 6);
    let reopen_tab_btn = text_button("Reopen");
    reopen_tab_btn.add_css_class("ws-tab-add-btn");
    reopen_tab_btn.set_visible(false);
    let reopen_popover = Popover::new();
    reopen_popover.add_css_class("context-menu-popover");
    reopen_popover.set_parent(&reopen_tab_btn);
    let reopen_menu = GBox::new(Orientation::Vertical, 4);
    reopen_menu.add_css_class("chat-menu-list");
    reopen_popover.set_child(Some(&reopen_menu));
    {
        let reopen_popover = reopen_popover.clone();
        reopen_tab_btn.connect_clicked(move |_| reopen_popover.popup());
    }
    tab_bar.append(&chat_tabs_scroll);
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
    let current_workspace_name = Rc::new(RefCell::new(ws.name.clone()));
    let selected_thread =
        Rc::new(RefCell::new(state.selected_chat_thread().or_else(|| {
            startup_chat_thread_selection(&known_threads.borrow())
        })));
    if state.selected_chat_thread().is_none() {
        state.set_selected_chat_thread(*selected_thread.borrow());
    }
    let external_thread_selection: session_surface::ExternalThreadSelectionController =
        Rc::new(RefCell::new(None));
    let closed_chat_tabs = Rc::new(RefCell::new(HashSet::<i64>::new()));
    let chat_tab_buttons = Rc::new(RefCell::new(HashMap::<i64, GBox>::new()));
    let file_tab_buttons = Rc::new(RefCell::new(HashMap::<String, GBox>::new()));
    let chat_nav_items_by_thread = Rc::new(RefCell::new(HashMap::<
        i64,
        crate::background_sync::WorkspaceChatNavItem,
    >::new()));
    let finished_unread_threads = Rc::new(RefCell::new(HashSet::<i64>::new()));
    let composer_dirty_threads = Rc::new(RefCell::new(HashSet::<i64>::new()));

    let setup_readiness = Rc::new(RefCell::new(SetupReadiness::from_host()));

    let add_tab_btn = text_button("+");
    add_tab_btn.add_css_class("ws-tab-add-btn");
    sync_workspace_chat_add_button_with_readiness(
        &add_tab_btn,
        &known_threads
            .borrow()
            .iter()
            .filter(|thread| workspace_chat_thread_is_visible(thread))
            .cloned()
            .collect::<Vec<_>>(),
        setup_readiness.as_ref(),
    );
    let refresh_sessions = refresh_hub.clone();
    let on_threads_changed: Rc<dyn Fn(Vec<ChatThreadRecord>, Option<i64>)> = {
        let chat_tabs = chat_tabs.clone();
        let known_threads = known_threads.clone();
        let selected_thread = selected_thread.clone();
        let closed_chat_tabs = closed_chat_tabs.clone();
        let chat_tab_buttons = chat_tab_buttons.clone();
        let file_tab_buttons = file_tab_buttons.clone();
        let reopen_tab_btn = reopen_tab_btn.clone();
        let reopen_menu = reopen_menu.clone();
        let reopen_popover = reopen_popover.clone();
        let add_tab_btn = add_tab_btn.clone();
        let setup_readiness = setup_readiness.clone();
        let db_path = db_path.to_path_buf();
        let current_workspace_name = current_workspace_name.clone();
        let archcar_paths = state.paths.clone();
        let state = state.clone();
        let external_thread_selection = external_thread_selection.clone();
        let content = content.clone();
        let chat_nav_items_by_thread = chat_nav_items_by_thread.clone();
        let finished_unread_threads = finished_unread_threads.clone();
        let composer_dirty_threads = composer_dirty_threads.clone();
        Rc::new(move |threads, selected| {
            *known_threads.borrow_mut() = threads.clone();
            if let Some(selected) = selected {
                closed_chat_tabs.borrow_mut().remove(&selected);
                finished_unread_threads.borrow_mut().remove(&selected);
            }
            let visible_threads = threads
                .iter()
                .filter(|thread| workspace_chat_thread_is_visible(thread))
                .filter(|thread| !closed_chat_tabs.borrow().contains(&thread.id))
                .take(WS_CHAT_TAB_LIMIT)
                .cloned()
                .collect::<Vec<_>>();
            sync_workspace_chat_add_button_with_readiness(
                &add_tab_btn,
                &visible_threads,
                setup_readiness.as_ref(),
            );
            let closed_threads = threads
                .iter()
                .filter(|thread| workspace_chat_thread_is_reopenable(thread))
                .cloned()
                .collect::<Vec<_>>();
            reopen_tab_btn.set_visible(!closed_threads.is_empty());
            while let Some(child) = reopen_menu.first_child() {
                reopen_menu.remove(&child);
            }
            for thread in closed_threads {
                let item = menu_text_button(&workspace_chat_tab_label(&thread));
                let db_path = db_path.clone();
                let current_workspace_name = current_workspace_name.clone();
                let known_threads = known_threads.clone();
                let selected_thread = selected_thread.clone();
                let closed_chat_tabs = closed_chat_tabs.clone();
                let state = state.clone();
                let external_thread_selection = external_thread_selection.clone();
                let content = content.clone();
                let reopen_popover = reopen_popover.clone();
                let thread_id = thread.id;
                item.connect_clicked(move |_| {
                    let Ok(store) = WorkspaceStore::open_app(db_path.clone()) else {
                        return;
                    };
                    if store.reopen_chat_thread(thread_id).is_err() {
                        return;
                    }
                    let workspace_name = current_workspace_name.borrow().clone();
                    let threads = store.list_chat_threads(&workspace_name).unwrap_or_default();
                    *known_threads.borrow_mut() = threads;
                    closed_chat_tabs.borrow_mut().remove(&thread_id);
                    *selected_thread.borrow_mut() = Some(thread_id);
                    state.set_selected_chat_thread(Some(thread_id));
                    content.set_visible_child_name("chat");
                    if let Some(select_thread) =
                        clone_external_thread_selection_controller(&external_thread_selection)
                    {
                        select_thread(Some(thread_id));
                    }
                    reopen_popover.popdown();
                });
                reopen_menu.append(&item);
            }
            let selected = selected
                .filter(|thread_id| visible_threads.iter().any(|thread| thread.id == *thread_id))
                .or_else(|| visible_threads.first().map(|thread| thread.id));
            *selected_thread.borrow_mut() = selected;
            state.set_selected_chat_thread(selected);
            let nav_items_by_thread = chat_nav_items_by_thread.borrow().clone();
            while let Some(child) = chat_tabs.first_child() {
                chat_tabs.remove(&child);
            }
            chat_tab_buttons.borrow_mut().clear();
            for thread in visible_threads {
                let tab_label = nav_items_by_thread
                    .get(&thread.id)
                    .map(workspace_chat_nav_label)
                    .unwrap_or_else(|| workspace_chat_tab_label(&thread));
                let (tab_shell, close_button) = ws_chat_tab_surface(&tab_label);
                let controller_for_click = external_thread_selection.clone();
                let content_for_click = content.clone();
                let selected_thread_for_click = selected_thread.clone();
                let closed_chat_tabs_for_click = closed_chat_tabs.clone();
                let chat_tab_buttons_for_click = chat_tab_buttons.clone();
                let file_tab_buttons_for_click = file_tab_buttons.clone();
                let chat_nav_items_for_click = chat_nav_items_by_thread.clone();
                let finished_unread_for_click = finished_unread_threads.clone();
                let composer_dirty_for_click = composer_dirty_threads.clone();
                let state_for_click = state.clone();
                let thread_id = thread.id;
                let select_tab: Rc<dyn Fn()> = Rc::new(move || {
                    closed_chat_tabs_for_click.borrow_mut().remove(&thread_id);
                    finished_unread_for_click.borrow_mut().remove(&thread_id);
                    *selected_thread_for_click.borrow_mut() = Some(thread_id);
                    state_for_click.set_selected_chat_thread(Some(thread_id));
                    content_for_click.set_visible_child_name("chat");
                    sync_workspace_chat_tabs(
                        chat_tab_buttons_for_click.as_ref(),
                        Some(thread_id),
                        chat_nav_items_for_click.as_ref(),
                        finished_unread_for_click.as_ref(),
                        composer_dirty_for_click.as_ref(),
                    );
                    sync_workspace_file_tabs(file_tab_buttons_for_click.as_ref(), None);
                    if let Some(select_thread) =
                        clone_external_thread_selection_controller(&controller_for_click)
                    {
                        select_thread(Some(thread_id));
                    }
                });
                let close_tab: Rc<dyn Fn()> = Rc::new({
                    let db_path = db_path.clone();
                    let current_workspace_name = current_workspace_name.clone();
                    let archcar_paths = archcar_paths.clone();
                    let chat_tabs = chat_tabs.clone();
                    let tab_shell = tab_shell.clone();
                    let closed_chat_tabs = closed_chat_tabs.clone();
                    let known_threads = known_threads.clone();
                    let selected_thread = selected_thread.clone();
                    let state = state.clone();
                    let external_thread_selection = external_thread_selection.clone();
                    let chat_tab_buttons = chat_tab_buttons.clone();
                    let file_tab_buttons = file_tab_buttons.clone();
                    let chat_nav_items_by_thread = chat_nav_items_by_thread.clone();
                    let finished_unread_threads = finished_unread_threads.clone();
                    let composer_dirty_threads = composer_dirty_threads.clone();
                    let content = content.clone();
                    let add_tab_btn = add_tab_btn.clone();
                    let setup_readiness = setup_readiness.clone();
                    move || {
                        let workspace_name = current_workspace_name.borrow().clone();
                        close_workspace_chat_thread(
                            &db_path,
                            &workspace_name,
                            thread_id,
                            archcar_paths.clone(),
                        );
                        closed_chat_tabs.borrow_mut().insert(thread_id);
                        finished_unread_threads.borrow_mut().remove(&thread_id);
                        composer_dirty_threads.borrow_mut().remove(&thread_id);
                        chat_tabs.remove(&tab_shell);
                        let visible_threads = known_threads
                            .borrow()
                            .iter()
                            .filter(|thread| workspace_chat_thread_is_visible(thread))
                            .filter(|thread| !closed_chat_tabs.borrow().contains(&thread.id))
                            .cloned()
                            .collect::<Vec<_>>();
                        sync_workspace_chat_add_button_with_readiness(
                            &add_tab_btn,
                            &visible_threads,
                            setup_readiness.as_ref(),
                        );
                        let next = known_threads
                            .borrow()
                            .iter()
                            .filter(|thread| workspace_chat_thread_is_visible(thread))
                            .find(|thread| !closed_chat_tabs.borrow().contains(&thread.id))
                            .map(|thread| thread.id);
                        *selected_thread.borrow_mut() = next;
                        state.set_selected_chat_thread(next);
                        content.set_visible_child_name("chat");
                        sync_workspace_chat_tabs(
                            chat_tab_buttons.as_ref(),
                            next,
                            chat_nav_items_by_thread.as_ref(),
                            finished_unread_threads.as_ref(),
                            composer_dirty_threads.as_ref(),
                        );
                        sync_workspace_file_tabs(file_tab_buttons.as_ref(), None);
                        if let Some(select_thread) =
                            clone_external_thread_selection_controller(&external_thread_selection)
                        {
                            select_thread(next);
                        }
                    }
                });
                connect_ws_tab_surface_clicks(&tab_shell, select_tab.clone());
                let close_tab_for_button = close_tab.clone();
                close_button.connect_clicked(move |_| close_tab_for_button());
                attach_context_menu(
                    &tab_shell,
                    vec![("Select", select_tab), ("Close tab", close_tab)],
                );
                chat_tab_buttons
                    .borrow_mut()
                    .insert(thread.id, tab_shell.clone());
                chat_tabs.append(&tab_shell);
            }
            sync_workspace_chat_tabs(
                chat_tab_buttons.as_ref(),
                selected,
                chat_nav_items_by_thread.as_ref(),
                finished_unread_threads.as_ref(),
                composer_dirty_threads.as_ref(),
            );
            chat_tabs.append(&reopen_tab_btn);
            chat_tabs.append(&add_tab_btn);
        })
    };
    let open_file_proxy_target: Rc<RefCell<Option<OpenWorkspaceFile>>> =
        Rc::new(RefCell::new(None));
    let open_file_proxy: OpenWorkspaceFile = Rc::new({
        let open_file_proxy_target = open_file_proxy_target.clone();
        move |rel_path: &str| {
            if let Some(open_file) = open_file_proxy_target.borrow().as_ref().cloned() {
                open_file(rel_path);
            }
        }
    });
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
        Some(setup_readiness.clone()),
        Some(session_surface::ExternalChatTabs {
            on_threads_changed: on_threads_changed.clone(),
            selection_controller: external_thread_selection.clone(),
            on_composer_draft_changed: {
                let chat_tab_buttons = chat_tab_buttons.clone();
                let selected_thread = selected_thread.clone();
                let chat_nav_items_by_thread = chat_nav_items_by_thread.clone();
                let finished_unread_threads = finished_unread_threads.clone();
                let composer_dirty_threads = composer_dirty_threads.clone();
                Rc::new(move |thread_id, dirty| {
                    let changed = if dirty {
                        composer_dirty_threads.borrow_mut().insert(thread_id)
                    } else {
                        composer_dirty_threads.borrow_mut().remove(&thread_id)
                    };
                    if changed {
                        sync_workspace_chat_tabs(
                            chat_tab_buttons.as_ref(),
                            *selected_thread.borrow(),
                            chat_nav_items_by_thread.as_ref(),
                            finished_unread_threads.as_ref(),
                            composer_dirty_threads.as_ref(),
                        );
                    }
                })
            },
            on_workspace_metadata_changed: {
                let current_workspace_name = current_workspace_name.clone();
                Rc::new(move |update| {
                    *current_workspace_name.borrow_mut() = update.workspace_name.clone();
                    branch_label.set_text(&update.branch_name);
                })
            },
            on_chat_surface_refresh_ready: {
                let state = state.clone();
                let refresh_hub = refresh_hub.clone();
                Some(Rc::new(move |refresh_chat_surface| {
                    let state = state.clone();
                    refresh_hub.set_workspace_chat_surface(move |event| {
                        let selected_workspace = state.selected_workspace();
                        let should_refresh = match event {
                            RefreshEvent::WorkspaceChatMessagesChanged {
                                workspace,
                                thread_id: _,
                            } => chat_message_event_matches_selected_workspace(
                                workspace,
                                selected_workspace.as_deref(),
                            ),
                            RefreshEvent::WorkspaceChatLifecycleChanged { workspace } => {
                                selected_workspace.as_deref() == Some(workspace.as_str())
                            }
                            _ => false,
                        };
                        if should_refresh {
                            refresh_chat_surface(session_surface::chat_refresh_kind_for_event(
                                event,
                            ));
                        }
                    });
                }))
            },
        }),
        Some(open_file_proxy),
        toast_manager.clone(),
    );
    content.add_named(&chat_widget, Some("chat"));
    {
        let db_path = db_path.to_path_buf();
        let current_workspace_name = current_workspace_name.clone();
        let selected_thread = selected_thread.clone();
        let on_threads_changed = on_threads_changed.clone();
        let chat_nav_items_by_thread = chat_nav_items_by_thread.clone();
        let finished_unread_threads = finished_unread_threads.clone();
        let chat_tab_snapshot_generation = Rc::new(Cell::new(0_u64));
        refresh_hub.set_workspace_chat_tabs(move |event| {
            let workspace_name = current_workspace_name.borrow().clone();
            match event {
                RefreshEvent::WorkspaceChatLifecycleChanged { workspace }
                | RefreshEvent::WorkspaceChatMessagesChanged { workspace, .. }
                    if workspace != &workspace_name =>
                {
                    return;
                }
                _ => {}
            }
            let selected = *selected_thread.borrow();
            let generation = chat_tab_snapshot_generation.get() + 1;
            chat_tab_snapshot_generation.set(generation);
            let db_path_for_job = db_path.clone();
            let workspace_name_for_job = workspace_name.clone();
            let current_workspace_name = current_workspace_name.clone();
            let on_threads_changed = on_threads_changed.clone();
            let chat_nav_items_by_thread = chat_nav_items_by_thread.clone();
            let finished_unread_threads = finished_unread_threads.clone();
            let chat_tab_snapshot_generation = chat_tab_snapshot_generation.clone();
            spawn_background_job(
                move || {
                    load_workspace_chat_tab_snapshot(
                        db_path_for_job,
                        workspace_name_for_job,
                        selected,
                    )
                },
                move |result| {
                    if chat_tab_snapshot_generation.get() != generation {
                        return;
                    }
                    let snapshot = match result {
                        Ok(snapshot) => snapshot,
                        Err(message) => {
                            error!(
                                workspace = %workspace_name,
                                error = %message,
                                "failed to load workspace chat tab snapshot"
                            );
                            return;
                        }
                    };
                    if current_workspace_name.borrow().as_str() != snapshot.workspace_name.as_str()
                    {
                        return;
                    }
                    reconcile_finished_workspace_chat_tabs(
                        chat_nav_items_by_thread.as_ref(),
                        finished_unread_threads.as_ref(),
                        &snapshot.nav_items_by_thread,
                        snapshot.selected,
                    );
                    *chat_nav_items_by_thread.borrow_mut() = snapshot.nav_items_by_thread;
                    (on_threads_changed)(snapshot.threads, snapshot.selected);
                },
            );
        });
    }
    {
        let db_path = db_path.to_path_buf();
        let current_workspace_name = current_workspace_name.clone();
        let known_threads = known_threads.clone();
        let selected_thread = selected_thread.clone();
        let state = state.clone();
        let external_thread_selection = external_thread_selection.clone();
        let on_threads_changed = on_threads_changed.clone();
        let content = content.clone();
        let setup_readiness = setup_readiness.clone();
        let add_tab_btn_for_feedback = add_tab_btn.clone();
        let closed_chat_tabs = closed_chat_tabs.clone();
        let toast_manager = toast_manager.clone();
        add_tab_btn.connect_clicked(move |_| {
            let workspace_name = current_workspace_name.borrow().clone();
            let existing = { known_threads.borrow().clone() };
            let visible_existing = existing
                .iter()
                .filter(|thread| workspace_chat_thread_is_visible(thread))
                .cloned()
                .collect::<Vec<_>>();
            if !workspace_chat_can_add_tab(&visible_existing) {
                return;
            }
            let active_thread = *selected_thread.borrow();
            *setup_readiness.borrow_mut() = SetupReadiness::from_host();
            sync_workspace_chat_add_button_with_readiness(
                &add_tab_btn_for_feedback,
                &visible_existing,
                setup_readiness.as_ref(),
            );
            let provider = ready_chat_provider_for_new_thread(
                &db_path,
                &workspace_name,
                active_thread,
                &visible_existing,
                setup_readiness.as_ref(),
            );
            let Some(provider) = provider else {
                error!(workspace = %workspace_name, "refusing to create chat without a ready launchable provider");
                return;
            };
            let title = workspace_chat_default_title(&visible_existing);
            let provider_kind = session_kind_for_chat_provider(&provider);
            let pending_target = state.create_pending_chat_target(workspace_name.clone(), provider_kind);
            state.mark_chat_phase(
                pending_target.clone(),
                ChatUiPhase::Creating {
                    provider: provider_kind,
                },
            );
            state.set_active_workspace_tab(WorkspaceTab::Chats);
            content.set_visible_child_name("chat");
            spawn_workspace_chat_thread_create(
                db_path.clone(),
                workspace_name,
                provider,
                provider_kind,
                title,
                pending_target,
                WorkspaceChatCreateUi {
                    state: state.clone(),
                    selected_thread: selected_thread.clone(),
                    known_threads: known_threads.clone(),
                    closed_chat_tabs: closed_chat_tabs.clone(),
                    external_thread_selection: external_thread_selection.clone(),
                    on_threads_changed: on_threads_changed.clone(),
                    toast_manager: toast_manager.clone(),
                },
            );
        });
    }
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

    // Open-file closure: opens the tab immediately and loads content off the GTK thread.
    let ws_path = ws.path.clone();
    let db_path_for_files = db_path.to_path_buf();
    let workspace_name_for_files = ws.name.clone();
    let content_ref = content.clone();
    let file_tabs_ref = file_tabs.clone();
    let chat_tab_buttons_for_files = chat_tab_buttons.clone();
    let file_tab_buttons_ref = file_tab_buttons.clone();
    let selected_thread_for_files = selected_thread.clone();
    let external_thread_selection_for_files = external_thread_selection.clone();
    let state_for_files = state.clone();
    let chat_nav_items_for_files = chat_nav_items_by_thread.clone();
    let finished_unread_for_files = finished_unread_threads.clone();
    let composer_dirty_for_files = composer_dirty_threads.clone();

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

            let edit_view = TextView::new();
            edit_view.set_monospace(true);
            edit_view.set_vexpand(true);
            edit_view.buffer().set_text("Loading file...");
            let edit_scroll = ScrolledWindow::new();
            edit_scroll.set_vexpand(true);
            edit_scroll.set_child(Some(&edit_view));
            mode_tabs.add_titled(&edit_scroll, Some("edit"), "Edit");

            let diff_view = TextView::new();
            diff_view.set_editable(false);
            diff_view.set_monospace(true);
            diff_view.set_vexpand(true);
            diff_view.buffer().set_text("Loading diff...");
            let diff_scroll = ScrolledWindow::new();
            diff_scroll.set_vexpand(true);
            diff_scroll.set_child(Some(&diff_view));
            mode_tabs.add_titled(&diff_scroll, Some("diff"), "Diff");

            let preview_view = TextView::new();
            preview_view.set_editable(false);
            preview_view.set_wrap_mode(WrapMode::WordChar);
            preview_view.set_vexpand(true);
            preview_view.buffer().set_text("Loading file...");
            let preview_scroll = ScrolledWindow::new();
            preview_scroll.set_vexpand(true);
            preview_scroll.set_child(Some(&preview_view));
            mode_tabs.add_titled(&preview_scroll, Some("preview"), "Preview");

            mode_tabs.set_visible_child_name("edit");
            file_pane.append(&mode_tabs);
            content_ref.add_named(&file_pane, Some(&tab_key));

            let edit_buffer = edit_view.buffer();
            let diff_buffer = diff_view.buffer();
            let preview_buffer = preview_view.buffer();
            let mode_tabs_apply = mode_tabs.clone();
            let ws_path = ws_path.clone();
            let db_path = db_path_for_files.clone();
            let workspace_name = workspace_name_for_files.clone();
            let rel_path_for_job = rel_path.to_owned();
            spawn_background_job(
                move || {
                    load_workspace_file_snapshot(ws_path, db_path, workspace_name, rel_path_for_job)
                },
                move |snapshot| {
                    edit_buffer.set_text(&snapshot.contents);
                    diff_buffer.set_text(&snapshot.diff_text);
                    preview_buffer.set_text(&snapshot.contents);
                    if snapshot.relative_path.ends_with(".md") {
                        mode_tabs_apply.set_visible_child_name("preview");
                    }
                },
            );

            // Tab button for this file
            let short_name = std::path::Path::new(rel_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(rel_path);
            let (tab_shell, close_button) = ws_tab_surface(short_name);
            let select_file_tab: Rc<dyn Fn()> = Rc::new({
                let content_ref = content_ref.clone();
                let tab_key = tab_key.clone();
                let chat_tab_buttons = chat_tab_buttons_for_files.clone();
                let file_tab_buttons = file_tab_buttons_ref.clone();
                let chat_nav_items = chat_nav_items_for_files.clone();
                let finished_unread_threads = finished_unread_for_files.clone();
                let composer_dirty_threads = composer_dirty_for_files.clone();
                move || {
                    content_ref.set_visible_child_name(&tab_key);
                    sync_workspace_chat_tabs(
                        chat_tab_buttons.as_ref(),
                        None,
                        chat_nav_items.as_ref(),
                        finished_unread_threads.as_ref(),
                        composer_dirty_threads.as_ref(),
                    );
                    sync_workspace_file_tabs(file_tab_buttons.as_ref(), Some(&tab_key));
                }
            });
            let close_file_tab: Rc<dyn Fn()> = Rc::new({
                let content_ref = content_ref.clone();
                let file_tabs_ref = file_tabs_ref.clone();
                let tab_shell = tab_shell.clone();
                let tab_key = tab_key.clone();
                let file_tab_buttons = file_tab_buttons_ref.clone();
                let chat_tab_buttons = chat_tab_buttons_for_files.clone();
                let selected_thread = selected_thread_for_files.clone();
                let external_thread_selection = external_thread_selection_for_files.clone();
                let state = state_for_files.clone();
                let chat_nav_items = chat_nav_items_for_files.clone();
                let finished_unread_threads = finished_unread_for_files.clone();
                let composer_dirty_threads = composer_dirty_for_files.clone();
                move || {
                    if let Some(child) = content_ref.child_by_name(&tab_key) {
                        content_ref.remove(&child);
                    }
                    file_tabs_ref.remove(&tab_shell);
                    file_tab_buttons.borrow_mut().remove(&tab_key);
                    content_ref.set_visible_child_name("chat");
                    let selected = *selected_thread.borrow();
                    state.set_selected_chat_thread(selected);
                    sync_workspace_chat_tabs(
                        chat_tab_buttons.as_ref(),
                        selected,
                        chat_nav_items.as_ref(),
                        finished_unread_threads.as_ref(),
                        composer_dirty_threads.as_ref(),
                    );
                    sync_workspace_file_tabs(file_tab_buttons.as_ref(), None);
                    if let Some(select_thread) =
                        clone_external_thread_selection_controller(&external_thread_selection)
                    {
                        select_thread(selected);
                    }
                }
            });
            connect_ws_tab_surface_clicks(&tab_shell, select_file_tab.clone());
            let close_file_tab_for_button = close_file_tab.clone();
            close_button.connect_clicked(move |_| close_file_tab_for_button());
            attach_context_menu(
                &tab_shell,
                vec![("Select", select_file_tab), ("Close tab", close_file_tab)],
            );
            file_tab_buttons_ref
                .borrow_mut()
                .insert(tab_key.clone(), tab_shell.clone());
            file_tabs_ref.append(&tab_shell);
        }
        content_ref.set_visible_child_name(&tab_key);
        sync_workspace_chat_tabs(
            chat_tab_buttons_for_files.as_ref(),
            None,
            chat_nav_items_for_files.as_ref(),
            finished_unread_for_files.as_ref(),
            composer_dirty_for_files.as_ref(),
        );
        sync_workspace_file_tabs(file_tab_buttons_ref.as_ref(), Some(&tab_key));
    });
    *open_file_proxy_target.borrow_mut() = Some(open_file.clone());

    let initial_threads = known_threads.borrow().clone();
    let initial_selected_thread = *selected_thread.borrow();
    (on_threads_changed)(initial_threads, initial_selected_thread);

    (panel, open_file)
}

fn ws_tab_surface(label: &str) -> (GBox, Button) {
    crate::tabs::closable_tab_surface(label)
}

fn ws_chat_tab_surface(label: &str) -> (GBox, Button) {
    let shell = GBox::new(Orientation::Horizontal, 6);
    shell.add_css_class("ws-tab-shell");
    shell.set_valign(Align::Center);

    let indicator = Stack::new();
    indicator.add_css_class("ws-chat-tab-indicator");
    let empty = GBox::new(Orientation::Horizontal, 0);
    indicator.add_named(&empty, Some("empty"));
    let spinner = Spinner::new();
    spinner.add_css_class("ws-chat-tab-spinner");
    spinner.start();
    indicator.add_named(&spinner, Some("spinner"));
    let dot = Label::new(Some("•"));
    dot.add_css_class("ws-chat-tab-dot");
    indicator.add_named(&dot, Some("dot"));
    indicator.set_visible_child_name("empty");
    shell.append(&indicator);

    let label = Label::new(Some(label));
    label.add_css_class("ws-tab-label");
    label.set_valign(Align::Center);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    shell.append(&label);

    let close = Button::new();
    close.add_css_class("ws-tab-close-button");
    close.set_valign(Align::Center);
    close.set_tooltip_text(Some("Close tab"));
    let close_icon =
        Image::from_icon_name(crate::buttons::resolve_icon_name("window-close-symbolic"));
    close_icon.add_css_class("ws-tab-close-icon");
    close_icon.set_valign(Align::Center);
    close.set_child(Some(&close_icon));
    shell.append(&close);

    (shell, close)
}

fn install_horizontal_wheel_scroll(scroll: &ScrolledWindow) {
    let controller = EventControllerScroll::new(
        EventControllerScrollFlags::VERTICAL | EventControllerScrollFlags::HORIZONTAL,
    );
    let scroll_for_event = scroll.clone();
    controller.connect_scroll(move |_, dx, dy| {
        let adjustment = scroll_for_event.hadjustment();
        let delta = if dx.abs() > dy.abs() { dx } else { dy };
        if delta.abs() <= f64::EPSILON {
            return gtk::glib::Propagation::Proceed;
        }

        let upper = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
        let next = (adjustment.value() + delta * 48.0).clamp(adjustment.lower(), upper);
        adjustment.set_value(next);
        gtk::glib::Propagation::Stop
    });
    scroll.add_controller(controller);
}

fn open_external_url(url: &str) {
    if url.trim().is_empty() {
        return;
    }
    let _ = gtk::gio::AppInfo::launch_default_for_uri(url, None::<&gtk::gio::AppLaunchContext>);
}

fn connect_ws_tab_surface_clicks(tab: &GBox, select: Rc<dyn Fn()>) {
    let click = GestureClick::new();
    click.set_button(1);
    click.connect_released(move |_, _, _, _| {
        select();
    });
    tab.add_controller(click);
}

fn sync_workspace_chat_tabs(
    buttons: &RefCell<HashMap<i64, GBox>>,
    selected: Option<i64>,
    nav_items_by_thread: &RefCell<HashMap<i64, crate::background_sync::WorkspaceChatNavItem>>,
    finished_unread_threads: &RefCell<HashSet<i64>>,
    composer_dirty_threads: &RefCell<HashSet<i64>>,
) {
    let buttons = buttons.borrow();
    let nav_items_by_thread = nav_items_by_thread.borrow();
    let finished_unread_threads = finished_unread_threads.borrow();
    let composer_dirty_threads = composer_dirty_threads.borrow();
    for (thread_id, button) in buttons.iter() {
        let state = reduce_workspace_chat_tab_state(WorkspaceChatTabStateInput {
            selected: Some(*thread_id) == selected,
            generating: nav_items_by_thread
                .get(thread_id)
                .is_some_and(|item| item.running),
            finished_unread: finished_unread_threads.contains(thread_id),
            composer_dirty: composer_dirty_threads.contains(thread_id),
        });
        apply_workspace_chat_tab_visual_state(button, state);
    }
}

fn apply_workspace_chat_tab_visual_state(tab: &GBox, state: WorkspaceChatTabVisualState) {
    tab.remove_css_class("ws-tab-active");
    tab.remove_css_class("ws-tab-running");
    tab.remove_css_class("ws-tab-unread");
    tab.remove_css_class("ws-tab-editing");

    let indicator = workspace_chat_tab_indicator_widget(tab);
    let indicator_child = match state {
        WorkspaceChatTabVisualState::Generating => {
            tab.add_css_class("ws-tab-running");
            "spinner"
        }
        WorkspaceChatTabVisualState::FinishedGenerating => {
            tab.add_css_class("ws-tab-unread");
            "dot"
        }
        WorkspaceChatTabVisualState::Read => "empty",
        WorkspaceChatTabVisualState::Selected => {
            tab.add_css_class("ws-tab-active");
            "empty"
        }
        WorkspaceChatTabVisualState::SelectedGenerating => {
            tab.add_css_class("ws-tab-active");
            tab.add_css_class("ws-tab-running");
            "spinner"
        }
        WorkspaceChatTabVisualState::Editing => {
            tab.add_css_class("ws-tab-editing");
            "empty"
        }
    };
    if let Some(indicator) = indicator {
        indicator.set_visible_child_name(indicator_child);
    }
}

fn workspace_chat_tab_indicator_widget(tab: &GBox) -> Option<Stack> {
    let mut child = tab.first_child();
    while let Some(widget) = child {
        if widget.has_css_class("ws-chat-tab-indicator") {
            return widget.downcast::<Stack>().ok();
        }
        child = widget.next_sibling();
    }
    None
}

fn reconcile_finished_workspace_chat_tabs(
    previous_nav_items_by_thread: &RefCell<
        HashMap<i64, crate::background_sync::WorkspaceChatNavItem>,
    >,
    finished_unread_threads: &RefCell<HashSet<i64>>,
    next_nav_items_by_thread: &HashMap<i64, crate::background_sync::WorkspaceChatNavItem>,
    selected: Option<i64>,
) {
    let previous_nav_items_by_thread = previous_nav_items_by_thread.borrow();
    let mut finished_unread_threads = finished_unread_threads.borrow_mut();
    for (thread_id, next) in next_nav_items_by_thread {
        let was_running = previous_nav_items_by_thread
            .get(thread_id)
            .is_some_and(|previous| previous.running);
        if Some(*thread_id) == selected || next.running {
            finished_unread_threads.remove(thread_id);
        } else if was_running {
            finished_unread_threads.insert(*thread_id);
        }
    }
}

fn sync_workspace_file_tabs(buttons: &RefCell<HashMap<String, GBox>>, selected: Option<&str>) {
    for (tab_key, button) in buttons.borrow().iter() {
        if Some(tab_key.as_str()) == selected {
            button.add_css_class("ws-tab-active");
        } else {
            button.remove_css_class("ws-tab-active");
        }
    }
}

fn attach_context_menu<W: IsA<gtk::Widget>>(anchor: &W, items: Vec<ContextMenuItem>) {
    let popover = Popover::new();
    popover.add_css_class("context-menu-popover");
    popover.set_parent(anchor);
    let menu = GBox::new(Orientation::Vertical, 4);
    menu.add_css_class("chat-menu-list");
    for (label, action) in items {
        let item = menu_text_button(label);
        let popover_for_item = popover.clone();
        item.connect_clicked(move |_| {
            action();
            popover_for_item.popdown();
        });
        menu.append(&item);
    }
    popover.set_child(Some(&menu));

    let gesture = GestureClick::new();
    gesture.set_button(3);
    let popover_for_click = popover.clone();
    gesture.connect_pressed(move |_, _, x, y| {
        let rect = gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1);
        popover_for_click.set_pointing_to(Some(&rect));
        popover_for_click.popup();
    });
    anchor.add_controller(gesture);
}

fn workspace_chat_default_title(threads: &[ChatThreadRecord]) -> String {
    let next = threads
        .iter()
        .filter(|thread| workspace_chat_thread_is_visible(thread))
        .count()
        + 1;
    if next == 1 {
        "New Chat".to_owned()
    } else {
        format!("New Chat {next}")
    }
}

fn workspace_chat_can_add_tab(visible_threads: &[ChatThreadRecord]) -> bool {
    visible_threads.len() < WS_CHAT_TAB_LIMIT
}

fn sync_workspace_chat_add_button_with_readiness(
    button: &Button,
    visible_threads: &[ChatThreadRecord],
    readiness: &RefCell<SetupReadiness>,
) {
    let readiness = readiness.borrow();
    sync_workspace_chat_add_button(button, visible_threads, &readiness);
}

fn sync_workspace_chat_add_button(
    button: &Button,
    visible_threads: &[ChatThreadRecord],
    readiness: &SetupReadiness,
) {
    let can_add = workspace_chat_can_add_tab(visible_threads);
    button.set_sensitive(can_add);
    if !can_add {
        button.set_label("+");
        button.set_tooltip_text(Some("Chat limit reached"));
    } else if readiness.first_ready_launchable_provider().is_some() {
        button.set_label("+");
        button.set_tooltip_text(Some("Add chat"));
    } else {
        button.set_label("Setup required");
        button.set_tooltip_text(Some(
            "Install and sign in to Codex or Claude before adding a chat.",
        ));
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

fn workspace_chat_nav_label(item: &crate::background_sync::WorkspaceChatNavItem) -> String {
    let title = item.title.trim();
    if title.is_empty() {
        "New Chat".to_owned()
    } else {
        title.to_owned()
    }
}

fn workspace_chat_thread_is_visible(thread: &ChatThreadRecord) -> bool {
    workspace_chat_thread_is_supported(thread) && thread.status == "active"
}

fn startup_chat_thread_selection(threads: &[ChatThreadRecord]) -> Option<i64> {
    threads
        .iter()
        .find(|thread| workspace_chat_thread_is_visible(thread))
        .map(|thread| thread.id)
}

fn workspace_chat_thread_is_reopenable(thread: &ChatThreadRecord) -> bool {
    workspace_chat_thread_is_supported(thread) && thread.status == "closed"
}

fn workspace_chat_thread_is_supported(thread: &ChatThreadRecord) -> bool {
    launchable_provider_key(&thread.provider).is_some()
}

fn ready_chat_provider_for_new_thread(
    db_path: &Path,
    workspace_name: &str,
    active_thread: Option<i64>,
    visible_existing: &[ChatThreadRecord],
    readiness: &RefCell<SetupReadiness>,
) -> Option<String> {
    let readiness = readiness.borrow();
    visible_existing
        .iter()
        .find(|thread| Some(thread.id) == active_thread)
        .filter(|thread| provider_is_ready_launchable(&thread.provider, &readiness))
        .map(|thread| thread.provider.clone())
        .or_else(|| {
            default_launchable_chat_provider_for_workspace(db_path, workspace_name, &readiness)
        })
}

fn session_kind_for_chat_provider(provider: &str) -> SessionKind {
    match provider {
        "claude" => SessionKind::Claude,
        "shell" => SessionKind::Shell,
        _ => SessionKind::Codex,
    }
}

struct WorkspaceChatCreateUi {
    state: AppState,
    selected_thread: Rc<RefCell<Option<i64>>>,
    known_threads: Rc<RefCell<Vec<ChatThreadRecord>>>,
    closed_chat_tabs: Rc<RefCell<HashSet<i64>>>,
    external_thread_selection: session_surface::ExternalThreadSelectionController,
    on_threads_changed: Rc<dyn Fn(Vec<ChatThreadRecord>, Option<i64>)>,
    toast_manager: ToastManager,
}

fn spawn_workspace_chat_thread_create(
    db_path: PathBuf,
    workspace_name: String,
    provider: String,
    provider_kind: SessionKind,
    title: String,
    pending_target: ChatUiTarget,
    ui: WorkspaceChatCreateUi,
) {
    let workspace_for_job = workspace_name.clone();
    spawn_background_job(
        move || {
            WorkspaceStore::open_app(db_path)
                .and_then(|store| {
                    let thread =
                        store.create_chat_thread(&workspace_for_job, &provider, &title, None)?;
                    let mut threads = store
                        .list_chat_threads(&workspace_for_job)
                        .unwrap_or_default();
                    if !threads.iter().any(|item| item.id == thread.id) {
                        threads.insert(0, thread.clone());
                    }
                    Ok((thread, threads))
                })
                .map_err(|err| format!("{err:#}"))
        },
        move |result| match result {
            Ok((thread, threads)) => {
                ui.state
                    .resolve_pending_chat_target(pending_target.clone(), thread.id);
                if ui.state.queued_chat_inputs_count(thread.id) > 0 {
                    ui.state.mark_chat_phase(
                        ChatUiTarget::Thread(thread.id),
                        ChatUiPhase::StartingAgent {
                            provider: provider_kind,
                        },
                    );
                }
                *ui.selected_thread.borrow_mut() = Some(thread.id);
                *ui.known_threads.borrow_mut() = threads.clone();
                ui.closed_chat_tabs.borrow_mut().remove(&thread.id);
                (ui.on_threads_changed)(threads, Some(thread.id));
                if let Some(select_thread) =
                    clone_external_thread_selection_controller(&ui.external_thread_selection)
                {
                    select_thread(Some(thread.id));
                }
                ui.state
                    .request_refresh(RefreshEvent::WorkspaceChatLifecycleChanged {
                        workspace: workspace_name,
                    });
            }
            Err(message) => {
                let failure = format!("Create chat thread failed: {message}");
                ui.state.mark_chat_phase(
                    pending_target,
                    ChatUiPhase::Failed {
                        message: failure.clone(),
                    },
                );
                ui.toast_manager.error(failure.clone());
                error!(workspace = %workspace_name, error = %failure, "failed to create workspace chat thread");
            }
        },
    );
}

fn default_launchable_chat_provider_for_workspace(
    db_path: &Path,
    workspace_name: &str,
    readiness: &SetupReadiness,
) -> Option<String> {
    let provider = readiness.first_ready_launchable_provider()?.to_owned();
    persist_default_chat_provider(db_path, workspace_name, &provider);
    Some(provider)
}

fn provider_is_ready_launchable(provider: &str, readiness: &SetupReadiness) -> bool {
    readiness.launchable_provider_ready(provider)
}

fn persist_default_chat_provider(db_path: &Path, workspace_name: &str, provider: &str) {
    let result = WorkspaceStore::save_local_default_agent_provider_for_database(
        db_path,
        workspace_name,
        provider,
    );
    if let Err(err) = result {
        error!("failed to persist chat provider {provider} for {workspace_name}: {err:#}");
    }
}

fn close_workspace_chat_thread(
    db_path: &Path,
    workspace_name: &str,
    thread_id: i64,
    archcar_paths: AppPaths,
) {
    let Ok(store) = WorkspaceStore::open_app(db_path) else {
        return;
    };
    let records = store.list_thread_processes(thread_id).unwrap_or_default();
    let _ = store.close_chat_thread(thread_id);
    for record in records
        .into_iter()
        .filter(|record| record.status == ProcessStatus::Running)
    {
        spawn_archcar_request(
            archcar_paths.clone(),
            ArchcarRequest::KillSession {
                session_id: record.id,
            },
        );
        let _ = store.stop_session_process(workspace_name, record.id);
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

// ── Left/right workspace panels ─────────────────────────────────

fn ws_right_panel(
    db_path: &Path,
    store: &WorkspaceStore,
    ws: &Workspace,
    state: &AppState,
    run_console_states: RunConsoleStateStore,
    run_console_terminals: RunConsoleTerminalStore,
    refresh_hub: RefreshHub,
    toast_overlay: ToastOverlay,
    open_file: OpenWorkspaceFile,
    collapse_sidebar: Rc<dyn Fn()>,
) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 0);
    panel.add_css_class("ws-right-panel");
    panel.set_vexpand(true);
    panel.set_hexpand(false);
    panel.set_width_request(WORKSPACE_RIGHT_PANEL_DEFAULT_WIDTH);
    panel.set_overflow(gtk::Overflow::Hidden);

    panel.append(&workspace_pr_status_panel(
        db_path,
        store,
        &ws.name,
        state,
        refresh_hub.clone(),
        &toast_overlay,
    ));
    panel.append(&Separator::new(Orientation::Horizontal));

    let tab_strip = GBox::new(Orientation::Horizontal, 4);
    tab_strip.add_css_class("command-center-strip");
    tab_strip.set_spacing(6);
    let content = Stack::new();
    content.set_vexpand(true);

    let all_btn = text_button("Browse");
    all_btn.add_css_class("nav-button");
    let files_widget = ws_simple_file_list(db_path, ws, open_file.clone());
    content.add_named(&files_widget, Some("files"));

    let changes_btn = text_button("Changes");
    changes_btn.add_css_class("nav-button");
    let changes_widget = workspace_changes_panel(
        db_path,
        store,
        &ws.name,
        state.selected_chat_thread(),
        Some(open_file.clone()),
        refresh_hub.clone(),
        toast_overlay.clone(),
    );
    content.add_named(&changes_widget, Some("changes"));
    {
        let c = content.clone();
        let all_btn_for_click = all_btn.clone();
        let changes_btn_for_click = changes_btn.clone();
        let state = state.clone();
        changes_btn.connect_clicked(move |_| {
            c.set_visible_child_name("changes");
            state.set_active_workspace_right_panel_tab(WorkspaceRightPanelTab::Changes);
            changes_btn_for_click.add_css_class("nav-button-active");
            all_btn_for_click.remove_css_class("nav-button-active");
        });
    }
    {
        let c = content.clone();
        let all_btn_for_click = all_btn.clone();
        let changes_btn_for_click = changes_btn.clone();
        let state = state.clone();
        all_btn.connect_clicked(move |_| {
            c.set_visible_child_name("files");
            state.set_active_workspace_right_panel_tab(WorkspaceRightPanelTab::Browse);
            all_btn_for_click.add_css_class("nav-button-active");
            changes_btn_for_click.remove_css_class("nav-button-active");
        });
    }
    tab_strip.append(&all_btn);
    tab_strip.append(&changes_btn);

    match state.active_workspace_right_panel_tab() {
        WorkspaceRightPanelTab::Browse => {
            content.set_visible_child_name("files");
            all_btn.add_css_class("nav-button-active");
            changes_btn.remove_css_class("nav-button-active");
        }
        WorkspaceRightPanelTab::Changes => {
            content.set_visible_child_name("changes");
            changes_btn.add_css_class("nav-button-active");
            all_btn.remove_css_class("nav-button-active");
        }
    }
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

    let collapsed_dirs: Rc<RefCell<HashSet<String>>> =
        Rc::new(RefCell::new(initial_collapsed_dirs(&dir_children)));

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

fn initial_collapsed_dirs(dir_children: &BTreeMap<String, BTreeSet<String>>) -> HashSet<String> {
    let mut collapsed = HashSet::new();
    for (parent, children) in dir_children {
        for child in children {
            collapsed.insert(tree_join_path(parent, child));
        }
    }
    collapsed
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

fn toggle_file_tree_directory(
    rows: &Rc<RefCell<Vec<FileTreeRow>>>,
    collapsed_dirs: &Rc<RefCell<HashSet<String>>>,
    dir_path: &str,
    toggle: &Button,
    icon: &Image,
) {
    let collapsed_now = {
        let mut collapsed = collapsed_dirs.borrow_mut();
        if !collapsed.remove(dir_path) {
            collapsed.insert(dir_path.to_owned());
            true
        } else {
            false
        }
    };
    toggle.set_label(if collapsed_now { "▸" } else { "▾" });
    icon.set_icon_name(Some(if collapsed_now {
        "folder-symbolic"
    } else {
        "folder-open-symbolic"
    }));
    update_tree_visibility(rows, collapsed_dirs);
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
            let row_box = GBox::new(Orientation::Horizontal, file_tree_row_spacing());
            row_box.add_css_class("ws-dir-row");
            row_box.set_margin_start(file_tree_indent_margin_start(depth));
            let toggle = Button::with_label("▸");
            toggle.add_css_class("ws-folder-toggle");
            toggle.add_css_class("flat");
            toggle.set_tooltip_text(Some("Expand or collapse directory"));
            let icon = Image::from_icon_name(file_tree_icon_name(true, &dir_path));
            icon.add_css_class("ws-folder-icon");
            let name_lbl = Label::new(Some(child));
            name_lbl.add_css_class("ws-folder-name");
            name_lbl.set_xalign(0.0);
            name_lbl.set_hexpand(true);
            row_box.append(&toggle);
            row_box.append(&icon);
            row_box.append(&name_lbl);
            let row = ListBoxRow::builder().child(&row_box).build();
            row.set_selectable(true);
            row.set_activatable(true);
            row.set_focusable(true);
            list.append(&row);
            rows.borrow_mut().push(FileTreeRow {
                row: row.clone(),
                path: dir_path.clone(),
                is_dir: true,
            });

            let rows_for_toggle = rows.clone();
            let collapsed_for_toggle = collapsed_dirs.clone();
            let dir_path_for_toggle = dir_path.clone();
            let icon_for_toggle = icon.clone();
            toggle.connect_clicked(move |button| {
                toggle_file_tree_directory(
                    &rows_for_toggle,
                    &collapsed_for_toggle,
                    &dir_path_for_toggle,
                    button,
                    &icon_for_toggle,
                );
            });
            let rows_for_row_activate = rows.clone();
            let collapsed_for_row_activate = collapsed_dirs.clone();
            let dir_path_for_row_activate = dir_path.clone();
            let toggle_for_row_activate = toggle.clone();
            let icon_for_row_activate = icon.clone();
            row.connect_activate(move |_| {
                toggle_file_tree_directory(
                    &rows_for_row_activate,
                    &collapsed_for_row_activate,
                    &dir_path_for_row_activate,
                    &toggle_for_row_activate,
                    &icon_for_row_activate,
                );
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
            let row_box = GBox::new(Orientation::Horizontal, file_tree_row_spacing());
            row_box.add_css_class("ws-file-row");
            row_box.set_margin_start(file_tree_indent_margin_start(depth));

            let icon = Image::from_icon_name(file_tree_icon_name(false, &file_path));
            icon.add_css_class("ws-file-icon");
            row_box.append(&icon);

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

fn file_tree_icon_name(is_dir: bool, path: &str) -> &'static str {
    if is_dir {
        return "folder-symbolic";
    }
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match ext {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" => "image-x-generic-symbolic",
        "sh" | "bash" | "zsh" | "fish" | "toml" | "yaml" | "yml" | "json" => {
            "text-x-script-symbolic"
        }
        _ => "text-x-generic-symbolic",
    }
}

fn file_tree_indent_margin_start(depth: usize) -> i32 {
    (depth as i32) * 12
}

fn file_tree_row_spacing() -> i32 {
    4
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
    let stage_btn = text_button(workspace_prompt_queue_button_label(title));
    stage_btn.add_css_class("suggested-action");
    let prompt_text = prompt.to_owned();
    stage_btn.connect_clicked(move |_| {
        queue_workspace_prompt_draft(&app_state, &prompt_text);
    });
    modal.append(&modal_title_label);
    modal.append(&stage_btn);
    prompt_overlay.add_overlay(&modal);
    panel.append(&prompt_overlay);

    panel
}

fn workspace_prompt_queue_button_label(title: &str) -> &'static str {
    match title {
        "Setup Prompt" => "Queue Bootstrap Draft",
        "Run Prompt" => "Queue Launch Draft",
        _ => "Queue Prompt",
    }
}

fn queue_workspace_prompt_draft(app_state: &AppState, prompt: &str) {
    let prompt = prompt.trim();
    if !prompt.is_empty() {
        app_state.queue_pending_chat_prompt(prompt.to_owned());
    }
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
        let write_result = connection.write(&(command + "\n"));
        let updated_transcript =
            if let Some(state) = state_for_run.borrow_mut().get_mut(&workspace_for_run) {
                if let Some(terminal) = state.terminal_by_name_mut(&tab_for_run) {
                    match write_result {
                        Ok(()) => terminal.append_result("[sent to runtime shell]\n"),
                        Err(err) => {
                            terminal.append_result(&format!("[terminal send paused]\n{err:#}\n"))
                        }
                    }
                    terminal.display_text().to_owned()
                } else {
                    transcript
                }
            } else {
                transcript
            };
        buffer_for_run.set_text(&updated_transcript);
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

    let Ok(connection) = spawn_workspace_terminal_session(db_path, workspace_name) else {
        return;
    };
    run_console_terminals.borrow_mut().insert(key, connection);
    if let Some(state) = run_console_states.borrow_mut().get_mut(workspace_name) {
        if let Some(terminal) = state.terminal_by_name_mut(tab_name) {
            if terminal.transcript.is_empty() {
                terminal
                    .transcript
                    .push_str("[terminal start requested through archcar]\n");
            }
        }
    }
}

fn spawn_workspace_terminal_session(
    db_path: &Path,
    workspace_name: &str,
) -> anyhow::Result<WorkspaceRunConsoleTerminalConnection> {
    let store = WorkspaceStore::open_app(db_path)?;
    let _ = store.session_launch(workspace_name, SessionKind::Shell)?;
    crate::archcar_async::spawn_archcar_request(
        archductor_core::paths::AppPaths::from_env(),
        ArchcarRequest::SpawnSession {
            workspace: workspace_name.to_owned(),
            kind: SessionKind::Shell,
            harness: None,
        },
    );
    Ok(WorkspaceRunConsoleTerminalConnection::runtime(
        db_path.to_path_buf(),
        workspace_name.to_owned(),
    ))
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
                edit_buffer.set_text("Loading file...");
                preview_buffer.set_text("Loading file...");
                diff_buffer.set_text("Loading diff...");
                let selected_file_apply = selected_file_open.clone();
                let edit_buffer_apply = edit_buffer.clone();
                let preview_buffer_apply = preview_buffer.clone();
                let diff_buffer_apply = diff_buffer.clone();
                let mode_stack_apply = mode_stack_open.clone();
                let workspace_path = workspace_path.clone();
                let db_path_open = db_path_open.clone();
                let workspace_name = workspace_name.clone();
                let relative_path = relative_path.clone();
                spawn_background_job(
                    move || {
                        load_workspace_file_snapshot(
                            workspace_path,
                            db_path_open,
                            workspace_name,
                            relative_path,
                        )
                    },
                    move |snapshot| {
                        if selected_file_apply.borrow().as_deref()
                            != Some(snapshot.relative_path.as_str())
                        {
                            return;
                        }
                        edit_buffer_apply.set_text(&snapshot.contents);
                        preview_buffer_apply.set_text(&snapshot.contents);
                        diff_buffer_apply.set_text(&snapshot.diff_text);
                        if snapshot.relative_path.ends_with(".md") {
                            mode_stack_apply.set_visible_child_name("preview");
                        } else {
                            mode_stack_apply.set_visible_child_name("edit");
                        }
                    },
                );
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
        current_file_reload.set_text(&relative_path);
        edit_buffer_reload.set_text("Loading file...");
        preview_buffer_reload.set_text("Loading file...");
        diff_buffer_reload.set_text("Loading diff...");
        let selected_file_apply = selected_file_reload.clone();
        let edit_buffer_apply = edit_buffer_reload.clone();
        let preview_buffer_apply = preview_buffer_reload.clone();
        let diff_buffer_apply = diff_buffer_reload.clone();
        let workspace_path_reload = workspace_path_reload.clone();
        let db_path_reload = db_path_reload.clone();
        let workspace_name_reload = workspace_name_reload.clone();
        spawn_background_job(
            move || {
                load_workspace_file_snapshot(
                    workspace_path_reload,
                    db_path_reload,
                    workspace_name_reload,
                    relative_path,
                )
            },
            move |snapshot| {
                if selected_file_apply.borrow().as_deref() != Some(snapshot.relative_path.as_str())
                {
                    return;
                }
                edit_buffer_apply.set_text(&snapshot.contents);
                preview_buffer_apply.set_text(&snapshot.contents);
                diff_buffer_apply.set_text(&snapshot.diff_text);
            },
        );
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
    checks: Option<&archductor_core::workspace::ChecksSummary>,
) -> GBox {
    let strip = GBox::new(Orientation::Horizontal, 10);
    strip.add_css_class("command-center-strip");
    strip.add_css_class("workspace-summary-strip");
    strip.append(&metric_card("Status", &ws.status));
    strip.append(&metric_card(
        "Files",
        &checks
            .map(|summary| summary.changed_files.to_string())
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
    toast_manager: ToastManager,
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
    let profile_names = WorkspaceStore::open_app(db_path)
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
            let general_prompt = WorkspaceStore::open_app(db_for_launch.clone())
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
        None,
        None,
        toast_manager.clone(),
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
        "These instructions are included with the first message in a new chat.",
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

    let setup_workspace = ws.name.clone();
    let db_path_setup = db_path.to_path_buf();
    let refresh_setup = refresh_hub.clone();
    let status_setup = status.clone();
    let toast_setup = toast_overlay.clone();
    setup_btn.connect_clicked(move |_| {
        status_setup.set_text("Starting setup...");
        match WorkspaceStore::open_app(db_path_setup.clone())
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
        refresh_setup.refresh_event(RefreshEvent::WorkspaceRuntimeChanged {
            workspace: setup_workspace.clone(),
        });
    });

    let run_workspace = ws.name.clone();
    let db_path_run = db_path.to_path_buf();
    let refresh_run = refresh_hub.clone();
    let status_run = status.clone();
    let toast_run = toast_overlay.clone();
    run_btn.connect_clicked(move |_| {
        status_run.set_text("Starting run...");
        match WorkspaceStore::open_app(db_path_run.clone())
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
        refresh_run.refresh_event(RefreshEvent::WorkspaceRuntimeChanged {
            workspace: run_workspace.clone(),
        });
    });

    let stop_workspace = ws.name.clone();
    let db_path_stop = db_path.to_path_buf();
    let refresh_stop = refresh_hub.clone();
    let status_stop = status.clone();
    let toast_stop = toast_overlay.clone();
    stop_btn.connect_clicked(move |_| {
        status_stop.set_text("Stopping run...");
        match WorkspaceStore::open_app(db_path_stop.clone())
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
        refresh_stop.refresh_event(RefreshEvent::WorkspaceRuntimeChanged {
            workspace: stop_workspace.clone(),
        });
    });

    let spotlight_workspace = ws.name.clone();
    let db_path_spotlight_on = db_path.to_path_buf();
    let refresh_spotlight_on = refresh_hub.clone();
    let status_spotlight_on = status.clone();
    let toast_spotlight_on = toast_overlay.clone();
    spotlight_on_btn.connect_clicked(move |_| {
        status_spotlight_on.set_text("Starting Spotlight...");
        match WorkspaceStore::open_app(db_path_spotlight_on.clone())
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
        refresh_spotlight_on.refresh_event(RefreshEvent::WorkspaceRuntimeChanged {
            workspace: spotlight_workspace.clone(),
        });
    });

    let spotlight_sync_workspace = ws.name.clone();
    let db_path_spotlight_sync = db_path.to_path_buf();
    let refresh_spotlight_sync = refresh_hub.clone();
    let status_spotlight_sync = status.clone();
    let toast_spotlight_sync = toast_overlay.clone();
    spotlight_sync_btn.connect_clicked(move |_| {
        status_spotlight_sync.set_text("Syncing Spotlight...");
        match WorkspaceStore::open_app(db_path_spotlight_sync.clone())
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
        refresh_spotlight_sync.refresh_event(RefreshEvent::WorkspaceRuntimeChanged {
            workspace: spotlight_sync_workspace.clone(),
        });
    });

    let spotlight_repair_workspace = ws.name.clone();
    let db_path_spotlight_repair = db_path.to_path_buf();
    let refresh_spotlight_repair = refresh_hub.clone();
    let status_spotlight_repair = status.clone();
    let toast_spotlight_repair = toast_overlay.clone();
    spotlight_repair_btn.connect_clicked(move |_| {
        status_spotlight_repair.set_text("Repairing Spotlight root: discarding root-only edits...");
        match WorkspaceStore::open_app(db_path_spotlight_repair.clone())
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
        refresh_spotlight_repair.refresh_event(RefreshEvent::WorkspaceRuntimeChanged {
            workspace: spotlight_repair_workspace.clone(),
        });
    });

    let spotlight_stop_workspace = ws.name.clone();
    let db_path_spotlight_off = db_path.to_path_buf();
    let refresh_spotlight_off = refresh_hub.clone();
    let status_spotlight_off = status.clone();
    let toast_spotlight_off = toast_overlay;
    spotlight_off_btn.connect_clicked(move |_| {
        status_spotlight_off.set_text("Stopping Spotlight...");
        match WorkspaceStore::open_app(db_path_spotlight_off.clone())
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
        refresh_spotlight_off.refresh_event(RefreshEvent::WorkspaceRuntimeChanged {
            workspace: spotlight_stop_workspace.clone(),
        });
    });

    let path = ws.path.clone();
    folder_btn.connect_clicked(move |_| {
        if let Ok(uri) = gtk::glib::filename_to_uri(&path, None) {
            let _ = gtk::gio::AppInfo::launch_default_for_uri(
                &uri,
                None::<&gtk::gio::AppLaunchContext>,
            );
        }
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
    let duplicate_name_entry = Entry::new();
    duplicate_name_entry.set_placeholder_text(Some("duplicate workspace name"));
    duplicate_name_entry.set_hexpand(true);
    let duplicate_branch_entry = Entry::new();
    duplicate_branch_entry.set_placeholder_text(Some("duplicate branch"));
    duplicate_branch_entry.set_hexpand(true);
    let duplicate_btn = secondary_button("Duplicate");
    let confirm = CheckButton::with_label("Confirm archive/discard/delete");
    let archive_btn = secondary_button("Archive");
    let restore_btn = flat_button("Restore");
    let discard_btn = destructive_button("Discard");
    let delete_btn = destructive_button("Delete");
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
        match WorkspaceStore::open_app(db_rename.clone())
            .and_then(|store| store.rename(&current_name, &new_name))
        {
            Ok(workspace) => {
                state_after_rename.rename_workspace_in_navigation(&current_name, &workspace.name);
                progress_rename.set_text(&format!("Renamed to {}", workspace.name));
                refresh_after_rename.refresh_event(RefreshEvent::WorkspaceMetadataChanged {
                    old_workspace: current_name.clone(),
                    workspace: workspace.name,
                    branch: None,
                });
            }
            Err(err) => apply_runtime_action_feedback(
                &progress_rename,
                &toast_rename,
                lifecycle_action_failure_feedback("Rename", &err),
            ),
        }
    });

    let db_duplicate = db_path.to_path_buf();
    let source_for_duplicate = ws.name.clone();
    let refresh_after_duplicate = refresh_hub.clone();
    let progress_duplicate = progress.clone();
    let toast_duplicate = toast_overlay.clone();
    let duplicate_name_entry_clone = duplicate_name_entry.clone();
    let duplicate_branch_entry_clone = duplicate_branch_entry.clone();
    duplicate_btn.connect_clicked(move |_| {
        let new_name = duplicate_name_entry_clone.text().trim().to_owned();
        let branch = duplicate_branch_entry_clone.text().trim().to_owned();
        if new_name.is_empty() {
            progress_duplicate.set_text("Enter a duplicate workspace name.");
            return;
        }
        progress_duplicate.set_text("Duplicating...");
        match WorkspaceStore::open_app(db_duplicate.clone()).and_then(|store| {
            store.duplicate(
                &source_for_duplicate,
                &new_name,
                (!branch.is_empty()).then_some(branch.as_str()),
            )
        }) {
            Ok(workspace) => {
                progress_duplicate.set_text(&format!(
                    "Duplicated to {} on {}",
                    workspace.name, workspace.branch
                ));
                duplicate_name_entry_clone.set_text("");
                duplicate_branch_entry_clone.set_text("");
            }
            Err(err) => apply_runtime_action_feedback(
                &progress_duplicate,
                &toast_duplicate,
                lifecycle_action_failure_feedback("Duplicate", &err),
            ),
        }
        refresh_after_duplicate.refresh_event(RefreshEvent::WorkspaceInventoryChanged);
    });

    for (button, action) in [
        (archive_btn.clone(), "archive"),
        (restore_btn.clone(), "restore"),
        (discard_btn.clone(), "discard"),
        (delete_btn.clone(), "delete"),
    ] {
        let workspace = ws.name.clone();
        let db_action = db_path.to_path_buf();
        let refresh_after_action = refresh_hub.clone();
        let confirm_action = confirm.clone();
        let progress_action = progress.clone();
        let toast_action = toast_overlay.clone();
        let state_after_action = state.clone();
        button.connect_clicked(move |_| {
            let workspace = current_workspace_action_target(&state_after_action, &workspace);
            if matches!(action, "archive" | "discard" | "delete") && !confirm_action.is_active() {
                progress_action.set_text("Check confirm before archive/discard/delete.");
                return;
            }
            progress_action.set_text(&format!("{action} in progress..."));
            if action == "delete" {
                progress_action.set_text("Deleting in background...");
                let db_delete = db_action.clone();
                let workspace_delete = workspace.clone();
                let refresh_after_delete = refresh_after_action.clone();
                let progress_after_delete = progress_action.clone();
                let toast_after_delete = toast_action.clone();
                let state_after_delete = state_after_action.clone();
                spawn_background_job(
                    move || {
                        WorkspaceStore::open_app(db_delete)
                            .and_then(|store| {
                                let result =
                                    store.delete_lifecycle_job(&workspace_delete, false, false)?;
                                if let Some(err) = result.cleanup_error {
                                    error!(
                                        workspace = %result.workspace.name,
                                        error = %err,
                                        "workspace artifact cleanup failed after metadata delete"
                                    );
                                }
                                Ok(result.workspace)
                            })
                            .map_err(|err| format!("{err:#}"))
                    },
                    move |result| {
                        match result {
                            Ok(deleted) => {
                                progress_after_delete
                                    .set_text(&workspace_delete_feedback(Ok(deleted.clone())));
                                apply_workspace_delete_navigation_result(
                                    &state_after_delete,
                                    &Ok(deleted.clone()),
                                );
                            }
                            Err(err) => {
                                let err = anyhow::anyhow!(err);
                                apply_runtime_action_feedback(
                                    &progress_after_delete,
                                    &toast_after_delete,
                                    lifecycle_action_failure_feedback("Delete", &err),
                                );
                            }
                        }
                        refresh_after_delete.refresh_event(RefreshEvent::WorkspaceInventoryChanged);
                    },
                );
                return;
            }
            let result =
                WorkspaceStore::open_app(db_action.clone()).and_then(|store| match action {
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
            refresh_after_action.refresh_event(RefreshEvent::WorkspaceInventoryChanged);
        });
    }

    let rename_row = make_action_row();
    rename_row.append(&rename_entry);
    rename_row.append(&rename_btn);
    let duplicate_row = make_action_row();
    duplicate_row.append(&duplicate_name_entry);
    duplicate_row.append(&duplicate_branch_entry);
    duplicate_row.append(&duplicate_btn);
    let lifecycle_row = make_action_row();
    lifecycle_row.append(&confirm);
    lifecycle_row.append(&archive_btn);
    lifecycle_row.append(&restore_btn);
    lifecycle_row.append(&discard_btn);
    lifecycle_row.append(&delete_btn);
    row.append(&rename_row);
    row.append(&duplicate_row);
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

    let work_detail_tabs = Stack::new();
    tabs.add_titled(
        &changes_checks_review_tabs(
            db_path,
            store,
            &ws.name,
            state.clone(),
            refresh_hub.clone(),
            toast_overlay.clone(),
            work_detail_tabs.clone(),
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
            ToastManager::new(&toast_overlay),
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
            ToastManager::new(&toast_overlay),
        ),
        Some("terminal"),
        "Terminal",
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
    tabs.add_titled(
        &workspace_branch_panel(
            db_path,
            store,
            ws,
            refresh_hub.clone(),
            toast_overlay.clone(),
        ),
        Some("branch"),
        "Branch",
    );
    tabs.add_titled(
        &workspace_timeline_panel(
            store,
            &ws.name,
            state.clone(),
            tabs.clone(),
            work_detail_tabs.clone(),
        ),
        Some("timeline"),
        "Timeline",
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
        WorkspaceTab::Changes
        | WorkspaceTab::Checks
        | WorkspaceTab::Review
        | WorkspaceTab::Todos => "work",
        WorkspaceTab::Checkpoints => "checkpoints",
        WorkspaceTab::Processes => "processes",
        WorkspaceTab::Terminal => "terminal",
    }
}

fn workspace_branch_panel(
    db_path: &Path,
    store: &WorkspaceStore,
    ws: &Workspace,
    refresh_hub: RefreshHub,
    toast_overlay: ToastOverlay,
) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 8);
    panel.add_css_class("command-panel");
    panel.append(&section_title("Branch"));
    panel.append(&detail_row("Current", &ws.branch));
    panel.append(&detail_row(
        "Upstream",
        &workspace_branch_state_text(store, &ws.name),
    ));

    let branch_entry = Entry::new();
    branch_entry.set_placeholder_text(Some("branch name"));
    branch_entry.set_hexpand(true);
    let create_btn = secondary_button("Create");
    let checkout_btn = secondary_button("Checkout");
    let rename_btn = secondary_button("Rename");
    let confirm_delete = CheckButton::with_label("Confirm delete");
    let delete_btn = destructive_button("Delete");
    let feedback = Label::new(None);
    feedback.add_css_class("card-meta");
    feedback.set_xalign(0.0);
    feedback.set_wrap(true);

    for (button, action) in [
        (create_btn.clone(), "create"),
        (checkout_btn.clone(), "checkout"),
        (rename_btn.clone(), "rename"),
        (delete_btn.clone(), "delete"),
    ] {
        let db_for_action = db_path.to_path_buf();
        let workspace_for_action = ws.name.clone();
        let entry_for_action = branch_entry.clone();
        let feedback_for_action = feedback.clone();
        let toast_for_action = toast_overlay.clone();
        let refresh_after_action = refresh_hub.clone();
        let confirm_delete_for_action = confirm_delete.clone();
        button.connect_clicked(move |_| {
            let branch = entry_for_action.text().trim().to_owned();
            if branch.is_empty() {
                apply_action_feedback(
                    &feedback_for_action,
                    &toast_for_action,
                    "Enter a branch name.",
                    true,
                );
                return;
            }
            if action == "delete" && !confirm_delete_for_action.is_active() {
                apply_action_feedback(
                    &feedback_for_action,
                    &toast_for_action,
                    "Confirm branch delete first.",
                    true,
                );
                return;
            }
            let result =
                WorkspaceStore::open_app(db_for_action.clone()).and_then(|store| match action {
                    "checkout" => {
                        store
                            .checkout_branch(&workspace_for_action, &branch)
                            .map(|workspace| {
                                (
                                    format!("Checked out {}.", workspace.branch),
                                    Some(workspace),
                                )
                            })
                    }
                    "rename" => {
                        store
                            .rename_branch(&workspace_for_action, &branch)
                            .map(|workspace| {
                                (
                                    format!("Renamed branch to {}.", workspace.branch),
                                    Some(workspace),
                                )
                            })
                    }
                    "create" => store
                        .create_branch(&workspace_for_action, &branch)
                        .map(|_| (format!("Created branch {branch}."), None)),
                    "delete" => store
                        .delete_branch(&workspace_for_action, &branch)
                        .map(|_| (format!("Deleted branch {branch}."), None)),
                    _ => unreachable!(),
                });
            match result {
                Ok((message, updated_workspace)) => {
                    apply_action_feedback(&feedback_for_action, &toast_for_action, &message, true);
                    if let Some(workspace) = updated_workspace {
                        refresh_after_action.refresh_event(
                            RefreshEvent::WorkspaceMetadataChanged {
                                old_workspace: workspace.name.clone(),
                                workspace: workspace.name.clone(),
                                branch: Some(workspace.branch.clone()),
                            },
                        );
                    }
                    refresh_after_action.refresh(RefreshScope::Workspace);
                }
                Err(err) => apply_action_feedback(
                    &feedback_for_action,
                    &toast_for_action,
                    &format!("Branch {action} failed: {err:#}"),
                    true,
                ),
            }
        });
    }

    let branch_row = make_action_row();
    branch_row.append(&branch_entry);
    branch_row.append(&create_btn);
    branch_row.append(&checkout_btn);
    branch_row.append(&rename_btn);
    let delete_row = make_action_row();
    delete_row.append(&confirm_delete);
    delete_row.append(&delete_btn);
    panel.append(&branch_row);
    panel.append(&delete_row);
    panel.append(&feedback);
    panel
}

fn workspace_timeline_panel(
    store: &WorkspaceStore,
    name: &str,
    app_state: AppState,
    workspace_tabs: Stack,
    work_detail_tabs: Stack,
) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 8);
    panel.add_css_class("command-panel");
    panel.append(&section_title("Timeline"));
    match store.workspace_timeline(name, None) {
        Ok(events) if events.is_empty() => {
            panel.append(&detail_row("Timeline", "No workspace timeline events yet."))
        }
        Ok(events) => {
            let list = GBox::new(Orientation::Vertical, 4);
            let (visible, hidden) = visible_timeline_events(&events);
            if hidden > 0 {
                list.append(&detail_row(
                    "Timeline",
                    &format!(
                        "Showing latest {} events; {hidden} older hidden.",
                        visible.len()
                    ),
                ));
            }
            for event in visible {
                list.append(&workspace_timeline_event_row(
                    event,
                    app_state.clone(),
                    workspace_tabs.clone(),
                    work_detail_tabs.clone(),
                ));
            }
            let scroll = ScrolledWindow::new();
            scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
            scroll.set_vexpand(true);
            scroll.set_child(Some(&list));
            panel.append(&scroll);
        }
        Err(err) => panel.append(&detail_row(
            "Timeline",
            &format!("Could not load timeline: {err:#}"),
        )),
    }
    panel
}

fn workspace_timeline_event_row(
    event: &WorkspaceTimelineEvent,
    app_state: AppState,
    workspace_tabs: Stack,
    work_detail_tabs: Stack,
) -> GBox {
    let row = GBox::new(Orientation::Horizontal, 8);
    row.add_css_class("ws-git-file-action-row");
    let label = Label::new(Some(&format_workspace_timeline_event(event)));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_wrap(true);
    row.append(&label);

    if let Some((tab, label)) = timeline_jump_target(event.kind.as_str()) {
        let button = text_button(label);
        let target_tab = tab.clone();
        button.connect_clicked(move |_| {
            app_state.set_active_workspace_tab(target_tab.clone());
            workspace_tabs.set_visible_child_name(workspace_tab_stack_name(&target_tab));
            if matches!(
                target_tab,
                WorkspaceTab::Changes | WorkspaceTab::Checks | WorkspaceTab::Review
            ) {
                work_detail_tabs
                    .set_visible_child_name(changes_checks_review_tab_stack_name(&target_tab));
            }
        });
        row.append(&button);
    }
    row
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
        match WorkspaceStore::open_app(db_for_create.clone())
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
    match WorkspaceStore::open_app(db_path).and_then(|store| store.checkpoint_list(name)) {
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
            match WorkspaceStore::open_app(db_for_restore.clone())
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
    tabs: Stack,
) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 8);
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
            app_state.selected_chat_thread(),
            None,
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
    toast_manager: ToastManager,
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
        None,
        None,
        toast_manager.clone(),
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
        toast_manager.clone(),
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
        toast_manager,
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
    toast_manager: ToastManager,
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
        &WorkspaceStore::open_app(db_path)
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
        None,
        None,
        toast_manager.clone(),
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
        refresh_hub,
        toast_manager.clone(),
    ));

    let right = GBox::new(Orientation::Vertical, 0);
    right.add_css_class("command-panel");
    right.add_css_class("session-tool-surface");

    let file_tabs = Stack::new();
    file_tabs.set_vexpand(true);
    let file_switcher = StackSwitcher::new();
    file_switcher.set_stack(Some(&file_tabs));
    file_switcher.add_css_class("panel-switcher");

    let changes_text = WorkspaceStore::open_app(db_path)
        .and_then(|store| store.diff_file_summaries(&ws.name))
        .map(|summaries| format_diff_file_summary(&summaries))
        .unwrap_or_else(|_| "No changes yet.\n".to_owned());
    file_tabs.add_titled(&text_panel(&changes_text), Some("changes"), "Changes");

    let checks_text = WorkspaceStore::open_app(db_path)
        .map(|store| workspace_checks_text(&store, &ws.name))
        .unwrap_or_else(|_| "No checks yet.\n".to_owned());
    file_tabs.add_titled(&text_panel(&checks_text), Some("checks"), "Checks");

    right.append(&file_switcher);
    right.append(&file_tabs);

    let run_label = section_title("Run");
    run_label.set_margin_top(8);
    run_label.set_margin_start(8);
    right.append(&run_label);

    let run_text = WorkspaceStore::open_app(db_path)
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

fn linked_directories_panel(
    db_path: &Path,
    name: &str,
    refresh_hub: RefreshHub,
    toast_manager: ToastManager,
) -> GBox {
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
    let toast_for_link = toast_manager.clone();
    link_btn.connect_clicked(move |_| {
        let target = target_for_link.text().trim().to_owned();
        if target.is_empty() {
            let message = "Enter a target workspace name to link.\n";
            toast_for_link.error(message.trim().to_owned());
            buffer_for_link.set_text(message);
            return;
        }
        match WorkspaceStore::open_app(db_for_link.clone())
            .and_then(|store| store.link_workspace_directory(&workspace_for_link, &target))
        {
            Ok(_) => {
                buffer_for_link
                    .set_text(&linked_directories_text(&db_for_link, &workspace_for_link));
                hub_for_link.refresh(RefreshScope::Workspace);
            }
            Err(err) => {
                let message = format!("Could not link directory: {err:#}\n");
                toast_for_link.error(message.trim().to_owned());
                buffer_for_link.set_text(&message);
            }
        }
    });

    let db_for_unlink = db_path.to_path_buf();
    let workspace_for_unlink = name.to_owned();
    let target_for_unlink = target_entry;
    let buffer_for_unlink = links_view.buffer();
    let hub_for_unlink = refresh_hub;
    let toast_for_unlink = toast_manager.clone();
    unlink_btn.connect_clicked(move |_| {
        let target = target_for_unlink.text().trim().to_owned();
        if target.is_empty() {
            let message = "Enter a target workspace name to unlink.\n";
            toast_for_unlink.error(message.trim().to_owned());
            buffer_for_unlink.set_text(message);
            return;
        }
        match WorkspaceStore::open_app(db_for_unlink.clone())
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
                let message = format!("Could not unlink directory: {err:#}\n");
                toast_for_unlink.error(message.trim().to_owned());
                buffer_for_unlink.set_text(&message);
            }
        }
    });

    panel
}

fn linked_directories_text(db_path: &Path, name: &str) -> String {
    match WorkspaceStore::open_app(db_path).and_then(|store| store.list_linked_directories(name)) {
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
    selected_chat_thread: Option<i64>,
    open_file: Option<OpenWorkspaceFile>,
    _refresh_hub: RefreshHub,
    toast_overlay: ToastOverlay,
) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 8);
    let header_row = GBox::new(Orientation::Horizontal, 8);
    header_row.add_css_class("ws-changes-header");
    let title = Label::new(Some("Changes"));
    title.add_css_class("ws-changes-title");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    let menu_btn = text_button("Uncommitted");
    menu_btn.add_css_class("ws-changes-menu-btn");
    header_row.append(&title);
    header_row.append(&menu_btn);
    panel.append(&header_row);

    let body_stack = Stack::new();
    body_stack.set_vexpand(true);
    body_stack.set_hexpand(true);

    let mut scope_items = Vec::new();
    let mut menu_entries = Vec::new();
    add_file_summary_scope(
        &body_stack,
        &mut scope_items,
        FileSummaryScopeConfig {
            label: "All Changes",
            stack_key: "all_changes",
            menu_label: "All Changes",
            persisted_scope: "all",
            show_state: true,
        },
        store.all_file_change_summaries(name),
        open_file.clone(),
    );
    if let Some(item) = scope_items.last().cloned() {
        menu_entries.push(ChangesScopeMenuEntry::Item(WorkspaceChangesMenuRow {
            item,
            title: "All Changes".to_owned(),
            subtitle: None,
            counts: None,
        }));
    }
    add_file_summary_scope(
        &body_stack,
        &mut scope_items,
        FileSummaryScopeConfig {
            label: "Uncommitted Changes",
            stack_key: "uncommitted",
            menu_label: "Uncommitted Changes",
            persisted_scope: "uncommitted",
            show_state: true,
        },
        store.diff_file_summaries(name),
        open_file.clone(),
    );
    if let Some(item) = scope_items.last().cloned() {
        menu_entries.push(ChangesScopeMenuEntry::Item(WorkspaceChangesMenuRow {
            item,
            title: "Uncommitted Changes".to_owned(),
            subtitle: None,
            counts: None,
        }));
    }

    let last_turn_key = "last_turn";
    let last_turn_view = match selected_chat_thread {
        Some(thread_id) => match store.last_turn_file_change_summary(name, thread_id) {
            Ok(Some(turn)) => {
                workspace_file_summary_scope_view(&turn.files, false, open_file.clone())
            }
            Ok(None) => workspace_empty_changes_scope_view(
                "No file changes recorded for the focused chat's last turn.",
            ),
            Err(err) => workspace_empty_changes_scope_view(&format!(
                "Could not read focused chat's last turn changes: {err:#}"
            )),
        },
        None => workspace_empty_changes_scope_view("Select a chat to see its last turn changes."),
    };
    body_stack.add_named(&last_turn_view, Some(last_turn_key));
    scope_items.push(WorkspaceChangesScopeItem {
        label: "Last Turn".to_owned(),
        stack_key: last_turn_key.to_owned(),
        menu_label: "Last Turn".to_owned(),
        persisted_scope: "last_turn".to_owned(),
    });
    if let Some(item) = scope_items.last().cloned() {
        menu_entries.push(ChangesScopeMenuEntry::Item(WorkspaceChangesMenuRow {
            item,
            title: "Last Turn".to_owned(),
            subtitle: None,
            counts: None,
        }));
    }

    match store.commit_file_change_summaries(name, WORKSPACE_COMMIT_CHANGE_LIMIT) {
        Ok(commits) => {
            for (index, commit) in commits.iter().enumerate() {
                let key = format!("commit_{index}");
                let menu_entry = commit_changes_menu_entry(&key, commit);
                body_stack.add_named(
                    &workspace_file_summary_scope_view(&commit.files, false, open_file.clone()),
                    Some(&key),
                );
                scope_items.push(menu_entry.item.clone());
                append_commit_changes_menu_entry(&mut menu_entries, menu_entry);
            }
        }
        Err(err) => {
            if !menu_entries.is_empty() {
                menu_entries.push(ChangesScopeMenuEntry::Separator);
            }
            menu_entries.push(ChangesScopeMenuEntry::Info(format!(
                "Could not read branch commits: {err:#}"
            )));
        }
    }

    let saved_scope = store.workspace_changes_scope(name).unwrap_or_default();
    let selected_scope =
        workspace_changes_selected_scope(&scope_items, saved_scope.as_deref()).clone();
    body_stack.set_visible_child_name(&selected_scope.stack_key);
    menu_btn.set_label(&selected_scope.menu_label);
    panel.append(&body_stack);
    let scope_feedback = Label::new(None);
    scope_feedback.add_css_class("status-text");
    scope_feedback.set_xalign(0.0);
    scope_feedback.set_wrap(true);
    panel.append(&scope_feedback);

    let popover = gtk::Popover::new();
    popover.add_css_class("ws-changes-popover");
    popover.set_parent(&menu_btn);
    let menu = GBox::new(Orientation::Vertical, 0);
    menu.add_css_class("ws-changes-menu-list");

    for entry in menu_entries {
        match entry {
            ChangesScopeMenuEntry::Separator => {
                let separator = Separator::new(Orientation::Horizontal);
                separator.add_css_class("ws-changes-menu-separator");
                menu.append(&separator);
            }
            ChangesScopeMenuEntry::Info(message) => {
                let label = Label::new(Some(&message));
                label.add_css_class("ws-changes-menu-info");
                label.set_xalign(0.0);
                label.set_wrap(true);
                menu.append(&label);
            }
            ChangesScopeMenuEntry::Item(row) => {
                let item = changes_scope_menu_row(&row);
                let body_stack_for_item = body_stack.clone();
                let menu_btn_for_item = menu_btn.clone();
                let popover_for_item = popover.clone();
                let db_path_for_item = db_path.to_path_buf();
                let workspace_name_for_item = name.to_owned();
                let feedback_for_item = scope_feedback.clone();
                let toast_for_item = toast_overlay.clone();
                item.connect_clicked(move |_| {
                    body_stack_for_item.set_visible_child_name(&row.item.stack_key);
                    menu_btn_for_item.set_label(&row.item.menu_label);
                    if let Err(err) =
                        WorkspaceStore::open_app(&db_path_for_item).and_then(|store| {
                            store.set_workspace_changes_scope(
                                &workspace_name_for_item,
                                Some(&row.item.persisted_scope),
                            )
                        })
                    {
                        let message = format!("Could not save changes scope: {err:#}");
                        error!("{message}");
                        apply_action_feedback(&feedback_for_item, &toast_for_item, &message, true);
                    }
                    popover_for_item.popdown();
                });
                menu.append(&item);
            }
        }
    }
    popover.set_child(Some(&menu));
    menu_btn.connect_clicked(move |_| {
        popover.popup();
    });

    panel
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceChangesMenuRow {
    item: WorkspaceChangesScopeItem,
    title: String,
    subtitle: Option<String>,
    counts: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ChangesScopeMenuEntry {
    Item(WorkspaceChangesMenuRow),
    Separator,
    Info(String),
}

impl ChangesScopeMenuEntry {
    #[cfg(test)]
    fn test_label(&self) -> &str {
        match self {
            ChangesScopeMenuEntry::Item(row) => row.title.as_str(),
            ChangesScopeMenuEntry::Separator => "---",
            ChangesScopeMenuEntry::Info(message) => message.as_str(),
        }
    }
}

fn commit_changes_menu_entry(
    stack_key: &str,
    commit: &archductor_core::workspace::CommitFileChangeSummary,
) -> WorkspaceChangesMenuRow {
    WorkspaceChangesMenuRow {
        item: WorkspaceChangesScopeItem {
            label: commit.subject.clone(),
            stack_key: stack_key.to_owned(),
            menu_label: short_commit(&commit.commit),
            persisted_scope: persisted_commit_changes_scope(&commit.commit),
        },
        title: commit.subject.clone(),
        subtitle: Some(short_commit(&commit.commit)),
        counts: diff_summaries_counts_text(&commit.files),
    }
}

fn append_commit_changes_menu_entries(
    entries: &mut Vec<ChangesScopeMenuEntry>,
    commits: &[archductor_core::workspace::CommitFileChangeSummary],
) {
    for (index, commit) in commits.iter().enumerate() {
        append_commit_changes_menu_entry(
            entries,
            commit_changes_menu_entry(&format!("commit_{index}"), commit),
        );
    }
}

fn append_commit_changes_menu_entry(
    entries: &mut Vec<ChangesScopeMenuEntry>,
    entry: WorkspaceChangesMenuRow,
) {
    if !matches!(entries.last(), Some(ChangesScopeMenuEntry::Separator)) {
        entries.push(ChangesScopeMenuEntry::Separator);
    }
    entries.push(ChangesScopeMenuEntry::Item(entry));
}

fn diff_summaries_counts_text(summaries: &[DiffFileSummary]) -> Option<String> {
    let mut additions = 0usize;
    let mut deletions = 0usize;
    let mut saw_text_counts = false;
    let mut saw_binary = false;
    for summary in summaries {
        match (summary.additions, summary.deletions) {
            (Some(add), Some(del)) => {
                additions += add;
                deletions += del;
                saw_text_counts = true;
            }
            _ => saw_binary = true,
        }
    }
    if saw_text_counts {
        Some(format!("+{additions} -{deletions}"))
    } else if saw_binary {
        Some("binary".to_owned())
    } else {
        None
    }
}

fn changes_scope_menu_row(row: &WorkspaceChangesMenuRow) -> Button {
    let button = Button::new();
    button.add_css_class("ws-changes-menu-row");
    button.set_halign(Align::Fill);
    button.set_hexpand(true);

    let content = GBox::new(Orientation::Horizontal, 12);
    content.set_hexpand(true);
    let labels = GBox::new(Orientation::Vertical, 1);
    labels.set_hexpand(true);

    let title = Label::new(Some(&row.title));
    title.add_css_class("ws-changes-menu-title");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    labels.append(&title);

    if let Some(subtitle) = &row.subtitle {
        let subtitle_label = Label::new(Some(subtitle));
        subtitle_label.add_css_class("ws-changes-menu-subtitle");
        subtitle_label.set_xalign(0.0);
        subtitle_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        labels.append(&subtitle_label);
    }
    content.append(&labels);

    if let Some(counts) = &row.counts {
        let counts_label = Label::new(Some(counts));
        counts_label.add_css_class("ws-changes-menu-counts");
        counts_label.set_xalign(1.0);
        content.append(&counts_label);
    }

    button.set_child(Some(&content));
    button
}

struct FileSummaryScopeConfig {
    label: &'static str,
    stack_key: &'static str,
    menu_label: &'static str,
    persisted_scope: &'static str,
    show_state: bool,
}

#[derive(Debug)]
struct FileSummaryScopeResult {
    item: WorkspaceChangesScopeItem,
    summaries: Option<Vec<DiffFileSummary>>,
    message: String,
}

fn file_summary_scope_from_result(
    label: &'static str,
    stack_key: &'static str,
    persisted_scope: &'static str,
    result: anyhow::Result<Vec<DiffFileSummary>>,
) -> FileSummaryScopeResult {
    file_summary_scope_from_result_with_menu(label, stack_key, label, persisted_scope, result)
}

fn file_summary_scope_from_result_with_menu(
    label: &'static str,
    stack_key: &'static str,
    menu_label: &'static str,
    persisted_scope: &'static str,
    result: anyhow::Result<Vec<DiffFileSummary>>,
) -> FileSummaryScopeResult {
    match result {
        Ok(summaries) => FileSummaryScopeResult {
            item: WorkspaceChangesScopeItem {
                label: label.to_owned(),
                stack_key: stack_key.to_owned(),
                menu_label: menu_label.to_owned(),
                persisted_scope: persisted_scope.to_owned(),
            },
            summaries: Some(summaries),
            message: String::new(),
        },
        Err(err) => FileSummaryScopeResult {
            item: WorkspaceChangesScopeItem {
                label: label.to_owned(),
                stack_key: stack_key.to_owned(),
                menu_label: menu_label.to_owned(),
                persisted_scope: persisted_scope.to_owned(),
            },
            summaries: None,
            message: format!("Could not read {label}: {err:#}"),
        },
    }
}

fn add_file_summary_scope(
    body_stack: &Stack,
    scope_items: &mut Vec<WorkspaceChangesScopeItem>,
    config: FileSummaryScopeConfig,
    result: anyhow::Result<Vec<DiffFileSummary>>,
    open_file: Option<OpenWorkspaceFile>,
) {
    let scope = file_summary_scope_from_result_with_menu(
        config.label,
        config.stack_key,
        config.menu_label,
        config.persisted_scope,
        result,
    );
    let view = match scope.summaries.as_deref() {
        Some(summaries) => {
            workspace_file_summary_scope_view(summaries, config.show_state, open_file)
        }
        None => workspace_empty_changes_scope_view(&scope.message),
    };
    body_stack.add_named(&view, Some(config.stack_key));
    scope_items.push(scope.item);
}

fn workspace_file_summary_scope_view(
    summaries: &[DiffFileSummary],
    show_state: bool,
    open_file: Option<OpenWorkspaceFile>,
) -> ScrolledWindow {
    let panel = GBox::new(Orientation::Vertical, 6);
    panel.add_css_class("ws-file-summary-panel");
    panel.set_vexpand(true);
    panel.set_hexpand(true);

    if summaries.is_empty() {
        let empty = Label::new(Some("No files changed."));
        empty.add_css_class("card-meta");
        empty.set_xalign(0.0);
        panel.append(&empty);
    } else {
        for summary in summaries {
            panel.append(&workspace_file_summary_row(
                summary,
                show_state,
                open_file.clone(),
            ));
        }
    }

    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    scroll.set_vexpand(true);
    scroll.set_child(Some(&panel));
    scroll
}

fn workspace_empty_changes_scope_view(message: &str) -> ScrolledWindow {
    let panel = GBox::new(Orientation::Vertical, 6);
    panel.add_css_class("ws-file-summary-panel");
    panel.set_vexpand(true);
    panel.set_hexpand(true);
    let empty = Label::new(Some(message));
    empty.add_css_class("card-meta");
    empty.set_xalign(0.0);
    empty.set_wrap(true);
    panel.append(&empty);
    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    scroll.set_vexpand(true);
    scroll.set_child(Some(&panel));
    scroll
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceChangeFileRowModel {
    icon_name: &'static str,
    path: String,
    counts: String,
    state: Option<&'static str>,
}

fn workspace_change_file_row_model(
    summary: &DiffFileSummary,
    show_state: bool,
) -> WorkspaceChangeFileRowModel {
    WorkspaceChangeFileRowModel {
        icon_name: file_tree_icon_name(false, &summary.path),
        path: summary.path.clone(),
        counts: diff_counts_text(summary),
        state: show_state.then_some(diff_state_label(summary)),
    }
}

fn workspace_file_summary_row(
    summary: &DiffFileSummary,
    show_state: bool,
    open_file: Option<OpenWorkspaceFile>,
) -> Widget {
    let model = workspace_change_file_row_model(summary, show_state);
    let row = GBox::new(Orientation::Horizontal, file_tree_row_spacing());
    row.add_css_class("ws-file-summary-row-content");
    row.set_hexpand(true);

    let icon = Image::from_icon_name(model.icon_name);
    icon.add_css_class("ws-file-icon");
    row.append(&icon);

    let path = Label::new(Some(&model.path));
    path.add_css_class("ws-file-name");
    path.set_xalign(0.0);
    path.set_hexpand(true);
    path.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    row.append(&path);

    if let Some(state_label) = model.state {
        let state = Label::new(Some(state_label));
        state.add_css_class("ws-file-summary-state");
        row.append(&state);
    }

    let counts = Label::new(Some(&model.counts));
    counts.add_css_class("ws-file-summary-counts");
    row.append(&counts);

    match workspace_file_summary_row_kind(open_file.is_some()) {
        WorkspaceFileSummaryRowKind::Button => {
            let button = Button::new();
            button.add_css_class("ws-file-summary-row");
            button.set_halign(Align::Fill);
            button.set_hexpand(true);
            if let Some(open_file) = open_file {
                let path = model.path.clone();
                button.connect_clicked(move |_| open_file(path.as_str()));
            }
            button.set_child(Some(&row));
            button.upcast()
        }
        WorkspaceFileSummaryRowKind::Static => {
            row.add_css_class("ws-file-summary-row");
            row.set_halign(Align::Fill);
            row.set_hexpand(true);
            row.upcast()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkspaceFileSummaryRowKind {
    Button,
    Static,
}

fn workspace_file_summary_row_kind(has_open_file: bool) -> WorkspaceFileSummaryRowKind {
    if has_open_file {
        WorkspaceFileSummaryRowKind::Button
    } else {
        WorkspaceFileSummaryRowKind::Static
    }
}

fn diff_counts_text(summary: &DiffFileSummary) -> String {
    match (summary.additions, summary.deletions) {
        (Some(additions), Some(deletions)) => format!("+{additions} -{deletions}"),
        _ => "binary".to_owned(),
    }
}

fn commit_scope_label(commit: &str, subject: &str) -> String {
    let short = short_commit(commit);
    if subject.trim().is_empty() {
        format!("Commit {short}")
    } else {
        format!("Commit {short}: {}", subject.trim())
    }
}

fn persisted_commit_changes_scope(commit: &str) -> String {
    format!("commit:{commit}")
}

fn short_commit(commit: &str) -> String {
    commit.chars().take(7).collect()
}

fn workspace_changes_selected_scope<'a>(
    items: &'a [WorkspaceChangesScopeItem],
    saved_scope: Option<&str>,
) -> &'a WorkspaceChangesScopeItem {
    if let Some(saved_scope) = saved_scope {
        if let Some(item) = items
            .iter()
            .find(|item| item.persisted_scope == saved_scope)
        {
            return item;
        }
        if saved_scope.starts_with("turn:") {
            if let Some(item) = items
                .iter()
                .find(|item| item.persisted_scope == "last_turn")
            {
                return item;
            }
        }
    }
    items
        .first()
        .expect("workspace changes scope menu should include uncommitted changes")
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
    workspace_diff_sections(store, name, path, Some(DIFF_RENDER_LIMIT_BYTES))
}

fn workspace_diff_sections(
    store: &WorkspaceStore,
    name: &str,
    path: Option<&str>,
    limit: Option<usize>,
) -> String {
    let path_ref = path.map(Path::new);
    let target = path.unwrap_or("workspace");
    let base_ref = store
        .workspace_base_ref(name)
        .unwrap_or_else(|_| "base".to_owned());
    let mut out =
        format!("Target: {target}\nBranch head comparison: HEAD\nReview base: {base_ref}\n\n");
    out.push_str(&format_diff_section(
        "Working tree changes",
        store.unified_diff_against_base(name, path_ref),
        limit,
    ));
    out.push('\n');
    out.push_str(&format_diff_section(
        "Unstaged changes",
        store.unified_diff(name, path_ref),
        limit,
    ));
    out.push('\n');
    out.push_str(&format_diff_section(
        "Staged changes",
        store.staged_diff(name, path_ref),
        limit,
    ));
    out
}

fn format_diff_section(
    title: &str,
    result: anyhow::Result<String>,
    limit: Option<usize>,
) -> String {
    let text = match result {
        Ok(text) => text,
        Err(err) => return format!("{title}\nCould not read diff: {err:#}\n"),
    };
    if text.trim().is_empty() {
        return format!("{title}\nNo changes.\n");
    }
    match limit {
        Some(limit) => {
            let (visible, truncated) = truncate_text_at_char_boundary(&text, limit);
            if truncated {
                format!(
                    "{title}\n{visible}\n[Diff truncated after {limit} bytes. Copy the diff or open the file for full context.]\n"
                )
            } else {
                format!("{title}\n{visible}")
            }
        }
        None => format!("{title}\n{text}"),
    }
}

fn truncate_text_at_char_boundary(text: &str, limit: usize) -> (&str, bool) {
    if text.len() <= limit {
        return (text, false);
    }
    let mut end = 0;
    for (index, _) in text.char_indices() {
        if index > limit {
            break;
        }
        end = index;
    }
    if end == 0 {
        end = limit.min(text.len());
    }
    (&text[..end], true)
}

fn workspace_diff_text_for_path(db_path: &Path, name: &str, path: Option<&str>) -> String {
    WorkspaceStore::open_app(db_path)
        .map(|store| workspace_diff_text(&store, name, path))
        .unwrap_or_else(|err| format!("Could not open workspace database: {err:#}\n"))
}

fn truncate_owned_with_marker(mut value: String, limit: usize, marker: &str) -> String {
    if value.len() + marker.len() <= limit {
        value.push_str(marker);
        return value;
    }
    let mut end = limit.saturating_sub(marker.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value.push_str(marker);
    value
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct WorkspaceContextEstimate {
    thread_count: usize,
    message_count: usize,
    event_count: usize,
    transcript_bytes: usize,
    estimated_tokens: u64,
    threads: Vec<WorkspaceThreadContextEstimate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceThreadContextEstimate {
    title: String,
    provider: String,
    message_count: usize,
    event_count: usize,
    transcript_bytes: usize,
    estimated_tokens: u64,
}

fn workspace_context_estimate_text(store: &WorkspaceStore, name: &str) -> String {
    match workspace_context_estimate(store, name) {
        Ok(estimate) => format_workspace_context_estimate(&estimate),
        Err(err) => format!("Could not estimate context usage: {err:#}\n"),
    }
}

fn workspace_context_estimate(
    store: &WorkspaceStore,
    name: &str,
) -> anyhow::Result<WorkspaceContextEstimate> {
    let threads = store.chat_thread_context_summaries(name)?;
    let mut estimate = WorkspaceContextEstimate {
        thread_count: threads.len(),
        ..Default::default()
    };
    for thread in threads {
        let thread_estimate = WorkspaceThreadContextEstimate {
            title: thread.title,
            provider: thread.provider,
            message_count: thread.message_count,
            event_count: thread.event_count,
            transcript_bytes: thread.transcript_bytes,
            estimated_tokens: estimate_tokens_from_bytes(thread.transcript_bytes),
        };
        estimate.message_count += thread_estimate.message_count;
        estimate.event_count += thread_estimate.event_count;
        estimate.transcript_bytes += thread_estimate.transcript_bytes;
        estimate.estimated_tokens += thread_estimate.estimated_tokens;
        estimate.threads.push(thread_estimate);
    }
    Ok(estimate)
}

fn format_workspace_context_estimate(estimate: &WorkspaceContextEstimate) -> String {
    let mut out = format!(
        "{} estimated tokens from {} persisted bytes across {} threads, {} messages, {} events.\nEstimate method: persisted transcript/event bytes divided by 4, rounded up.\n",
        format_context_number(estimate.estimated_tokens),
        format_context_number(estimate.transcript_bytes as u64),
        format_context_number(estimate.thread_count as u64),
        format_context_number(estimate.message_count as u64),
        format_context_number(estimate.event_count as u64),
    );
    if estimate.threads.is_empty() {
        out.push_str("No chat transcripts recorded.\n");
        return out;
    }
    for thread in &estimate.threads {
        out.push_str(&format!(
            "- {} [{}]: {} tokens, {} bytes, {} messages, {} events\n",
            thread.title,
            thread.provider,
            format_context_number(thread.estimated_tokens),
            format_context_number(thread.transcript_bytes as u64),
            format_context_number(thread.message_count as u64),
            format_context_number(thread.event_count as u64),
        ));
    }
    out
}

fn estimate_tokens_from_bytes(bytes: usize) -> u64 {
    (bytes as u64).div_ceil(4)
}

fn format_context_number(value: u64) -> String {
    let raw = value.to_string();
    let mut out = String::new();
    for (index, ch) in raw.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
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
    Pending,
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
            PullRequestStateKind::Pending
            | PullRequestStateKind::Ready
            | PullRequestStateKind::Failed
            | PullRequestStateKind::Merged => Some(self.label.as_str()),
        }
    }

    pub(crate) fn attention_css_class(&self) -> Option<&'static str> {
        match self.kind {
            PullRequestStateKind::Open => None,
            PullRequestStateKind::Pending
            | PullRequestStateKind::Ready
            | PullRequestStateKind::Failed
            | PullRequestStateKind::Merged => Some(self.css_class),
        }
    }
}

#[derive(Clone, Debug)]
struct WorkspacePrStatusSnapshot {
    pr: Option<PullRequest>,
    status: Option<PullRequestStatusSummary>,
    summary: Option<archductor_core::workspace::ChecksSummary>,
}

pub(crate) fn pull_request_status_summary(
    pr: &PullRequest,
    readiness: Option<&archductor_core::workspace::PullRequestReadiness>,
    summary: &archductor_core::workspace::ChecksSummary,
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
        if pull_request_is_pending(readiness) {
            return PullRequestStatusSummary {
                label: "checks pending".to_owned(),
                css_class: "ws-pr-status-pending",
                kind: PullRequestStateKind::Pending,
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

fn workspace_pr_status_snapshot(
    store: &WorkspaceStore,
    workspace_name: &str,
) -> WorkspacePrStatusSnapshot {
    let pr = store.pull_request(workspace_name).ok().flatten();
    let summary = store.checks_summary(workspace_name).ok();
    let status = pr
        .as_ref()
        .map(|pr| workspace_pull_request_status_summary(store, workspace_name, pr));
    WorkspacePrStatusSnapshot {
        pr,
        status,
        summary,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkspacePrTopActionKind {
    CreatePr,
    CommitAndPush,
    Push,
    MergeSourceBranch,
    MergePr,
    FixBlocked,
    ViewPr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WorkspacePrTopAction {
    label: &'static str,
    tooltip: &'static str,
    css_class: &'static str,
    kind: WorkspacePrTopActionKind,
}

fn workspace_pr_primary_action(
    snapshot: &WorkspacePrStatusSnapshot,
) -> Option<WorkspacePrTopAction> {
    let has_conflicts = snapshot
        .summary
        .as_ref()
        .is_some_and(|summary| !summary.conflicting_workspaces.is_empty());
    if has_conflicts {
        return Some(WorkspacePrTopAction {
            label: "Resolve Conflicts",
            tooltip: "Queue a prompt to resolve workspace conflicts",
            css_class: "ws-pr-status-failed",
            kind: WorkspacePrTopActionKind::FixBlocked,
        });
    }

    let changed_files = snapshot
        .summary
        .as_ref()
        .map(|summary| summary.changed_files)
        .unwrap_or_default();
    if changed_files > 0 {
        return Some(WorkspacePrTopAction {
            label: "Commit and Push",
            tooltip: "Ask the current chat to commit and push local changes",
            css_class: "ws-pr-status-muted",
            kind: WorkspacePrTopActionKind::CommitAndPush,
        });
    }

    let source_branch_ahead = snapshot
        .summary
        .as_ref()
        .map(|summary| summary.source_branch_ahead)
        .unwrap_or_default();
    if source_branch_ahead > 0 {
        return Some(WorkspacePrTopAction {
            label: "Merge",
            tooltip: "Ask the current chat to merge the source branch into this workspace",
            css_class: "ws-pr-status-pending",
            kind: WorkspacePrTopActionKind::MergeSourceBranch,
        });
    }

    let needs_push = snapshot
        .summary
        .as_ref()
        .and_then(|summary| summary.branch_push_state.as_ref())
        .is_some_and(|push| push.ahead > 0);
    if needs_push {
        if snapshot.pr.is_some() {
            return Some(WorkspacePrTopAction {
                label: "Push Branch",
                tooltip: "Push the workspace branch",
                css_class: "ws-pr-status-muted",
                kind: WorkspacePrTopActionKind::Push,
            });
        }
        return Some(WorkspacePrTopAction {
            label: "Create PR",
            tooltip: "Ask the current chat to create a pull request",
            css_class: "ws-pr-status-missing",
            kind: WorkspacePrTopActionKind::CreatePr,
        });
    }

    match snapshot.status.as_ref().map(|status| status.kind) {
        Some(PullRequestStateKind::Ready) => Some(WorkspacePrTopAction {
            label: "Merge PR",
            tooltip: "Merge the ready pull request",
            css_class: "ws-pr-status-ready",
            kind: WorkspacePrTopActionKind::MergePr,
        }),
        Some(PullRequestStateKind::Failed) => Some(WorkspacePrTopAction {
            label: "Fix Checks",
            tooltip: "Queue a prompt to fix failing checks",
            css_class: "ws-pr-status-failed",
            kind: WorkspacePrTopActionKind::FixBlocked,
        }),
        Some(PullRequestStateKind::Merged) => Some(WorkspacePrTopAction {
            label: "Merged",
            tooltip: "Open the merged pull request",
            css_class: "ws-pr-status-merged",
            kind: WorkspacePrTopActionKind::ViewPr,
        }),
        Some(PullRequestStateKind::Pending) => Some(WorkspacePrTopAction {
            label: "View PR",
            tooltip: "Open pull request with pending checks",
            css_class: "ws-pr-status-muted",
            kind: WorkspacePrTopActionKind::ViewPr,
        }),
        Some(PullRequestStateKind::Open) => Some(WorkspacePrTopAction {
            label: "View PR",
            tooltip: "Open pull request",
            css_class: "ws-pr-status-muted",
            kind: WorkspacePrTopActionKind::ViewPr,
        }),
        None => None,
    }
}

fn workspace_pr_primary_action_label(snapshot: &WorkspacePrStatusSnapshot) -> Option<&'static str> {
    workspace_pr_primary_action(snapshot).map(|action| action.label)
}

fn connect_create_pr_prompt_button(
    button: &Button,
    db_path: PathBuf,
    workspace_name: String,
    state: AppState,
    refresh_hub: RefreshHub,
) {
    button.connect_clicked(move |_| {
        let prompt = workspace_create_pr_chat_prompt(&db_path, &workspace_name);
        state.queue_pending_chat_prompt(prompt);
        state.set_active_workspace_tab(WorkspaceTab::Chats);
        refresh_hub.refresh(RefreshScope::Workspace);
    });
}

fn connect_commit_and_push_prompt_button(
    button: &Button,
    db_path: PathBuf,
    workspace_name: String,
    state: AppState,
    refresh_hub: RefreshHub,
) {
    button.connect_clicked(move |_| {
        let prompt = workspace_commit_and_push_chat_prompt(&db_path, &workspace_name);
        state.queue_pending_chat_prompt(prompt);
        state.set_active_workspace_tab(WorkspaceTab::Chats);
        refresh_hub.refresh(RefreshScope::Workspace);
    });
}

fn connect_merge_source_branch_prompt_button(
    button: &Button,
    db_path: PathBuf,
    workspace_name: String,
    state: AppState,
    refresh_hub: RefreshHub,
) {
    button.connect_clicked(move |_| {
        let prompt = workspace_merge_source_branch_chat_prompt(&db_path, &workspace_name);
        state.queue_pending_chat_prompt(prompt);
        state.set_active_workspace_tab(WorkspaceTab::Chats);
        refresh_hub.refresh(RefreshScope::Workspace);
    });
}

fn connect_fix_blocked_prompt_button(
    button: &Button,
    db_path: PathBuf,
    workspace_name: String,
    state: AppState,
    feedback: &Label,
    toast_overlay: &ToastOverlay,
) {
    let feedback_for_fix = feedback.clone();
    let toast_for_fix = toast_overlay.clone();
    button.connect_clicked(move |_| {
        let failed_checks = WorkspaceStore::open_app(db_path.clone())
            .and_then(|store| store.pull_request_check_runs(&workspace_name))
            .ok()
            .and_then(|checks| failed_pull_request_checks_text(&checks));
        let prompt = workspace_conflict_resolution_prompt(
            &db_path,
            &workspace_name,
            failed_checks.as_deref(),
        );
        state.queue_pending_chat_prompt(prompt);
        state.set_active_workspace_tab(WorkspaceTab::Chats);
        apply_action_feedback(
            &feedback_for_fix,
            &toast_for_fix,
            "Queued a prompt for the selected chat.",
            true,
        );
    });
}

fn connect_merge_pr_button(
    button: &Button,
    db_path: &Path,
    workspace_name: &str,
    feedback: &Label,
    toast_overlay: &ToastOverlay,
    refresh_hub: RefreshHub,
) {
    let db_for_merge = db_path.to_path_buf();
    let workspace_for_merge = workspace_name.to_owned();
    let feedback_for_merge = feedback.clone();
    let toast_for_merge = toast_overlay.clone();
    button.connect_clicked(move |_| {
        let result = WorkspaceStore::open_app(db_for_merge.clone()).and_then(|store| {
            store.merge_and_maybe_archive_pull_request(&workspace_for_merge, Some("squash"))
        });
        let refresh_event = pull_request_merge_refresh_event(&workspace_for_merge, &result);
        apply_action_feedback(
            &feedback_for_merge,
            &toast_for_merge,
            &pull_request_merge_and_archive_feedback(result),
            true,
        );
        refresh_hub.refresh_event(refresh_event);
    });
}

fn workspace_pr_status_panel(
    db_path: &Path,
    store: &WorkspaceStore,
    workspace_name: &str,
    state: &AppState,
    refresh_hub: RefreshHub,
    toast_overlay: &ToastOverlay,
) -> GBox {
    let snapshot = workspace_pr_status_snapshot(store, workspace_name);
    let action = workspace_pr_primary_action(&snapshot);
    let panel = GBox::new(Orientation::Horizontal, 8);
    panel.add_css_class("ws-pr-compact-panel");
    crate::window_chrome::configure_column_header(&panel);
    panel.add_css_class(
        action
            .as_ref()
            .map(|action| action.css_class)
            .unwrap_or("ws-pr-status-muted"),
    );
    let title = Label::new(Some(match action {
        Some(action) => workspace_pr_status_title(&snapshot, action),
        None => "No changes",
    }));
    title.add_css_class("ws-pr-compact-title");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    panel.append(&title);

    let Some(action) = action else {
        return panel;
    };

    let feedback = Label::new(None);
    feedback.add_css_class("card-meta");
    feedback.set_xalign(0.0);
    feedback.set_wrap(true);

    let primary_btn = secondary_button(action.label);
    primary_btn.add_css_class("ws-pr-action-button");
    primary_btn.add_css_class(action.css_class);
    primary_btn.set_tooltip_text(Some(action.tooltip));
    match action.kind {
        WorkspacePrTopActionKind::CreatePr => connect_create_pr_prompt_button(
            &primary_btn,
            db_path.to_path_buf(),
            workspace_name.to_owned(),
            state.clone(),
            refresh_hub.clone(),
        ),
        WorkspacePrTopActionKind::CommitAndPush => connect_commit_and_push_prompt_button(
            &primary_btn,
            db_path.to_path_buf(),
            workspace_name.to_owned(),
            state.clone(),
            refresh_hub.clone(),
        ),
        WorkspacePrTopActionKind::MergeSourceBranch => connect_merge_source_branch_prompt_button(
            &primary_btn,
            db_path.to_path_buf(),
            workspace_name.to_owned(),
            state.clone(),
            refresh_hub.clone(),
        ),
        WorkspacePrTopActionKind::Push => {
            connect_push_branch_button(
                &primary_btn,
                db_path,
                workspace_name,
                &feedback,
                toast_overlay,
            );
        }
        WorkspacePrTopActionKind::MergePr => connect_merge_pr_button(
            &primary_btn,
            db_path,
            workspace_name,
            &feedback,
            toast_overlay,
            refresh_hub.clone(),
        ),
        WorkspacePrTopActionKind::FixBlocked => connect_fix_blocked_prompt_button(
            &primary_btn,
            db_path.to_path_buf(),
            workspace_name.to_owned(),
            state.clone(),
            &feedback,
            toast_overlay,
        ),
        WorkspacePrTopActionKind::ViewPr => {
            if let Some(pr) = snapshot.pr.as_ref() {
                let url = pr.url.clone();
                primary_btn.connect_clicked(move |_| open_external_url(&url));
            }
        }
    }
    panel.append(&primary_btn);

    panel
}

fn workspace_pr_status_title(
    snapshot: &WorkspacePrStatusSnapshot,
    action: WorkspacePrTopAction,
) -> &'static str {
    match action.kind {
        WorkspacePrTopActionKind::CommitAndPush => return "Commit and push branch",
        WorkspacePrTopActionKind::Push => return "Push branch",
        WorkspacePrTopActionKind::MergeSourceBranch => return "Merge source branch",
        WorkspacePrTopActionKind::CreatePr => return "No pull request yet",
        WorkspacePrTopActionKind::FixBlocked if action.css_class == "ws-pr-status-failed" => {
            return "Blocked";
        }
        _ => {}
    }
    match snapshot.status.as_ref().map(|status| status.kind) {
        Some(PullRequestStateKind::Pending) => "Checks pending",
        Some(PullRequestStateKind::Ready) => "Ready to merge",
        Some(PullRequestStateKind::Failed) => "Checks failed",
        Some(PullRequestStateKind::Merged) => "Pull request merged",
        Some(PullRequestStateKind::Open) => "Pull request open",
        None => "No pull request yet",
    }
}

fn pull_request_status_summary_without_checks_summary(
    pr: &PullRequest,
    readiness: Option<&archductor_core::workspace::PullRequestReadiness>,
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
        if pull_request_is_pending(readiness) {
            return PullRequestStatusSummary {
                label: "checks pending".to_owned(),
                css_class: "ws-pr-status-pending",
                kind: PullRequestStateKind::Pending,
            };
        }
    }

    PullRequestStatusSummary {
        label: pr.state.to_owned(),
        css_class: "ws-pr-status-muted",
        kind: PullRequestStateKind::Open,
    }
}

fn pull_request_is_failed(readiness: &archductor_core::workspace::PullRequestReadiness) -> bool {
    readiness.checks.iter().any(|check| check.is_failure())
        || readiness
            .deployments
            .iter()
            .any(|deployment| deployment.is_failure())
}

fn pull_request_is_pending(readiness: &archductor_core::workspace::PullRequestReadiness) -> bool {
    readiness.checks.iter().any(|check| check.is_pending())
        || readiness
            .deployments
            .iter()
            .any(|deployment| deployment.is_pending())
}

fn pull_request_is_ready(
    readiness: &archductor_core::workspace::PullRequestReadiness,
    summary: &archductor_core::workspace::ChecksSummary,
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
    workspace_script_prompt(db_path, name, "setup", "Setup", PromptKind::SetupScript)
}

fn workspace_run_prompt(db_path: &Path, name: &str) -> String {
    workspace_script_prompt(db_path, name, "run", "Run", PromptKind::RunScript)
}

fn workspace_continue_prompt(db_path: &Path, name: &str) -> String {
    let configured = WorkspaceStore::open_app(db_path)
        .and_then(|store| store.resolved_prompt(name, PromptKind::ContinueWork))
        .ok()
        .flatten();
    workspace_continue_prompt_from_parts(name, configured.as_deref())
}

fn workspace_continue_prompt_from_parts(name: &str, configured: Option<&str>) -> String {
    let mut prompt = format!(
        "Continue after the merged PR for workspace {name}.\n\
         Check remaining todos, decide the next branch or follow-up PR, and keep going."
    );
    append_configured_prompt(&mut prompt, configured);
    prompt
}

fn workspace_create_pr_chat_prompt(db_path: &Path, name: &str) -> String {
    let mut repo_prompt = None;
    let mut context_brief = None;
    if let Ok(store) = WorkspaceStore::open_app(db_path) {
        repo_prompt = store
            .resolved_prompt(name, PromptKind::CreatePr)
            .ok()
            .flatten();
        context_brief = store.read_context_brief(name).ok().flatten();
    }
    workspace_create_pr_chat_prompt_from_parts(
        name,
        repo_prompt.as_deref(),
        context_brief.as_deref(),
    )
}

fn workspace_create_pr_chat_prompt_from_parts(
    name: &str,
    repo_prompt: Option<&str>,
    context_brief: Option<&str>,
) -> String {
    let mut prompt = format!(
        "Create a GitHub pull request for workspace {name}.\n\n\
         Push the branch if needed. Write a clear PR title and full PR body, then create the PR. \
         Include a concise summary and testing/verification section in the body."
    );
    if let Some(repo_prompt) = repo_prompt
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty())
    {
        prompt.push_str("\n\nRepository PR instructions:\n");
        prompt.push_str(repo_prompt);
    }
    if let Some(context_brief) = context_brief
        .map(str::trim)
        .filter(|context| !context.is_empty())
    {
        prompt.push_str("\n\nContext brief for the PR body:\n");
        prompt.push_str(context_brief);
    }
    prompt
}

fn workspace_commit_and_push_chat_prompt(db_path: &Path, name: &str) -> String {
    let mut prompt = format!(
        "Commit and push workspace {name}.\n\n\
         Review staged, unstaged, and untracked changes. Make the smallest coherent commit for \
         the current work, push the workspace branch, and report the commit plus push result."
    );
    append_configured_prompt(
        &mut prompt,
        resolved_workspace_prompt(db_path, name, PromptKind::CommitGeneration).as_deref(),
    );
    prompt
}

fn workspace_merge_source_branch_chat_prompt(db_path: &Path, name: &str) -> String {
    let source = WorkspaceStore::open_app(db_path)
        .and_then(|store| store.workspace_base_ref(name))
        .unwrap_or_else(|_| "the source branch".to_owned());
    let mut prompt = format!(
        "Merge {source} into workspace {name} before creating a pull request.\n\n\
         Fetch the latest source branch if needed, merge it into the workspace branch, resolve \
         conflicts carefully, run the relevant verification, then report the merge result and any \
         remaining blockers."
    );
    append_configured_prompt(
        &mut prompt,
        resolved_workspace_prompt(db_path, name, PromptKind::ResolveMergeConflicts).as_deref(),
    );
    prompt
}

fn workspace_conflict_resolution_prompt(
    db_path: &Path,
    name: &str,
    failed_checks: Option<&str>,
) -> String {
    workspace_conflict_resolution_prompt_from_parts(
        name,
        failed_checks,
        resolved_workspace_prompt(db_path, name, PromptKind::ResolveMergeConflicts).as_deref(),
        resolved_workspace_prompt(db_path, name, PromptKind::FixErrors).as_deref(),
    )
}

fn failed_pull_request_checks_text(checks: &[PullRequestCheckRun]) -> Option<String> {
    let failures = checks
        .iter()
        .filter(|check| check.is_failure())
        .map(|check| match check.detail.as_deref() {
            Some(detail) => format!("- {}: {} - {detail}", check.name, check.status),
            None => format!("- {}: {}", check.name, check.status),
        })
        .collect::<Vec<_>>();
    (!failures.is_empty()).then(|| failures.join("\n"))
}

fn workspace_conflict_resolution_prompt_from_parts(
    name: &str,
    failed_checks: Option<&str>,
    resolve_conflicts_prompt: Option<&str>,
    fix_errors_prompt: Option<&str>,
) -> String {
    let mut prompt = format!(
        "Resolve the blockers for workspace {name} before PR/merge.\n\n\
         Inspect workspace conflicts, failing checks, and branch state. Make the smallest safe \
         fix, rerun the relevant verification, and report what changed."
    );
    append_configured_prompt(&mut prompt, resolve_conflicts_prompt);
    if let Some(failed_checks) = failed_checks {
        prompt.push_str("\n\nFailed checks:\n");
        prompt.push_str(failed_checks);
        append_configured_prompt(&mut prompt, fix_errors_prompt);
    }
    prompt
}

fn workspace_script_prompt(
    db_path: &Path,
    name: &str,
    script_key: &str,
    label: &str,
    prompt_kind: PromptKind,
) -> String {
    let current = WorkspaceStore::open_app(db_path)
        .and_then(|store| store.workspace_repo_settings(name))
        .ok()
        .and_then(|settings| match script_key {
            "setup" => settings.scripts.setup,
            "run" => settings.scripts.run,
            _ => None,
        });
    let mut prompt = match current {
        Some(script) if !script.trim().is_empty() => format!(
            "Create or update .archductor/settings.toml for workspace {name}.\n\
             Set scripts.{script_key} to this multiline shell block of successive commands:\n\n{script}\n"
        ),
        _ => format!(
            "Create or update .archductor/settings.toml for workspace {name}.\n\
             Define scripts.{script_key} as a multiline shell block so the {label} tab can run successive commands in order.\n\
             Keep the commands short, reliable, and checked into the repo."
        ),
    };
    append_configured_prompt(
        &mut prompt,
        resolved_workspace_prompt(db_path, name, prompt_kind).as_deref(),
    );
    prompt
}

fn resolved_workspace_prompt(db_path: &Path, name: &str, kind: PromptKind) -> Option<String> {
    WorkspaceStore::open_app(db_path)
        .and_then(|store| store.resolved_prompt(name, kind))
        .ok()
        .flatten()
}

fn append_configured_prompt(prompt: &mut String, configured: Option<&str>) {
    if let Some(configured) = configured
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty())
    {
        prompt.push_str("\n\nRepository instructions:\n");
        prompt.push_str(configured);
    }
}

fn workspace_file_comments_text(db_path: &Path, name: &str, path: &str) -> String {
    WorkspaceStore::open_app(db_path)
        .and_then(|store| store.list_review_comments(name))
        .map(|comments| file_inline_comments_text(&comments, path))
        .unwrap_or_else(|err| format!("Could not read comments for {path}: {err:#}\n"))
}

fn diff_summary_label(summary: &DiffFileSummary) -> String {
    let counts = match (summary.additions, summary.deletions) {
        (Some(additions), Some(deletions)) => format!("+{additions} -{deletions}"),
        _ => "binary".to_owned(),
    };
    format!("{} {} {}", summary.path, diff_state_label(summary), counts)
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
        out.push_str(&format!(
            "{} {} {}\n",
            summary.path,
            diff_state_label(summary),
            counts
        ));
    }
    out
}

fn diff_state_label(summary: &DiffFileSummary) -> &'static str {
    match (summary.staged, summary.unstaged, summary.untracked) {
        (_, _, true) => "[untracked]",
        (true, true, _) => "[staged+unstaged]",
        (true, false, _) => "[staged]",
        (false, true, _) => "[unstaged]",
        _ => "[clean]",
    }
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
                "Changed files: {}\nRun: {}\nChecks: {}\nSessions: {}\nPR: {}\n{}\nTodos: {} open / {} total\nReview comments: {} open\nBranch: {}\nConflicts:\n{}",
                summary.changed_files,
                summary
                    .run_status
                    .map(|status| status.as_str().to_owned())
                    .unwrap_or_else(|| "none".to_owned()),
                latest_check_status_line(store, name),
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

fn format_workspace_timeline(events: &[WorkspaceTimelineEvent]) -> String {
    if events.is_empty() {
        return "No workspace timeline events yet.".to_owned();
    }
    let mut out = String::new();
    let (visible, hidden) = visible_timeline_events(events);
    if hidden > 0 {
        out.push_str(&format!(
            "Showing latest {} events; {hidden} older hidden.\n",
            visible.len()
        ));
    }
    for event in visible {
        out.push_str(&format_workspace_timeline_event(event));
        out.push('\n');
    }
    out
}

fn visible_timeline_events(
    events: &[WorkspaceTimelineEvent],
) -> (&[WorkspaceTimelineEvent], usize) {
    const TIMELINE_EVENT_LIMIT: usize = 200;
    if events.len() <= TIMELINE_EVENT_LIMIT {
        return (events, 0);
    }
    let hidden = events.len() - TIMELINE_EVENT_LIMIT;
    (&events[hidden..], hidden)
}

fn format_workspace_timeline_event(event: &WorkspaceTimelineEvent) -> String {
    let jump = timeline_jump_target(event.kind.as_str())
        .map(|(_, label)| format!(" -> {label}"))
        .unwrap_or_default();
    format!(
        "#{} {} [{}] {}{}",
        event.id, event.created_at, event.kind, event.summary, jump
    )
}

fn timeline_jump_target(kind: &str) -> Option<(WorkspaceTab, &'static str)> {
    if kind.contains("check")
        || kind.contains("run")
        || kind.contains("setup")
        || kind.contains("failed")
    {
        return Some((WorkspaceTab::Checks, "Open Checks"));
    }
    if kind.contains("git")
        || kind.contains("branch")
        || kind.contains("commit")
        || kind.contains("file.")
        || kind.contains("base_ref")
    {
        return Some((WorkspaceTab::Changes, "Open Changes"));
    }
    if kind.contains("review") || kind.contains("pull_request") || kind.contains("pr.") {
        return Some((WorkspaceTab::Review, "Open Review"));
    }
    if kind.contains("checkpoint") || kind.contains("archive") {
        return Some((WorkspaceTab::Checkpoints, "Open Checkpoints"));
    }
    if kind.contains("chat")
        || kind.contains("prompt")
        || kind.contains("assistant")
        || kind.contains("tool")
        || kind.contains("skill")
        || kind.contains("session")
    {
        return Some((WorkspaceTab::Chats, "Open Chat"));
    }
    None
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

fn connect_push_branch_button(
    push_btn: &Button,
    db_path: &Path,
    name: &str,
    feedback: &Label,
    toast_overlay: &ToastOverlay,
) {
    let db_for_push = db_path.to_path_buf();
    let workspace_for_push = name.to_owned();
    let feedback_for_push = feedback.clone();
    let toast_for_push = toast_overlay.clone();
    push_btn.connect_clicked(move |_| {
        match WorkspaceStore::open_app(db_for_push.clone())
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
                &push_branch_error_feedback(&err),
                true,
            ),
        }
    });
}

fn connect_force_push_button(
    force_push_btn: &Button,
    db_path: &Path,
    name: &str,
    feedback: &Label,
    toast_overlay: &ToastOverlay,
) {
    let db_for_force_push = db_path.to_path_buf();
    let workspace_for_force_push = name.to_owned();
    let feedback_for_force_push = feedback.clone();
    let toast_for_force_push = toast_overlay.clone();
    let force_confirmed = Rc::new(RefCell::new(false));
    let force_confirmed_for_click = force_confirmed.clone();
    let force_push_btn_for_click = force_push_btn.clone();
    force_push_btn.connect_clicked(move |_| {
        if !*force_confirmed_for_click.borrow() {
            *force_confirmed_for_click.borrow_mut() = true;
            force_push_btn_for_click.set_label("Confirm Force Push");
            apply_action_feedback(
                &feedback_for_force_push,
                &toast_for_force_push,
                "Click Confirm Force Push to run git push --force-with-lease.",
                true,
            );
            return;
        }
        force_push_btn_for_click.set_sensitive(false);
        match WorkspaceStore::open_app(db_for_force_push.clone())
            .and_then(|store| store.force_push_branch_with_lease(&workspace_for_force_push))
        {
            Ok(output) => {
                *force_confirmed_for_click.borrow_mut() = false;
                force_push_btn_for_click.set_sensitive(true);
                force_push_btn_for_click.set_label("Force Push");
                let message = output
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .map(|line| format!("Force pushed with lease: {line}"))
                    .unwrap_or_else(|| "Force pushed with lease.".to_owned());
                apply_action_feedback(
                    &feedback_for_force_push,
                    &toast_for_force_push,
                    &message,
                    true,
                );
            }
            Err(err) => {
                *force_confirmed_for_click.borrow_mut() = false;
                force_push_btn_for_click.set_sensitive(true);
                force_push_btn_for_click.set_label("Force Push");
                apply_action_feedback(
                    &feedback_for_force_push,
                    &toast_for_force_push,
                    &push_branch_error_feedback(&err),
                    true,
                );
            }
        }
    });
}

fn workspace_check_runner_panel(
    db_path: &Path,
    store: &WorkspaceStore,
    name: &str,
    app_state: AppState,
    refresh_hub: RefreshHub,
    toast_overlay: ToastOverlay,
    feedback: &Label,
) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 6);
    panel.append(&section_title("Configured Checks"));

    match store.configured_check_commands(name) {
        Ok(commands) if commands.is_empty() => {
            panel.append(&detail_row(
                "Checks",
                "No test/lint/typecheck/build commands configured.",
            ));
        }
        Ok(commands) => {
            for command in commands {
                let row = make_action_row();
                let label = Label::new(Some(&format!("{}: {}", command.label, command.command)));
                label.set_xalign(0.0);
                label.set_wrap(true);
                label.set_hexpand(true);
                let run_btn = text_button(&format!("Run {}", command.label));
                run_btn.add_css_class("suggested-action");

                let db_for_run = db_path.to_path_buf();
                let workspace_for_run = name.to_owned();
                let key_for_run = command.key.clone();
                let label_for_run = command.label.clone();
                let feedback_for_run = feedback.clone();
                let toast_for_run = toast_overlay.clone();
                let refresh_after_run = refresh_hub.clone();
                run_btn.connect_clicked(move |_| {
                    let result = WorkspaceStore::open_app(db_for_run.clone()).and_then(|store| {
                        store.run_workspace_check(&workspace_for_run, &key_for_run)
                    });
                    match result {
                        Ok(process) => {
                            apply_action_feedback(
                                &feedback_for_run,
                                &toast_for_run,
                                &format!(
                                    "Started {} check as process #{}.",
                                    label_for_run, process.id
                                ),
                                true,
                            );
                            refresh_after_run.refresh_event(
                                RefreshEvent::WorkspaceRuntimeChanged {
                                    workspace: workspace_for_run.clone(),
                                },
                            );
                        }
                        Err(err) => apply_action_feedback(
                            &feedback_for_run,
                            &toast_for_run,
                            &format!("Run {} check failed: {err:#}", label_for_run),
                            true,
                        ),
                    }
                });

                row.append(&label);
                row.append(&run_btn);
                panel.append(&row);
            }
        }
        Err(err) => panel.append(&detail_row(
            "Checks",
            &format!("Could not load configured checks: {err:#}"),
        )),
    }

    let latest_row = make_action_row();
    let latest = Label::new(Some(&latest_check_status_line(store, name)));
    latest.set_xalign(0.0);
    latest.set_wrap(true);
    latest.set_hexpand(true);
    let stage_btn = secondary_button("Queue Latest Check Output");
    let db_for_stage = db_path.to_path_buf();
    let workspace_for_stage = name.to_owned();
    let app_state_for_stage = app_state;
    let feedback_for_stage = feedback.clone();
    let toast_for_stage = toast_overlay;
    stage_btn.connect_clicked(move |_| {
        match WorkspaceStore::open_app(db_for_stage.clone())
            .and_then(|store| latest_check_agent_prompt(&store, &workspace_for_stage))
        {
            Ok(prompt) => {
                app_state_for_stage.queue_pending_chat_prompt(prompt);
                apply_action_feedback(
                    &feedback_for_stage,
                    &toast_for_stage,
                    "Latest local check output queued for the selected agent session.",
                    true,
                );
            }
            Err(err) => apply_action_feedback(
                &feedback_for_stage,
                &toast_for_stage,
                &format!("Could not queue latest check output: {err:#}"),
                true,
            ),
        }
    });
    latest_row.append(&latest);
    latest_row.append(&stage_btn);
    panel.append(&latest_row);
    panel.append(&pull_request_text_section(
        "Latest Local Check Output",
        &latest_check_log_line(store, name),
    ));

    panel
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
    header.append(&workspace_check_runner_panel(
        db_path,
        store,
        name,
        app_state.clone(),
        refresh_hub.clone(),
        toast_overlay.clone(),
        &feedback,
    ));

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
                PullRequestStateKind::Open | PullRequestStateKind::Pending => {
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
                        match WorkspaceStore::open_app(db_for_stage.clone()).and_then(|store| {
                            store.pull_request_readiness_agent_prompt(&workspace_for_stage)
                        }) {
                            Ok(prompt) => {
                                app_state_for_stage.queue_pending_chat_prompt(prompt);
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
                        match WorkspaceStore::open_app(db_for_review.clone()).and_then(|store| {
                            store.pull_request_review_agent_prompt(&workspace_for_review)
                        }) {
                            Ok(prompt) => {
                                app_state_for_review.queue_pending_chat_prompt(prompt);
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
                            WorkspaceStore::open_app(db_for_refresh.clone()).and_then(|store| {
                                store.refresh_pull_request_state(&workspace_for_refresh)
                            });
                        let message = pull_request_refresh_feedback(result);
                        apply_action_feedback(
                            &feedback_for_refresh,
                            &toast_for_refresh,
                            &message,
                            true,
                        );
                        refresh_after.refresh_event(RefreshEvent::WorkspaceReviewChanged {
                            workspace: workspace_for_refresh.clone(),
                        });
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
                        let result =
                            WorkspaceStore::open_app(db_for_merge.clone()).and_then(|store| {
                                store.merge_and_maybe_archive_pull_request(
                                    &workspace_for_merge,
                                    Some(&method),
                                )
                            });
                        let refresh_event =
                            pull_request_merge_refresh_event(&workspace_for_merge, &result);
                        apply_action_feedback(
                            &feedback_for_merge,
                            &toast_for_merge,
                            &pull_request_merge_and_archive_feedback(result),
                            true,
                        );
                        refresh_after_merge.refresh_event(refresh_event);
                    });

                    let db_for_stage = db_path.to_path_buf();
                    let workspace_for_stage = name.to_owned();
                    let app_state_for_stage = app_state.clone();
                    let feedback_for_stage = feedback.clone();
                    let toast_for_stage = toast_overlay.clone();
                    summary_btn.connect_clicked(move |_| {
                        match WorkspaceStore::open_app(db_for_stage.clone()).and_then(|store| {
                            store.pull_request_readiness_agent_prompt(&workspace_for_stage)
                        }) {
                            Ok(prompt) => {
                                app_state_for_stage.queue_pending_chat_prompt(prompt);
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
                            WorkspaceStore::open_app(db_for_refresh.clone()).and_then(|store| {
                                store.refresh_pull_request_state(&workspace_for_refresh)
                            });
                        let message = pull_request_refresh_feedback(result);
                        apply_action_feedback(
                            &feedback_for_refresh,
                            &toast_for_refresh,
                            &message,
                            true,
                        );
                        refresh_after.refresh_event(RefreshEvent::WorkspaceReviewChanged {
                            workspace: workspace_for_refresh.clone(),
                        });
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
                        match WorkspaceStore::open_app(db_for_fix.clone()).and_then(|store| {
                            store.pull_request_checks_agent_prompt(&workspace_for_fix)
                        }) {
                            Ok(prompt) => {
                                app_state_for_fix.queue_pending_chat_prompt(prompt);
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
                        match WorkspaceStore::open_app(db_for_stage.clone()).and_then(|store| {
                            store.pull_request_readiness_agent_prompt(&workspace_for_stage)
                        }) {
                            Ok(prompt) => {
                                app_state_for_stage.queue_pending_chat_prompt(prompt);
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
                            WorkspaceStore::open_app(db_for_refresh.clone()).and_then(|store| {
                                store.refresh_pull_request_state(&workspace_for_refresh)
                            });
                        let message = pull_request_refresh_feedback(result);
                        apply_action_feedback(
                            &feedback_for_refresh,
                            &toast_for_refresh,
                            &message,
                            true,
                        );
                        refresh_after.refresh_event(RefreshEvent::WorkspaceReviewChanged {
                            workspace: workspace_for_refresh.clone(),
                        });
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
                        app_state_for_continue.queue_pending_chat_prompt(prompt);
                    });

                    let db_for_archive = db_path.to_path_buf();
                    let workspace_for_archive = name.to_owned();
                    let refresh_after_archive = refresh_hub.clone();
                    let feedback_for_archive = feedback.clone();
                    let toast_for_archive = toast_overlay.clone();
                    archive_btn.connect_clicked(move |_| {
                        let result = WorkspaceStore::open_app(db_for_archive.clone())
                            .and_then(|store| store.archive(&workspace_for_archive, false));
                        apply_action_feedback(
                            &feedback_for_archive,
                            &toast_for_archive,
                            &pull_request_archive_feedback(result),
                            true,
                        );
                        refresh_after_archive
                            .refresh_event(RefreshEvent::WorkspaceInventoryChanged);
                    });

                    top_row.append(&continue_btn);
                    top_row.append(&archive_btn);
                    actions.append(&top_row);
                }
            }
            if !matches!(status_summary.kind, PullRequestStateKind::Merged) {
                let force_row = make_action_row();
                let force_push_btn = destructive_button("Force Push Lease");
                connect_force_push_button(
                    &force_push_btn,
                    db_path,
                    name,
                    &feedback,
                    &toast_overlay,
                );
                force_row.append(&force_push_btn);
                actions.append(&force_row);
            }
            header.append(&actions);
        }
        None => {
            let status_row = GBox::new(Orientation::Horizontal, 8);
            status_row.add_css_class("ws-pr-nav");
            status_row.set_hexpand(true);
            let empty = Label::new(Some(
                "No pull request yet. Use the Branch / PR controls at the top of the right column.",
            ));
            empty.add_css_class("card-meta");
            empty.set_xalign(0.0);
            empty.set_wrap(true);
            empty.set_hexpand(true);
            status_row.append(&empty);
            header.append(&status_row);
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
        Some(PullRequestStateKind::Pending) => vec!["PR summary", "Reviews", "Refresh"],
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
    toast_manager: ToastManager,
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
            refresh_for_open.refresh_event(RefreshEvent::WorkspaceSelectionChanged);
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
        let toast_for_diff_all = toast_manager.clone();
        diff_all_btn.connect_clicked(move |_| {
            let mut sections = Vec::new();
            for file in &files_for_diff_all {
                let file_path = Path::new(file).to_path_buf();
                match WorkspaceStore::open_app(db_for_diff_all.clone()).and_then(|store| {
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
                        surface_label_error(
                            &feedback_for_diff_all,
                            &toast_for_diff_all,
                            format!("Could not read diff for {file}: {err:#}"),
                        );
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
        let toast_for_copy_all = toast_manager.clone();
        copy_all_btn.connect_clicked(move |_| {
            let mut copied = 0usize;
            let mut failures = Vec::new();
            for file in &files_for_copy_all {
                let result = WorkspaceStore::open_app(db_for_copy_all.clone()).and_then(|store| {
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
                    surface_label_error(
                        &feedback_for_copy_all,
                        &toast_for_copy_all,
                        format!("No files available to copy from {source_workspace}."),
                    );
                }
                (0, false) => {
                    surface_label_error(
                        &feedback_for_copy_all,
                        &toast_for_copy_all,
                        format!(
                            "Failed to copy files from {source_workspace}: {}",
                            failures.join("; ")
                        ),
                    );
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
                    surface_label_error(
                        &feedback_for_copy_all,
                        &toast_for_copy_all,
                        format!(
                            "Copied {copied} file(s) from {source_workspace}, but {} failed: {}",
                            failures.len(),
                            failures.join("; ")
                        ),
                    );
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
                let output = WorkspaceStore::open_app(db_for_diff.clone())
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
            let toast_for_copy = toast_manager.clone();
            copy_btn.connect_clicked(move |_| {
                let result = WorkspaceStore::open_app(db_for_copy.clone()).and_then(|store| {
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
                        surface_label_error(
                            &feedback_for_copy,
                            &toast_for_copy,
                            format!("Could not copy {file_for_copy}: {err:#}"),
                        );
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
        Ok(output) => {
            if let Some(url) = output.lines().find_map(|line| {
                line.trim()
                    .strip_prefix("Existing PR:")
                    .map(str::trim)
                    .filter(|url| !url.is_empty())
            }) {
                return format!("Existing PR: {url}");
            }
            output
                .lines()
                .rev()
                .map(str::trim)
                .find(|line| line.starts_with("https://"))
                .map(|url| format!("Created PR: {url}"))
                .unwrap_or_else(|| "Created PR.".to_owned())
        }
        Err(err) => format!("Create PR failed: {err:#}"),
    }
}

fn push_branch_error_feedback(err: &anyhow::Error) -> String {
    format!(
        "Push branch failed: {err:#}\nSuggested fix: fetch latest origin state, inspect branch divergence, then retry normal push or use Force Push Lease when replacing your own remote branch is intended."
    )
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
    result: anyhow::Result<archductor_core::workspace::MergePullRequestResult>,
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

fn pull_request_merge_refresh_event(
    workspace: &str,
    result: &anyhow::Result<archductor_core::workspace::MergePullRequestResult>,
) -> RefreshEvent {
    if result
        .as_ref()
        .map(|result| result.archived_workspace.is_some())
        .unwrap_or(false)
    {
        RefreshEvent::WorkspaceInventoryChanged
    } else {
        RefreshEvent::WorkspaceReviewChanged {
            workspace: workspace.to_owned(),
        }
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

fn workspace_delete_feedback(result: anyhow::Result<Workspace>) -> String {
    match result {
        Ok(workspace) => format!("Deleted workspace {}.", workspace.name),
        Err(err) => format!("Delete failed: {err:#}"),
    }
}

fn current_workspace_action_target(state: &crate::state::AppState, fallback: &str) -> String {
    state
        .selected_workspace()
        .unwrap_or_else(|| fallback.to_owned())
}

fn workspace_delete_navigation_target(
    result: &std::result::Result<Workspace, String>,
) -> Option<&str> {
    result
        .as_ref()
        .ok()
        .map(|workspace| workspace.name.as_str())
}

fn apply_workspace_delete_navigation_result(
    state: &crate::state::AppState,
    result: &std::result::Result<Workspace, String>,
) {
    if let Some(workspace_name) = workspace_delete_navigation_target(result) {
        state.remove_workspace_from_navigation(workspace_name, crate::state::AppPage::Dashboard);
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
                            let result = WorkspaceStore::open_app(db_for_resolution.clone())
                                .and_then(|store| {
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
                                refresh_after_resolution.refresh_event(
                                    RefreshEvent::WorkspaceReviewChanged {
                                        workspace: workspace_for_resolution.clone(),
                                    },
                                );
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
        match WorkspaceStore::open_app(db_for_add.clone())
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
                refresh_after_add.refresh_event(RefreshEvent::WorkspaceReviewChanged {
                    workspace: workspace_for_add.clone(),
                });
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
        match WorkspaceStore::open_app(db_for_stage.clone()).and_then(|store| {
            let prompt = store.review_comments_agent_prompt(&workspace_for_stage)?;
            if prompt.contains("No open review comments.") {
                anyhow::bail!("No open review comments to stage");
            }
            Ok(prompt)
        }) {
            Ok(prompt) => {
                app_state.queue_pending_chat_prompt(prompt);
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
                    let workspace_for_resolve = name.to_owned();
                    let comment_id = comment.id;
                    let feedback_for_resolve = feedback.clone();
                    let toast_for_resolve = toast_overlay.clone();
                    button.connect_clicked(move |_| {
                        let result = WorkspaceStore::open_app(db_for_resolve.clone())
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
                            refresh_after_resolve.refresh_event(
                                RefreshEvent::WorkspaceReviewChanged {
                                    workspace: workspace_for_resolve.clone(),
                                },
                            );
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
    let db_path = archductor_core::paths::AppPaths::from_env().database_path;
    let workspace = name.to_owned();
    let entry_clone = entry.clone();
    add_btn.connect_clicked(move |_| {
        let text = entry_clone.text().trim().to_owned();
        if text.is_empty() {
            return;
        }
        if let Ok(store) = WorkspaceStore::open_app(db_path.clone()) {
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
    out.push_str("\nChecks\n");
    match store.list_checks(name) {
        Ok(records) if records.is_empty() => out.push_str("No check runs recorded.\n"),
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
        Err(err) => out.push_str(&format!("Could not read checks: {err:#}\n")),
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

fn latest_check_status_line(store: &WorkspaceStore, name: &str) -> String {
    match store.list_checks(name) {
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
            .unwrap_or_else(|| "No local checks recorded.".to_owned()),
        Err(err) => format!("Could not read local checks: {err:#}"),
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

fn latest_check_log_line(store: &WorkspaceStore, name: &str) -> String {
    match store.read_latest_check_log(name) {
        Ok(log) => tail_lines(&log, 16),
        Err(_) => "No local check output yet.".to_owned(),
    }
}

fn latest_check_agent_prompt(store: &WorkspaceStore, name: &str) -> anyhow::Result<String> {
    let record = store
        .list_checks(name)?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("No local check runs recorded"))?;
    let output = store.read_latest_check_log(name)?;
    let output = bounded_check_prompt_output(&output);
    let mut prompt = format!(
        "Address the latest local check output for workspace {name}. If it failed, make the smallest safe fix and rerun the relevant check.\n\nProcess #{}: {}\nStatus: {}\nExit: {}\nStarted: {}\n\nOutput:\n```text\n{}\n```",
        record.id,
        record.command,
        record.status.as_str(),
        exit_code_label(record.exit_code),
        record.started_at,
        output
    );
    append_configured_prompt(
        &mut prompt,
        store
            .resolved_prompt(name, PromptKind::TestFixing)?
            .as_deref(),
    );
    Ok(prompt)
}

fn bounded_check_prompt_output(output: &str) -> String {
    const MAX_BYTES: usize = 32 * 1024;
    let trimmed = output.trim();
    if trimmed.len() <= MAX_BYTES {
        return trimmed.to_owned();
    }
    let mut start = trimmed.len() - MAX_BYTES;
    while start < trimmed.len() && !trimmed.is_char_boundary(start) {
        start += 1;
    }
    format!(
        "[output truncated to last {} KiB]\n{}",
        MAX_BYTES / 1024,
        &trimmed[start..]
    )
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
        emit_toast(toast_overlay, ToastMessage::error(toast_text));
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
        emit_toast(toast_overlay, ToastMessage::info(text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_workspace() -> Workspace {
        Workspace {
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
        }
    }

    fn test_pull_request(state: &str) -> archductor_core::workspace::PullRequest {
        archductor_core::workspace::PullRequest {
            id: 1,
            workspace_id: 2,
            provider: "github".to_owned(),
            number: 42,
            url: "https://github.com/example/demo/pull/42".to_owned(),
            state: state.to_owned(),
            created_at: "then".to_owned(),
            updated_at: "now".to_owned(),
        }
    }

    #[test]
    fn chat_outcome_nav_refresh_ignores_message_only_changes() {
        let messages_only = session_surface::ChatRefreshOutcome {
            messages_changed: true,
            ..Default::default()
        };
        assert!(!chat_outcome_requires_nav_refresh(&messages_only));

        for outcome in [
            session_surface::ChatRefreshOutcome {
                thread_title_changed: true,
                ..Default::default()
            },
            session_surface::ChatRefreshOutcome {
                workspace_name_changed: true,
                ..Default::default()
            },
            session_surface::ChatRefreshOutcome {
                branch_changed: true,
                ..Default::default()
            },
            session_surface::ChatRefreshOutcome {
                session_lifecycle_changed: true,
                ..Default::default()
            },
        ] {
            assert!(chat_outcome_requires_nav_refresh(&outcome));
        }
    }

    #[test]
    fn chat_message_event_matches_selected_workspace_for_background_cache_warm() {
        assert!(chat_message_event_matches_selected_workspace(
            "berlin",
            Some("berlin"),
        ));
        assert!(!chat_message_event_matches_selected_workspace(
            "berlin", None,
        ));
        assert!(!chat_message_event_matches_selected_workspace(
            "berlin",
            Some("tokyo"),
        ));
    }

    #[test]
    fn workspace_phase_status_text_shows_only_pending_or_failed_states() {
        assert_eq!(
            workspace_phase_status_text(Some(WorkspaceUiPhase::Creating {
                detail: "Preparing workspace".to_owned(),
            })),
            Some("Preparing workspace".to_owned())
        );
        assert_eq!(
            workspace_phase_status_text(Some(WorkspaceUiPhase::StartingAgent {
                detail: "Ready".to_owned(),
            })),
            Some("Ready".to_owned())
        );
        assert_eq!(
            workspace_phase_status_text(Some(WorkspaceUiPhase::Failed {
                message: "failed".to_owned(),
            })),
            Some("failed".to_owned())
        );
        assert_eq!(
            workspace_phase_status_text(Some(WorkspaceUiPhase::Ready)),
            None
        );
        assert_eq!(workspace_phase_status_text(None), None);
    }

    #[test]
    fn setup_and_run_prompt_tabs_use_specific_queue_button_labels() {
        assert_eq!(
            workspace_prompt_queue_button_label("Setup Prompt"),
            "Queue Bootstrap Draft"
        );
        assert_eq!(
            workspace_prompt_queue_button_label("Run Prompt"),
            "Queue Launch Draft"
        );
        assert_eq!(
            workspace_prompt_queue_button_label("Review Prompt"),
            "Queue Prompt"
        );
    }

    #[test]
    fn prompt_tab_queue_button_queues_prompt_for_active_chat() {
        let state = AppState::new(
            archductor_core::paths::AppPaths::from_env(),
            Some("berlin".to_owned()),
            WorkspaceTab::Chats,
            crate::AppPage::Workspace,
        );

        queue_workspace_prompt_draft(&state, "  Bootstrap repo setup.  ");

        assert_eq!(
            state.take_pending_chat_prompt().as_deref(),
            Some("Bootstrap repo setup.")
        );
        assert_eq!(state.staged_review_prompt(), None);
    }

    fn test_checks_summary(
        changed_files: usize,
        branch_push_state: Option<archductor_core::workspace::BranchPushState>,
        conflicting_workspaces: Vec<(String, Vec<String>)>,
    ) -> archductor_core::workspace::ChecksSummary {
        archductor_core::workspace::ChecksSummary {
            workspace: test_workspace(),
            changed_files,
            run_status: None,
            session_status: None,
            check_status: None,
            active_sessions: 0,
            pull_request: None,
            open_todos: 0,
            total_todos: 0,
            branch_push_state,
            source_branch_ahead: 0,
            open_review_comments: 0,
            conflicting_workspaces,
        }
    }

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
        assert_eq!(workspace_tab_stack_name(&WorkspaceTab::Todos), "work");
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
            archductor_core::workspace::DiffFileSummary {
                path: "README.md".to_owned(),
                additions: Some(2),
                deletions: Some(1),
                staged: true,
                unstaged: true,
                untracked: false,
            },
            archductor_core::workspace::DiffFileSummary {
                path: "assets/logo.png".to_owned(),
                additions: None,
                deletions: None,
                staged: false,
                unstaged: false,
                untracked: true,
            },
        ];

        let rendered = format_diff_file_summary(&summaries);

        assert!(rendered.contains("Files changed"));
        assert!(rendered.contains("README.md [staged+unstaged] +2 -1"));
        assert!(rendered.contains("assets/logo.png [untracked] binary"));
    }

    #[test]
    fn diff_section_truncates_large_output() {
        let rendered = format_diff_section("Base comparison", Ok("x".repeat(96)), Some(32));

        assert!(rendered.contains("Base comparison"));
        assert!(rendered.contains("Diff truncated after 32 bytes"));
        assert!(rendered.len() < 160);
    }

    #[test]
    fn diff_section_reports_empty_diff() {
        let rendered = format_diff_section("Staged changes", Ok(String::new()), Some(32));

        assert_eq!(rendered, "Staged changes\nNo changes.\n");
    }

    #[test]
    fn browse_file_tree_starts_with_every_directory_collapsed() {
        let mut dir_children = BTreeMap::new();
        dir_children.insert(
            String::new(),
            BTreeSet::from(["src".to_owned(), "tests".to_owned()]),
        );
        dir_children.insert("src".to_owned(), BTreeSet::from(["ui".to_owned()]));

        let collapsed = initial_collapsed_dirs(&dir_children);

        assert!(collapsed.contains("src"));
        assert!(collapsed.contains("src/ui"));
        assert!(collapsed.contains("tests"));
        assert!(!tree_row_visible("src/lib.rs", false, &collapsed));
        assert!(!tree_row_visible("src/ui/panel.rs", false, &collapsed));
        assert!(tree_row_visible("README.md", false, &collapsed));
    }

    #[test]
    fn browse_file_tree_uses_file_browser_icons() {
        assert_eq!(file_tree_icon_name(true, "src"), "folder-symbolic");
        assert_eq!(
            file_tree_icon_name(false, "README.md"),
            "text-x-generic-symbolic"
        );
        assert_eq!(
            file_tree_icon_name(false, "assets/logo.png"),
            "image-x-generic-symbolic"
        );
        assert_eq!(
            file_tree_icon_name(false, "Cargo.toml"),
            "text-x-script-symbolic"
        );
    }

    #[test]
    fn browse_file_tree_uses_balanced_filesystem_spacing() {
        assert_eq!(file_tree_indent_margin_start(0), 0);
        assert_eq!(file_tree_indent_margin_start(1), 12);
        assert_eq!(file_tree_indent_margin_start(3), 36);
        assert_eq!(file_tree_row_spacing(), 4);
    }

    #[test]
    fn review_comment_summary_marks_open_comments_resolvable() {
        let comment = archductor_core::workspace::ReviewComment {
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
            archductor_core::workspace::ReviewComment {
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
            archductor_core::workspace::ReviewComment {
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
            archductor_core::workspace::DiffFileSummary {
                path: "src/lib.rs".to_owned(),
                additions: Some(1),
                deletions: Some(0),
                staged: false,
                unstaged: true,
                untracked: false,
            },
            archductor_core::workspace::DiffFileSummary {
                path: "src/ui/panel.rs".to_owned(),
                additions: Some(3),
                deletions: Some(1),
                staged: true,
                unstaged: false,
                untracked: false,
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
            "    src/ui/panel.rs [staged] +3 -1"
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
    fn workspace_split_position_uses_five_three_center_right_ratio() {
        assert_eq!(workspace_split_position_for_width(1280), 800);
        assert_eq!(workspace_split_position_for_width(700), 437);
        assert_eq!(workspace_split_position_for_width(500), 280);
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
    fn workspace_context_estimate_formats_workspace_and_thread_totals() {
        let estimate = WorkspaceContextEstimate {
            thread_count: 2,
            message_count: 3,
            event_count: 1,
            transcript_bytes: 12_345,
            estimated_tokens: estimate_tokens_from_bytes(12_345),
            threads: vec![WorkspaceThreadContextEstimate {
                title: "Fix parser".to_owned(),
                provider: "codex".to_owned(),
                message_count: 2,
                event_count: 1,
                transcript_bytes: 40,
                estimated_tokens: estimate_tokens_from_bytes(40),
            }],
        };

        let text = format_workspace_context_estimate(&estimate);

        assert!(text.contains("3,087 estimated tokens"));
        assert!(text.contains("12,345 persisted bytes"));
        assert!(text.contains("persisted transcript/event bytes divided by 4"));
        assert!(text.contains("- Fix parser [codex]: 10 tokens, 40 bytes, 2 messages, 1 events"));
    }

    #[test]
    fn commit_scope_labels_use_short_hash_and_subject() {
        assert_eq!(
            commit_scope_label("abcdef1234567890", "fix parser"),
            "Commit abcdef1: fix parser"
        );
        assert_eq!(commit_scope_label("abcdef1234567890", ""), "Commit abcdef1");
    }

    #[test]
    fn workspace_changes_scope_defaults_to_all_changes() {
        let items = vec![
            WorkspaceChangesScopeItem {
                label: "All changes".to_owned(),
                stack_key: "all_changes".to_owned(),
                menu_label: "All changes".to_owned(),
                persisted_scope: "all".to_owned(),
            },
            WorkspaceChangesScopeItem {
                label: "Uncommitted changes".to_owned(),
                stack_key: "uncommitted".to_owned(),
                menu_label: "Uncommitted".to_owned(),
                persisted_scope: "uncommitted".to_owned(),
            },
        ];

        let selected = workspace_changes_selected_scope(&items, None);

        assert_eq!(selected.stack_key, "all_changes");
        assert_eq!(selected.menu_label, "All changes");
    }

    #[test]
    fn changed_file_rows_use_file_style_metadata_with_counts() {
        let summary = archductor_core::workspace::DiffFileSummary {
            path: "src/ui/panel.rs".to_owned(),
            additions: Some(12),
            deletions: Some(3),
            staged: true,
            unstaged: false,
            untracked: false,
        };

        let row = workspace_change_file_row_model(&summary, true);

        assert_eq!(row.icon_name, "text-x-generic-symbolic");
        assert_eq!(row.path, "src/ui/panel.rs");
        assert_eq!(row.counts, "+12 -3");
        assert_eq!(row.state, Some("[staged]"));
    }

    #[test]
    fn changed_file_rows_are_interactive_only_when_open_action_exists() {
        assert_eq!(
            workspace_file_summary_row_kind(false),
            WorkspaceFileSummaryRowKind::Static
        );
        assert_eq!(
            workspace_file_summary_row_kind(true),
            WorkspaceFileSummaryRowKind::Button
        );
    }

    #[test]
    fn summary_query_errors_render_error_scopes() {
        let scope = file_summary_scope_from_result(
            "All changes",
            "all_changes",
            "all",
            Err(anyhow::anyhow!("bad revision")),
        );

        assert_eq!(scope.item.label, "All changes");
        assert_eq!(scope.item.stack_key, "all_changes");
        assert!(scope.message.contains("Could not read All changes"));
        assert!(scope.message.contains("bad revision"));
    }

    #[test]
    fn commit_changes_menu_entry_shows_subject_hash_and_counts() {
        let commit = archductor_core::workspace::CommitFileChangeSummary {
            commit: "abcdef1234567890".to_owned(),
            subject: "feat: wire diff viewer".to_owned(),
            files: vec![
                DiffFileSummary {
                    path: "src/app.rs".to_owned(),
                    additions: Some(5),
                    deletions: Some(1),
                    staged: false,
                    unstaged: false,
                    untracked: false,
                },
                DiffFileSummary {
                    path: "assets/logo.png".to_owned(),
                    additions: None,
                    deletions: None,
                    staged: false,
                    unstaged: false,
                    untracked: false,
                },
            ],
        };

        let entry = commit_changes_menu_entry("commit_0", &commit);

        assert_eq!(entry.title, "feat: wire diff viewer");
        assert_eq!(entry.subtitle.as_deref(), Some("abcdef1"));
        assert_eq!(entry.counts.as_deref(), Some("+5 -1"));
        assert_eq!(entry.item.persisted_scope, "commit:abcdef1234567890");
    }

    #[test]
    fn changes_menu_entries_place_commits_inline_after_last_turn() {
        let mut entries = Vec::new();
        entries.push(ChangesScopeMenuEntry::Item(WorkspaceChangesMenuRow {
            item: WorkspaceChangesScopeItem {
                label: "All changes".to_owned(),
                stack_key: "all_changes".to_owned(),
                menu_label: "All changes".to_owned(),
                persisted_scope: "all".to_owned(),
            },
            title: "All changes".to_owned(),
            subtitle: None,
            counts: None,
        }));
        entries.push(ChangesScopeMenuEntry::Item(WorkspaceChangesMenuRow {
            item: WorkspaceChangesScopeItem {
                label: "Uncommitted changes".to_owned(),
                stack_key: "uncommitted".to_owned(),
                menu_label: "Uncommitted".to_owned(),
                persisted_scope: "uncommitted".to_owned(),
            },
            title: "Uncommitted changes".to_owned(),
            subtitle: None,
            counts: None,
        }));
        entries.push(ChangesScopeMenuEntry::Item(WorkspaceChangesMenuRow {
            item: WorkspaceChangesScopeItem {
                label: "Last turn".to_owned(),
                stack_key: "last_turn".to_owned(),
                menu_label: "Last turn".to_owned(),
                persisted_scope: "last_turn".to_owned(),
            },
            title: "Last turn".to_owned(),
            subtitle: None,
            counts: None,
        }));
        append_commit_changes_menu_entries(
            &mut entries,
            &[archductor_core::workspace::CommitFileChangeSummary {
                commit: "1234567890abcdef".to_owned(),
                subject: "fix: base diff".to_owned(),
                files: vec![DiffFileSummary {
                    path: "README.md".to_owned(),
                    additions: Some(2),
                    deletions: Some(0),
                    staged: false,
                    unstaged: false,
                    untracked: false,
                }],
            }],
        );

        assert_eq!(
            entries
                .iter()
                .map(ChangesScopeMenuEntry::test_label)
                .collect::<Vec<_>>(),
            vec![
                "All changes",
                "Uncommitted changes",
                "Last turn",
                "---",
                "fix: base diff",
            ]
        );
        assert!(!entries.iter().any(|entry| entry.test_label() == "Commits"));
    }

    #[test]
    fn workspace_changes_scope_uses_stable_last_turn_scope() {
        let items = vec![
            WorkspaceChangesScopeItem {
                label: "Uncommitted changes".to_owned(),
                stack_key: "uncommitted".to_owned(),
                menu_label: "Uncommitted".to_owned(),
                persisted_scope: "uncommitted".to_owned(),
            },
            WorkspaceChangesScopeItem {
                label: "Last turn".to_owned(),
                stack_key: "last_turn".to_owned(),
                menu_label: "Last turn".to_owned(),
                persisted_scope: "last_turn".to_owned(),
            },
        ];

        let selected = workspace_changes_selected_scope(&items, Some("turn:thread:7:user:41"));

        assert_eq!(selected.stack_key, "last_turn");
        assert_eq!(selected.menu_label, "Last turn");
    }

    #[test]
    fn workspace_changes_scope_selects_saved_commit_scope() {
        let items = vec![
            WorkspaceChangesScopeItem {
                label: "Uncommitted changes".to_owned(),
                stack_key: "uncommitted".to_owned(),
                menu_label: "Uncommitted".to_owned(),
                persisted_scope: "uncommitted".to_owned(),
            },
            WorkspaceChangesScopeItem {
                label: "Commit abc1234".to_owned(),
                stack_key: "commit_0".to_owned(),
                menu_label: "Commit abc1234".to_owned(),
                persisted_scope: "commit:abc123456789".to_owned(),
            },
        ];

        let selected = workspace_changes_selected_scope(&items, Some("commit:abc123456789"));

        assert_eq!(selected.stack_key, "commit_0");
        assert_eq!(selected.menu_label, "Commit abc1234");
    }

    #[test]
    fn diff_counts_text_formats_text_and_binary_changes() {
        let text = DiffFileSummary {
            path: "README.md".to_owned(),
            additions: Some(12),
            deletions: Some(3),
            staged: false,
            unstaged: true,
            untracked: false,
        };
        let binary = DiffFileSummary {
            path: "assets/logo.png".to_owned(),
            additions: None,
            deletions: None,
            staged: false,
            unstaged: false,
            untracked: true,
        };

        assert_eq!(diff_counts_text(&text), "+12 -3");
        assert_eq!(diff_counts_text(&binary), "binary");
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
    fn workspace_chat_nav_label_keeps_status_out_of_visible_text() {
        let base = crate::background_sync::WorkspaceChatNavItem {
            thread_id: 7,
            title: "Fix auth".to_owned(),
            provider: "codex".to_owned(),
            status: "active".to_owned(),
            running: false,
            unread: false,
            updated_at: "now".to_owned(),
        };
        assert_eq!(workspace_chat_nav_label(&base), "Fix auth");

        let mut running = base.clone();
        running.running = true;
        assert_eq!(workspace_chat_nav_label(&running), "Fix auth");

        let mut unread = running;
        unread.unread = true;
        assert_eq!(workspace_chat_nav_label(&unread), "Fix auth");
    }

    #[test]
    fn workspace_chat_tab_state_reducer_maps_generating_finished_read_selected_and_editing() {
        assert_eq!(
            reduce_workspace_chat_tab_state(WorkspaceChatTabStateInput {
                selected: false,
                generating: true,
                finished_unread: false,
                composer_dirty: false,
            }),
            WorkspaceChatTabVisualState::Generating
        );
        assert_eq!(
            reduce_workspace_chat_tab_state(WorkspaceChatTabStateInput {
                selected: false,
                generating: false,
                finished_unread: true,
                composer_dirty: false,
            }),
            WorkspaceChatTabVisualState::FinishedGenerating
        );
        assert_eq!(
            reduce_workspace_chat_tab_state(WorkspaceChatTabStateInput {
                selected: false,
                generating: false,
                finished_unread: false,
                composer_dirty: false,
            }),
            WorkspaceChatTabVisualState::Read
        );
        assert_eq!(
            reduce_workspace_chat_tab_state(WorkspaceChatTabStateInput {
                selected: true,
                generating: false,
                finished_unread: false,
                composer_dirty: false,
            }),
            WorkspaceChatTabVisualState::Selected
        );
        assert_eq!(
            reduce_workspace_chat_tab_state(WorkspaceChatTabStateInput {
                selected: true,
                generating: true,
                finished_unread: true,
                composer_dirty: true,
            }),
            WorkspaceChatTabVisualState::SelectedGenerating
        );
        assert_eq!(
            reduce_workspace_chat_tab_state(WorkspaceChatTabStateInput {
                selected: true,
                generating: false,
                finished_unread: false,
                composer_dirty: true,
            }),
            WorkspaceChatTabVisualState::Editing
        );
    }

    #[test]
    fn finished_chat_tab_reconciliation_marks_only_nonselected_stopped_threads() {
        let mut previous = HashMap::new();
        previous.insert(
            7,
            crate::background_sync::WorkspaceChatNavItem {
                thread_id: 7,
                title: "Fix auth".to_owned(),
                provider: "codex".to_owned(),
                status: "active".to_owned(),
                running: true,
                unread: true,
                updated_at: "then".to_owned(),
            },
        );
        previous.insert(
            8,
            crate::background_sync::WorkspaceChatNavItem {
                thread_id: 8,
                title: "Selected".to_owned(),
                provider: "codex".to_owned(),
                status: "active".to_owned(),
                running: true,
                unread: false,
                updated_at: "then".to_owned(),
            },
        );
        let mut next = previous.clone();
        next.get_mut(&7).unwrap().running = false;
        next.get_mut(&8).unwrap().running = false;

        let previous = RefCell::new(previous);
        let finished = RefCell::new(HashSet::new());
        reconcile_finished_workspace_chat_tabs(&previous, &finished, &next, Some(8));

        assert!(finished.borrow().contains(&7));
        assert!(!finished.borrow().contains(&8));
    }

    #[test]
    fn chat_tab_interactions_do_not_request_workspace_shell_refresh() {
        let source = include_str!("workspace_command_center.rs");
        let start = source
            .find("let on_threads_changed")
            .expect("chat tab refresh closure exists");
        let end = source[start..]
            .find("let chat_widget = session_surface::agent_session_panel")
            .map(|offset| start + offset)
            .expect("chat widget construction follows chat tabs");
        let chat_region = &source[start..end];

        assert!(
            !chat_region.contains("RefreshScope::Workspace"),
            "chat tab region must not rebuild the workspace shell"
        );
    }

    #[test]
    fn chat_tab_selection_clears_unread_without_duplicate_refresh_events() {
        let source = include_str!("workspace_command_center.rs");
        let start = source.find("let select_tab: Rc<dyn Fn()>").unwrap();
        let end = source[start..]
            .find("let close_tab: Rc<dyn Fn()>")
            .map(|offset| start + offset)
            .unwrap();
        let select_tab_region = &source[start..end];

        assert!(
            !select_tab_region.contains("WorkspaceChatLifecycleChanged")
                && !select_tab_region.contains("WorkspaceChatMessagesChanged"),
            "selecting a chat tab should use the direct thread selection refresh only"
        );
        assert!(
            source.contains("finished_unread_for_click.borrow_mut().remove(&thread_id);"),
            "selected chat tab sync should clear finished unread state locally"
        );
    }

    #[test]
    fn chat_tab_refresh_ignores_message_events_for_other_workspaces() {
        let source = include_str!("workspace_command_center.rs");
        let start = source
            .find("refresh_hub.set_workspace_chat_tabs(move |event| {")
            .expect("chat tab refresh handler exists");
        let end = source[start..]
            .find("let setup_readiness = setup_readiness.clone();")
            .map(|offset| start + offset)
            .expect("chat add button setup follows chat tab handler");
        let handler_region = &source[start..end];

        assert!(
            handler_region.contains("RefreshEvent::WorkspaceChatMessagesChanged { workspace, .. }"),
            "message events should be filtered to the visible workspace before refreshing chat tabs"
        );
        assert!(
            handler_region.contains("RefreshEvent::WorkspaceChatLifecycleChanged { workspace }"),
            "lifecycle events should remain filtered to the visible workspace"
        );
        assert!(
            handler_region.contains("workspace != &workspace_name"),
            "chat tab refreshes for other workspaces must not rebuild visible tabs"
        );
    }

    #[test]
    fn chat_tab_refresh_loads_thread_nav_in_background() {
        let source = include_str!("workspace_command_center.rs");
        let start = source
            .find("refresh_hub.set_workspace_chat_tabs(move |event| {")
            .expect("chat tab refresh handler exists");
        let end = source[start..]
            .find("let setup_readiness = setup_readiness.clone();")
            .map(|offset| start + offset)
            .expect("chat add button setup follows chat tab handler");
        let handler_region = &source[start..end];

        assert!(
            handler_region.contains("spawn_background_job("),
            "chat tab refresh must not load nav snapshots on the GTK thread"
        );
        assert!(
            !handler_region.contains("WorkspaceStore::open_app"),
            "chat tab refresh must not open SQLite on the GTK thread"
        );
        assert!(
            !handler_region.contains("list_chat_threads"),
            "chat tab refresh must not list chat threads on the GTK thread"
        );
    }

    #[test]
    fn chat_tab_snapshot_reuses_open_workspace_store_for_nav() {
        let source = include_str!("workspace_command_center.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("production source exists");
        let start = production
            .find("fn load_workspace_chat_tab_snapshot(")
            .expect("chat tab snapshot loader exists");
        let end = production[start..]
            .find("fn load_workspace_file_snapshot(")
            .map(|offset| start + offset)
            .expect("file snapshot loader follows chat tab snapshot loader");
        let region = &production[start..end];

        assert_eq!(region.matches("WorkspaceStore::open_app(").count(), 1);
        assert!(region.contains("load_workspace_chat_nav(&store"));
        assert!(!region.contains("load_workspace_chat_nav(\n                &db_path"));
    }

    #[test]
    fn workspace_file_open_and_reload_load_snapshots_in_background() {
        let source = include_str!("workspace_command_center.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("production source exists");
        for (name, start_needle, end_needle) in [
            (
                "workspace file tab",
                "let open_file: Rc<dyn Fn(&str)> = Rc::new(move |rel_path: &str| {",
                "let initial_threads = known_threads.borrow().clone();",
            ),
            (
                "workspace files panel",
                "fn workspace_files_panel(",
                "fn list_workspace_files(",
            ),
        ] {
            let start = production.find(start_needle).expect(name);
            let end = production[start..]
                .find(end_needle)
                .map(|offset| start + offset)
                .expect(name);
            let region = &production[start..end];

            assert!(
                region.contains("spawn_background_job("),
                "{name} must load file snapshots off the GTK thread"
            );
            assert!(
                !region.contains("fs::read_to_string"),
                "{name} must not read files on the GTK thread"
            );
            assert!(
                !region.contains("workspace_diff_text_for_path("),
                "{name} must not calculate diffs on the GTK thread"
            );
        }
    }

    #[test]
    fn chat_tab_reopen_close_and_add_do_not_emit_duplicate_refresh_events() {
        let source = include_str!("workspace_command_center.rs");
        for (name, start_needle, end_needle) in [
            (
                "reopen",
                "item.connect_clicked(move |_| {",
                "reopen_menu.append(&item);",
            ),
            (
                "close",
                "let close_tab: Rc<dyn Fn()>",
                "connect_ws_tab_surface_clicks(&tab_shell, select_tab.clone());",
            ),
            (
                "add",
                "add_tab_btn.connect_clicked(move |_| {",
                "// Sync active tab state",
            ),
        ] {
            let start = source.find(start_needle).unwrap();
            let end = source[start..]
                .find(end_needle)
                .map(|offset| start + offset)
                .unwrap();
            let region = &source[start..end];

            assert!(
                !region.contains("WorkspaceChatLifecycleChanged")
                    && !region.contains("WorkspaceChatMessagesChanged"),
                "{name} chat tab handler should not emit duplicate refresh hub events"
            );
        }
    }

    #[test]
    fn workspace_chat_tabs_hide_shell_threads() {
        let shell = ChatThreadRecord {
            id: 1,
            workspace_id: 2,
            provider: "shell".to_owned(),
            title: "Shell Chat 1".to_owned(),
            status: "active".to_owned(),
            native_thread_id: None,
            harness_metadata: None,
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
            archived_at: None,
        };
        let codex = ChatThreadRecord {
            id: 2,
            workspace_id: 2,
            provider: "codex".to_owned(),
            title: "New Chat".to_owned(),
            status: "active".to_owned(),
            native_thread_id: None,
            harness_metadata: None,
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
            archived_at: None,
        };
        let closed = ChatThreadRecord {
            id: 3,
            workspace_id: 2,
            provider: "codex".to_owned(),
            title: "Closed Chat".to_owned(),
            status: "closed".to_owned(),
            native_thread_id: None,
            harness_metadata: None,
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
            archived_at: Some("now".to_owned()),
        };

        assert!(!workspace_chat_thread_is_visible(&shell));
        assert!(workspace_chat_thread_is_visible(&codex));
        assert!(!workspace_chat_thread_is_visible(&closed));
        assert!(workspace_chat_thread_is_reopenable(&closed));
        assert_eq!(workspace_chat_default_title(&[shell, closed]), "New Chat");
    }

    #[test]
    fn startup_chat_selection_uses_leftmost_visible_thread() {
        let closed = ChatThreadRecord {
            id: 1,
            workspace_id: 2,
            provider: "codex".to_owned(),
            title: "Closed".to_owned(),
            status: "closed".to_owned(),
            native_thread_id: None,
            harness_metadata: None,
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
            archived_at: Some("now".to_owned()),
        };
        let leftmost = ChatThreadRecord {
            id: 2,
            workspace_id: 2,
            provider: "claude".to_owned(),
            title: "Claude".to_owned(),
            status: "active".to_owned(),
            native_thread_id: None,
            harness_metadata: None,
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
            archived_at: None,
        };
        let next = ChatThreadRecord {
            id: 3,
            workspace_id: 2,
            provider: "codex".to_owned(),
            title: "Codex".to_owned(),
            status: "active".to_owned(),
            native_thread_id: None,
            harness_metadata: None,
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
            archived_at: None,
        };

        assert_eq!(
            startup_chat_thread_selection(&[closed, leftmost, next]),
            Some(2)
        );
    }

    #[test]
    fn workspace_chat_add_is_disabled_at_limit() {
        let threads = (0..WS_CHAT_TAB_LIMIT)
            .map(|index| ChatThreadRecord {
                id: index as i64,
                workspace_id: 2,
                provider: "codex".to_owned(),
                title: format!("New Chat {}", index + 1),
                status: "active".to_owned(),
                native_thread_id: None,
                harness_metadata: None,
                created_at: "now".to_owned(),
                updated_at: "now".to_owned(),
                archived_at: None,
            })
            .collect::<Vec<_>>();

        assert!(!workspace_chat_can_add_tab(&threads));
        assert!(workspace_chat_can_add_tab(
            &threads[..WS_CHAT_TAB_LIMIT - 1]
        ));
    }

    #[test]
    fn create_pr_chat_prompt_includes_body_instructions_and_context() {
        let prompt = workspace_create_pr_chat_prompt_from_parts(
            "berlin",
            Some("Use the repo PR template."),
            Some("Changed chat tabs."),
        );

        assert!(prompt.contains("Create a GitHub pull request for workspace berlin."));
        assert!(prompt.contains("full PR body"));
        assert!(prompt.contains("summary and testing/verification section"));
        assert!(prompt.contains("Use the repo PR template."));
        assert!(prompt.contains("Changed chat tabs."));
    }

    #[test]
    fn continue_prompt_uses_continue_work_not_create_pr() {
        let prompt = workspace_continue_prompt_from_parts("berlin", Some("Continue carefully."));

        assert!(prompt.contains("Continue carefully."));
        assert!(!prompt.contains("Write a concise PR."));
    }

    #[test]
    fn blocker_prompt_excludes_fix_errors_for_passing_checks() {
        let checks = [archductor_core::workspace::PullRequestCheckRun {
            name: "ci".to_owned(),
            status: "success".to_owned(),
            detail: Some("https://example.test/ci".to_owned()),
        }];
        let failed_checks = failed_pull_request_checks_text(&checks);
        let prompt = workspace_conflict_resolution_prompt_from_parts(
            "berlin",
            failed_checks.as_deref(),
            Some("Resolve configured conflicts."),
            Some("Fix configured errors."),
        );

        assert!(prompt.contains("Resolve configured conflicts."));
        assert!(!prompt.contains("Failed checks:"));
        assert_eq!(prompt.matches("Fix configured errors.").count(), 0);
    }

    #[test]
    fn blocker_prompt_includes_fix_errors_for_failing_checks() {
        let checks = [
            archductor_core::workspace::PullRequestCheckRun {
                name: "lint".to_owned(),
                status: "success".to_owned(),
                detail: None,
            },
            archductor_core::workspace::PullRequestCheckRun {
                name: "ci".to_owned(),
                status: "failure".to_owned(),
                detail: Some("https://example.test/ci".to_owned()),
            },
        ];
        let failed_checks = failed_pull_request_checks_text(&checks);
        let prompt = workspace_conflict_resolution_prompt_from_parts(
            "berlin",
            failed_checks.as_deref(),
            Some("Resolve configured conflicts."),
            Some("Fix configured errors."),
        );

        assert!(prompt.contains("Failed checks:\n- ci: failure - https://example.test/ci"));
        assert!(!prompt.contains("lint: success"));
        assert_eq!(prompt.matches("Fix configured errors.").count(), 1);
    }

    #[test]
    fn pr_primary_action_switches_by_workspace_state() {
        let no_pr = WorkspacePrStatusSnapshot {
            pr: None,
            status: None,
            summary: Some(test_checks_summary(0, None, Vec::new())),
        };
        assert_eq!(workspace_pr_primary_action_label(&no_pr), None);
        assert!(workspace_pr_primary_action(&no_pr).is_none());

        let dirty = WorkspacePrStatusSnapshot {
            pr: None,
            status: None,
            summary: Some(test_checks_summary(3, None, Vec::new())),
        };
        assert_eq!(
            workspace_pr_primary_action_label(&dirty),
            Some("Commit and Push")
        );
        let dirty_action = workspace_pr_primary_action(&dirty).unwrap();
        assert_eq!(
            workspace_pr_status_title(&dirty, dirty_action),
            "Commit and push branch"
        );
        assert_eq!(dirty_action.css_class, "ws-pr-status-muted");

        let needs_push = WorkspacePrStatusSnapshot {
            pr: None,
            status: None,
            summary: Some(test_checks_summary(
                0,
                Some(archductor_core::workspace::BranchPushState {
                    ahead: 2,
                    behind: 0,
                    has_upstream: true,
                }),
                Vec::new(),
            )),
        };
        assert_eq!(
            workspace_pr_primary_action_label(&needs_push),
            Some("Create PR")
        );
        assert_eq!(
            workspace_pr_primary_action(&needs_push).unwrap().css_class,
            "ws-pr-status-missing"
        );

        let mut source_ahead_summary = test_checks_summary(
            0,
            Some(archductor_core::workspace::BranchPushState {
                ahead: 2,
                behind: 0,
                has_upstream: true,
            }),
            Vec::new(),
        );
        source_ahead_summary.source_branch_ahead = 1;
        let source_ahead = WorkspacePrStatusSnapshot {
            pr: None,
            status: None,
            summary: Some(source_ahead_summary),
        };
        assert_eq!(
            workspace_pr_primary_action_label(&source_ahead),
            Some("Merge")
        );
        let source_ahead_action = workspace_pr_primary_action(&source_ahead).unwrap();
        assert_eq!(
            source_ahead_action.kind,
            WorkspacePrTopActionKind::MergeSourceBranch
        );
        assert_eq!(
            workspace_pr_status_title(&source_ahead, source_ahead_action),
            "Merge source branch"
        );

        let clean_without_upstream = WorkspacePrStatusSnapshot {
            pr: None,
            status: None,
            summary: Some(test_checks_summary(
                0,
                Some(archductor_core::workspace::BranchPushState {
                    ahead: 0,
                    behind: 0,
                    has_upstream: false,
                }),
                Vec::new(),
            )),
        };
        assert_eq!(
            workspace_pr_primary_action_label(&clean_without_upstream),
            None
        );

        let snapshot = WorkspacePrStatusSnapshot {
            pr: Some(test_pull_request("OPEN")),
            status: Some(PullRequestStatusSummary {
                label: "ready to merge".to_owned(),
                css_class: "ws-pr-status-ready",
                kind: PullRequestStateKind::Ready,
            }),
            summary: Some(test_checks_summary(0, None, Vec::new())),
        };

        assert_eq!(
            workspace_pr_primary_action_label(&snapshot),
            Some("Merge PR")
        );
        assert_eq!(
            workspace_pr_primary_action(&snapshot).unwrap().css_class,
            "ws-pr-status-ready"
        );

        let blocked = WorkspacePrStatusSnapshot {
            pr: Some(test_pull_request("OPEN")),
            status: Some(PullRequestStatusSummary {
                label: "checks failed".to_owned(),
                css_class: "ws-pr-status-failed",
                kind: PullRequestStateKind::Failed,
            }),
            summary: Some(test_checks_summary(0, None, Vec::new())),
        };
        assert_eq!(
            workspace_pr_primary_action_label(&blocked),
            Some("Fix Checks")
        );
        assert_eq!(
            workspace_pr_primary_action(&blocked).unwrap().css_class,
            "ws-pr-status-failed"
        );

        let conflict = WorkspacePrStatusSnapshot {
            pr: Some(test_pull_request("OPEN")),
            status: Some(PullRequestStatusSummary {
                label: "ready to merge".to_owned(),
                css_class: "ws-pr-status-ready",
                kind: PullRequestStateKind::Ready,
            }),
            summary: Some(test_checks_summary(
                0,
                None,
                vec![("paris".to_owned(), vec!["src/main.rs".to_owned()])],
            )),
        };
        assert_eq!(
            workspace_pr_primary_action(&conflict).unwrap().kind,
            WorkspacePrTopActionKind::FixBlocked
        );
        assert_eq!(
            workspace_pr_primary_action(&conflict).unwrap().css_class,
            "ws-pr-status-failed"
        );

        let merged = WorkspacePrStatusSnapshot {
            pr: Some(test_pull_request("merged")),
            status: Some(PullRequestStatusSummary {
                label: "merged".to_owned(),
                css_class: "ws-pr-status-merged",
                kind: PullRequestStateKind::Merged,
            }),
            summary: Some(test_checks_summary(0, None, Vec::new())),
        };
        assert_eq!(workspace_pr_primary_action_label(&merged), Some("Merged"));
        assert_eq!(
            workspace_pr_primary_action(&merged).unwrap().css_class,
            "ws-pr-status-merged"
        );
    }

    #[test]
    fn pull_request_status_summary_marks_pending_checks() {
        let status = pull_request_status_summary(
            &archductor_core::workspace::PullRequest {
                id: 1,
                workspace_id: 2,
                provider: "github".to_owned(),
                number: 42,
                url: "https://github.com/example/demo/pull/42".to_owned(),
                state: "OPEN".to_owned(),
                created_at: "then".to_owned(),
                updated_at: "now".to_owned(),
            },
            Some(&archductor_core::workspace::PullRequestReadiness {
                review_decision: None,
                latest_reviews: Vec::new(),
                comments: Vec::new(),
                review_threads: Vec::new(),
                checks: vec![archductor_core::workspace::PullRequestCheckRun {
                    name: "ci".to_owned(),
                    status: "in_progress".to_owned(),
                    detail: None,
                }],
                deployments: Vec::new(),
            }),
            &archductor_core::workspace::ChecksSummary {
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
                check_status: None,
                active_sessions: 0,
                pull_request: None,
                open_todos: 0,
                total_todos: 0,
                branch_push_state: None,
                source_branch_ahead: 0,
                open_review_comments: 0,
                conflicting_workspaces: Vec::new(),
            },
        );

        assert_eq!(status.label, "checks pending");
        assert_eq!(status.css_class, "ws-pr-status-pending");
        assert_eq!(status.kind, PullRequestStateKind::Pending);
        assert_eq!(status.attention_label(), Some("checks pending"));
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
        let thread = archductor_core::workspace::PullRequestReviewThread {
            id: Some("PRRT_fake".to_owned()),
            path: Some("src/lib.rs".to_owned()),
            line: Some(42),
            resolved: false,
            comments: vec![archductor_core::workspace::PullRequestThreadComment {
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
            archductor_core::workspace::MergePullRequestResult {
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
            archductor_core::workspace::MergePullRequestResult {
                merge_output: "Merged pull request #42\n".to_owned(),
                archived_workspace: None,
            },
        ));
        assert_eq!(no_archive, "Merged PR: Merged pull request #42");
    }

    #[test]
    fn pull_request_merge_refresh_event_uses_inventory_after_archive() {
        let archived = Ok(archductor_core::workspace::MergePullRequestResult {
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
        });
        assert_eq!(
            pull_request_merge_refresh_event("berlin", &archived),
            RefreshEvent::WorkspaceInventoryChanged
        );

        let not_archived = Ok(archductor_core::workspace::MergePullRequestResult {
            merge_output: "Merged pull request #42\n".to_owned(),
            archived_workspace: None,
        });
        assert_eq!(
            pull_request_merge_refresh_event("berlin", &not_archived),
            RefreshEvent::WorkspaceReviewChanged {
                workspace: "berlin".to_owned()
            }
        );

        let failed: anyhow::Result<archductor_core::workspace::MergePullRequestResult> =
            Err(anyhow::anyhow!("merge failed"));
        assert_eq!(
            pull_request_merge_refresh_event("berlin", &failed),
            RefreshEvent::WorkspaceReviewChanged {
                workspace: "berlin".to_owned()
            }
        );
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
        let success =
            pull_request_refresh_feedback(Ok(Some(archductor_core::workspace::PullRequest {
                id: 1,
                workspace_id: 2,
                provider: "github".to_owned(),
                number: 42,
                url: "https://github.com/example/demo/pull/42".to_owned(),
                state: "MERGED".to_owned(),
                created_at: "now".to_owned(),
                updated_at: "now".to_owned(),
            })));
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
            &archductor_core::workspace::PullRequest {
                id: 1,
                workspace_id: 2,
                provider: "github".to_owned(),
                number: 42,
                url: "https://github.com/example/demo/pull/42".to_owned(),
                state: "OPEN".to_owned(),
                created_at: "then".to_owned(),
                updated_at: "now".to_owned(),
            },
            Some(&archductor_core::workspace::PullRequestReadiness {
                review_decision: None,
                latest_reviews: Vec::new(),
                comments: Vec::new(),
                review_threads: Vec::new(),
                checks: vec![archductor_core::workspace::PullRequestCheckRun {
                    name: "ci".to_owned(),
                    status: "failure".to_owned(),
                    detail: None,
                }],
                deployments: Vec::new(),
            }),
            &archductor_core::workspace::ChecksSummary {
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
                check_status: None,
                active_sessions: 0,
                pull_request: None,
                open_todos: 0,
                total_todos: 0,
                branch_push_state: None,
                source_branch_ahead: 0,
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
            &archductor_core::workspace::PullRequest {
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
            &archductor_core::workspace::ChecksSummary {
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
                check_status: None,
                active_sessions: 0,
                pull_request: None,
                open_todos: 0,
                total_todos: 0,
                branch_push_state: None,
                source_branch_ahead: 0,
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
        let pr = archductor_core::workspace::PullRequest {
            id: 1,
            workspace_id: 2,
            provider: "github".to_owned(),
            number: 42,
            url: "https://github.com/example/demo/pull/42".to_owned(),
            state: "OPEN".to_owned(),
            created_at: "then".to_owned(),
            updated_at: "now".to_owned(),
        };
        let readiness = archductor_core::workspace::PullRequestReadiness {
            review_decision: None,
            latest_reviews: Vec::new(),
            comments: Vec::new(),
            review_threads: Vec::new(),
            checks: vec![archductor_core::workspace::PullRequestCheckRun {
                name: "ci".to_owned(),
                status: "failure".to_owned(),
                detail: None,
            }],
            deployments: Vec::new(),
        };

        let status = pull_request_status_summary_without_checks_summary(&pr, Some(&readiness));

        assert_eq!(status.label, "checks failed");
        assert_eq!(status.css_class, "ws-pr-status-failed");
        assert_eq!(status.kind, PullRequestStateKind::Failed);
        assert_eq!(status.attention_label(), Some("checks failed"));
    }

    #[test]
    fn pull_request_status_summary_keeps_review_blocked_state_grey() {
        let status = pull_request_status_summary(
            &archductor_core::workspace::PullRequest {
                id: 1,
                workspace_id: 2,
                provider: "github".to_owned(),
                number: 42,
                url: "https://github.com/example/demo/pull/42".to_owned(),
                state: "OPEN".to_owned(),
                created_at: "then".to_owned(),
                updated_at: "now".to_owned(),
            },
            Some(&archductor_core::workspace::PullRequestReadiness {
                review_decision: Some("CHANGES_REQUESTED".to_owned()),
                latest_reviews: Vec::new(),
                comments: Vec::new(),
                review_threads: Vec::new(),
                checks: Vec::new(),
                deployments: Vec::new(),
            }),
            &archductor_core::workspace::ChecksSummary {
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
                check_status: None,
                active_sessions: 0,
                pull_request: None,
                open_todos: 0,
                total_todos: 0,
                branch_push_state: None,
                source_branch_ahead: 0,
                open_review_comments: 0,
                conflicting_workspaces: Vec::new(),
            },
        );

        assert_eq!(status.label, "OPEN");
        assert_eq!(status.css_class, "ws-pr-status-muted");
        assert_eq!(status.kind, PullRequestStateKind::Open);
    }

    #[test]
    fn pull_request_review_thread_action_feedback_reports_state_and_id() {
        let success = pull_request_review_thread_action_feedback(
            "Resolve",
            Ok(archductor_core::workspace::PullRequestReviewThread {
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
    fn workspace_delete_feedback_summarizes_permanent_delete() {
        let success = workspace_delete_feedback(Ok(Workspace {
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
        }));
        assert_eq!(success, "Deleted workspace berlin.");

        let failure = workspace_delete_feedback(Err(anyhow::anyhow!("worktree remove failed")));
        assert_eq!(failure, "Delete failed: worktree remove failed");
    }

    #[test]
    fn failed_workspace_creation_status_exposes_delete_action() {
        assert!(workspace_creation_status_allows_delete("failed"));
        assert!(!workspace_creation_status_allows_delete("creating"));
        assert!(!workspace_creation_status_allows_delete("active"));
    }

    #[test]
    fn workspace_timeline_formatter_lists_events() {
        let text = format_workspace_timeline(&[WorkspaceTimelineEvent {
            id: 9,
            workspace_id: 1,
            workspace_name: "berlin".to_owned(),
            kind: "branch.renamed".to_owned(),
            summary: "Renamed branch lc/a to lc/b".to_owned(),
            created_at: "2026-07-09T12:00:00Z".to_owned(),
        }]);

        assert_eq!(
            text,
            "#9 2026-07-09T12:00:00Z [branch.renamed] Renamed branch lc/a to lc/b -> Open Changes\n"
        );
    }

    #[test]
    fn workspace_timeline_formatter_lists_jump_targets() {
        let text = format_workspace_timeline(&[
            WorkspaceTimelineEvent {
                id: 1,
                workspace_id: 1,
                workspace_name: "berlin".to_owned(),
                kind: "prompt.submitted".to_owned(),
                summary: "Build the fix".to_owned(),
                created_at: "2026-07-09T12:00:00Z".to_owned(),
            },
            WorkspaceTimelineEvent {
                id: 2,
                workspace_id: 1,
                workspace_name: "berlin".to_owned(),
                kind: "run.failed".to_owned(),
                summary: "cargo test failed".to_owned(),
                created_at: "2026-07-09T12:01:00Z".to_owned(),
            },
            WorkspaceTimelineEvent {
                id: 3,
                workspace_id: 1,
                workspace_name: "berlin".to_owned(),
                kind: "pull_request.ready".to_owned(),
                summary: "PR ready".to_owned(),
                created_at: "2026-07-09T12:02:00Z".to_owned(),
            },
        ]);

        assert!(text.contains("prompt.submitted] Build the fix -> Open Chat"));
        assert!(text.contains("run.failed] cargo test failed -> Open Checks"));
        assert!(text.contains("pull_request.ready] PR ready -> Open Review"));
    }

    #[test]
    fn workspace_timeline_formatter_trims_long_lists() {
        let events = (1..=205)
            .map(|id| WorkspaceTimelineEvent {
                id,
                workspace_id: 1,
                workspace_name: "berlin".to_owned(),
                kind: "session.event".to_owned(),
                summary: format!("Event {id}"),
                created_at: "2026-07-09T12:00:00Z".to_owned(),
            })
            .collect::<Vec<_>>();

        let text = format_workspace_timeline(&events);

        assert!(text.starts_with("Showing latest 200 events; 5 older hidden.\n"));
        assert!(!text.contains("#5 2026-07-09T12:00:00Z"));
        assert!(text.contains("#6 2026-07-09T12:00:00Z"));
        assert!(text.contains("#205 2026-07-09T12:00:00Z"));
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

    #[test]
    fn workspace_delete_navigation_target_is_only_returned_for_successful_delete() {
        let success = Ok(test_workspace());
        assert_eq!(workspace_delete_navigation_target(&success), Some("berlin"));

        let failure: std::result::Result<Workspace, String> =
            Err("worktree remove failed".to_owned());
        assert_eq!(workspace_delete_navigation_target(&failure), None);
    }

    #[test]
    fn workspace_delete_navigation_result_mutates_state_only_after_success() {
        let state = crate::state::AppState::new(
            archductor_core::paths::AppPaths::from_env(),
            Some("berlin".to_owned()),
            crate::state::WorkspaceTab::Chats,
            crate::state::AppPage::Workspace,
        );
        state.set_selected_chat_thread(Some(42));
        state.navigate_to_page(crate::state::AppPage::History);

        let failure: std::result::Result<Workspace, String> =
            Err("worktree remove failed".to_owned());
        apply_workspace_delete_navigation_result(&state, &failure);
        let failed_snapshot = state.snapshot();

        assert_eq!(
            failed_snapshot.selected_workspace.as_deref(),
            Some("berlin")
        );
        assert_eq!(failed_snapshot.selected_chat_thread, Some(42));
        assert_eq!(failed_snapshot.active_page, crate::state::AppPage::History);

        apply_workspace_delete_navigation_result(&state, &Ok(test_workspace()));
        let success_snapshot = state.snapshot();

        assert_eq!(success_snapshot.selected_workspace, None);
        assert_eq!(success_snapshot.selected_chat_thread, None);
        assert_eq!(success_snapshot.active_page, crate::state::AppPage::History);
        while state.navigate_back() {
            assert_ne!(
                state.snapshot().selected_workspace.as_deref(),
                Some("berlin")
            );
        }
    }

    #[test]
    fn workspace_action_target_prefers_current_selection_after_metadata_rename() {
        let state = crate::state::AppState::new(
            archductor_core::paths::AppPaths::from_env(),
            Some("renamed-workspace".to_owned()),
            crate::state::WorkspaceTab::Chats,
            crate::state::AppPage::Workspace,
        );

        assert_eq!(
            current_workspace_action_target(&state, "1"),
            "renamed-workspace"
        );
    }

    #[test]
    fn workspace_action_rename_uses_metadata_refresh() {
        let source = include_str!("workspace_command_center.rs");
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("workspace command center source should contain production code");

        assert!(production_source.contains("RefreshEvent::WorkspaceMetadataChanged"));
        assert!(!production_source.contains(
            "state_after_rename.set_selected_workspace(Some(workspace.name.clone()));\n                progress_rename.set_text(&format!(\"Renamed to {}\", workspace.name));\n            }\n            Err(err) => apply_runtime_action_feedback(\n                &progress_rename,\n                &toast_rename,\n                lifecycle_action_failure_feedback(\"Rename\", &err),\n            ),\n        }\n        refresh_after_rename.refresh_event(RefreshEvent::WorkspaceInventoryChanged);"
        ));
    }

    #[test]
    fn branch_checkout_and_rename_publish_metadata_refresh_with_branch() {
        let source = include_str!("workspace_command_center.rs");
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("workspace command center source should contain production code");
        let branch_panel = production_source
            .split("fn workspace_branch_panel")
            .nth(1)
            .and_then(|source| source.split("fn workspace_branch_state_text").next())
            .expect("workspace branch panel should exist");

        assert!(branch_panel.contains("RefreshEvent::WorkspaceMetadataChanged"));
        assert!(branch_panel.contains("branch: Some(workspace.branch.clone())"));
    }

    #[test]
    fn workspace_lifecycle_actions_use_current_selected_workspace_name() {
        let source = include_str!("workspace_command_center.rs");
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("workspace command center source should contain production code");
        let action_handler = production_source
            .split("for (button, action) in [")
            .nth(1)
            .and_then(|source| source.split("let result =").next())
            .expect("workspace lifecycle action handler should exist");

        assert!(
            action_handler.contains("current_workspace_action_target(&state_after_action, &workspace)"),
            "workspace lifecycle actions must use the current selected workspace after agent metadata renames"
        );
    }

    #[test]
    fn cloned_external_thread_selection_drops_borrow_before_callback_runs() {
        let controller: session_surface::ExternalThreadSelectionController =
            Rc::new(RefCell::new(None));
        let observed = Rc::new(RefCell::new(None));
        *controller.borrow_mut() = Some(Rc::new({
            let controller = controller.clone();
            let observed = observed.clone();
            move |thread_id| {
                let _same_controller = clone_external_thread_selection_controller(&controller);
                *observed.borrow_mut() = thread_id;
            }
        }));

        let Some(select_thread) = clone_external_thread_selection_controller(&controller) else {
            panic!("expected selection controller");
        };
        select_thread(Some(42));

        assert_eq!(*observed.borrow(), Some(42));
    }

    #[test]
    fn chat_tab_refresh_drops_selected_thread_borrow_before_callback_runs() {
        let source = include_str!("workspace_command_center.rs");
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("workspace command center source should contain production code");

        assert!(
            !production_source
                .contains("(on_threads_changed)(threads, *selected_thread.borrow());"),
            "chat tab refresh must copy selected_thread before invoking on_threads_changed"
        );
    }

    #[test]
    fn workspace_chat_add_does_not_create_thread_sync_in_click_handler() {
        let source = include_str!("workspace_command_center.rs");
        let start = source
            .find("add_tab_btn.connect_clicked")
            .expect("add chat button handler exists");
        let end = source[start..]
            .find("content.connect_visible_child_name_notify")
            .map(|offset| start + offset)
            .expect("add chat handler end exists");
        let handler = &source[start..end];

        assert!(
            !handler.contains("store.create_chat_thread("),
            "chat creation must be spawned so GTK stays responsive"
        );
        assert!(
            handler.contains("create_pending_chat_target"),
            "chat creation must select an optimistic pending chat immediately"
        );
    }

    #[test]
    fn workspace_chat_create_failure_surfaces_toast_and_log() {
        let source = include_str!("workspace_command_center.rs");
        let create_fn = source
            .split("fn spawn_workspace_chat_thread_create")
            .nth(1)
            .and_then(|rest| {
                rest.split("fn default_launchable_chat_provider_for_workspace")
                    .next()
            })
            .expect("workspace chat create helper exists");

        assert!(create_fn.contains("ChatUiPhase::Failed"));
        assert!(create_fn.contains("toast_manager.error"));
        assert!(create_fn.contains("failed to create workspace chat thread"));
    }
}
