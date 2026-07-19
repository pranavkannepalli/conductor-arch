use archductor_core::agent_tools::{
    launchable_agent_tools, launchable_provider_key, tool_by_provider,
};
use archductor_core::archcar::harness::managed_harness_for_kind;
use archductor_core::archcar::harness_contract::{
    HarnessCapability, HarnessDescriptor, ProviderInteractionKind, RequiredHarnessFeature,
    SupportMode,
};
use archductor_core::archcar::protocol::{
    ArchcarEvent, ArchcarInputDelivery, ArchcarInputKind, ArchcarResponse,
};
use archductor_core::archcar::session::CODEX_RECOVERY_CURRENT_USER_MESSAGE_HEADER;
use archductor_core::codex_tui::{
    parse_codex_context_usage, parse_codex_file_change_block, parse_codex_inline_event,
    CodexFileChangeAction as CoreCodexFileChangeAction,
    CodexFileReference as CoreCodexFileReference, CodexInlineEvent as CoreCodexInlineEvent,
    CodexTranscriptEvent,
};
use archductor_core::doctor::SetupReadiness;
use archductor_core::model_registry::{model_choices_for_provider, CODEX_DEFAULT_MODEL};
use archductor_core::provider_events::{
    ProviderEventKind, ProviderEventPhase, ProviderEventRecord, ProviderEventStore,
};
use archductor_core::provider_interactions::ProviderInteractionRecord;
use archductor_core::provider_projection::{
    provider_projection_from_records, provider_projection_item_is_relevant_chat_event,
    ProjectionRenderClass, ProviderProjectionCategory, ProviderProjectionItem,
    ProviderProjectionStatus, ProviderProjectionStreamState,
};
#[cfg(test)]
use archductor_core::session_state::AgentSessionState;
use archductor_core::workflow_actions::gtk_live_controls_for_provider;
use archductor_core::workspace::{
    strip_archductor_metadata_block, ChatEventRecord, ChatMessageRecord, ChatThreadRecord,
    ProcessRecord, ProcessStatus, SessionHarnessOptions, SessionKind, WorkspaceStore,
};
use gtk::prelude::*;
use gtk::{
    Adjustment, Align, Box as GBox, Button, CheckButton, ComboBoxText, DrawingArea, Entry,
    EventControllerKey, GestureClick, Image, Label, Orientation, Overlay, Popover, Revealer,
    RevealerTransitionType, ScrolledWindow, Spinner, TextBuffer, TextView, ToggleButton, Widget,
};
use std::any::Any;
use std::backtrace::Backtrace;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::f64::consts::TAU;
use std::fs;
use std::io::Read;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{env, fs as stdfs};
use tracing::{debug, error, info, trace, warn};

use crate::archcar_async::{
    clear_archcar_ready, note_archcar_ready, spawn_background_job, AsyncArchcarBridge,
    AsyncArchcarMessage, AsyncArchcarResponse,
};
use crate::buttons::{
    icon_button, resolve_icon_name, style_icon_button, style_text_button, text_button,
};
use crate::motion::{append_revealed, clear_box};
use crate::refresh::RefreshEvent;
use crate::state::{
    AppPage, AppState, AppStateSnapshot, ChatUiPhase, ChatUiTarget, QueuedChatInputDraft,
};
use crate::terminal::terminal_display_text;
use crate::toast::ToastManager;

const SESSION_SCROLLBACK_LINES: usize = 2_000;
const SESSION_TAIL_HISTORY: usize = 120;
const DEFAULT_CHAT_TITLE_PREFIX: &str = "New Chat";
const REVEAL_EXISTING_CHAT_REFRESH_ROWS: bool = false;
const CONTEXT_WARNING_PERCENT: u8 = 70;
const CONTEXT_COMPACTION_RISK_PERCENT: u8 = 90;
#[cfg(test)]
const CONTEXT_DETAIL_HISTORY_LIMIT: usize = 6;
#[cfg(test)]
const CONTEXT_DETAIL_CONTRIBUTOR_LIMIT: usize = 5;
const CHAT_SCROLL_BOTTOM_EPSILON: f64 = 48.0;
const CHAT_SCROLL_RESTORE_LAYOUT_PASSES: u8 = 4;
const CHAT_SCROLL_RESTORE_LAYOUT_PASS_MS: u64 = 16;
const CHAT_REFRESH_WAKE_DELAY_MS: u64 = 150;
const INLINE_EVENT_BODY_MAX_HEIGHT: i32 = 220;
static NEXT_CHAT_WAKE_ID: AtomicUsize = AtomicUsize::new(1);

thread_local! {
    static CHAT_WAKE_REGISTRY: RefCell<HashMap<usize, Rc<dyn Fn()>>> = RefCell::new(HashMap::new());
}

pub type ExternalThreadSelectionController = Rc<RefCell<Option<Rc<dyn Fn(Option<i64>)>>>>;
type RefreshChatSurfaceController = Rc<RefCell<Option<Rc<dyn Fn()>>>>;
pub(crate) type RegisterChatSurfaceRefresh = Rc<dyn Fn(Rc<dyn Fn(ChatRefreshKind)>)>;
type SwitchChatHarnessController = Rc<RefCell<Option<Rc<dyn Fn(SessionKind)>>>>;
type SendChatTextController = Rc<RefCell<Option<Rc<dyn Fn(String, bool) -> bool>>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChatRefreshKind {
    Full,
    Messages { thread_id: i64 },
    ThreadNav,
}

pub(crate) fn chat_refresh_kind_for_event(event: &RefreshEvent) -> ChatRefreshKind {
    match event {
        RefreshEvent::WorkspaceChatMessagesChanged { thread_id, .. } => ChatRefreshKind::Messages {
            thread_id: *thread_id,
        },
        RefreshEvent::WorkspaceChatLifecycleChanged { .. } => ChatRefreshKind::ThreadNav,
        _ => ChatRefreshKind::Full,
    }
}

fn dispatch_chat_surface_refresh_kind(
    kind: ChatRefreshKind,
    selected_thread: Option<i64>,
    refresh_full: &dyn Fn(),
    refresh_messages: &dyn Fn(i64),
    refresh_thread_nav: &dyn Fn(),
) {
    match kind {
        ChatRefreshKind::Full => refresh_full(),
        ChatRefreshKind::Messages { thread_id } => {
            if selected_thread == Some(thread_id) {
                refresh_messages(thread_id);
            }
        }
        ChatRefreshKind::ThreadNav => refresh_thread_nav(),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ChatRefreshOutcome {
    pub messages_changed: bool,
    pub thread_title_changed: bool,
    pub workspace_name_changed: bool,
    pub branch_changed: bool,
    pub session_lifecycle_changed: bool,
    pub provider_controls_changed: bool,
    pub composer_state_changed: bool,
}

impl ChatRefreshOutcome {
    pub(crate) fn requires_nav_refresh(&self) -> bool {
        self.thread_title_changed
            || self.workspace_name_changed
            || self.branch_changed
            || self.session_lifecycle_changed
    }
}

fn clone_refresh_chat_surface_controller(
    controller: &RefreshChatSurfaceController,
) -> Option<Rc<dyn Fn()>> {
    controller.borrow().as_ref().cloned()
}

fn selected_harness_snapshot(selected_harness: &RefCell<SessionKind>) -> SessionKind {
    *selected_harness.borrow()
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ChatScrollSnapshot {
    value: f64,
    pinned_to_bottom: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatStatusBannerKind {
    None,
    Startup,
    Working,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexInlineEventKind {
    Tool,
    Skill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexInlineEventStatus {
    Loading,
    Complete,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CodexInlineEvent {
    kind: CodexInlineEventKind,
    title: String,
    subtitle: Option<String>,
    body: Option<String>,
    path: Option<PathBuf>,
    status: CodexInlineEventStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ChatTimelineItem {
    Message(ChatMessageRecord),
    Event(ChatEventRecord),
    ProviderProjection(ProviderProjectionItem),
    InterruptedNotice { sequence: i64 },
    OptimisticUserInput(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ChatTimelineItemKey {
    Message(ChatRenderMessageSignature),
    Event(ChatRenderEventSignature),
    Provider(ChatRenderProviderEventSignature),
    InterruptedNotice(i64),
    OptimisticUserInput(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChatTimelineRenderState {
    thread_id: i64,
    transcript_display: String,
    leading_rows: usize,
    keys: Vec<ChatTimelineItemKey>,
}

struct ChatTimelineSnapshot {
    thread_messages: Vec<ChatMessageRecord>,
    thread_events: Vec<ChatEventRecord>,
    provider_events: Vec<ProviderEventRecord>,
    transcript_display: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatTimelineRefreshPlan {
    Skip,
    Append { start: usize },
    RebuildFrom { start: usize },
    RebuildMessages,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CodexContextUsage {
    used_tokens: Option<u64>,
    max_tokens: Option<u64>,
    percent: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InlineEventBodyPreview {
    preview: String,
    full: String,
    truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveChatSource {
    StructuredStore,
}

fn live_chat_source() -> LiveChatSource {
    LiveChatSource::StructuredStore
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatRefreshWorkspaceDecision {
    Refresh,
    SkipStaleSurface,
    ClearDeletedWorkspace,
}

fn chat_refresh_workspace_decision(
    workspace_name: &str,
    snapshot: &AppStateSnapshot,
    workspace_exists: bool,
) -> ChatRefreshWorkspaceDecision {
    if snapshot.selected_workspace.as_deref() != Some(workspace_name)
        || !matches!(snapshot.active_page, AppPage::Workspace | AppPage::Review)
    {
        return ChatRefreshWorkspaceDecision::SkipStaleSurface;
    }
    if !workspace_exists {
        return ChatRefreshWorkspaceDecision::ClearDeletedWorkspace;
    }
    ChatRefreshWorkspaceDecision::Refresh
}

fn workspace_lookup_returned_no_rows(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<rusqlite::Error>(),
            Some(rusqlite::Error::QueryReturnedNoRows)
        )
    })
}

fn clear_deleted_workspace_surface(
    app_state: &AppState,
    workspace_name: &str,
    thread_row: &GBox,
    messages: &GBox,
    context_usage: &ContextUsageWidget,
) {
    app_state.remove_workspace_from_navigation(workspace_name, AppPage::Dashboard);
    clear_box(thread_row);
    clear_box(messages);
    apply_context_usage_state(context_usage, None);
}

fn load_chat_timeline_snapshot(
    database_path: PathBuf,
    workspace: String,
    thread_id: i64,
) -> Result<ChatTimelineSnapshot, String> {
    WorkspaceStore::open_app(database_path.clone())
        .and_then(|store| {
            let thread_messages = store.list_chat_messages(thread_id)?;
            let thread_events = store.list_chat_events(thread_id)?;
            let provider_events =
                ProviderEventStore::new(database_path.clone()).list_for_chat_thread(thread_id)?;
            let transcript_display = transcript_display_for_workspace(&database_path, &workspace);
            Ok(ChatTimelineSnapshot {
                thread_messages,
                thread_events,
                provider_events,
                transcript_display,
            })
        })
        .map_err(|err| format!("{err:#}"))
}

struct SessionChatCreateUi {
    app_state: AppState,
    selected_thread: Rc<RefCell<Option<i64>>>,
    thread_state: Rc<RefCell<Vec<ChatThreadRecord>>>,
    external_chat_tabs: Option<ExternalChatTabs>,
    refresh_view: Rc<dyn Fn()>,
    eager_start: Rc<dyn Fn(String, i64, SessionKind)>,
    on_selected: Rc<dyn Fn()>,
    toast_manager: ToastManager,
}

fn spawn_session_chat_thread_create(
    database_path: PathBuf,
    workspace_name: String,
    kind: SessionKind,
    title: String,
    metadata: Option<String>,
    pending_target: ChatUiTarget,
    ui: SessionChatCreateUi,
) {
    let workspace_for_job = workspace_name.clone();
    spawn_background_job(
        move || {
            WorkspaceStore::open_app(database_path)
                .and_then(|store| {
                    store.create_chat_thread(
                        &workspace_for_job,
                        session_kind_provider(kind),
                        &title,
                        metadata.as_deref(),
                    )
                })
                .map_err(|err| format!("{err:#}"))
        },
        move |result| match result {
            Ok(thread) => {
                ui.thread_state.borrow_mut().insert(0, thread.clone());
                ui.app_state
                    .resolve_pending_chat_target(pending_target.clone(), thread.id);
                *ui.selected_thread.borrow_mut() = Some(thread.id);
                (ui.on_selected)();
                if let Some(external_chat_tabs) = ui.external_chat_tabs.as_ref() {
                    (external_chat_tabs.on_threads_changed)(
                        ui.thread_state.borrow().clone(),
                        Some(thread.id),
                    );
                }
                (ui.eager_start)(workspace_name.clone(), thread.id, kind);
                (ui.refresh_view)();
            }
            Err(message) => {
                ui.app_state.mark_chat_phase(
                    pending_target,
                    ChatUiPhase::Failed {
                        message: format!("Create chat thread failed: {message}"),
                    },
                );
                ui.toast_manager
                    .error(format!("Create chat thread failed: {message}"));
                error!(workspace = %workspace_name, error = %message, "failed to create chat thread");
            }
        },
    );
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ContextUsageDisplayState {
    percent_label: String,
    css_class: &'static str,
}

#[derive(Clone)]
struct ContextUsageWidget {
    container: GBox,
    donut: DrawingArea,
    label: Label,
    percent: Rc<RefCell<Option<u8>>>,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct ContextUsageHistoryPoint {
    label: String,
    usage: CodexContextUsage,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct ContextDetailSummary {
    usage: Option<CodexContextUsage>,
    transcript_bytes: usize,
    estimated_tokens: u64,
    estimate_method: &'static str,
    recent_growth: String,
    history: Vec<ContextUsageHistoryPoint>,
    compaction_events: Vec<String>,
    top_contributors: Vec<String>,
    message_count: usize,
    event_count: usize,
}

type ChatRenderRecordSignature = (i64, Option<i64>, ProcessStatus, Option<i32>, Option<String>);
type ChatRenderThreadSignature = (i64, String, String, String, String);
type ChatRenderMessageSignature = (i64, String, Option<i64>, String, String);
type ChatRenderEventSignature = (i64, String, i64, String, String, usize);
type ChatRenderProviderEventSignature = (
    String,
    i64,
    Option<i64>,
    ProviderEventKind,
    ProviderEventPhase,
    usize,
);
type ChatRenderPendingInputSignature = (usize, String);
type ChatThreadNavSignature = (i64, String, String, String, Option<String>);

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChatRenderSignature {
    current_kind: SessionKind,
    selected_thread_id: Option<i64>,
    active_record: Option<i64>,
    startup_state: CodexStartupState,
    working_elapsed_seconds: Option<u64>,
    records: Vec<ChatRenderRecordSignature>,
    threads: Vec<ChatRenderThreadSignature>,
    messages: Vec<ChatRenderMessageSignature>,
    events: Vec<ChatRenderEventSignature>,
    provider_events: Vec<ChatRenderProviderEventSignature>,
    pending_inputs: Vec<ChatRenderPendingInputSignature>,
    transcript_display: String,
    render_state: &'static str,
    runtime_summary: Option<String>,
}

fn chat_thread_nav_signature(threads: &[ChatThreadRecord]) -> Vec<ChatThreadNavSignature> {
    threads
        .iter()
        .map(|thread| {
            (
                thread.id,
                thread.provider.clone(),
                thread.title.clone(),
                thread.status.clone(),
                thread.archived_at.clone(),
            )
        })
        .collect()
}

#[derive(Clone)]
pub(crate) struct ExternalChatTabs {
    pub on_threads_changed: Rc<dyn Fn(Vec<ChatThreadRecord>, Option<i64>)>,
    pub selection_controller: ExternalThreadSelectionController,
    pub on_workspace_metadata_changed: Rc<dyn Fn(&AgentMetadataUiUpdate)>,
    pub on_chat_surface_refresh_ready: Option<RegisterChatSurfaceRefresh>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentMetadataUiUpdate {
    pub workspace_name: String,
    pub branch_name: String,
    pub thread: ChatThreadRecord,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct AgentMetadataUiChanges {
    workspace_changed: bool,
    branch_changed: bool,
    chat_title_changed: bool,
}

#[derive(Clone)]
struct EditorChoice {
    name: String,
    icon: &'static str,
    command: String,
}

fn session_action_row() -> GBox {
    let row = GBox::new(Orientation::Horizontal, 8);
    row.add_css_class("action-row");
    row
}

fn session_secondary_button(label: &str) -> Button {
    let button = text_button(label);
    button.add_css_class("secondary-action");
    button
}

fn session_flat_button(label: &str) -> Button {
    let button = text_button(label);
    button.add_css_class("flat-action");
    button
}

fn session_destructive_button(label: &str) -> Button {
    let button = text_button(label);
    button.add_css_class("destructive-action");
    button
}

pub fn agent_session_panel(
    database_path: PathBuf,
    _workspace_name: &str,
    repository_name: &str,
    branch_name: &str,
    collapse_sidebar: Rc<dyn Fn()>,
    app_state: AppState,
    refresh: impl Fn() + Clone + 'static,
    include_header: bool,
    setup_readiness: Option<Rc<RefCell<SetupReadiness>>>,
    external_chat_tabs: Option<ExternalChatTabs>,
    toast_manager: ToastManager,
) -> GBox {
    let root = GBox::new(Orientation::Vertical, 0);
    root.add_css_class("chat-surface");
    root.set_vexpand(true);
    root.set_hexpand(true);
    let current_workspace_name = Rc::new(RefCell::new(_workspace_name.to_owned()));
    let current_branch_name = Rc::new(RefCell::new(branch_name.to_owned()));

    if include_header {
        root.append(&session_header_row(
            repository_name,
            branch_name,
            collapse_sidebar.clone(),
        ));
    }

    let setup_readiness =
        setup_readiness.unwrap_or_else(|| Rc::new(RefCell::new(SetupReadiness::from_host())));
    let initial_harness = {
        let readiness = setup_readiness.borrow();
        initial_chat_harness_from_setup(&database_path, _workspace_name, &readiness)
    };
    let selected_harness = Rc::new(RefCell::new(initial_harness));
    let selected_model = Rc::new(RefCell::new(None::<String>));
    let reasoning_mode = Rc::new(RefCell::new(Some("high".to_owned())));
    let thread_state = Rc::new(RefCell::new(Vec::<ChatThreadRecord>::new()));
    let selected_thread: Rc<RefCell<Option<i64>>> =
        Rc::new(RefCell::new(app_state.selected_chat_thread()));
    let composer_drafts = Rc::new(RefCell::new(HashMap::<i64, String>::new()));
    let restoring_composer_draft = Rc::new(Cell::new(false));
    let pending_commands = Rc::new(RefCell::new(HashMap::<i64, Vec<String>>::new()));
    let pending_archcar_inputs =
        Rc::new(RefCell::new(HashMap::<i64, Vec<QueuedArchcarInput>>::new()));
    let queued_auto_drain_holds = Rc::new(RefCell::new(HashSet::<i64>::new()));
    let codex_ready: Rc<RefCell<bool>> = Rc::new(RefCell::new(true));
    let codex_startup_states = Rc::new(RefCell::new(HashMap::<i64, CodexStartupState>::new()));
    let archcar_bridge = AsyncArchcarBridge::new(app_state.paths.clone());
    let archcar_ready_cache = Rc::new(RefCell::new(HashMap::<i64, bool>::new()));
    let archcar_session_threads = Rc::new(RefCell::new(HashMap::<i64, i64>::new()));
    let inflight_archcar_actions =
        Rc::new(RefCell::new(HashMap::<u64, PendingArchcarAction>::new()));
    let working_threads = Rc::new(RefCell::new(HashMap::<i64, Instant>::new()));
    let bridge_error_state = Rc::new(RefCell::new(BridgeErrorUiState::default()));
    let refresh_chat_surface: RefreshChatSurfaceController = Rc::new(RefCell::new(None));
    let switch_chat_harness: SwitchChatHarnessController = Rc::new(RefCell::new(None));
    let sync_live_controls: RefreshChatSurfaceController = Rc::new(RefCell::new(None));
    let send_text_after_ready_queue: SendChatTextController = Rc::new(RefCell::new(None));
    let send_immediate_after_ready_queue: SendChatTextController = Rc::new(RefCell::new(None));
    let refresh_queue_overlay: RefreshChatSurfaceController = Rc::new(RefCell::new(None));

    let thread_row = GBox::new(Orientation::Horizontal, 8);
    thread_row.add_css_class("chat-thread-row");
    thread_row.set_hexpand(true);
    if external_chat_tabs.is_none() {
        root.append(&thread_row);
    }

    // ── Messages scroll area ──────────────────────────────────────────
    let messages = GBox::new(Orientation::Vertical, 0);
    messages.add_css_class("chat-messages");
    messages.set_vexpand(true);
    messages.set_hexpand(true);

    let scroll = ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_vexpand(true);
    scroll.set_propagate_natural_width(false);
    scroll.set_child(Some(&messages));

    let chat_overlay = Overlay::new();
    chat_overlay.add_css_class("chat-content-overlay");
    chat_overlay.set_vexpand(true);
    chat_overlay.set_hexpand(true);
    chat_overlay.set_child(Some(&scroll));
    root.append(&chat_overlay);

    // ── Composer ─────────────────────────────────────────────────────
    let composer_wrap = GBox::new(Orientation::Vertical, 0);
    composer_wrap.add_css_class("chat-composer");
    composer_wrap.set_halign(Align::Fill);
    composer_wrap.set_valign(Align::End);
    composer_wrap.set_hexpand(true);

    let composer_box = GBox::new(Orientation::Vertical, 0);
    composer_box.add_css_class("chat-composer-box");

    let input_view = TextView::new();
    input_view.set_wrap_mode(gtk::WrapMode::WordChar);
    input_view.add_css_class("chat-input-view");
    input_view.set_accepts_tab(false);
    input_view.set_left_margin(16);
    input_view.set_right_margin(16);
    input_view.set_top_margin(14);
    input_view.set_bottom_margin(6);
    let buffer = input_view.buffer();
    let restore_composer_draft = Rc::new({
        let buffer = buffer.clone();
        let selected_thread = selected_thread.clone();
        let composer_drafts = composer_drafts.clone();
        let app_state = app_state.clone();
        let restoring_composer_draft = restoring_composer_draft.clone();
        move || {
            let thread_id = *selected_thread.borrow();
            let text = thread_id
                .and_then(|thread_id| app_state.editing_queued_chat_input(thread_id))
                .map(|editing| queued_chat_input_visible_text(&editing.original))
                .unwrap_or_else(|| composer_draft_for_thread(&composer_drafts, thread_id));
            if text_buffer_text(&buffer) == text {
                return;
            }
            restoring_composer_draft.set(true);
            buffer.set_text(&text);
            restoring_composer_draft.set(false);
        }
    });

    let input_scroll = ScrolledWindow::new();
    input_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    input_scroll.set_min_content_height(54);
    input_scroll.set_max_content_height(120);
    input_scroll.set_propagate_natural_height(true);
    input_scroll.set_propagate_natural_width(false);
    input_scroll.add_css_class("chat-input-scroll");
    input_scroll.set_child(Some(&input_view));
    let input_shell = GBox::new(Orientation::Horizontal, 8);
    input_shell.set_hexpand(true);
    let input_overlay = Overlay::new();
    input_overlay.set_hexpand(true);
    input_overlay.set_child(Some(&input_scroll));
    let input_focus_click = GestureClick::new();
    input_focus_click.connect_pressed({
        let input_view = input_view.clone();
        move |_, _, _, _| {
            input_view.grab_focus();
        }
    });
    input_overlay.add_controller(input_focus_click);
    let placeholder = Label::new(Some(
        "Ask to make changes, @mention files, or run /commands",
    ));
    placeholder.add_css_class("chat-placeholder");
    placeholder.set_halign(Align::Start);
    placeholder.set_valign(Align::Start);
    placeholder.set_margin_start(16);
    placeholder.set_margin_top(14);
    placeholder.set_can_target(false);
    input_overlay.add_overlay(&placeholder);
    input_shell.append(&input_overlay);

    let focus_btn = icon_button("focus-windows-symbolic", "Quick focus composer (Ctrl+L)");
    focus_btn.add_css_class("chat-focus-btn");
    focus_btn.set_tooltip_text(Some("Quick focus composer (Ctrl+L)"));
    input_shell.append(&focus_btn);

    let toolbar = GBox::new(Orientation::Horizontal, 8);
    toolbar.add_css_class("chat-toolbar");
    toolbar.set_hexpand(true);

    let left_group = GBox::new(Orientation::Horizontal, 8);
    left_group.set_hexpand(true);

    let provider_model_choices = {
        let readiness = setup_readiness.borrow();
        Rc::new(provider_model_choices(&readiness, initial_harness))
    };
    let provider_model_btn = provider_model_menu_button(
        provider_model_choices.clone(),
        selected_provider_model_choice_index(
            provider_model_choices.as_ref(),
            initial_harness,
            selected_model.borrow().as_deref(),
        ),
        {
            let provider_model_choices_for_menu = provider_model_choices.clone();
            let database_path = database_path.to_path_buf();
            let current_workspace_name = current_workspace_name.clone();
            let selected_harness = selected_harness.clone();
            let reasoning_mode = reasoning_mode.clone();
            let refresh_chat_surface = refresh_chat_surface.clone();
            let selected_model = selected_model.clone();
            let pending_commands = pending_commands.clone();
            let selected_thread = selected_thread.clone();
            let thread_state = thread_state.clone();
            let external_chat_tabs = external_chat_tabs.clone();
            let app_state = app_state.clone();
            let sync_live_controls = sync_live_controls.clone();
            let archcar_bridge = archcar_bridge.clone();
            let archcar_ready_cache = archcar_ready_cache.clone();
            let inflight_archcar_actions = inflight_archcar_actions.clone();
            let toast_manager = toast_manager.clone();
            let restore_composer_draft = restore_composer_draft.clone();
            let refresh_queue_overlay = refresh_queue_overlay.clone();
            Rc::new(move |index| {
                let Some(choice) = provider_model_choices_for_menu.get(index).cloned() else {
                    return;
                };
                let workspace_name = current_workspace_name.borrow().clone();
                let current_kind = *selected_harness.borrow();
                match model_selection_route(current_kind, &choice) {
                    ModelSelectionRoute::SameProvider => {
                        *selected_model.borrow_mut() = choice.model.clone();
                        if let Some(thread_id) = *selected_thread.borrow() {
                            let existing_metadata = thread_state
                                .borrow()
                                .iter()
                                .find(|thread| thread.id == thread_id)
                                .and_then(|thread| thread.harness_metadata.clone());
                            let metadata = provider_model_harness_metadata(
                                existing_metadata.as_deref(),
                                choice.model.as_deref(),
                            );
                            match WorkspaceStore::open_app(database_path.clone()).and_then(
                                |store| {
                                    store.update_chat_thread_harness_metadata(
                                        thread_id,
                                        metadata.as_deref(),
                                    )
                                },
                            ) {
                                Ok(updated) => {
                                    replace_thread_state_record(thread_state.as_ref(), updated)
                                }
                                Err(err) => {
                                    toast_manager.error(format!("Save chat model failed: {err:#}"));
                                    warn!(
                                        workspace = %workspace_name,
                                        thread_id,
                                        error = %err,
                                        "failed to persist selected chat model"
                                    );
                                }
                            }
                        }
                        if let Some(thread_id) = *selected_thread.borrow() {
                            if current_kind == SessionKind::Codex {
                                if let Ok(records) = WorkspaceStore::open_app(database_path.clone())
                                    .and_then(|store| store.list_thread_processes(thread_id))
                                {
                                    if let Some(session_id) = any_running_archcar_codex_ready(
                                        &records,
                                        &archcar_ready_cache,
                                    ) {
                                        queue_archcar_model_update(
                                            &archcar_bridge,
                                            inflight_archcar_actions.as_ref(),
                                            thread_id,
                                            session_id,
                                            choice.model.clone(),
                                        );
                                        let pending_controls = flush_pending_commands_for_send(
                                            &pending_commands,
                                            thread_id,
                                        );
                                        for (index, control) in pending_controls.iter().enumerate()
                                        {
                                            if !queue_archcar_control_send(
                                                &archcar_bridge,
                                                inflight_archcar_actions.as_ref(),
                                                thread_id,
                                                session_id,
                                                control.clone(),
                                            ) {
                                                requeue_pending_controls(
                                                    &pending_commands,
                                                    thread_id,
                                                    &pending_controls,
                                                    index,
                                                );
                                                warn!(
                                                    thread_id,
                                                    session_id,
                                                    "archcar control send failed; requeued pending controls"
                                                );
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    ModelSelectionRoute::CrossProvider(kind) => {
                        let source_provider = session_kind_provider(current_kind);
                        let source_messages = match *selected_thread.borrow() {
                            Some(thread_id) => {
                                match model_switch_context_items(&database_path, thread_id) {
                                    Ok(messages) => Some(messages),
                                    Err(err) => {
                                        toast_manager.error(format!(
                                            "Read source chat history failed: {err:#}"
                                        ));
                                        warn!(
                                            workspace = %workspace_name,
                                            thread_id,
                                            error = %err,
                                            "failed to read source chat for provider switch"
                                        );
                                        return;
                                    }
                                }
                            }
                            None => None,
                        };
                        let metadata =
                            provider_model_harness_metadata(None, choice.model.as_deref());
                        let title = default_chat_thread_title(kind, &thread_state.borrow());
                        match WorkspaceStore::open_app(database_path.clone()).and_then(|store| {
                            let thread = store.create_chat_thread(
                                &workspace_name,
                                &choice.provider,
                                &title,
                                metadata.as_deref(),
                            )?;
                            if let Some(attachment) =
                                source_messages.as_deref().and_then(|messages| {
                                    model_switch_context_attachment(
                                        source_provider,
                                        &choice.provider,
                                        messages,
                                    )
                                })
                            {
                                if let Err(err) = store.append_chat_message(
                                    thread.id,
                                    "system",
                                    &attachment,
                                    "model_switch_context",
                                ) {
                                    let _ = store.close_chat_thread(thread.id);
                                    return Err(err);
                                }
                            }
                            Ok(thread)
                        }) {
                            Ok(thread) => {
                                persist_selected_provider(
                                    &database_path,
                                    &workspace_name,
                                    &choice.provider,
                                );
                                *selected_harness.borrow_mut() = kind;
                                if matches!(kind, SessionKind::Codex | SessionKind::Claude) {
                                    *reasoning_mode.borrow_mut() = Some("high".to_owned());
                                }
                                *selected_model.borrow_mut() = choice.model.clone();
                                thread_state.borrow_mut().insert(0, thread.clone());
                                apply_thread_selection(
                                    selected_thread.as_ref(),
                                    Some(thread.id),
                                    |thread_id| app_state.set_selected_chat_thread(thread_id),
                                    || {
                                        restore_composer_draft();
                                        if let Some(refresh) =
                                            refresh_queue_overlay.borrow().as_ref().cloned()
                                        {
                                            refresh();
                                        }
                                    },
                                );
                                if let Some(external_chat_tabs) = external_chat_tabs.as_ref() {
                                    (external_chat_tabs.on_threads_changed)(
                                        thread_state.borrow().clone(),
                                        Some(thread.id),
                                    );
                                }
                                if let Some(refresh_view) =
                                    clone_refresh_chat_surface_controller(&refresh_chat_surface)
                                {
                                    refresh_view();
                                }
                            }
                            Err(err) => {
                                toast_manager
                                    .error(format!("Create provider chat failed: {err:#}"));
                                warn!(
                                    workspace = %workspace_name,
                                    provider = %choice.provider,
                                    error = %err,
                                    "failed to create provider chat for model selection"
                                );
                            }
                        }
                    }
                }
                if let Some(sync) = clone_refresh_chat_surface_controller(&sync_live_controls) {
                    sync();
                }
            })
        },
    );
    let thinking_btn =
        mode_menu_button("High", "◔", &["Low", "Medium", "High", "Extra high"], 2, {
            let reasoning_mode = reasoning_mode.clone();
            let selected_harness = selected_harness.clone();
            let selected_thread = selected_thread.clone();
            let pending_commands = pending_commands.clone();
            let database_path = database_path.clone();
            let archcar_bridge = archcar_bridge.clone();
            let inflight_archcar_actions = inflight_archcar_actions.clone();
            let archcar_ready_cache = archcar_ready_cache.clone();
            let toast_manager = toast_manager.clone();
            Rc::new(move |index| {
                let level = session_reasoning_mode_from_index(index);
                *reasoning_mode.borrow_mut() = Some(level.clone());
                let current_kind = *selected_harness.borrow();
                if let Some(thread_id) = *selected_thread.borrow() {
                    if managed_harness_for_kind(current_kind).is_some() {
                        match WorkspaceStore::open_app(database_path.clone())
                            .and_then(|store| store.list_thread_processes(thread_id))
                        {
                            Ok(records) => {
                                if let Some(session_id) =
                                    running_session_for_thread(&records, thread_id, current_kind)
                                {
                                    if queue_archcar_effort_update(
                                        &archcar_bridge,
                                        inflight_archcar_actions.as_ref(),
                                        thread_id,
                                        session_id,
                                        Some(level.clone()),
                                        current_kind,
                                    ) {
                                        note_archcar_ready(
                                            &mut archcar_ready_cache.borrow_mut(),
                                            session_id,
                                            false,
                                        );
                                        return;
                                    }
                                    toast_manager.error(
                                        "Could not update thinking because archcar is unavailable."
                                            .to_owned(),
                                    );
                                }
                            }
                            Err(err) => {
                                toast_manager
                                    .error(format!("Read session processes failed: {err:#}"));
                            }
                        }
                    }
                    if current_kind == SessionKind::Codex {
                        if let Some(command) = codex_reasoning_command(&level) {
                            queue_thread_command(&pending_commands, thread_id, command);
                        }
                    }
                }
            })
        });
    thinking_btn.add_css_class("chat-thinking-menu");

    left_group.append(&provider_model_btn);
    left_group.append(&thinking_btn);

    let record_state = Rc::new(RefCell::new(Vec::<ProcessRecord>::new()));
    let selected_session: Rc<RefCell<Option<i64>>> = Rc::new(RefCell::new(None));
    let active_sessions: Rc<RefCell<HashSet<i64>>> = Rc::new(RefCell::new(HashSet::new()));
    let last_output = Rc::new(RefCell::new(HashMap::<i64, Instant>::new()));

    let right_group = GBox::new(Orientation::Horizontal, 8);
    right_group.set_halign(Align::End);
    right_group.set_hexpand(false);
    let new_chat_btn = session_secondary_button("New Chat");
    new_chat_btn.set_tooltip_text(Some("Create a new chat thread"));
    let context_usage = context_usage_widget();
    let send_btn = icon_button("send-symbolic", "Send message");
    send_btn.add_css_class("chat-send-btn");
    send_btn.set_tooltip_text(Some("Send message"));

    right_group.append(&new_chat_btn);
    right_group.append(&context_usage.container);
    right_group.append(&send_btn);

    toolbar.append(&left_group);
    toolbar.append(&right_group);

    let queue_overlay = GBox::new(Orientation::Vertical, 4);
    queue_overlay.add_css_class("chat-queue-overlay");
    queue_overlay.set_halign(Align::Fill);
    queue_overlay.set_hexpand(true);
    queue_overlay.set_visible(false);

    let sync_live_controls_fn: Rc<dyn Fn()> = Rc::new({
        let selected_harness = selected_harness.clone();
        let thinking_btn = thinking_btn.clone();
        move || {
            let controls = visible_live_controls_for_provider(session_kind_provider(
                *selected_harness.borrow(),
            ));
            thinking_btn.set_visible(controls.iter().any(|control| control == "thinking"));
        }
    });
    *sync_live_controls.borrow_mut() = Some(sync_live_controls_fn.clone());
    sync_live_controls_fn();

    composer_box.append(&input_shell);
    composer_box.append(&toolbar);
    composer_wrap.append(&queue_overlay);
    composer_wrap.append(&composer_box);
    chat_overlay.add_overlay(&composer_wrap);
    chat_overlay.set_measure_overlay(&composer_wrap, false);

    let last_queue_overlay_signature = Rc::new(RefCell::new(None::<QueueOverlaySignature>));
    let refresh_queue_overlay_fn: Rc<dyn Fn()> = Rc::new({
        let queue_overlay = queue_overlay.clone();
        let selected_thread = selected_thread.clone();
        let selected_harness = selected_harness.clone();
        let app_state = app_state.clone();
        let queued_auto_drain_holds = queued_auto_drain_holds.clone();
        let buffer = buffer.clone();
        let send_immediate_after_ready_queue = send_immediate_after_ready_queue.clone();
        let refresh_chat_surface = refresh_chat_surface.clone();
        let refresh_queue_overlay = refresh_queue_overlay.clone();
        let last_queue_overlay_signature = last_queue_overlay_signature.clone();
        move || {
            let thread_id = *selected_thread.borrow();
            let queued_items = queued_composer_overlay_items_for_thread(
                &app_state,
                thread_id,
                *selected_harness.borrow(),
            );
            let editing_queued_message = thread_id
                .is_some_and(|thread_id| app_state.editing_queued_chat_input(thread_id).is_some());
            let signature = QueueOverlaySignature {
                thread_id,
                editing: editing_queued_message,
                items: queued_items.clone(),
            };
            if !queue_overlay_requires_rebuild(
                last_queue_overlay_signature.borrow().as_ref(),
                &signature,
            ) {
                return;
            }
            *last_queue_overlay_signature.borrow_mut() = Some(signature);

            clear_box(&queue_overlay);
            queue_overlay.set_visible(editing_queued_message || !queued_items.is_empty());
            let Some(thread_id) = thread_id else {
                return;
            };
            if editing_queued_message {
                let cancel_edit = Rc::new({
                    let app_state = app_state.clone();
                    let buffer = buffer.clone();
                    let refresh_after_action = {
                        let refresh_chat_surface = refresh_chat_surface.clone();
                        let refresh_queue_overlay = refresh_queue_overlay.clone();
                        Rc::new(move || {
                            if let Some(refresh) = refresh_queue_overlay.borrow().as_ref().cloned()
                            {
                                refresh();
                            }
                            if let Some(refresh) =
                                clone_refresh_chat_surface_controller(&refresh_chat_surface)
                            {
                                gtk::glib::idle_add_local_once(move || refresh());
                            }
                        })
                    };
                    move || {
                        if let Some(previous) =
                            app_state.cancel_editing_queued_chat_input(thread_id)
                        {
                            buffer.set_text(&previous);
                        }
                        refresh_after_action();
                    }
                });
                queue_overlay.append(&queued_composer_editing_row(cancel_edit));
            }
            for item in queued_items {
                let refresh_after_action: Rc<dyn Fn()> = Rc::new({
                    let refresh_chat_surface = refresh_chat_surface.clone();
                    let refresh_queue_overlay = refresh_queue_overlay.clone();
                    move || {
                        if let Some(refresh) = refresh_queue_overlay.borrow().as_ref().cloned() {
                            refresh();
                        }
                        if let Some(refresh) =
                            clone_refresh_chat_surface_controller(&refresh_chat_surface)
                        {
                            gtk::glib::idle_add_local_once(move || refresh());
                        }
                    }
                });
                let delete_item = Rc::new({
                    let app_state = app_state.clone();
                    let queued_auto_drain_holds = queued_auto_drain_holds.clone();
                    let refresh_after_action = refresh_after_action.clone();
                    let index = item.index;
                    move || {
                        let _ = remove_queued_chat_input_at(&app_state, thread_id, index);
                        release_queued_auto_drain_if_queue_empty(
                            queued_auto_drain_holds.as_ref(),
                            thread_id,
                            queued_chat_inputs_count(&app_state, thread_id),
                        );
                        refresh_after_action();
                    }
                });
                let edit_item = Rc::new({
                    let app_state = app_state.clone();
                    let buffer = buffer.clone();
                    let refresh_after_action = refresh_after_action.clone();
                    let index = item.index;
                    move || {
                        let current = buffer
                            .text(&buffer.start_iter(), &buffer.end_iter(), true)
                            .to_string();
                        if let Some(editing) =
                            app_state.begin_editing_queued_chat_input(thread_id, index, current)
                        {
                            buffer.set_text(&queued_chat_input_visible_text(&editing.original));
                        }
                        refresh_after_action();
                    }
                });
                let send_immediately = Rc::new({
                    let app_state = app_state.clone();
                    let queued_auto_drain_holds = queued_auto_drain_holds.clone();
                    let send_immediate_after_ready_queue = send_immediate_after_ready_queue.clone();
                    let refresh_after_action = refresh_after_action.clone();
                    let index = item.index;
                    move || {
                        if let Some(input) =
                            remove_queued_chat_input_at(&app_state, thread_id, index)
                        {
                            let staged_review =
                                matches!(input.kind, ArchcarInputKind::ReviewPrompt);
                            let sent = send_immediate_after_ready_queue
                                .borrow()
                                .as_ref()
                                .cloned()
                                .is_some_and(|send| send(input.input.clone(), staged_review));
                            if !sent {
                                requeue_pending_input_front(&app_state, thread_id, input);
                            } else {
                                release_queued_auto_drain_if_queue_empty(
                                    queued_auto_drain_holds.as_ref(),
                                    thread_id,
                                    queued_chat_inputs_count(&app_state, thread_id),
                                );
                            }
                        }
                        refresh_after_action();
                    }
                });
                let row =
                    queued_composer_overlay_row(&item, delete_item, edit_item, send_immediately);
                queue_overlay.append(&row);
            }
        }
    });
    *refresh_queue_overlay.borrow_mut() = Some(refresh_queue_overlay_fn.clone());
    refresh_queue_overlay_fn();

    let last_render_signature = Rc::new(RefCell::new(None::<ChatRenderSignature>));
    let last_timeline_render_state = Rc::new(RefCell::new(None::<ChatTimelineRenderState>));
    let buffer_for_update = buffer.clone();
    let update_composer_state = {
        let placeholder = placeholder.clone();
        let send_btn = send_btn.clone();
        let input_view = input_view.clone();
        let selected_harness = selected_harness.clone();
        let selected_thread = selected_thread.clone();
        let record_state = record_state.clone();
        let archcar_ready_cache = archcar_ready_cache.clone();
        let codex_startup_states = codex_startup_states.clone();
        let app_state = app_state.clone();
        let working_threads = working_threads.clone();
        Rc::new(move || {
            let start = buffer_for_update.start_iter();
            let end = buffer_for_update.end_iter();
            let text = buffer_for_update.text(&start, &end, true);
            let has_text = !text.as_str().trim().is_empty();
            let thread_id = *selected_thread.borrow();
            let chat_target = selected_chat_target_for_submit(&app_state, thread_id);
            let action_thread_id = composer_thread_for_target(chat_target.as_ref(), thread_id);
            let has_active_generation = action_thread_id.is_some_and(|thread_id| {
                active_generation_for_thread(
                    &record_state.borrow(),
                    &working_threads,
                    thread_id,
                    *selected_harness.borrow(),
                )
            });
            let latest_status = action_thread_id.and_then(|thread_id| {
                latest_session_status_for_thread(
                    &record_state.borrow(),
                    thread_id,
                    *selected_harness.borrow(),
                )
            });
            let queued_count = action_thread_id
                .map(|thread_id| queued_chat_inputs_count(&app_state, thread_id))
                .unwrap_or_default();
            let current_harness = *selected_harness.borrow();
            let managed_harness_waits = managed_harness_for_kind(current_harness).is_some();
            let managed_thread_ready = if managed_harness_waits {
                action_thread_id.is_none_or(|thread_id| {
                    managed_thread_ready_for_ui(
                        current_harness,
                        thread_id,
                        &record_state.borrow(),
                        archcar_ready_cache.as_ref(),
                        codex_startup_states.borrow().get(&thread_id),
                    )
                })
            } else {
                true
            };
            let managed_waiting_for_startup = managed_harness_waits
                && action_thread_id.is_some_and(|thread_id| {
                    chat_thread_waiting_for_starting_agent(&app_state, thread_id)
                })
                && !managed_thread_ready
                && !has_active_generation;
            placeholder.set_text("Ask to make changes, @mention files, or run /commands");
            placeholder.set_visible(!has_text);
            let is_editing_queued_message = action_thread_id
                .is_some_and(|thread_id| app_state.editing_queued_chat_input(thread_id).is_some());
            let action = if is_editing_queued_message {
                if has_text {
                    ComposerAction::SaveQueuedEdit
                } else {
                    ComposerAction::Disabled
                }
            } else {
                composer_action_for_startup_state(
                    has_text,
                    has_active_generation,
                    latest_status == Some(ProcessStatus::Stopped),
                    queued_count,
                    managed_waiting_for_startup,
                )
            };
            set_composer_send_button_action(&send_btn, action);
            if action != ComposerAction::Disabled {
                send_btn.add_css_class("chat-send-btn-active");
            } else {
                send_btn.remove_css_class("chat-send-btn-active");
            }
            input_view.queue_draw();
        })
    };
    buffer.connect_changed({
        let update = update_composer_state.clone();
        let selected_thread = selected_thread.clone();
        let composer_drafts = composer_drafts.clone();
        let restoring_composer_draft = restoring_composer_draft.clone();
        move |buffer| {
            if !restoring_composer_draft.get() {
                remember_composer_draft(
                    &composer_drafts,
                    *selected_thread.borrow(),
                    &text_buffer_text(buffer),
                );
            }
            update();
        }
    });
    update_composer_state();

    let eager_start_chat_agent: Rc<dyn Fn(String, i64, SessionKind)> = Rc::new({
        let app_state = app_state.clone();
        let record_state = record_state.clone();
        let archcar_ready_cache = archcar_ready_cache.clone();
        let inflight_archcar_actions = inflight_archcar_actions.clone();
        let codex_startup_states = codex_startup_states.clone();
        let working_threads = working_threads.clone();
        let codex_ready = codex_ready.clone();
        let update_composer_state = update_composer_state.clone();
        let archcar_bridge = archcar_bridge.clone();
        move |workspace: String, thread_id: i64, kind: SessionKind| {
            let records = record_state.borrow().clone();
            let _ = eager_chat_agent_start(
                &app_state,
                &records,
                archcar_ready_cache.as_ref(),
                inflight_archcar_actions.as_ref(),
                codex_startup_states.as_ref(),
                working_threads.as_ref(),
                codex_ready.as_ref(),
                update_composer_state.as_ref(),
                workspace,
                thread_id,
                kind,
                |workspace, thread_id, harness| {
                    request_archcar_ensure(
                        &archcar_bridge,
                        inflight_archcar_actions.as_ref(),
                        workspace,
                        Some(thread_id),
                        harness,
                    )
                },
            );
        }
    });

    let refresh_chat_surface_for_view = refresh_chat_surface.clone();
    let refresh_for_metadata = refresh.clone();

    let refresh_view = {
        let database_path = database_path.clone();
        let current_workspace_name = current_workspace_name.clone();
        let current_branch_name = current_branch_name.clone();
        let messages = messages.clone();
        let scroll = scroll.clone();
        let thread_row = thread_row.clone();
        let record_state = record_state.clone();
        let thread_state = thread_state.clone();
        let selected_session = selected_session.clone();
        let selected_thread = selected_thread.clone();
        let last_render_signature = last_render_signature.clone();
        let last_timeline_render_state = last_timeline_render_state.clone();
        let selected_harness = selected_harness.clone();
        let selected_model = selected_model.clone();
        let reasoning_mode = reasoning_mode.clone();
        let provider_model_btn = provider_model_btn.clone();
        let provider_model_choices = provider_model_choices.clone();
        let active_sessions = active_sessions.clone();
        let last_output = last_output.clone();
        let app_state = app_state.clone();
        let app_state_for_thread_select = app_state.clone();
        let codex_startup_states = codex_startup_states.clone();
        let archcar_ready_cache = archcar_ready_cache.clone();
        let archcar_session_threads = archcar_session_threads.clone();
        let inflight_archcar_actions = inflight_archcar_actions.clone();
        let pending_commands = pending_commands.clone();
        let pending_archcar_inputs = pending_archcar_inputs.clone();
        let working_threads = working_threads.clone();
        let archcar_bridge = archcar_bridge.clone();
        let bridge_error_state = bridge_error_state.clone();
        let codex_ready = codex_ready.clone();
        let update_composer_for_view = update_composer_state.clone();
        let queued_auto_drain_holds = queued_auto_drain_holds.clone();
        let external_chat_tabs = external_chat_tabs.clone();
        let context_usage = context_usage.clone();
        let toast_manager = toast_manager.clone();
        let restore_composer_draft = restore_composer_draft.clone();
        let sync_live_controls = sync_live_controls.clone();
        let send_text_after_ready_queue = send_text_after_ready_queue.clone();
        let refresh_queue_overlay = refresh_queue_overlay.clone();
        let eager_start_chat_agent = eager_start_chat_agent.clone();
        let refresh_for_metadata = refresh_for_metadata.clone();
        Rc::new(move || {
            let mut outcome = ChatRefreshOutcome::default();
            let workspace = current_workspace_name.borrow().clone();
            debug!(workspace = %workspace, "chat refresh_view start");
            let chat_scroll = capture_chat_scroll(&scroll);
            if chat_refresh_workspace_decision(&workspace, &app_state.snapshot(), true)
                == ChatRefreshWorkspaceDecision::SkipStaleSurface
            {
                return;
            }
            match WorkspaceStore::open_app(database_path.clone())
                .and_then(|store| store.get_workspace_record_by_name(&workspace).map(|_| ()))
            {
                Ok(()) => {}
                Err(err) if workspace_lookup_returned_no_rows(&err) => {
                    let can_recover_from_selected_thread =
                        selected_thread.borrow().is_some_and(|thread_id| {
                            WorkspaceStore::open_app(database_path.clone())
                                .and_then(|store| {
                                    let thread = store.get_chat_thread_record(thread_id)?;
                                    store.get_workspace_record(thread.workspace_id).map(|_| ())
                                })
                                .is_ok()
                        });
                    if !can_recover_from_selected_thread {
                        clear_deleted_workspace_surface(
                            &app_state,
                            &workspace,
                            &thread_row,
                            &messages,
                            &context_usage,
                        );
                        restore_chat_scroll_after_refresh(&scroll, chat_scroll);
                        return;
                    }
                }
                Err(_) => {}
            }
            let (workspace_name, loaded, loaded_threads) = match WorkspaceStore::open_app(
                database_path.clone(),
            )
            .and_then(|store| {
                let sessions = store.list_sessions(&workspace)?;
                let threads = store.list_chat_threads(&workspace)?;
                Ok((workspace.clone(), sessions, threads))
            }) {
                Ok(loaded) => loaded,
                Err(err) => {
                    if let Some(thread_id) = *selected_thread.borrow() {
                        if let Ok(recovered) = WorkspaceStore::open_app(database_path.clone())
                            .and_then(|store| {
                                let thread = store.get_chat_thread_record(thread_id)?;
                                let workspace_record =
                                    store.get_workspace_record(thread.workspace_id)?;
                                let sessions = store.list_sessions(&workspace_record.name)?;
                                let threads = store.list_chat_threads(&workspace_record.name)?;
                                Ok((workspace_record.name, sessions, threads))
                            })
                        {
                            let (new_workspace, sessions, threads) = recovered;
                            if new_workspace != workspace {
                                app_state
                                    .rename_workspace_in_navigation(&workspace, &new_workspace);
                                *current_workspace_name.borrow_mut() = new_workspace.clone();
                            }
                            (new_workspace, sessions, threads)
                        } else {
                            error!(workspace = %workspace, error = %err, "chat refresh_view failed to load workspace state");
                            clear_box(&thread_row);
                            clear_box(&messages);
                            apply_context_usage_state(&context_usage, None);
                            let message = session_refresh_error_text("load sessions", &err);
                            toast_manager.error(message.clone());
                            let label = Label::new(Some(&message));
                            label.add_css_class("chat-agent-text");
                            label.set_selectable(true);
                            label.set_wrap(true);
                            label.set_xalign(0.0);
                            append_chat_refresh_row(&messages, &label);
                            restore_chat_scroll_after_refresh(&scroll, chat_scroll);
                            return;
                        }
                    } else {
                        error!(workspace = %workspace, error = %err, "chat refresh_view failed to load workspace state");
                        clear_box(&thread_row);
                        clear_box(&messages);
                        *last_timeline_render_state.borrow_mut() = None;
                        apply_context_usage_state(&context_usage, None);
                        let message = session_refresh_error_text("load sessions", &err);
                        toast_manager.error(message.clone());
                        let label = Label::new(Some(&message));
                        label.add_css_class("chat-agent-text");
                        label.set_selectable(true);
                        label.set_wrap(true);
                        label.set_xalign(0.0);
                        append_chat_refresh_row(&messages, &label);
                        restore_chat_scroll_after_refresh(&scroll, chat_scroll);
                        return;
                    }
                }
            };
            let previous_threads = thread_state.borrow().clone();
            let previous_selected_thread = *selected_thread.borrow();
            {
                let mut current = record_state.borrow_mut();
                current.clear();
                current.extend(loaded);
            }
            {
                let mut current = thread_state.borrow_mut();
                current.clear();
                current.extend(loaded_threads);
            }
            debug!(
                workspace = %workspace_name,
                session_count = record_state.borrow().len(),
                thread_count = thread_state.borrow().len(),
                "chat refresh_view loaded workspace state"
            );

            let preferred_thread = *selected_thread.borrow();
            let fallback_kind = *selected_harness.borrow();
            let active_thread = {
                let current = thread_state.borrow();
                preferred_thread_for_selected_chat(&current, preferred_thread, fallback_kind)
            };
            apply_thread_selection(
                selected_thread.as_ref(),
                active_thread,
                |thread_id| app_state.set_selected_chat_thread(thread_id),
                || {
                    restore_composer_draft();
                    if let Some(refresh) = refresh_queue_overlay.borrow().as_ref().cloned() {
                        refresh();
                    }
                },
            );
            if chat_thread_nav_signature(&previous_threads)
                != chat_thread_nav_signature(&thread_state.borrow())
                || previous_selected_thread != *selected_thread.borrow()
            {
                outcome.session_lifecycle_changed = true;
            }
            let selected_thread_record = {
                let selected_thread_id = *selected_thread.borrow();
                let threads = thread_state.borrow();
                selected_thread_id
                    .and_then(|thread_id| threads.iter().find(|thread| thread.id == thread_id))
                    .cloned()
            };
            if let Some(thread) = selected_thread_record {
                let (kind, model) = selected_thread_harness_state(&thread);
                *selected_harness.borrow_mut() = kind;
                if matches!(kind, SessionKind::Codex | SessionKind::Claude) {
                    *reasoning_mode.borrow_mut() = Some("high".to_owned());
                }
                *selected_model.borrow_mut() = model.clone();
                let index = selected_provider_model_choice_index(
                    provider_model_choices.as_ref(),
                    kind,
                    model.as_deref(),
                );
                if let Some(choice) = provider_model_choices.get(index) {
                    provider_model_menu_set_child(&provider_model_btn, choice);
                    provider_model_btn.set_tooltip_text(Some(&choice.button_label()));
                }
                if let Some(sync) = clone_refresh_chat_surface_controller(&sync_live_controls) {
                    sync();
                }
                eager_start_chat_agent(workspace_name.clone(), thread.id, kind);
                restore_composer_draft();
            }
            update_composer_for_view();
            let current_kind = *selected_harness.borrow();
            let selected_thread_id = *selected_thread.borrow();
            let mut archcar_changed = false;
            let mut archcar_intent = ArchcarRefreshIntent::default();
            while let Some(message) = archcar_bridge.try_recv() {
                archcar_intent.merge(archcar_message_refresh_intent(&message));
                match message {
                    AsyncArchcarMessage::Event(event) => {
                        let _ = update_working_indicator_for_archcar_event(
                            &event,
                            &record_state.borrow(),
                            archcar_session_threads.as_ref(),
                            current_kind,
                            selected_thread_id,
                            pending_archcar_inputs.as_ref(),
                            &working_threads,
                        );
                        handle_archcar_event(
                            &event,
                            &record_state.borrow(),
                            archcar_ready_cache.as_ref(),
                            codex_startup_states.as_ref(),
                            archcar_session_threads.as_ref(),
                            inflight_archcar_actions.as_ref(),
                            &app_state,
                            current_kind,
                            selected_thread_id,
                            &codex_ready,
                            update_composer_for_view.as_ref(),
                            queued_auto_drain_holds.as_ref(),
                            &toast_manager,
                        );
                        archcar_changed = true;
                    }
                    AsyncArchcarMessage::Response(response) => {
                        archcar_changed |= handle_archcar_response(
                            response,
                            &database_path,
                            &workspace,
                            archcar_bridge.clone(),
                            archcar_ready_cache.as_ref(),
                            inflight_archcar_actions.as_ref(),
                            pending_commands.as_ref(),
                            pending_archcar_inputs.as_ref(),
                            queued_auto_drain_holds.as_ref(),
                            codex_startup_states.as_ref(),
                            &working_threads,
                            &codex_ready,
                            update_composer_for_view.as_ref(),
                            &toast_manager,
                        );
                    }
                    AsyncArchcarMessage::BridgeError { message } => {
                        if let Some(visible_message) =
                            bridge_error_state.borrow_mut().record(&message)
                        {
                            warn!(workspace = %workspace, error = %visible_message, "archcar bridge error");
                            toast_manager.error(visible_message.clone());
                            if let Some(thread_id) = selected_thread_id {
                                hold_queued_auto_drain(queued_auto_drain_holds.as_ref(), thread_id);
                            }
                            if current_kind == SessionKind::Codex {
                                if let Some(thread_id) = selected_thread_id {
                                    apply_codex_startup_signal(
                                        &mut codex_startup_states.borrow_mut(),
                                        CodexStartupSignal::Error {
                                            thread_id,
                                            message: visible_message,
                                        },
                                    );
                                    clear_thread_working(&working_threads, thread_id);
                                    set_codex_ready_state(
                                        &codex_ready,
                                        update_composer_for_view.as_ref(),
                                        false,
                                    );
                                    archcar_changed = true;
                                }
                            }
                        }
                    }
                }
            }
            if archcar_changed {
                let flushed_pending = flush_pending_archcar_inputs(
                    &archcar_bridge,
                    &database_path,
                    &workspace,
                    pending_commands.as_ref(),
                    pending_archcar_inputs.as_ref(),
                    archcar_ready_cache.as_ref(),
                    inflight_archcar_actions.as_ref(),
                    &app_state,
                );
                if !flushed_pending {
                    if let Some(thread_id) = selected_thread_id {
                        let can_send_next = queued_chat_auto_drain_ready(
                            current_kind,
                            &record_state.borrow(),
                            thread_id,
                            archcar_ready_cache.as_ref(),
                            inflight_archcar_actions.as_ref(),
                            pending_archcar_inputs.as_ref(),
                            queued_auto_drain_holds.as_ref(),
                            &working_threads,
                        );
                        if can_send_next {
                            if let Some(send_text) =
                                send_text_after_ready_queue.borrow().as_ref().cloned()
                            {
                                if let Some(queued_input) =
                                    pop_next_queued_chat_input(&app_state, thread_id)
                                {
                                    let app_state_for_idle = app_state.clone();
                                    let queued_auto_drain_holds_for_idle =
                                        queued_auto_drain_holds.clone();
                                    let refresh_queue_overlay_for_idle =
                                        refresh_queue_overlay.clone();
                                    let staged_review =
                                        matches!(queued_input.kind, ArchcarInputKind::ReviewPrompt);
                                    if let Some(refresh) =
                                        refresh_queue_overlay.borrow().as_ref().cloned()
                                    {
                                        refresh();
                                    }
                                    gtk::glib::idle_add_local_once(move || {
                                        if !send_text(queued_input.input.clone(), staged_review) {
                                            requeue_pending_input_front(
                                                &app_state_for_idle,
                                                thread_id,
                                                queued_input,
                                            );
                                        } else {
                                            release_queued_auto_drain_if_queue_empty(
                                                queued_auto_drain_holds_for_idle.as_ref(),
                                                thread_id,
                                                queued_chat_inputs_count(
                                                    &app_state_for_idle,
                                                    thread_id,
                                                ),
                                            );
                                        }
                                        if let Some(refresh) = refresh_queue_overlay_for_idle
                                            .borrow()
                                            .as_ref()
                                            .cloned()
                                        {
                                            refresh();
                                        }
                                    });
                                }
                            }
                        }
                    }
                }
                update_composer_for_view();
            }
            outcome.messages_changed |= archcar_intent.chat_surface;
            outcome.session_lifecycle_changed |=
                archcar_intent.workspace_nav || archcar_intent.global_summary;
            if outcome.requires_nav_refresh() {
                if let Some(external_chat_tabs) = external_chat_tabs.as_ref() {
                    (external_chat_tabs.on_threads_changed)(
                        thread_state.borrow().clone(),
                        *selected_thread.borrow(),
                    );
                }
            }

            clear_box(&thread_row);
            {
                let current = thread_state.borrow();
                let selected = *selected_thread.borrow();
                let current_kind = *selected_harness.borrow();
                let provider = session_kind_provider(current_kind);
                let mut visible_threads = current
                    .iter()
                    .filter(|thread| provider == thread.provider && thread.status == "active")
                    .cloned()
                    .collect::<Vec<_>>();
                if visible_threads.is_empty() {
                    let empty = Label::new(Some("No chats yet for this provider."));
                    empty.add_css_class("card-meta");
                    empty.set_xalign(0.0);
                    append_chat_refresh_row(&thread_row, &empty);
                } else {
                    for thread in visible_threads.drain(..) {
                        let button = thread_chip_button(&thread, Some(thread.id) == selected, {
                            let selected_thread = selected_thread.clone();
                            let refresh_chat_surface = refresh_chat_surface_for_view.clone();
                            let app_state = app_state_for_thread_select.clone();
                            let update_composer_state = update_composer_for_view.clone();
                            let restore_composer_draft = restore_composer_draft.clone();
                            let refresh_queue_overlay = refresh_queue_overlay.clone();
                            let eager_start_chat_agent = eager_start_chat_agent.clone();
                            let workspace_name = workspace_name.clone();
                            move |thread_id| {
                                apply_thread_selection(
                                    selected_thread.as_ref(),
                                    Some(thread_id),
                                    |selected| app_state.set_selected_chat_thread(selected),
                                    || {
                                        restore_composer_draft();
                                        update_composer_state();
                                        if let Some(refresh) =
                                            refresh_queue_overlay.borrow().as_ref().cloned()
                                        {
                                            refresh();
                                        }
                                    },
                                );
                                eager_start_chat_agent(
                                    workspace_name.clone(),
                                    thread_id,
                                    current_kind,
                                );
                                if let Some(refresh_view) =
                                    clone_refresh_chat_surface_controller(&refresh_chat_surface)
                                {
                                    refresh_view();
                                }
                            }
                        });
                        button.set_tooltip_text(Some(&format!(
                            "{} chat",
                            session_kind_name(current_kind)
                        )));
                        append_chat_refresh_row(&thread_row, &button);
                    }
                }
            }

            let preferred = *selected_session.borrow();
            let active_record = {
                let current = record_state.borrow();
                if let Some(thread_id) = *selected_thread.borrow() {
                    let thread_records = current
                        .iter()
                        .filter(|record| record.chat_thread_id == Some(thread_id))
                        .cloned()
                        .collect::<Vec<_>>();
                    preferred_session_for_kind(&thread_records, preferred, current_kind)
                } else {
                    preferred_session_for_kind(&current, preferred, current_kind)
                }
            };
            *selected_session.borrow_mut() = active_record;
            app_state.set_selected_agent_session(active_record);

            let current = record_state.borrow();
            let selected_thread_id = *selected_thread.borrow();
            match (selected_thread_id, active_record) {
                (Some(thread_id), maybe_process_id) => {
                    let thread_exists = thread_state
                        .borrow()
                        .iter()
                        .find(|thread| thread.id == thread_id)
                        .is_some();
                    if !thread_exists {
                        let signature = chat_render_signature(
                            current_kind,
                            Some(thread_id),
                            active_record,
                            CodexStartupState::Idle,
                            None,
                            &current,
                            &thread_state.borrow(),
                            &[],
                            &[],
                            &[],
                            &[],
                            "structured",
                            "missing_thread",
                            None,
                        );
                        if chat_render_is_unchanged(last_render_signature.as_ref(), signature) {
                            return;
                        }
                        clear_box(&messages);
                        apply_context_usage_state(&context_usage, None);
                        let label = Label::new(Some("No chat selected."));
                        label.add_css_class("chat-agent-text");
                        label.set_selectable(true);
                        label.set_wrap(true);
                        label.set_xalign(0.0);
                        append_chat_refresh_row(&messages, &label);
                        restore_chat_scroll_after_refresh(&scroll, chat_scroll);
                        return;
                    }
                    let record = maybe_process_id
                        .and_then(|process_id| {
                            current
                                .iter()
                                .find(|record| record.id == process_id)
                                .cloned()
                        })
                        .or_else(|| {
                            current
                                .iter()
                                .filter(|record| record.chat_thread_id == Some(thread_id))
                                .max_by_key(|record| record.id)
                                .cloned()
                        });
                    let startup_state = if managed_harness_for_kind(current_kind).is_some() {
                        codex_startup_state_for_thread(
                            thread_id,
                            &current,
                            archcar_ready_cache.as_ref(),
                            codex_startup_states.borrow().get(&thread_id).cloned(),
                        )
                    } else {
                        CodexStartupState::Idle
                    };
                    let working_elapsed = working_elapsed_for_thread(&working_threads, thread_id);
                    if let Some(harness) = managed_harness_for_kind(current_kind) {
                        let descriptor = harness.descriptor();
                        if thread_has_live_managed_session(&current, thread_id, descriptor)
                            && !matches!(startup_state, CodexStartupState::Error { .. })
                            && !managed_thread_ready_for_ui(
                                current_kind,
                                thread_id,
                                &current,
                                archcar_ready_cache.as_ref(),
                                codex_startup_states.borrow().get(&thread_id),
                            )
                            && !has_pending_archcar_ensure_for_thread(
                                inflight_archcar_actions.as_ref(),
                                thread_id,
                            )
                        {
                            request_archcar_ensure(
                                &archcar_bridge,
                                inflight_archcar_actions.as_ref(),
                                workspace.clone(),
                                Some(thread_id),
                                descriptor,
                            );
                        }
                    }
                    let runtime_summary = record.as_ref().map(|record| {
                        let attached_sessions = active_sessions
                            .borrow()
                            .iter()
                            .copied()
                            .collect::<HashSet<_>>();
                        let attached = attached_sessions.contains(&record.id);
                        let last_seen = last_output.borrow().get(&record.id).copied();
                        let runtime_state = session_runtime_state(
                            record,
                            if record.status == ProcessStatus::Running {
                                last_seen
                            } else {
                                None
                            },
                            attached,
                        );
                        format!(
                            "{}  Status: {}  State: {}",
                            session_kind_name(current_kind),
                            record.status.as_str(),
                            runtime_state,
                        )
                    });
                    match live_chat_source() {
                        LiveChatSource::StructuredStore => {
                            let (thread_messages, thread_events, provider_events) =
                                match WorkspaceStore::open_app(database_path.clone()).and_then(
                                    |store| {
                                        let messages = store.list_chat_messages(thread_id)?;
                                        let events = store.list_chat_events(thread_id)?;
                                        let provider_events =
                                            ProviderEventStore::new(database_path.clone())
                                                .list_for_chat_thread(thread_id)?;
                                        Ok((messages, events, provider_events))
                                    },
                                ) {
                                    Ok(timeline) => timeline,
                                    Err(err) => {
                                        error!(workspace = %workspace, thread_id, error = %err, "chat refresh_view failed to load thread timeline");
                                        let message =
                                            session_refresh_error_text("load chat timeline", &err);
                                        toast_manager.error(message.clone());
                                        let label = Label::new(Some(&message));
                                        label.add_css_class("chat-agent-text");
                                        label.set_selectable(true);
                                        label.set_wrap(true);
                                        label.set_xalign(0.0);
                                        clear_box(&messages);
                                        *last_timeline_render_state.borrow_mut() = None;
                                        *last_render_signature.borrow_mut() = None;
                                        apply_context_usage_state(&context_usage, None);
                                        append_chat_refresh_row(&messages, &label);
                                        restore_chat_scroll_after_refresh(&scroll, chat_scroll);
                                        return;
                                    }
                                };
                            let transcript_display =
                                transcript_display_for_workspace(&database_path, &workspace);
                            let submitted_user_inputs = submitted_user_input_texts_for_thread(
                                thread_id,
                                pending_archcar_inputs.as_ref(),
                                inflight_archcar_actions.as_ref(),
                                &thread_messages,
                            );
                            let render_legacy_inline_events =
                                render_legacy_inline_events_for_thread(
                                    &thread_events,
                                    &transcript_display,
                                );
                            debug!(
                                workspace = %workspace,
                                thread_id,
                                thread_message_count = thread_messages.len(),
                                thread_timeline_count = thread_messages.len() + thread_events.len() + provider_events.len(),
                                render_legacy_inline_events,
                                "chat refresh_view loaded persisted chat timeline"
                            );
                            if !thread_messages.is_empty()
                                || !thread_events.is_empty()
                                || !provider_events.is_empty()
                            {
                                let provider_projection =
                                    provider_projection_from_records(&provider_events);
                                if let Some(metadata_update) =
                                    apply_provider_projection_agent_metadata(
                                        &database_path,
                                        thread_id,
                                        &provider_projection.items,
                                    )
                                {
                                    let changes = apply_agent_metadata_ui_update(
                                        &app_state,
                                        current_workspace_name.as_ref(),
                                        current_branch_name.as_ref(),
                                        thread_state.as_ref(),
                                        metadata_update.clone(),
                                    );
                                    outcome.workspace_name_changed |= changes.workspace_changed;
                                    outcome.branch_changed |= changes.branch_changed;
                                    outcome.thread_title_changed |= changes.chat_title_changed;
                                    let metadata_workspace_changed =
                                        outcome.workspace_name_changed || outcome.branch_changed;
                                    if metadata_workspace_changed {
                                        if let Some(external_chat_tabs) =
                                            external_chat_tabs.as_ref()
                                        {
                                            (external_chat_tabs.on_workspace_metadata_changed)(
                                                &metadata_update,
                                            );
                                        }
                                        refresh_for_metadata();
                                    }
                                    if outcome.thread_title_changed {
                                        if let Some(external_chat_tabs) =
                                            external_chat_tabs.as_ref()
                                        {
                                            (external_chat_tabs.on_threads_changed)(
                                                thread_state.borrow().clone(),
                                                *selected_thread.borrow(),
                                            );
                                        }
                                    }
                                }
                                let status_banner =
                                    chat_status_banner_kind(&startup_state, working_elapsed, true);
                                let signature = chat_render_signature(
                                    current_kind,
                                    Some(thread_id),
                                    active_record,
                                    startup_state.clone(),
                                    working_elapsed_seconds_for_status_banner(
                                        status_banner,
                                        working_elapsed,
                                    ),
                                    &current,
                                    &thread_state.borrow(),
                                    &thread_messages,
                                    &thread_events,
                                    &provider_events,
                                    &submitted_user_inputs,
                                    &transcript_display,
                                    "timeline",
                                    runtime_summary.clone(),
                                );
                                if chat_render_is_unchanged(
                                    last_render_signature.as_ref(),
                                    signature,
                                ) {
                                    debug!(?outcome, "chat refresh_view outcome");
                                    return;
                                }
                                let timeline = chat_structured_items_for_render(
                                    thread_messages.clone(),
                                    thread_events,
                                    provider_projection.items,
                                    Vec::new(),
                                    submitted_user_inputs,
                                );
                                let timeline_leading_rows =
                                    usize::from(status_banner != ChatStatusBannerKind::None);
                                let next_timeline_state =
                                    chat_timeline_render_state_with_leading_rows(
                                        thread_id,
                                        &transcript_display,
                                        &timeline,
                                        timeline_leading_rows,
                                    );
                                let previous_timeline_state =
                                    last_timeline_render_state.borrow().clone();
                                let previous_timeline_leading_rows = previous_timeline_state
                                    .as_ref()
                                    .map_or(0, |state| state.leading_rows);
                                let mut plan = chat_timeline_refresh_plan(
                                    previous_timeline_state.as_ref(),
                                    &next_timeline_state,
                                );
                                if status_banner != ChatStatusBannerKind::None {
                                    plan = ChatTimelineRefreshPlan::RebuildMessages;
                                }
                                outcome.messages_changed =
                                    !matches!(plan, ChatTimelineRefreshPlan::Skip);
                                apply_context_usage_state(
                                    &context_usage,
                                    latest_context_usage_from_messages(&thread_messages),
                                );
                                match plan {
                                    ChatTimelineRefreshPlan::Skip => {
                                        debug!(?outcome, "chat refresh_view outcome");
                                        restore_chat_scroll_after_refresh(&scroll, chat_scroll);
                                        return;
                                    }
                                    ChatTimelineRefreshPlan::Append { start } => {
                                        append_chat_timeline_items(
                                            &messages,
                                            &timeline[start..],
                                            &transcript_display,
                                            render_legacy_inline_events,
                                        );
                                    }
                                    ChatTimelineRefreshPlan::RebuildFrom { start } => {
                                        remove_box_children_from(
                                            &messages,
                                            previous_timeline_leading_rows + start,
                                        );
                                        append_chat_timeline_items(
                                            &messages,
                                            &timeline[start..],
                                            &transcript_display,
                                            render_legacy_inline_events,
                                        );
                                    }
                                    ChatTimelineRefreshPlan::RebuildMessages => {
                                        clear_box(&messages);
                                        append_chat_status_banner(
                                            &messages,
                                            status_banner,
                                            &startup_state,
                                            working_elapsed,
                                        );
                                        append_chat_timeline_items(
                                            &messages,
                                            &timeline,
                                            &transcript_display,
                                            render_legacy_inline_events,
                                        );
                                    }
                                }
                                *last_timeline_render_state.borrow_mut() =
                                    Some(next_timeline_state);
                                debug!(?outcome, "chat refresh_view outcome");
                                restore_chat_scroll_after_refresh(&scroll, chat_scroll);
                                return;
                            }
                        }
                    }

                    let submitted_user_inputs = submitted_user_input_texts_for_thread(
                        thread_id,
                        pending_archcar_inputs.as_ref(),
                        inflight_archcar_actions.as_ref(),
                        &[],
                    );
                    let status_banner = chat_status_banner_kind(
                        &startup_state,
                        working_elapsed,
                        !submitted_user_inputs.is_empty(),
                    );
                    let signature = chat_render_signature(
                        current_kind,
                        Some(thread_id),
                        active_record,
                        startup_state.clone(),
                        working_elapsed_seconds_for_status_banner(status_banner, working_elapsed),
                        &current,
                        &thread_state.borrow(),
                        &[],
                        &[],
                        &[],
                        &submitted_user_inputs,
                        "structured",
                        "empty",
                        runtime_summary.clone(),
                    );
                    if chat_render_is_unchanged(last_render_signature.as_ref(), signature) {
                        return;
                    }
                    outcome.messages_changed = true;
                    clear_box(&messages);
                    *last_timeline_render_state.borrow_mut() =
                        Some(chat_timeline_render_state(thread_id, "structured", &[]));
                    apply_context_usage_state(&context_usage, None);
                    append_chat_status_banner(
                        &messages,
                        status_banner,
                        &startup_state,
                        working_elapsed,
                    );
                    if submitted_user_inputs.is_empty() {
                        let empty = Label::new(Some("No messages yet."));
                        empty.add_css_class("chat-agent-text");
                        empty.set_selectable(true);
                        empty.set_wrap(true);
                        empty.set_xalign(0.0);
                        append_chat_refresh_row(&messages, &empty);
                    } else {
                        for input in submitted_user_inputs {
                            append_chat_refresh_row(&messages, &chat_user_bubble(&input));
                        }
                    }
                    restore_chat_scroll_after_refresh(&scroll, chat_scroll);
                }
                (None, _) => {
                    let signature = chat_render_signature(
                        current_kind,
                        None,
                        active_record,
                        CodexStartupState::Idle,
                        None,
                        &current,
                        &thread_state.borrow(),
                        &[],
                        &[],
                        &[],
                        &[],
                        "structured",
                        "no_thread",
                        None,
                    );
                    if chat_render_is_unchanged(last_render_signature.as_ref(), signature) {
                        return;
                    }
                    clear_box(&messages);
                    *last_timeline_render_state.borrow_mut() = None;
                    apply_context_usage_state(&context_usage, None);
                    let prompt = format!(
                        "No {} chat yet. Create one or send a message to start one.",
                        session_kind_name(current_kind)
                    );
                    append_chat_refresh_row(&messages, &chat_user_bubble(&prompt));
                    restore_chat_scroll_after_refresh(&scroll, chat_scroll);
                }
            }
            debug!(workspace = %workspace, ?outcome, "chat refresh_view complete");
        }) as Rc<dyn Fn()>
    };
    *refresh_chat_surface.borrow_mut() = Some(refresh_view.clone());
    if let Some(external_chat_tabs) = external_chat_tabs.as_ref() {
        if let Some(register_refresh) = external_chat_tabs.on_chat_surface_refresh_ready.as_ref() {
            let refresh_view_for_external = refresh_view.clone();
            let refresh_messages_for_external: Rc<dyn Fn(i64)> = {
                let database_path = database_path.clone();
                let current_workspace_name = current_workspace_name.clone();
                let selected_thread = selected_thread.clone();
                let messages = messages.clone();
                let scroll = scroll.clone();
                let context_usage = context_usage.clone();
                let pending_archcar_inputs = pending_archcar_inputs.clone();
                let inflight_archcar_actions = inflight_archcar_actions.clone();
                let last_timeline_render_state = last_timeline_render_state.clone();
                let toast_manager = toast_manager.clone();
                let message_refresh_generation = Rc::new(Cell::new(0_u64));
                Rc::new(move |thread_id| {
                    if *selected_thread.borrow() != Some(thread_id) {
                        return;
                    }
                    let workspace = current_workspace_name.borrow().clone();
                    let chat_scroll = capture_chat_scroll(&scroll);
                    let generation = message_refresh_generation.get() + 1;
                    message_refresh_generation.set(generation);
                    let database_path_for_job = database_path.clone();
                    let workspace_for_job = workspace.clone();
                    let selected_thread = selected_thread.clone();
                    let messages = messages.clone();
                    let scroll = scroll.clone();
                    let context_usage = context_usage.clone();
                    let pending_archcar_inputs = pending_archcar_inputs.clone();
                    let inflight_archcar_actions = inflight_archcar_actions.clone();
                    let last_timeline_render_state = last_timeline_render_state.clone();
                    let toast_manager = toast_manager.clone();
                    let message_refresh_generation = message_refresh_generation.clone();
                    spawn_background_job(
                        move || {
                            load_chat_timeline_snapshot(
                                database_path_for_job,
                                workspace_for_job,
                                thread_id,
                            )
                        },
                        move |result| {
                            if message_refresh_generation.get() != generation
                                || *selected_thread.borrow() != Some(thread_id)
                            {
                                return;
                            }
                            let snapshot = match result {
                                Ok(snapshot) => snapshot,
                                Err(message) => {
                                    error!(workspace = %workspace, thread_id, error = %message, "chat message refresh failed to load thread timeline");
                                    let message = format!("Load chat timeline failed: {message}");
                                    toast_manager.error(message.clone());
                                    let label = Label::new(Some(&message));
                                    label.add_css_class("chat-agent-text");
                                    label.set_selectable(true);
                                    label.set_wrap(true);
                                    label.set_xalign(0.0);
                                    clear_box(&messages);
                                    *last_timeline_render_state.borrow_mut() = None;
                                    apply_context_usage_state(&context_usage, None);
                                    append_chat_refresh_row(&messages, &label);
                                    restore_chat_scroll_after_refresh(&scroll, chat_scroll);
                                    return;
                                }
                            };
                            let thread_messages = snapshot.thread_messages;
                            let thread_events = snapshot.thread_events;
                            let provider_events = snapshot.provider_events;
                            let transcript_display = snapshot.transcript_display;
                            let submitted_user_inputs = submitted_user_input_texts_for_thread(
                                thread_id,
                                pending_archcar_inputs.as_ref(),
                                inflight_archcar_actions.as_ref(),
                                &thread_messages,
                            );
                            let render_legacy_inline_events =
                                render_legacy_inline_events_for_thread(
                                    &thread_events,
                                    &transcript_display,
                                );
                            let provider_projection =
                                provider_projection_from_records(&provider_events);
                            let timeline = chat_structured_items_for_render(
                                thread_messages.clone(),
                                thread_events,
                                provider_projection.items,
                                Vec::new(),
                                submitted_user_inputs,
                            );
                            let next_state = chat_timeline_render_state(
                                thread_id,
                                &transcript_display,
                                &timeline,
                            );
                            let previous_timeline_state =
                                last_timeline_render_state.borrow().clone();
                            let previous_timeline_leading_rows = previous_timeline_state
                                .as_ref()
                                .map_or(0, |state| state.leading_rows);
                            let plan = chat_timeline_refresh_plan(
                                previous_timeline_state.as_ref(),
                                &next_state,
                            );
                            match plan {
                                ChatTimelineRefreshPlan::Skip => return,
                                ChatTimelineRefreshPlan::Append { start } => {
                                    apply_context_usage_state(
                                        &context_usage,
                                        latest_context_usage_from_messages(&thread_messages),
                                    );
                                    append_chat_timeline_items(
                                        &messages,
                                        &timeline[start..],
                                        &transcript_display,
                                        render_legacy_inline_events,
                                    );
                                }
                                ChatTimelineRefreshPlan::RebuildFrom { start } => {
                                    remove_box_children_from(
                                        &messages,
                                        previous_timeline_leading_rows + start,
                                    );
                                    apply_context_usage_state(
                                        &context_usage,
                                        latest_context_usage_from_messages(&thread_messages),
                                    );
                                    append_chat_timeline_items(
                                        &messages,
                                        &timeline[start..],
                                        &transcript_display,
                                        render_legacy_inline_events,
                                    );
                                }
                                ChatTimelineRefreshPlan::RebuildMessages => {
                                    clear_box(&messages);
                                    apply_context_usage_state(
                                        &context_usage,
                                        latest_context_usage_from_messages(&thread_messages),
                                    );
                                    append_chat_timeline_items(
                                        &messages,
                                        &timeline,
                                        &transcript_display,
                                        render_legacy_inline_events,
                                    );
                                }
                            }
                            *last_timeline_render_state.borrow_mut() = Some(next_state);
                            restore_chat_scroll_after_refresh(&scroll, chat_scroll);
                        },
                    );
                })
            };
            let refresh_thread_nav_for_external = refresh_view.clone();
            let selected_thread_for_external = selected_thread.clone();
            register_refresh(Rc::new(move |kind| {
                dispatch_chat_surface_refresh_kind(
                    kind,
                    *selected_thread_for_external.borrow(),
                    refresh_view_for_external.as_ref(),
                    refresh_messages_for_external.as_ref(),
                    refresh_thread_nav_for_external.as_ref(),
                );
            }));
        }
    }
    install_archcar_wake(&root, &archcar_bridge, refresh_view.clone());
    install_working_indicator_tick(&root, working_threads.clone(), refresh_view.clone());

    seed_chat_running_sessions(
        &database_path,
        _workspace_name,
        &active_sessions,
        &last_output,
    );
    let refresh_session_surface = refresh_view.clone();
    refresh_session_surface();

    let db_for_send = database_path.clone();
    let current_workspace_name_for_send = current_workspace_name.clone();
    let selected_harness_for_send = selected_harness.clone();
    let selected_model_for_send = selected_model.clone();
    let thread_state_for_send = thread_state.clone();
    let selected_thread_for_send = selected_thread.clone();
    let external_chat_tabs_for_send = external_chat_tabs.clone();
    let pending_commands_for_send = pending_commands.clone();
    let pending_archcar_inputs_for_send = pending_archcar_inputs.clone();
    let active_sessions_for_send = active_sessions.clone();
    let selected_session_for_send = selected_session.clone();
    let record_state_for_send = record_state.clone();
    let refresh_view_for_send = refresh_view.clone();
    let app_state_for_send = app_state.clone();
    let messages_for_send = messages.clone();
    let archcar_bridge_for_send = archcar_bridge.clone();
    let archcar_ready_cache_for_send = archcar_ready_cache.clone();
    let inflight_archcar_actions_for_send = inflight_archcar_actions.clone();
    let working_threads_for_send = working_threads.clone();
    let codex_ready_for_send = codex_ready.clone();
    let codex_startup_states_for_send = codex_startup_states.clone();
    let update_composer_for_send = update_composer_state.clone();
    let setup_readiness_for_send = setup_readiness.clone();
    let toast_for_send = toast_manager.clone();
    let send_text_with_delivery = Rc::new(
        move |text: String, staged_review: bool, delivery: ArchcarInputDelivery| {
            let command = text.trim().to_owned();
            if command.is_empty() {
                return false;
            }
            let workspace_for_send = current_workspace_name_for_send.borrow().clone();
            let selected_kind = *selected_harness_for_send.borrow();
            if let Some(message) =
                selected_provider_blocker_after_refresh(selected_kind, &setup_readiness_for_send)
            {
                toast_for_send.error(message.clone());
                let error = Label::new(Some(&message));
                error.add_css_class("chat-agent-text");
                error.set_selectable(true);
                error.set_wrap(true);
                error.set_xalign(0.0);
                append_revealed(&messages_for_send, &error);
                return false;
            }
            info!(
                workspace = %workspace_for_send,
                harness = ?selected_kind,
                staged_review,
                chars = command.len(),
                "session send requested"
            );
            let mut records = record_state_for_send.borrow().clone();
            debug!(
                workspace = %workspace_for_send,
                harness = ?selected_kind,
                session_records = records.len(),
                "session send stage: cloned record state"
            );
            if records.is_empty() {
                if let Ok(store) = WorkspaceStore::open_app(db_for_send.clone()) {
                    records = store.list_sessions(&workspace_for_send).unwrap_or_default();
                    debug!(
                        workspace = %workspace_for_send,
                        harness = ?selected_kind,
                        session_records = records.len(),
                        "session send stage: loaded record state from store"
                    );
                }
            }
            let previous_thread_nav_signature =
                chat_thread_nav_signature(&thread_state_for_send.borrow());
            let previous_selected_thread = *selected_thread_for_send.borrow();
            let thread_id = match resolve_or_create_thread_id_for_send(
                thread_state_for_send.as_ref(),
                selected_thread_for_send.as_ref(),
                selected_kind,
                |title| {
                    WorkspaceStore::open_app(db_for_send.clone()).and_then(|store| {
                        store.create_chat_thread(
                            &workspace_for_send,
                            session_kind_provider(selected_kind),
                            &title,
                            provider_model_harness_metadata(
                                None,
                                selected_model_for_send.borrow().as_deref(),
                            )
                            .as_deref(),
                        )
                    })
                },
            ) {
                Ok(thread_id) => {
                    app_state_for_send.set_selected_chat_thread(Some(thread_id));
                    if previous_thread_nav_signature
                        != chat_thread_nav_signature(&thread_state_for_send.borrow())
                        || previous_selected_thread != Some(thread_id)
                    {
                        if let Some(external_chat_tabs) = external_chat_tabs_for_send.as_ref() {
                            (external_chat_tabs.on_threads_changed)(
                                thread_state_for_send.borrow().clone(),
                                Some(thread_id),
                            );
                        }
                    }
                    thread_id
                }
                Err(err) => {
                    let message = format!("[chat thread] {err:#}");
                    toast_for_send.error(message.clone());
                    let error = Label::new(Some(&message));
                    error.add_css_class("chat-agent-text");
                    error.set_selectable(true);
                    error.set_wrap(true);
                    error.set_xalign(0.0);
                    append_revealed(&messages_for_send, &error);
                    return false;
                }
            };
            debug!(
                workspace = %workspace_for_send,
                harness = ?selected_kind,
                thread_id,
                "session send stage: resolved thread"
            );
            let selected_harness_descriptor =
                managed_harness_for_kind(selected_kind).map(|harness| harness.descriptor());
            let thread = match WorkspaceStore::open_app(db_for_send.clone())
                .and_then(|store| store.get_chat_thread_record(thread_id))
            {
                Ok(thread) => thread,
                Err(err) => {
                    let message = format!("[chat thread] {err:#}");
                    toast_for_send.error(message.clone());
                    let error = Label::new(Some(&message));
                    error.add_css_class("chat-agent-text");
                    error.set_selectable(true);
                    error.set_wrap(true);
                    error.set_xalign(0.0);
                    append_revealed(&messages_for_send, &error);
                    return false;
                }
            };
            let thread_messages_for_send = match WorkspaceStore::open_app(db_for_send.clone())
                .and_then(|store| store.list_chat_messages(thread_id))
            {
                Ok(messages) => messages,
                Err(err) => {
                    let message = format!("[chat history] {err:#}");
                    toast_for_send.error(message.clone());
                    let error = Label::new(Some(&message));
                    error.add_css_class("chat-agent-text");
                    error.set_selectable(true);
                    error.set_wrap(true);
                    error.set_xalign(0.0);
                    append_revealed(&messages_for_send, &error);
                    return false;
                }
            };
            let branch_prefix = WorkspaceStore::open_app(db_for_send.clone())
                .and_then(|store| store.workspace_branch_prefix(&workspace_for_send))
                .unwrap_or_else(|err| {
                    warn!(
                        workspace = %workspace_for_send,
                        error = %err,
                        "failed to load workspace branch prefix for metadata prompt"
                    );
                    "lc".to_owned()
                });
            let prepared_input = prepare_session_send_input(
                &command,
                &workspace_for_send,
                &branch_prefix,
                staged_review,
                selected_kind,
                &thread,
                &thread_messages_for_send,
            );
            let send_input = prepared_input.input;
            let visible_input = prepared_input.visible_input;
            debug!(
                workspace = %workspace_for_send,
                harness = ?selected_kind,
                thread_id,
                has_native_thread_id = thread.native_thread_id.is_some(),
                "session send stage: loaded thread record"
            );
            let thread_records = records
                .iter()
                .filter(|record| record.chat_thread_id == Some(thread_id))
                .cloned()
                .collect::<Vec<_>>();
            let selected_record = {
                let selected_id = *selected_session_for_send.borrow();
                thread_records
                    .iter()
                    .find(|record| Some(record.id) == selected_id)
                    .cloned()
                    .or_else(|| thread_records.first().cloned())
            };
            debug!(
                workspace = %workspace_for_send,
                harness = ?selected_kind,
                thread_id,
                thread_record_count = thread_records.len(),
                selected_record_id = selected_record.as_ref().map(|record| record.id),
                "session send stage: selected thread record"
            );
            let create_turn_checkpoint =
                |session_id: Option<i64>| match create_turn_checkpoint_for_send(
                    &db_for_send,
                    &workspace_for_send,
                    thread_id,
                    session_id,
                    staged_review,
                ) {
                    Ok(checkpoint_id) => Some(checkpoint_id),
                    Err(err) => {
                        warn!(
                            workspace = %workspace_for_send,
                            thread_id,
                            error = %err,
                            "turn checkpoint creation failed before session send queued"
                        );
                        append_session_status_message(
                            &messages_for_send,
                            &format!("[checkpoint] Could not create turn checkpoint: {err:#}"),
                        );
                        None
                    }
                };
            if let Some(harness_descriptor) = selected_harness_descriptor {
                let running_record = thread_records
                    .iter()
                    .find(|record| {
                        record.status == ProcessStatus::Running
                            && session_kind_matches_record(record, harness_descriptor.kind)
                    })
                    .cloned();
                if let Some(record) = running_record.as_ref() {
                    *selected_session_for_send.borrow_mut() = Some(record.id);
                    app_state_for_send.set_selected_agent_session(Some(record.id));
                    if archcar_ready_cache_for_send
                        .borrow()
                        .get(&record.id)
                        .copied()
                        .unwrap_or(false)
                    {
                        let pending_controls =
                            flush_pending_commands_for_send(&pending_commands_for_send, thread_id);
                        for (index, control) in pending_controls.iter().enumerate() {
                            if !queue_archcar_control_send(
                                &archcar_bridge_for_send,
                                inflight_archcar_actions_for_send.as_ref(),
                                thread_id,
                                record.id,
                                control.clone(),
                            ) {
                                requeue_pending_controls(
                                    &pending_commands_for_send,
                                    thread_id,
                                    &pending_controls,
                                    index,
                                );
                                warn!(
                                    thread_id,
                                    process_id = record.id,
                                    "archcar control send failed; requeued pending controls"
                                );
                                break;
                            }
                        }
                        let kind = if staged_review {
                            ArchcarInputKind::ReviewPrompt
                        } else {
                            ArchcarInputKind::User
                        };
                        let checkpoint_id = create_turn_checkpoint(Some(record.id));
                        if !queue_archcar_user_send(
                            &archcar_bridge_for_send,
                            inflight_archcar_actions_for_send.as_ref(),
                            thread_id,
                            record.id,
                            send_input.clone(),
                            visible_input.clone(),
                            kind.clone(),
                            delivery,
                            checkpoint_id,
                            harness_descriptor.kind,
                        ) {
                            if let Some(checkpoint_id) = checkpoint_id {
                                discard_turn_checkpoint(
                                    &db_for_send,
                                    &workspace_for_send,
                                    checkpoint_id,
                                );
                            }
                            append_session_status_message(
                            &messages_for_send,
                            "[archcar] Request channel is closed. Reopen the workspace or restart the app.",
                        );
                            return false;
                        }
                        note_archcar_ready(
                            &mut archcar_ready_cache_for_send.borrow_mut(),
                            record.id,
                            false,
                        );
                        mark_thread_working(working_threads_for_send.as_ref(), thread_id);
                        if staged_review {
                            app_state_for_send.set_staged_review_prompt(None);
                        }
                        info!(
                            workspace = %workspace_for_send,
                            thread_id,
                            process_id = record.id,
                            staged_review,
                            chars = command.len(),
                            "archcar send queued"
                        );
                        refresh_view_for_send();
                        return true;
                    }
                }

                if request_archcar_ensure(
                    &archcar_bridge_for_send,
                    inflight_archcar_actions_for_send.as_ref(),
                    workspace_for_send.clone(),
                    Some(thread_id),
                    harness_descriptor,
                ) {
                    queue_pending_archcar_input(
                        &pending_archcar_inputs_for_send,
                        thread_id,
                        QueuedArchcarInput {
                            input: send_input.clone(),
                            visible_input: visible_input.clone(),
                            kind: if staged_review {
                                ArchcarInputKind::ReviewPrompt
                            } else {
                                ArchcarInputKind::User
                            },
                            session_kind: harness_descriptor.kind,
                        },
                    );
                    apply_codex_startup_signal(
                        &mut codex_startup_states_for_send.borrow_mut(),
                        CodexStartupSignal::Loading { thread_id },
                    );
                    set_codex_ready_state(
                        codex_ready_for_send.as_ref(),
                        update_composer_for_send.as_ref(),
                        false,
                    );
                    mark_thread_working(working_threads_for_send.as_ref(), thread_id);
                } else {
                    apply_codex_startup_signal(
                    &mut codex_startup_states_for_send.borrow_mut(),
                    CodexStartupSignal::Error {
                        thread_id,
                        message:
                            "Request channel is closed. Reopen the workspace or restart the app."
                                .to_owned(),
                    },
                );
                    set_codex_ready_state(
                        codex_ready_for_send.as_ref(),
                        update_composer_for_send.as_ref(),
                        false,
                    );
                    append_session_status_message(
                    &messages_for_send,
                    "[archcar] Request channel is closed. Reopen the workspace or restart the app.",
                );
                    return false;
                }
                info!(
                    workspace = %workspace_for_send,
                    thread_id,
                    running_record = running_record.as_ref().map(|record| record.id),
                    staged_review,
                    chars = command.len(),
                    "queued archcar input while codex session is starting or absent"
                );
                if staged_review {
                    app_state_for_send.set_staged_review_prompt(None);
                }
                refresh_view_for_send();
                return true;
            }
            let running_record = thread_records
                .iter()
                .find(|record| {
                    record.status == ProcessStatus::Running
                        && session_kind_matches_record(record, selected_kind)
                })
                .cloned();

            let Some(record) = running_record else {
                let token = request_archcar_spawn_session(
                    &archcar_bridge_for_send,
                    workspace_for_send.clone(),
                    selected_kind,
                );
                let Some(token) = token else {
                    append_session_status_message(
                    &messages_for_send,
                    "[archcar] Request channel is closed. Reopen the workspace or restart the app.",
                );
                    return false;
                };
                inflight_archcar_actions_for_send.borrow_mut().insert(
                    token,
                    PendingArchcarAction::EnsureWorkspace {
                        workspace: workspace_for_send.clone(),
                        thread_id: Some(thread_id),
                        kind: selected_kind,
                    },
                );
                queue_pending_archcar_input(
                    &pending_archcar_inputs_for_send,
                    thread_id,
                    QueuedArchcarInput {
                        input: send_input.clone(),
                        visible_input: visible_input.clone(),
                        kind: if staged_review {
                            ArchcarInputKind::ReviewPrompt
                        } else {
                            ArchcarInputKind::User
                        },
                        session_kind: selected_kind,
                    },
                );
                let queued = Label::new(Some(
                "[session start] Runtime session requested through archcar. Queued message will send when the session is ready.",
            ));
                queued.add_css_class("chat-agent-text");
                queued.set_selectable(true);
                queued.set_wrap(true);
                queued.set_xalign(0.0);
                append_revealed(&messages_for_send, &queued);
                refresh_view_for_send();
                return true;
            };

            let process_id = record.id;
            *selected_session_for_send.borrow_mut() = Some(process_id);
            app_state_for_send.set_selected_agent_session(Some(process_id));
            active_sessions_for_send.borrow_mut().insert(process_id);

            let pending_controls =
                flush_pending_commands_for_send(&pending_commands_for_send, thread_id);
            for (index, control) in pending_controls.iter().enumerate() {
                if !queue_archcar_control_send(
                    &archcar_bridge_for_send,
                    inflight_archcar_actions_for_send.as_ref(),
                    thread_id,
                    process_id,
                    control.clone(),
                ) {
                    requeue_pending_controls(
                        &pending_commands_for_send,
                        thread_id,
                        &pending_controls,
                        index,
                    );
                    warn!(
                        thread_id,
                        process_id, "archcar control send failed; requeued pending controls"
                    );
                    break;
                }
            }
            let input_kind = if staged_review {
                ArchcarInputKind::ReviewPrompt
            } else {
                ArchcarInputKind::User
            };
            let checkpoint_id = create_turn_checkpoint(Some(process_id));
            if !queue_archcar_user_send(
                &archcar_bridge_for_send,
                inflight_archcar_actions_for_send.as_ref(),
                thread_id,
                process_id,
                send_input.clone(),
                visible_input.clone(),
                input_kind,
                delivery,
                checkpoint_id,
                selected_kind,
            ) {
                if let Some(checkpoint_id) = checkpoint_id {
                    discard_turn_checkpoint(&db_for_send, &workspace_for_send, checkpoint_id);
                }
                append_session_status_message(
                    &messages_for_send,
                    "[archcar] Request channel is closed. Reopen the workspace or restart the app.",
                );
                return false;
            }
            note_archcar_ready(
                &mut archcar_ready_cache_for_send.borrow_mut(),
                process_id,
                false,
            );
            if staged_review {
                app_state_for_send.set_staged_review_prompt(None);
            }
            refresh_view_for_send();
            true
        },
    );
    let send_text: Rc<dyn Fn(String, bool) -> bool> = {
        let send_text_with_delivery = send_text_with_delivery.clone();
        Rc::new(move |text: String, staged_review: bool| {
            send_text_with_delivery(text, staged_review, ArchcarInputDelivery::Auto)
        })
    };
    *send_text_after_ready_queue.borrow_mut() = Some(send_text.clone());

    {
        let db_for_switch = database_path.clone();
        let current_workspace_name_for_switch = current_workspace_name.clone();
        let messages_for_switch = messages.clone();
        let selected_session_for_switch = selected_session.clone();
        let record_state_for_switch = record_state.clone();
        let active_sessions_for_switch = active_sessions.clone();
        let last_output_for_switch = last_output.clone();
        let refresh_view_for_switch = refresh_view.clone();
        let refresh_for_switch = refresh.clone();
        let app_state_for_switch = app_state.clone();
        let thread_state_for_switch = thread_state.clone();
        let selected_thread_for_switch = selected_thread.clone();
        let update_composer_for_switch = update_composer_state.clone();
        let restore_composer_draft_for_switch = restore_composer_draft.clone();
        let refresh_queue_overlay_for_switch = refresh_queue_overlay.clone();
        let toast_for_switch = toast_manager.clone();
        let eager_start_chat_agent_for_switch = eager_start_chat_agent.clone();
        let switch_action = Rc::new(move |next_kind: SessionKind| {
            let workspace_for_switch = current_workspace_name_for_switch.borrow().clone();
            let (records, threads) =
                match WorkspaceStore::open_app(db_for_switch.clone()).map(|store| {
                    (
                        store
                            .list_sessions(&workspace_for_switch)
                            .unwrap_or_default(),
                        store
                            .list_chat_threads(&workspace_for_switch)
                            .unwrap_or_default(),
                    )
                }) {
                    Ok(result) => result,
                    Err(err) => {
                        let message = format!("[session switch] {err:#}");
                        toast_for_switch.error(message.clone());
                        let error = Label::new(Some(&message));
                        error.add_css_class("chat-agent-text");
                        error.set_selectable(true);
                        error.set_wrap(true);
                        error.set_xalign(0.0);
                        append_revealed(&messages_for_switch, &error);
                        return;
                    }
                };
            {
                let mut current = record_state_for_switch.borrow_mut();
                current.clear();
                current.extend(records.clone());
            }
            {
                let mut current = thread_state_for_switch.borrow_mut();
                current.clear();
                current.extend(threads.clone());
            }

            let next_thread = preferred_thread_for_kind(
                &threads,
                *selected_thread_for_switch.borrow(),
                next_kind,
            );
            apply_thread_selection(
                selected_thread_for_switch.as_ref(),
                next_thread,
                |thread_id| app_state_for_switch.set_selected_chat_thread(thread_id),
                || {
                    restore_composer_draft_for_switch();
                    update_composer_for_switch();
                    if let Some(refresh) =
                        refresh_queue_overlay_for_switch.borrow().as_ref().cloned()
                    {
                        refresh();
                    }
                },
            );
            if let Some(thread_id) = next_thread {
                eager_start_chat_agent_for_switch(
                    workspace_for_switch.clone(),
                    thread_id,
                    next_kind,
                );
            }
            let next_process = next_thread.and_then(|thread_id| {
                let thread_records = records
                    .iter()
                    .filter(|record| record.chat_thread_id == Some(thread_id))
                    .cloned()
                    .collect::<Vec<_>>();
                preferred_session_for_kind(
                    &thread_records,
                    *selected_session_for_switch.borrow(),
                    next_kind,
                )
            });
            *selected_session_for_switch.borrow_mut() = next_process;
            app_state_for_switch.set_selected_agent_session(next_process);
            refresh_view_for_switch();
            let _ = (
                &db_for_switch,
                &workspace_for_switch,
                &messages_for_switch,
                &active_sessions_for_switch,
                &last_output_for_switch,
                &refresh_for_switch,
            );
        });
        *switch_chat_harness.borrow_mut() = Some(switch_action.clone());
        let initial_kind = selected_harness_snapshot(selected_harness.as_ref());
        switch_action(initial_kind);
    }

    if let Some(external_chat_tabs) = external_chat_tabs.as_ref() {
        let selected_thread = selected_thread.clone();
        let refresh_chat_surface = refresh_chat_surface.clone();
        let app_state = app_state.clone();
        let update_composer_state = update_composer_state.clone();
        let restore_composer_draft = restore_composer_draft.clone();
        let refresh_queue_overlay = refresh_queue_overlay.clone();
        let eager_start_chat_agent = eager_start_chat_agent.clone();
        let thread_state = thread_state.clone();
        let current_workspace_name = current_workspace_name.clone();
        *external_chat_tabs.selection_controller.borrow_mut() = Some(Rc::new(move |thread_id| {
            apply_thread_selection(
                selected_thread.as_ref(),
                thread_id,
                |selected| app_state.set_selected_chat_thread(selected),
                || {
                    restore_composer_draft();
                    update_composer_state();
                    if let Some(refresh) = refresh_queue_overlay.borrow().as_ref().cloned() {
                        refresh();
                    }
                },
            );
            if let Some(thread_id) = thread_id {
                if let Some(thread) = thread_state
                    .borrow()
                    .iter()
                    .find(|thread| thread.id == thread_id)
                    .cloned()
                {
                    let (kind, _) = selected_thread_harness_state(&thread);
                    eager_start_chat_agent(
                        current_workspace_name.borrow().clone(),
                        thread_id,
                        kind,
                    );
                }
            }
            if let Some(refresh_view) = clone_refresh_chat_surface_controller(&refresh_chat_surface)
            {
                refresh_view();
            }
        }));
    }

    if let Some(prompt) = app_state.take_pending_chat_prompt() {
        let buffer = buffer.clone();
        let input_view = input_view.clone();
        let update_composer_state = update_composer_state.clone();
        gtk::glib::idle_add_local_once(move || {
            let current = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), true)
                .to_string();
            if current.trim().is_empty() {
                buffer.set_text(&prompt);
            } else {
                buffer.set_text(&format!("{}\n\n{}", current.trim_end(), prompt));
            }
            input_view.grab_focus();
            update_composer_state();
        });
    }

    let interrupt_current_session = Rc::new({
        let selected_thread = selected_thread.clone();
        let selected_harness = selected_harness.clone();
        let record_state = record_state.clone();
        let archcar_bridge = archcar_bridge.clone();
        let inflight_archcar_actions = inflight_archcar_actions.clone();
        let archcar_ready_cache = archcar_ready_cache.clone();
        let queued_auto_drain_holds = queued_auto_drain_holds.clone();
        let codex_ready = codex_ready.clone();
        let toast_manager = toast_manager.clone();
        move || {
            let Some(thread_id) = *selected_thread.borrow() else {
                return;
            };
            let Some(session_id) = running_session_for_thread(
                &record_state.borrow(),
                thread_id,
                *selected_harness.borrow(),
            ) else {
                return;
            };
            if queue_archcar_turn_interrupt(
                &archcar_bridge,
                inflight_archcar_actions.as_ref(),
                thread_id,
                session_id,
            ) {
                hold_queued_auto_drain(queued_auto_drain_holds.as_ref(), thread_id);
                note_archcar_ready(&mut archcar_ready_cache.borrow_mut(), session_id, false);
                set_codex_ready_state(&codex_ready, &|| {}, false);
            } else {
                toast_manager.error(
                    "Could not interrupt the active agent turn because archcar is unavailable."
                        .to_owned(),
                );
            }
        }
    });

    let send_immediate_input: Rc<dyn Fn(String, bool) -> bool> = Rc::new({
        let send_text_with_delivery = send_text_with_delivery.clone();
        move |command: String, staged_review: bool| -> bool {
            send_text_with_delivery(command, staged_review, ArchcarInputDelivery::Immediate)
        }
    });
    *send_immediate_after_ready_queue.borrow_mut() = Some(send_immediate_input.clone());

    let submit_composer_action = Rc::new({
        let buffer = buffer.clone();
        let selected_thread = selected_thread.clone();
        let selected_harness = selected_harness.clone();
        let record_state = record_state.clone();
        let archcar_ready_cache = archcar_ready_cache.clone();
        let codex_startup_states = codex_startup_states.clone();
        let app_state = app_state.clone();
        let working_threads = working_threads.clone();
        let refresh_view = refresh_view.clone();
        let send_text = send_text.clone();
        let update_composer_state = update_composer_state.clone();
        let interrupt_current_session = interrupt_current_session.clone();
        let send_immediate_input = send_immediate_input.clone();
        let refresh_queue_overlay = refresh_queue_overlay.clone();
        move |intent: ComposerSubmitIntent| {
            let command = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), true)
                .to_string();
            let has_text = !command.trim().is_empty();
            let thread_id = *selected_thread.borrow();
            let chat_target = selected_chat_target_for_submit(&app_state, thread_id);
            let action_thread_id = composer_thread_for_target(chat_target.as_ref(), thread_id);
            if let Some(thread_id) = action_thread_id {
                if app_state.editing_queued_chat_input(thread_id).is_some() {
                    if has_text {
                        if let Some(previous) =
                            app_state.save_editing_queued_chat_input(thread_id, command.clone())
                        {
                            buffer.set_text(&previous);
                            refresh_view();
                        }
                    }
                    update_composer_state();
                    if let Some(refresh) = refresh_queue_overlay.borrow().as_ref().cloned() {
                        refresh();
                    }
                    return;
                }
            }
            let has_active_generation = thread_id.is_some_and(|thread_id| {
                active_generation_for_thread(
                    &record_state.borrow(),
                    &working_threads,
                    thread_id,
                    *selected_harness.borrow(),
                )
            });
            let latest_status = action_thread_id.and_then(|thread_id| {
                latest_session_status_for_thread(
                    &record_state.borrow(),
                    thread_id,
                    *selected_harness.borrow(),
                )
            });
            let queued_count = action_thread_id
                .map(|thread_id| queued_chat_inputs_count(&app_state, thread_id))
                .unwrap_or_default();
            let current_harness = *selected_harness.borrow();
            let managed_harness_waits = managed_harness_for_kind(current_harness).is_some();
            let managed_thread_ready = if managed_harness_waits {
                thread_id.is_none_or(|thread_id| {
                    managed_thread_ready_for_ui(
                        current_harness,
                        thread_id,
                        &record_state.borrow(),
                        archcar_ready_cache.as_ref(),
                        codex_startup_states.borrow().get(&thread_id),
                    )
                })
            } else {
                true
            };
            let managed_waiting_for_startup = managed_harness_waits
                && action_thread_id.is_some_and(|thread_id| {
                    chat_thread_waiting_for_starting_agent(&app_state, thread_id)
                })
                && !managed_thread_ready
                && !has_active_generation;
            let action = composer_action_for_startup_state(
                has_text,
                has_active_generation,
                latest_status == Some(ProcessStatus::Stopped),
                queued_count,
                managed_waiting_for_startup,
            );
            let selected_kind = *selected_harness.borrow();
            match composer_action_for_submit_intent(
                action,
                intent,
                has_text,
                selected_kind,
                managed_waiting_for_startup,
            ) {
                ComposerAction::Queue => {
                    if let Some(target @ ChatUiTarget::Pending { .. }) = chat_target.clone() {
                        queue_archcar_input_for_target(
                            &app_state,
                            target,
                            command.clone(),
                            None,
                            ArchcarInputKind::User,
                            *selected_harness.borrow(),
                        );
                        buffer.set_text("");
                        refresh_view();
                        update_composer_state();
                        return;
                    }
                    let Some(thread_id) = thread_id else {
                        buffer.set_text("");
                        (send_text)(command, false);
                        update_composer_state();
                        return;
                    };
                    queue_archcar_input(
                        &app_state,
                        thread_id,
                        command.clone(),
                        None,
                        ArchcarInputKind::User,
                        *selected_harness.borrow(),
                    );
                    buffer.set_text("");
                    refresh_view();
                    update_composer_state();
                }
                ComposerAction::Interrupt => {
                    interrupt_current_session();
                    update_composer_state();
                }
                ComposerAction::SendQueued => {
                    let Some(thread_id) = action_thread_id else {
                        return;
                    };
                    if let Some(queued_input) = pop_next_queued_chat_input(&app_state, thread_id) {
                        let staged_review =
                            matches!(queued_input.kind, ArchcarInputKind::ReviewPrompt);
                        if !(send_text)(queued_input.input.clone(), staged_review) {
                            requeue_pending_input_front(&app_state, thread_id, queued_input);
                        } else {
                            release_queued_auto_drain_if_queue_empty(
                                queued_auto_drain_holds.as_ref(),
                                thread_id,
                                queued_chat_inputs_count(&app_state, thread_id),
                            );
                        }
                    }
                    update_composer_state();
                }
                ComposerAction::Retry => {
                    (send_text)(retry_agent_prompt().to_owned(), false);
                    update_composer_state();
                }
                ComposerAction::Send => {
                    if has_text {
                        buffer.set_text("");
                        if let Some(target @ ChatUiTarget::Pending { .. }) = chat_target.clone() {
                            queue_archcar_input_for_target(
                                &app_state,
                                target,
                                command.clone(),
                                None,
                                ArchcarInputKind::User,
                                *selected_harness.borrow(),
                            );
                            refresh_view();
                            update_composer_state();
                            return;
                        }
                        if intent == ComposerSubmitIntent::Immediate
                            && managed_harness_for_kind(selected_kind).is_some()
                        {
                            if !send_immediate_input(command.clone(), false) {
                                (send_text)(command, false);
                            }
                        } else {
                            (send_text)(command, false);
                        }
                    }
                    update_composer_state();
                }
                ComposerAction::SaveQueuedEdit => {}
                ComposerAction::Disabled => {}
            }
            if let Some(refresh) = refresh_queue_overlay.borrow().as_ref().cloned() {
                refresh();
            }
        }
    });

    let composer_keybind = EventControllerKey::new();
    composer_keybind.connect_key_pressed({
        let submit_composer_action = submit_composer_action.clone();
        move |_, keyval, _, modifiers| {
            guarded_gtk_callback(gtk::glib::Propagation::Proceed, || {
                if !should_send_composer_message(keyval, modifiers) {
                    return gtk::glib::Propagation::Proceed;
                }

                submit_composer_action(composer_submit_intent_for_modifiers(modifiers));
                gtk::glib::Propagation::Stop
            })
        }
    });
    input_view.add_controller(composer_keybind);
    focus_btn.connect_clicked({
        let input_view = input_view.clone();
        move |_| {
            input_view.grab_focus();
        }
    });
    let keybind = EventControllerKey::new();
    keybind.connect_key_pressed({
        let input_view = input_view.clone();
        move |_, keyval, _, modifiers| {
            let ctrl = modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK);
            if ctrl
                && keyval
                    .to_unicode()
                    .is_some_and(|ch| ch.eq_ignore_ascii_case(&'l'))
            {
                input_view.grab_focus();
                return gtk::glib::Propagation::Stop;
            }
            gtk::glib::Propagation::Proceed
        }
    });
    root.add_controller(keybind);

    send_btn.connect_clicked({
        let submit_composer_action = submit_composer_action.clone();
        move |_| {
            submit_composer_action(composer_send_button_submit_intent());
        }
    });
    new_chat_btn.connect_clicked({
        let database_path = database_path.clone();
        let current_workspace_name = current_workspace_name.clone();
        let selected_harness = selected_harness.clone();
        let selected_model = selected_model.clone();
        let thread_state = thread_state.clone();
        let selected_thread = selected_thread.clone();
        let external_chat_tabs = external_chat_tabs.clone();
        let app_state = app_state.clone();
        let refresh_view = refresh_view.clone();
        let update_composer_state = update_composer_state.clone();
        let restore_composer_draft = restore_composer_draft.clone();
        let refresh_queue_overlay = refresh_queue_overlay.clone();
        let setup_readiness = setup_readiness.clone();
        let toast_manager = toast_manager.clone();
        let eager_start_chat_agent = eager_start_chat_agent.clone();
        move |_| {
            let workspace_name = current_workspace_name.borrow().clone();
            let kind = *selected_harness.borrow();
            if let Some(message) = selected_provider_blocker_after_refresh(kind, &setup_readiness) {
                toast_manager.error(message);
                error!(workspace = %workspace_name, harness = ?kind, "refusing to create chat for unready provider");
                return;
            }
            let title = default_chat_thread_title(kind, &thread_state.borrow());
            let metadata =
                provider_model_harness_metadata(None, selected_model.borrow().as_deref());
            let pending_target = app_state.create_pending_chat_target(workspace_name.clone(), kind);
            app_state.mark_chat_phase(
                pending_target.clone(),
                ChatUiPhase::Creating { provider: kind },
            );
            let on_selected: Rc<dyn Fn()> = Rc::new({
                let restore_composer_draft = restore_composer_draft.clone();
                let update_composer_state = update_composer_state.clone();
                let refresh_queue_overlay = refresh_queue_overlay.clone();
                move || {
                    restore_composer_draft();
                    update_composer_state();
                    if let Some(refresh) = refresh_queue_overlay.borrow().as_ref().cloned() {
                        refresh();
                    }
                }
            });
            update_composer_state();
            if let Some(refresh) = refresh_queue_overlay.borrow().as_ref().cloned() {
                refresh();
            }
            spawn_session_chat_thread_create(
                database_path.clone(),
                workspace_name,
                kind,
                title,
                metadata,
                pending_target,
                SessionChatCreateUi {
                    app_state: app_state.clone(),
                    selected_thread: selected_thread.clone(),
                    thread_state: thread_state.clone(),
                    external_chat_tabs: external_chat_tabs.clone(),
                    refresh_view: refresh_view.clone(),
                    eager_start: eager_start_chat_agent.clone(),
                    on_selected,
                    toast_manager: toast_manager.clone(),
                },
            );
        }
    });
    root
}

pub(crate) fn session_header_row(
    repository_name: &str,
    branch_name: &str,
    collapse_sidebar: Rc<dyn Fn()>,
) -> GBox {
    session_header_row_with_branch_label(repository_name, branch_name, collapse_sidebar).0
}

pub(crate) fn session_header_row_with_branch_label(
    repository_name: &str,
    branch_name: &str,
    collapse_sidebar: Rc<dyn Fn()>,
) -> (GBox, Label) {
    let header = GBox::new(Orientation::Horizontal, 10);
    header.add_css_class("chat-header-row");
    header.set_hexpand(true);

    let breadcrumb = GBox::new(Orientation::Horizontal, 8);
    breadcrumb.set_hexpand(true);

    let repo_icon = Image::from_icon_name(resolve_icon_name("folder-symbolic"));
    repo_icon.add_css_class("chat-repo-icon");
    breadcrumb.append(&repo_icon);

    let repo_label = Label::new(Some(repository_name));
    repo_label.add_css_class("chat-repo-label");
    repo_label.set_xalign(0.0);
    repo_label.set_hexpand(false);
    repo_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    repo_label.set_width_chars(1);
    repo_label.set_max_width_chars(34);
    breadcrumb.append(&repo_label);

    let branch_sep = Label::new(Some(">"));
    branch_sep.add_css_class("chat-branch-separator");
    breadcrumb.append(&branch_sep);

    let branch_label = Label::new(Some(branch_name));
    branch_label.add_css_class("chat-branch-label");
    branch_label.set_xalign(0.0);
    branch_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    branch_label.set_width_chars(1);
    breadcrumb.append(&branch_label);

    let editor_picker = editor_picker_button();
    let sidebar_btn = icon_button("sidebar-hide-symbolic", "Collapse right sidebar");
    sidebar_btn.add_css_class("chat-focus-btn");
    sidebar_btn.set_tooltip_text(Some("Collapse right sidebar"));
    sidebar_btn.connect_clicked(move |_| collapse_sidebar());
    header.append(&breadcrumb);
    header.append(&editor_picker);
    header.append(&sidebar_btn);
    (header, branch_label)
}

fn chat_user_bubble(text: &str) -> GBox {
    let row = GBox::new(Orientation::Horizontal, 0);
    row.set_halign(gtk::Align::Fill);
    row.set_hexpand(true);
    row.add_css_class("chat-user-row");

    let bubble = Label::new(None);
    bubble.set_markup(&chat_text_markup(text));
    bubble.add_css_class("chat-user-bubble");
    bubble.set_selectable(true);
    bubble.set_wrap(true);
    bubble.set_xalign(0.0);
    bubble.set_hexpand(true);
    bubble.set_halign(gtk::Align::Fill);
    row.append(&bubble);
    row
}

fn queued_composer_overlay_row(
    item: &QueuedComposerItem,
    on_delete: Rc<dyn Fn()>,
    on_edit: Rc<dyn Fn()>,
    on_send_immediately: Rc<dyn Fn()>,
) -> GBox {
    let row = GBox::new(Orientation::Horizontal, 8);
    row.add_css_class("chat-queued-composer-row");
    row.set_halign(gtk::Align::Fill);
    row.set_hexpand(true);

    let body = Label::new(None);
    body.set_markup(&chat_text_markup(&item.preview));
    body.add_css_class("chat-queued-composer-body");
    body.set_xalign(0.0);
    body.set_wrap(true);
    body.set_hexpand(true);
    body.set_halign(gtk::Align::Fill);
    row.append(&body);

    let actions = GBox::new(Orientation::Horizontal, 2);
    actions.add_css_class("chat-queued-actions");
    actions.set_halign(Align::End);

    if queued_composer_item_allows_action(item, QueuedComposerAction::Edit) {
        let edit_btn = icon_button("document-edit-symbolic", "Edit queued message");
        edit_btn.add_css_class("chat-queued-action-btn");
        edit_btn.connect_clicked(move |_| on_edit());
        actions.append(&edit_btn);
    }

    if queued_composer_item_allows_action(item, QueuedComposerAction::Delete) {
        let delete_btn = icon_button("user-trash-symbolic", "Delete queued message");
        delete_btn.add_css_class("chat-queued-action-btn");
        delete_btn.connect_clicked(move |_| on_delete());
        actions.append(&delete_btn);
    }

    if queued_composer_item_allows_action(item, QueuedComposerAction::SendImmediately) {
        let send_btn = icon_button("send-symbolic", "Steer now");
        send_btn.add_css_class("chat-queued-action-btn");
        send_btn.connect_clicked(move |_| on_send_immediately());
        actions.append(&send_btn);
    }

    row.append(&actions);
    row
}

fn queued_composer_editing_row(on_cancel: Rc<dyn Fn()>) -> GBox {
    let row = GBox::new(Orientation::Horizontal, 8);
    row.add_css_class("chat-queue-editing-row");
    row.set_halign(gtk::Align::Fill);
    row.set_hexpand(true);

    let label = Label::new(Some("Editing queued message"));
    label.add_css_class("chat-queue-editing-label");
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_halign(gtk::Align::Fill);
    row.append(&label);

    let cancel_btn = icon_button("window-close-symbolic", "Cancel queued message edit");
    cancel_btn.add_css_class("chat-queued-action-btn");
    cancel_btn.connect_clicked(move |_| on_cancel());
    row.append(&cancel_btn);

    row
}

fn queued_composer_item_allows_action(
    item: &QueuedComposerItem,
    action: QueuedComposerAction,
) -> bool {
    item.actions.contains(&action)
}

fn append_chat_refresh_row<W: IsA<Widget>>(container: &GBox, child: &W) {
    if reveal_existing_chat_refresh_rows() {
        append_revealed(container, child);
    } else {
        container.append(child);
    }
}

fn append_chat_timeline_items(
    container: &GBox,
    items: &[ChatTimelineItem],
    transcript_display: &str,
    render_legacy_inline_events: bool,
) {
    for item in items {
        if let Some(widget) = chat_timeline_item_widget(
            item,
            render_raw_message_content(transcript_display),
            render_legacy_inline_events,
        ) {
            append_chat_refresh_row(container, &widget);
        }
    }
}

fn remove_box_children_from(container: &GBox, start: usize) {
    let mut index = 0;
    let mut child = container.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if index >= start {
            container.remove(&widget);
        }
        index += 1;
    }
}

fn capture_chat_scroll(scroll: &ScrolledWindow) -> ChatScrollSnapshot {
    let adjustment = scroll.vadjustment();
    ChatScrollSnapshot {
        value: adjustment.value(),
        pinned_to_bottom: chat_scroll_is_pinned_to_bottom(
            adjustment.value(),
            adjustment.lower(),
            adjustment.upper(),
            adjustment.page_size(),
        ),
    }
}

fn restore_chat_scroll_after_refresh(scroll: &ScrolledWindow, snapshot: ChatScrollSnapshot) {
    let adjustment = scroll.vadjustment();
    schedule_chat_scroll_restore_pass(
        adjustment,
        snapshot,
        chat_scroll_restore_layout_passes(snapshot),
    );
}

fn schedule_chat_scroll_restore_pass(
    adjustment: Adjustment,
    snapshot: ChatScrollSnapshot,
    remaining_passes: u8,
) {
    if remaining_passes == 0 {
        return;
    }
    gtk::glib::idle_add_local_once(move || {
        apply_chat_scroll_restore(&adjustment, snapshot);
        if remaining_passes > 1 {
            // PER-190: One-shot layout settling pass for chat scroll restore;
            // bounded retries end after the current refresh has stabilized.
            gtk::glib::timeout_add_local_once(
                Duration::from_millis(CHAT_SCROLL_RESTORE_LAYOUT_PASS_MS),
                move || {
                    schedule_chat_scroll_restore_pass(adjustment, snapshot, remaining_passes - 1);
                },
            );
        }
    });
}

fn chat_scroll_is_pinned_to_bottom(value: f64, lower: f64, upper: f64, page_size: f64) -> bool {
    chat_scroll_max_value(lower, upper, page_size) - value <= CHAT_SCROLL_BOTTOM_EPSILON
}

fn apply_chat_scroll_restore(adjustment: &Adjustment, snapshot: ChatScrollSnapshot) {
    let value = restored_chat_scroll_value(
        snapshot,
        adjustment.lower(),
        adjustment.upper(),
        adjustment.page_size(),
    );
    adjustment.set_value(value);
}

fn chat_scroll_restore_layout_passes(_snapshot: ChatScrollSnapshot) -> u8 {
    CHAT_SCROLL_RESTORE_LAYOUT_PASSES
}

fn restored_chat_scroll_value(
    snapshot: ChatScrollSnapshot,
    lower: f64,
    upper: f64,
    page_size: f64,
) -> f64 {
    let max_value = chat_scroll_max_value(lower, upper, page_size);
    if snapshot.pinned_to_bottom {
        max_value
    } else {
        snapshot.value.clamp(lower, max_value)
    }
}

fn chat_scroll_max_value(lower: f64, upper: f64, page_size: f64) -> f64 {
    (upper - page_size).max(lower)
}

fn reveal_existing_chat_refresh_rows() -> bool {
    REVEAL_EXISTING_CHAT_REFRESH_ROWS
}

fn install_archcar_wake(root: &GBox, bridge: &AsyncArchcarBridge, refresh_view: Rc<dyn Fn()>) {
    let wake_id = NEXT_CHAT_WAKE_ID.fetch_add(1, Ordering::Relaxed);
    let wake_pending = Arc::new(AtomicBool::new(false));
    CHAT_WAKE_REGISTRY.with(|registry| {
        registry.borrow_mut().insert(wake_id, refresh_view);
    });
    root.connect_destroy(move |_| {
        CHAT_WAKE_REGISTRY.with(|registry| {
            registry.borrow_mut().remove(&wake_id);
        });
    });

    let main_context = gtk::glib::MainContext::default();
    let root_ref = gtk::glib::SendWeakRef::from(root.downgrade());
    bridge.set_waker(move || {
        if !mark_chat_refresh_wake_pending(wake_pending.as_ref()) {
            return;
        }
        let root_ref = root_ref.clone();
        let wake_pending = Arc::clone(&wake_pending);
        main_context.invoke(move || {
            // PER-190: One-shot debounce for Archcar event bursts; the pending
            // flag is cleared when the scheduled refresh runs or the surface is
            // already gone.
            gtk::glib::timeout_add_local_once(
                Duration::from_millis(CHAT_REFRESH_WAKE_DELAY_MS),
                move || {
                    clear_chat_refresh_wake_pending(wake_pending.as_ref());
                    if root_ref.upgrade().is_none() {
                        CHAT_WAKE_REGISTRY.with(|registry| {
                            registry.borrow_mut().remove(&wake_id);
                        });
                        return;
                    }
                    let refresh = CHAT_WAKE_REGISTRY
                        .with(|registry| registry.borrow().get(&wake_id).cloned());
                    if let Some(refresh) = refresh {
                        refresh();
                    }
                },
            );
        });
    });
}

fn mark_chat_refresh_wake_pending(pending: &AtomicBool) -> bool {
    !pending.swap(true, Ordering::AcqRel)
}

fn clear_chat_refresh_wake_pending(pending: &AtomicBool) {
    pending.store(false, Ordering::Release);
}

fn install_working_indicator_tick(
    root: &GBox,
    working_threads: Rc<RefCell<HashMap<i64, Instant>>>,
    refresh_view: Rc<dyn Fn()>,
) {
    let root_ref = root.downgrade();
    // PER-190: User-visible elapsed-time UI for active Codex generation.
    // The timer exits when the owning chat surface is destroyed.
    gtk::glib::timeout_add_seconds_local(1, move || {
        if root_ref.upgrade().is_none() {
            return gtk::glib::ControlFlow::Break;
        }
        if !working_threads.borrow().is_empty() {
            refresh_view();
        }
        gtk::glib::ControlFlow::Continue
    });
}

fn session_transcript_event_widget(event: &SessionTranscriptEvent) -> Widget {
    match event.role {
        SessionTranscriptRole::User | SessionTranscriptRole::ReviewPrompt => {
            chat_user_bubble(&event.body).upcast()
        }
        SessionTranscriptRole::Tool | SessionTranscriptRole::Skill => {
            let inline_events = session_transcript_inline_events(event);
            if !inline_events.is_empty() {
                return inline_events_widget(&inline_events);
            }
            session_transcript_label_widget(event)
        }
        _ => session_transcript_label_widget(event),
    }
}

fn session_transcript_label_widget(event: &SessionTranscriptEvent) -> Widget {
    chat_text_label(&format!("{}\n{}", event.role.label(), event.body)).upcast()
}

fn session_transcript_inline_events(event: &SessionTranscriptEvent) -> Vec<CodexInlineEvent> {
    match event.role {
        SessionTranscriptRole::Tool | SessionTranscriptRole::Skill => event
            .body
            .lines()
            .next()
            .and_then(|line| parse_session_transcript_inline_event(event.role, line, &event.body))
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_session_transcript_inline_event(
    role: SessionTranscriptRole,
    header: &str,
    body: &str,
) -> Option<CodexInlineEvent> {
    if let Some(command) = header.trim().strip_prefix("Ran ") {
        if let Some(event) = read_only_command_inline_event(command, body) {
            return Some(event);
        }
        return Some(CodexInlineEvent {
            kind: CodexInlineEventKind::Tool,
            title: command.trim().to_owned(),
            subtitle: Some("Command result".to_owned()),
            body: Some(body.to_owned()),
            path: None,
            status: codex_event_status_from_line(body),
        });
    }
    if let Some(event) = read_file_inline_event(role, header, body) {
        return Some(event);
    }
    if role == SessionTranscriptRole::Tool {
        if let Some(event) = raw_write_tool_inline_event(header, body) {
            return Some(event);
        }
    }
    let event = if role == SessionTranscriptRole::Tool && is_raw_file_change_event_line(header) {
        CoreCodexInlineEvent::FileChange(parse_codex_file_change_block(body)?)
    } else {
        parse_codex_inline_event(header.trim())?
    };
    let mut inline = codex_inline_event_from_core(event, header.trim())?;
    inline.body = Some(body.to_owned());
    if role == SessionTranscriptRole::Skill {
        inline.kind = CodexInlineEventKind::Skill;
    }
    Some(inline)
}

fn read_file_inline_event(
    role: SessionTranscriptRole,
    header: &str,
    body: &str,
) -> Option<CodexInlineEvent> {
    let path = read_file_path_from_header(header)?;
    let title = if let Some(skill_name) = skill_name_for_skill_md_read(
        &path,
        skill_name_from_read_header(header).map(str::to_owned),
    ) {
        format!("Read SKILL.md for {skill_name}")
    } else {
        format!("Read {}", read_path_display_name(&path))
    };
    let is_skill_read =
        role == SessionTranscriptRole::Skill || skill_name_for_skill_md_read(&path, None).is_some();
    Some(CodexInlineEvent {
        kind: if is_skill_read {
            CodexInlineEventKind::Skill
        } else {
            CodexInlineEventKind::Tool
        },
        title,
        subtitle: Some(if is_skill_read {
            "Skill".to_owned()
        } else {
            "File preview".to_owned()
        }),
        body: Some(body.to_owned()),
        path: Some(path),
        status: CodexInlineEventStatus::Complete,
    })
}

fn read_file_path_from_header(header: &str) -> Option<PathBuf> {
    let rest = header
        .trim()
        .strip_prefix('•')
        .map(str::trim_start)
        .unwrap_or_else(|| header.trim())
        .strip_prefix("Read ")?;
    let raw_path = rest.split_whitespace().next()?.trim_matches(|ch: char| {
        matches!(ch, '`' | '"' | '\'' | ',' | ';' | '(' | ')' | '[' | ']')
    });
    is_readable_path_token(raw_path).then(|| PathBuf::from(raw_path))
}

fn is_readable_path_token(value: &str) -> bool {
    !value.is_empty()
        && (value.starts_with('/')
            || value.starts_with("./")
            || value.starts_with("../")
            || value.contains('/')
            || value.contains('\\')
            || value.contains('.'))
}

fn skill_name_from_read_header(header: &str) -> Option<&str> {
    let start = header.rfind('(')?;
    let end = header[start + 1..].find(')')? + start + 1;
    let skill = header[start + 1..end].trim();
    (!skill.is_empty()).then_some(skill)
}

fn read_only_command_inline_event(command: &str, body: &str) -> Option<CodexInlineEvent> {
    let path = read_only_command_path(command)?;
    let title = if let Some(skill_name) = skill_name_for_skill_md_read(&path, None) {
        format!("Read SKILL.md for {skill_name}")
    } else {
        format!("Read {}", read_path_display_name(&path))
    };
    let is_skill_read = skill_name_for_skill_md_read(&path, None).is_some();
    Some(CodexInlineEvent {
        kind: if is_skill_read {
            CodexInlineEventKind::Skill
        } else {
            CodexInlineEventKind::Tool
        },
        title,
        subtitle: Some(if is_skill_read {
            "Skill".to_owned()
        } else {
            "File preview".to_owned()
        }),
        body: Some(body.to_owned()),
        path: Some(path),
        status: codex_event_status_from_line(body),
    })
}

fn read_only_command_path(command: &str) -> Option<PathBuf> {
    let trimmed = command.trim();
    if let Some(inner) = shell_command_inner(trimmed) {
        if let Some(path) = read_only_command_path(&inner) {
            return Some(path);
        }
    }
    let tokens = shell_command_tokens(trimmed);
    let executable = tokens.first()?;
    let executable = command_executable_name(executable);
    if !matches!(
        executable,
        "sed" | "cat" | "head" | "tail" | "nl" | "bat" | "batcat" | "less" | "more"
    ) {
        return None;
    }
    tokens
        .iter()
        .filter_map(|token| clean_read_command_path_token(token))
        .find(|token| is_readable_path_token(token))
        .map(PathBuf::from)
}

fn shell_command_inner(command: &str) -> Option<String> {
    let tokens = shell_command_tokens(command);
    let executable = tokens.first().map(|token| command_executable_name(token))?;
    if !matches!(executable, "bash" | "sh" | "zsh" | "dash") {
        return None;
    }
    tokens
        .windows(2)
        .find_map(|window| matches!(window[0].as_str(), "-c" | "-lc").then(|| window[1].clone()))
        .filter(|inner| !inner.trim().is_empty())
}

fn command_executable_name(executable: &str) -> &str {
    executable
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(executable)
}

fn shell_command_tokens(command: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None::<char>;
    let mut escaped = false;

    for ch in command.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        if matches!(ch, '"' | '\'') {
            quote = Some(ch);
            continue;
        }
        if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(ch);
    }

    if escaped {
        current.push('\\');
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn clean_read_command_path_token(token: &str) -> Option<&str> {
    let trimmed = token.trim_matches(|ch: char| {
        matches!(
            ch,
            '`' | '"' | '\'' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}'
        )
    });
    if let Some(index) = trimmed.find("SKILL.md") {
        return Some(&trimmed[..index + "SKILL.md".len()]);
    }
    Some(trimmed).filter(|value| {
        !value.starts_with('-')
            && !value.chars().all(|ch| ch.is_ascii_digit() || ch == ',')
            && !matches!(*value, "|" | "&&" | "||" | "\\")
    })
}

fn read_path_display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| read_value_display_name(&path.display().to_string()))
}

fn read_value_display_name(value: &str) -> String {
    let trimmed = value.trim().trim_matches(|ch: char| {
        matches!(
            ch,
            '`' | '"' | '\'' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}'
        )
    });
    trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(trimmed)
        .to_owned()
}

fn skill_name_for_skill_md_read(path: &Path, explicit_name: Option<String>) -> Option<String> {
    if path.file_name().and_then(|name| name.to_str()) != Some("SKILL.md") {
        return None;
    }
    explicit_name.or_else(|| {
        path.parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
    })
}

fn merge_chat_timeline_for_render(
    messages: Vec<ChatMessageRecord>,
    events: Vec<ChatEventRecord>,
) -> Vec<ChatTimelineItem> {
    let mut items = messages
        .into_iter()
        .map(ChatTimelineItem::Message)
        .chain(events.into_iter().map(ChatTimelineItem::Event))
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        let left_seq = chat_timeline_item_sort_key(left);
        let right_seq = chat_timeline_item_sort_key(right);
        left_seq.cmp(&right_seq)
    });
    items
}

fn chat_timeline_item_sort_key(item: &ChatTimelineItem) -> (i64, u8, i64) {
    match item {
        ChatTimelineItem::Message(message) => {
            (message.timeline_seq.unwrap_or(i64::MAX - 2), 0, message.id)
        }
        ChatTimelineItem::Event(event) => (event.timeline_seq, 1, event.id),
        ChatTimelineItem::ProviderProjection(item) => (
            i64::MAX - 3,
            2,
            i64::try_from(item.sequence).unwrap_or(i64::MAX),
        ),
        ChatTimelineItem::InterruptedNotice { sequence } => (*sequence, 3, i64::MAX - 1),
        ChatTimelineItem::OptimisticUserInput(_) => (i64::MAX, 4, i64::MAX),
    }
}

fn chat_structured_items_for_render(
    messages: Vec<ChatMessageRecord>,
    events: Vec<ChatEventRecord>,
    provider_projection_items: Vec<ProviderProjectionItem>,
    _pending_inputs: Vec<String>,
    optimistic_immediate_inputs: Vec<String>,
) -> Vec<ChatTimelineItem> {
    let (mut items, mut unsequenced_messages): (Vec<_>, Vec<_>) =
        merge_chat_timeline_for_render(messages.clone(), events)
            .into_iter()
            .filter(|item| match item {
                ChatTimelineItem::Message(message) => chat_message_is_renderable(message),
                _ => true,
            })
            .partition(|item| {
                !matches!(
                    item,
                    ChatTimelineItem::Message(message) if message.timeline_seq.is_none()
                )
            });
    let mut provider_items =
        provider_projection_items_for_timeline(provider_projection_items, &messages);
    let interrupted_sequence = interrupted_notice_sequence(&messages, &provider_items);
    provider_items.retain(|item| !provider_projection_item_is_interrupted_notice(item));
    if provider_items
        .iter()
        .any(|item| item.render_class == ProjectionRenderClass::UserChat)
    {
        items.append(&mut unsequenced_messages);
        items = chat_timeline_items_anchored_to_provider_user_events(items, provider_items);
    } else {
        items.extend(
            provider_items
                .into_iter()
                .map(ChatTimelineItem::ProviderProjection),
        );
        items.append(&mut unsequenced_messages);
    }
    if let Some(sequence) = interrupted_sequence {
        items.push(ChatTimelineItem::InterruptedNotice { sequence });
        items.sort_by(|left, right| {
            let left_seq = chat_timeline_item_sort_key(left);
            let right_seq = chat_timeline_item_sort_key(right);
            left_seq.cmp(&right_seq)
        });
    }
    items.extend(
        optimistic_immediate_inputs
            .into_iter()
            .map(ChatTimelineItem::OptimisticUserInput),
    );
    items
}

fn chat_timeline_items_anchored_to_provider_user_events(
    mut persisted_items: Vec<ChatTimelineItem>,
    provider_items: Vec<ProviderProjectionItem>,
) -> Vec<ChatTimelineItem> {
    let mut items = Vec::new();
    for provider_item in provider_items {
        if provider_item.render_class == ProjectionRenderClass::UserChat {
            let user_body = provider_projection_user_body_for_render(&provider_item);
            if let Some(index) = matching_persisted_user_message_index(&persisted_items, &user_body)
            {
                items.push(persisted_items.remove(index));
            } else if !user_body.is_empty() {
                items.push(ChatTimelineItem::ProviderProjection(provider_item));
            }
            continue;
        }
        items.push(ChatTimelineItem::ProviderProjection(provider_item));
    }
    items.extend(persisted_items);
    items
}

fn matching_persisted_user_message_index(items: &[ChatTimelineItem], body: &str) -> Option<usize> {
    items.iter().position(|item| {
        matches!(
            item,
            ChatTimelineItem::Message(message)
                if message.role == "user" && message.content.trim() == body.trim()
        )
    })
}

fn interrupted_notice_sequence(
    messages: &[ChatMessageRecord],
    provider_items: &[ProviderProjectionItem],
) -> Option<i64> {
    let interrupted = provider_items
        .iter()
        .filter(|item| provider_projection_item_is_interrupted_notice(item))
        .filter_map(|item| item.timeline_seq)
        .max()?;
    let latest_user = messages
        .iter()
        .filter(|message| message.role == "user")
        .filter_map(|message| message.timeline_seq)
        .max();
    if latest_user.is_some_and(|sequence| sequence > interrupted) {
        None
    } else {
        Some(interrupted)
    }
}

fn chat_message_is_renderable(message: &ChatMessageRecord) -> bool {
    message.role != "user" || !message.content.trim().is_empty()
}

fn chat_timeline_item_key(item: &ChatTimelineItem) -> ChatTimelineItemKey {
    match item {
        ChatTimelineItem::Message(message) => ChatTimelineItemKey::Message((
            message.id,
            message.role.clone(),
            message.timeline_seq,
            message.updated_at.clone(),
            message.content.clone(),
        )),
        ChatTimelineItem::Event(event) => ChatTimelineItemKey::Event((
            event.id,
            event.kind.clone(),
            event.timeline_seq,
            event.title.clone(),
            event.updated_at.clone(),
            event.body.len() + event.payload_json.len(),
        )),
        ChatTimelineItem::ProviderProjection(item) => ChatTimelineItemKey::Provider((
            item.id.clone(),
            i64::try_from(item.sequence).unwrap_or(i64::MAX),
            item.timeline_seq,
            ProviderEventKind::Unknown,
            ProviderEventPhase::Unknown,
            item.title.len() + item.body.len() + item.raw_payload.as_deref().unwrap_or("").len(),
        )),
        ChatTimelineItem::InterruptedNotice { sequence } => {
            ChatTimelineItemKey::InterruptedNotice(*sequence)
        }
        ChatTimelineItem::OptimisticUserInput(input) => {
            ChatTimelineItemKey::OptimisticUserInput(input.clone())
        }
    }
}

fn chat_timeline_render_state(
    thread_id: i64,
    transcript_display: &str,
    items: &[ChatTimelineItem],
) -> ChatTimelineRenderState {
    chat_timeline_render_state_with_leading_rows(thread_id, transcript_display, items, 0)
}

fn chat_timeline_render_state_with_leading_rows(
    thread_id: i64,
    transcript_display: &str,
    items: &[ChatTimelineItem],
    leading_rows: usize,
) -> ChatTimelineRenderState {
    ChatTimelineRenderState {
        thread_id,
        transcript_display: transcript_display.to_owned(),
        leading_rows,
        keys: items.iter().map(chat_timeline_item_key).collect(),
    }
}

fn chat_timeline_refresh_plan(
    previous: Option<&ChatTimelineRenderState>,
    next: &ChatTimelineRenderState,
) -> ChatTimelineRefreshPlan {
    let Some(previous) = previous else {
        return ChatTimelineRefreshPlan::RebuildMessages;
    };
    if previous.thread_id != next.thread_id
        || previous.transcript_display != next.transcript_display
        || previous.leading_rows != next.leading_rows
    {
        return ChatTimelineRefreshPlan::RebuildMessages;
    }
    let first_changed = previous
        .keys
        .iter()
        .zip(next.keys.iter())
        .position(|(left, right)| left != right);
    if first_changed.is_none() && previous.keys.len() == next.keys.len() {
        return ChatTimelineRefreshPlan::Skip;
    }
    if first_changed.is_none() && previous.keys.len() > next.keys.len() {
        if next.keys.is_empty() {
            return ChatTimelineRefreshPlan::RebuildMessages;
        }
        return ChatTimelineRefreshPlan::RebuildFrom {
            start: next.keys.len(),
        };
    }
    if first_changed.is_none() {
        if previous.keys.is_empty() {
            return ChatTimelineRefreshPlan::RebuildMessages;
        }
        return ChatTimelineRefreshPlan::Append {
            start: previous.keys.len(),
        };
    }
    let first_changed = first_changed.unwrap_or(0);
    if first_changed == 0 {
        return ChatTimelineRefreshPlan::RebuildMessages;
    }
    ChatTimelineRefreshPlan::RebuildFrom {
        start: first_changed,
    }
}

fn chat_render_is_unchanged(
    last: &RefCell<Option<ChatRenderSignature>>,
    next: ChatRenderSignature,
) -> bool {
    if last.borrow().as_ref() == Some(&next) {
        return true;
    }
    *last.borrow_mut() = Some(next);
    false
}

fn chat_render_signature(
    current_kind: SessionKind,
    selected_thread_id: Option<i64>,
    active_record: Option<i64>,
    startup_state: CodexStartupState,
    working_elapsed_seconds: Option<u64>,
    records: &[ProcessRecord],
    threads: &[ChatThreadRecord],
    messages: &[ChatMessageRecord],
    events: &[ChatEventRecord],
    provider_events: &[ProviderEventRecord],
    pending_inputs: &[String],
    transcript_display: &str,
    render_state: &'static str,
    runtime_summary: Option<String>,
) -> ChatRenderSignature {
    ChatRenderSignature {
        current_kind,
        selected_thread_id,
        active_record,
        startup_state,
        working_elapsed_seconds,
        records: records
            .iter()
            .map(|record| {
                (
                    record.id,
                    record.chat_thread_id,
                    record.status,
                    record.exit_code,
                    record.ended_at.clone(),
                )
            })
            .collect(),
        threads: threads
            .iter()
            .map(|thread| {
                (
                    thread.id,
                    thread.provider.clone(),
                    thread.title.clone(),
                    thread.status.clone(),
                    thread.updated_at.clone(),
                )
            })
            .collect(),
        messages: messages
            .iter()
            .map(|message| {
                (
                    message.id,
                    message.role.clone(),
                    message.timeline_seq,
                    message.updated_at.clone(),
                    message.content.clone(),
                )
            })
            .collect(),
        events: events
            .iter()
            .map(|event| {
                (
                    event.id,
                    event.kind.clone(),
                    event.timeline_seq,
                    event.title.clone(),
                    event.updated_at.clone(),
                    event.body.len() + event.payload_json.len(),
                )
            })
            .collect(),
        provider_events: provider_events
            .iter()
            .map(|event| {
                (
                    event.identity_key.clone(),
                    event.received_sequence,
                    event.timeline_seq,
                    event.kind,
                    event.phase,
                    event.normalized_payload.to_string().len() + event.raw_json.to_string().len(),
                )
            })
            .collect(),
        pending_inputs: pending_inputs
            .iter()
            .enumerate()
            .map(|(index, input)| (index, input.clone()))
            .collect(),
        transcript_display: transcript_display.to_owned(),
        render_state,
        runtime_summary,
    }
}

fn chat_message_widget(
    message: &ChatMessageRecord,
    render_raw_message_content: bool,
    render_legacy_inline_events: bool,
) -> Option<Widget> {
    if !chat_message_is_renderable(message) {
        return None;
    }
    match message.role.as_str() {
        "user" => Some(chat_user_bubble(&message.content).upcast()),
        "system" => {
            let label = Label::new(Some(&message.content));
            label.add_css_class("card-meta");
            label.set_selectable(true);
            label.set_wrap(true);
            label.set_xalign(0.0);
            label.set_margin_bottom(12);
            Some(label.upcast())
        }
        _ => {
            let inline_events =
                legacy_inline_events_for_message(message, render_legacy_inline_events);
            if !inline_events.is_empty() {
                return Some(inline_events_widget(&inline_events));
            }
            let content = chat_agent_message_display_content(message, render_raw_message_content);
            if content.trim().is_empty() {
                return None;
            }
            Some(chat_text_label(&content).upcast())
        }
    }
}

fn chat_timeline_item_widget(
    item: &ChatTimelineItem,
    render_raw_message_content: bool,
    render_legacy_inline_events: bool,
) -> Option<Widget> {
    match item {
        ChatTimelineItem::Message(message) => chat_message_widget(
            message,
            render_raw_message_content,
            render_legacy_inline_events,
        ),
        ChatTimelineItem::Event(event) => Some(chat_event_widget(event)),
        ChatTimelineItem::ProviderProjection(item) => Some(provider_projection_item_widget(item)),
        ChatTimelineItem::InterruptedNotice { .. } => Some(chat_interrupted_notice_widget()),
        ChatTimelineItem::OptimisticUserInput(input) => Some(chat_user_bubble(input).upcast()),
    }
}

fn chat_agent_message_display_content(
    message: &ChatMessageRecord,
    render_raw_message_content: bool,
) -> String {
    let content = if render_raw_message_content {
        message.content.clone()
    } else {
        strip_codex_status_blocks(&message.content)
    };
    strip_archductor_metadata_block(&content)
}

fn strip_codex_status_blocks(content: &str) -> String {
    let mut kept = Vec::new();
    let mut skipping_status_block = false;

    for line in content.lines() {
        if is_codex_status_block_header(line) {
            skipping_status_block = true;
            continue;
        }

        if skipping_status_block && is_codex_status_block_continuation(line) {
            continue;
        }

        skipping_status_block = false;
        kept.push(line.to_owned());
    }

    kept.join("\n").trim().to_owned()
}

fn is_codex_status_block_header(line: &str) -> bool {
    normalize_chat_status_line(line) == "Explored"
}

fn is_codex_status_block_continuation(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.is_empty()
        || line.starts_with(' ')
        || line.starts_with('\t')
        || trimmed.starts_with('└')
        || trimmed.starts_with('↳')
}

fn normalize_chat_status_line(line: &str) -> &str {
    line.trim()
        .strip_prefix('•')
        .map(str::trim_start)
        .unwrap_or_else(|| line.trim())
}

fn legacy_inline_events_for_message(
    message: &ChatMessageRecord,
    render_legacy_inline_events: bool,
) -> Vec<CodexInlineEvent> {
    if render_legacy_inline_events && message.role != "user" && message.role != "system" {
        parse_codex_inline_events_local(&message.content)
    } else {
        Vec::new()
    }
}

fn chat_event_widget(event: &ChatEventRecord) -> Widget {
    stored_chat_event_inline_event(event)
        .map(|inline| inline_event_widget(&inline))
        .unwrap_or_else(|| chat_text_label(&event.body).upcast())
}

fn provider_projection_items_for_render(
    items: Vec<ProviderProjectionItem>,
    messages: &[ChatMessageRecord],
) -> Vec<ProviderProjectionItem> {
    items
        .into_iter()
        .filter(provider_projection_item_is_relevant_chat_event)
        .filter(|item| item.render_class != ProjectionRenderClass::HookCard)
        .filter(|item| !provider_projection_item_is_interrupted_notice(item))
        .filter(provider_projection_item_has_renderable_content)
        .filter(|item| !provider_projection_item_has_persisted_message(item, messages))
        .collect()
}

fn provider_projection_items_for_timeline(
    items: Vec<ProviderProjectionItem>,
    messages: &[ChatMessageRecord],
) -> Vec<ProviderProjectionItem> {
    items
        .into_iter()
        .filter(provider_projection_item_is_relevant_chat_event)
        .filter(|item| item.render_class != ProjectionRenderClass::HookCard)
        .filter(provider_projection_timeline_item_has_renderable_content)
        .filter(|item| {
            item.render_class == ProjectionRenderClass::UserChat
                || !provider_projection_item_has_persisted_message(item, messages)
        })
        .collect()
}

fn provider_projection_item_is_interrupted_notice(item: &ProviderProjectionItem) -> bool {
    item.category == ProviderProjectionCategory::Status
        && item.status == ProviderProjectionStatus::Canceled
        && (item.title.to_ascii_lowercase().contains("turn")
            || item.body.to_ascii_lowercase().contains("interrupt")
            || item.body.to_ascii_lowercase().contains("cancel"))
}

fn provider_projection_item_has_renderable_content(item: &ProviderProjectionItem) -> bool {
    match item.render_class {
        ProjectionRenderClass::UserChat => false,
        ProjectionRenderClass::AssistantChat => {
            !provider_projection_assistant_body_for_render(item)
                .trim()
                .is_empty()
        }
        ProjectionRenderClass::ReasoningCard => !item.body.trim().is_empty(),
        _ => true,
    }
}

fn provider_projection_timeline_item_has_renderable_content(item: &ProviderProjectionItem) -> bool {
    match item.render_class {
        ProjectionRenderClass::UserChat => {
            !provider_projection_user_body_for_render(item).is_empty()
        }
        _ => provider_projection_item_has_renderable_content(item),
    }
}

fn provider_projection_item_has_persisted_message(
    item: &ProviderProjectionItem,
    messages: &[ChatMessageRecord],
) -> bool {
    item.render_class == ProjectionRenderClass::UserChat
        && messages.iter().any(|message| {
            message.role == "user"
                && message.content.trim() == provider_projection_user_body_for_render(item)
        })
}

fn provider_projection_user_body_for_render(item: &ProviderProjectionItem) -> String {
    recovered_current_user_message(&item.body).unwrap_or_else(|| item.body.trim().to_owned())
}

fn recovered_current_user_message(body: &str) -> Option<String> {
    let (_, current) = body.split_once(CODEX_RECOVERY_CURRENT_USER_MESSAGE_HEADER)?;
    Some(current.trim().to_owned()).filter(|value| !value.is_empty())
}

fn apply_provider_projection_agent_metadata(
    database_path: &Path,
    thread_id: i64,
    items: &[ProviderProjectionItem],
) -> Option<AgentMetadataUiUpdate> {
    if !items.iter().any(|item| {
        item.render_class == ProjectionRenderClass::AssistantChat
            && item.body.contains("<archductor_metadata>")
    }) {
        return None;
    }

    let store = WorkspaceStore::open_app(database_path).ok()?;
    for item in items.iter().filter(|item| {
        item.render_class == ProjectionRenderClass::AssistantChat
            && item.body.contains("<archductor_metadata>")
    }) {
        if let Err(err) = store.apply_agent_chat_metadata_directive(thread_id, &item.body) {
            warn!(
                thread_id,
                error = %err,
                "failed to apply provider projection metadata"
            );
        }
    }
    let thread = store.get_chat_thread_record(thread_id).ok()?;
    let workspace = store.get_workspace_record(thread.workspace_id).ok()?;
    Some(AgentMetadataUiUpdate {
        workspace_name: workspace.name,
        branch_name: workspace.branch,
        thread,
    })
}

fn apply_agent_metadata_ui_update(
    app_state: &AppState,
    current_workspace_name: &RefCell<String>,
    current_branch_name: &RefCell<String>,
    thread_state: &RefCell<Vec<ChatThreadRecord>>,
    update: AgentMetadataUiUpdate,
) -> AgentMetadataUiChanges {
    let old_workspace_name = current_workspace_name.borrow().clone();
    let workspace_changed = old_workspace_name != update.workspace_name;
    if workspace_changed {
        app_state.rename_workspace_in_navigation(&old_workspace_name, &update.workspace_name);
        *current_workspace_name.borrow_mut() = update.workspace_name.clone();
    }

    let branch_changed = *current_branch_name.borrow() != update.branch_name;
    if branch_changed {
        *current_branch_name.borrow_mut() = update.branch_name.clone();
    }

    let mut threads = thread_state.borrow_mut();
    let chat_title_changed = threads
        .iter()
        .find(|thread| thread.id == update.thread.id)
        .is_some_and(|thread| thread.title != update.thread.title);
    if chat_title_changed {
        if let Some(thread) = threads
            .iter_mut()
            .find(|thread| thread.id == update.thread.id)
        {
            *thread = update.thread;
        }
    }

    AgentMetadataUiChanges {
        workspace_changed,
        branch_changed,
        chat_title_changed,
    }
}

fn provider_projection_item_widget(item: &ProviderProjectionItem) -> Widget {
    if let Some(inline_event) = provider_projection_inline_event(item) {
        return inline_event_widget(&inline_event);
    }

    match item.render_class {
        ProjectionRenderClass::UserChat => {
            chat_user_bubble(&provider_projection_user_body_for_render(item)).upcast()
        }
        ProjectionRenderClass::AssistantChat => {
            provider_projection_text_widget(&provider_projection_assistant_body_for_render(item))
        }
        ProjectionRenderClass::ReasoningCard => provider_projection_reasoning_widget(item),
        _ => {
            let container = GBox::new(Orientation::Vertical, 4);
            container.set_hexpand(true);
            if provider_projection_item_shows_status_chrome(item) {
                let status = Label::new(Some(&provider_projection_status_line(item)));
                status.add_css_class("card-meta");
                status.add_css_class(provider_projection_status_css_class(item.status));
                status.set_xalign(0.0);
                status.set_wrap(true);
                container.append(&status);
            }
            container.append(&provider_projection_text_widget(
                &provider_projection_card_text(item),
            ));
            container.upcast()
        }
    }
}

fn provider_projection_assistant_body_for_render(item: &ProviderProjectionItem) -> String {
    strip_archductor_metadata_block(&item.body)
}

fn provider_projection_reasoning_text(item: &ProviderProjectionItem) -> Option<String> {
    let body = item.body.trim();
    (!body.is_empty()).then(|| body.to_owned())
}

fn provider_projection_reasoning_widget(item: &ProviderProjectionItem) -> Widget {
    let label = Label::new(None);
    label.set_markup(&chat_text_markup(
        &provider_projection_reasoning_text(item).unwrap_or_default(),
    ));
    label.add_css_class("chat-reasoning-text");
    label.set_selectable(true);
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.upcast()
}

fn provider_projection_item_shows_status_chrome(item: &ProviderProjectionItem) -> bool {
    matches!(
        item.status,
        ProviderProjectionStatus::Failed | ProviderProjectionStatus::Canceled
    )
}

fn provider_projection_inline_event(item: &ProviderProjectionItem) -> Option<CodexInlineEvent> {
    let title = provider_projection_display_title(item);
    let body = provider_projection_inline_event_body(item);
    if item.render_class == ProjectionRenderClass::DiffCard {
        if let Some(event) = provider_projection_file_change_inline_event(item, body.as_deref()) {
            return Some(event);
        }
    }
    if matches!(
        item.render_class,
        ProjectionRenderClass::CommandCard
            | ProjectionRenderClass::ProcessCard
            | ProjectionRenderClass::BackgroundCard
    ) {
        if let Some(mut event) =
            read_only_command_inline_event(&title, body.as_deref().unwrap_or_default())
        {
            event.body = body;
            event.status = codex_inline_status_from_provider_projection(item.status);
            return Some(event);
        }
    }

    let kind = match item.render_class {
        ProjectionRenderClass::SkillCard => CodexInlineEventKind::Skill,
        ProjectionRenderClass::CommandCard
        | ProjectionRenderClass::ProcessCard
        | ProjectionRenderClass::FileCard
        | ProjectionRenderClass::DiffCard
        | ProjectionRenderClass::ToolCard
        | ProjectionRenderClass::PluginCard
        | ProjectionRenderClass::SubagentCard
        | ProjectionRenderClass::NestedTranscriptCard
        | ProjectionRenderClass::BackgroundCard => CodexInlineEventKind::Tool,
        _ => return None,
    };
    let title = provider_projection_inline_event_title(item);

    Some(CodexInlineEvent {
        kind,
        title,
        subtitle: Some(provider_projection_inline_event_subtitle(item).to_owned()),
        body,
        path: None,
        status: codex_inline_status_from_provider_projection(item.status),
    })
}

#[derive(Debug, Clone)]
struct ProviderFileChangeSummary {
    action: CoreCodexFileChangeAction,
    path: String,
    additions: Option<u32>,
    deletions: Option<u32>,
    diff: Option<String>,
}

fn provider_projection_file_change_inline_event(
    item: &ProviderProjectionItem,
    body: Option<&str>,
) -> Option<CodexInlineEvent> {
    let change = provider_file_change_from_raw_payload(item.raw_payload.as_deref())
        .or_else(|| provider_file_change_from_body(body.unwrap_or_default()))?;
    let display_name = read_value_display_name(&change.path);
    let title = format!(
        "{} {}",
        codex_file_change_action_label(change.action),
        display_name
    );
    let counts = codex_file_change_counts(change.additions, change.deletions);
    let body = provider_file_change_body(&change);

    Some(CodexInlineEvent {
        kind: CodexInlineEventKind::Tool,
        title,
        subtitle: counts,
        body: Some(body),
        path: Some(PathBuf::from(change.path)),
        status: codex_inline_status_from_provider_projection(item.status),
    })
}

fn provider_file_change_body(change: &ProviderFileChangeSummary) -> String {
    let mut header = format!(
        "{} {}",
        codex_file_change_action_label(change.action),
        change.path
    );
    if let Some(counts) = codex_file_change_counts(change.additions, change.deletions) {
        header.push_str(&format!(" ({counts})"));
    }
    if let Some(diff) = change
        .diff
        .as_deref()
        .map(str::trim)
        .filter(|diff| !diff.is_empty())
    {
        format!("{header}\n{diff}")
    } else {
        header
    }
}

fn provider_file_change_from_raw_payload(
    raw_payload: Option<&str>,
) -> Option<ProviderFileChangeSummary> {
    let payload = serde_json::from_str::<serde_json::Value>(raw_payload?).ok()?;
    let changes = payload.pointer("/params/item/changes")?.as_array()?;
    let change = changes.first()?;
    let path = change.get("path").and_then(serde_json::Value::as_str)?;
    let action = provider_file_change_action(
        change
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default(),
    );
    let diff = change
        .get("diff")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .filter(|diff| !diff.trim().is_empty());
    let (diff_additions, diff_deletions) = diff
        .as_deref()
        .map(provider_diff_counts)
        .unwrap_or((None, None));

    Some(ProviderFileChangeSummary {
        action,
        path: path.to_owned(),
        additions: change
            .get("additions")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .or(diff_additions),
        deletions: change
            .get("deletions")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .or(diff_deletions),
        diff,
    })
}

fn provider_file_change_from_body(body: &str) -> Option<ProviderFileChangeSummary> {
    body.lines().find_map(|line| {
        let trimmed = line.trim();
        let (action, path) = provider_file_change_body_line(trimmed)?;
        Some(ProviderFileChangeSummary {
            action,
            path: path.to_owned(),
            additions: parse_file_change_count_from_text(trimmed, '+'),
            deletions: parse_file_change_count_from_text(trimmed, '-'),
            diff: None,
        })
    })
}

fn provider_file_change_body_line(line: &str) -> Option<(CoreCodexFileChangeAction, &str)> {
    for (prefix, action) in [
        ("added ", CoreCodexFileChangeAction::Added),
        ("created ", CoreCodexFileChangeAction::Added),
        ("edited ", CoreCodexFileChangeAction::Edited),
        ("modified ", CoreCodexFileChangeAction::Edited),
        ("changed ", CoreCodexFileChangeAction::Edited),
        ("deleted ", CoreCodexFileChangeAction::Deleted),
        ("removed ", CoreCodexFileChangeAction::Deleted),
    ] {
        if let Some(path) = line.strip_prefix(prefix) {
            return Some((action, path.split_whitespace().next()?));
        }
        let title_prefix = title_case_prefix(prefix);
        if let Some(path) = line.strip_prefix(&title_prefix) {
            return Some((action, path.split_whitespace().next()?));
        }
    }
    None
}

fn title_case_prefix(prefix: &str) -> String {
    let mut chars = prefix.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    format!("{}{}", first.to_ascii_uppercase(), chars.as_str())
}

fn provider_file_change_action(kind: &str) -> CoreCodexFileChangeAction {
    match kind.to_ascii_lowercase().as_str() {
        "added" | "add" | "created" | "create" | "new" => CoreCodexFileChangeAction::Added,
        "deleted" | "delete" | "removed" | "remove" => CoreCodexFileChangeAction::Deleted,
        _ => CoreCodexFileChangeAction::Edited,
    }
}

fn provider_diff_counts(diff: &str) -> (Option<u32>, Option<u32>) {
    let mut additions = 0_u32;
    let mut deletions = 0_u32;
    for line in diff.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            additions = additions.saturating_add(1);
        } else if line.starts_with('-') && !line.starts_with("---") {
            deletions = deletions.saturating_add(1);
        }
    }
    (Some(additions), Some(deletions))
}

fn parse_file_change_count_from_text(text: &str, sign: char) -> Option<u32> {
    text.split(|ch: char| ch.is_whitespace() || matches!(ch, '(' | ')' | ',' | ';'))
        .find_map(|token| {
            let digits = token.strip_prefix(sign)?;
            (!digits.is_empty())
                .then(|| digits.parse::<u32>().ok())
                .flatten()
        })
}

fn provider_projection_inline_event_title(item: &ProviderProjectionItem) -> String {
    let title = provider_projection_display_title(item);
    let trimmed = title.trim();
    match item.render_class {
        ProjectionRenderClass::CommandCard | ProjectionRenderClass::ProcessCard => {
            format!("Ran {trimmed}")
        }
        ProjectionRenderClass::FileCard | ProjectionRenderClass::DiffCard => {
            format!("Read {}", read_value_display_name(trimmed))
        }
        ProjectionRenderClass::SkillCard => format!("Read {trimmed}"),
        ProjectionRenderClass::PluginCard => format!("Used {trimmed}"),
        ProjectionRenderClass::ToolCard => format!("Used {trimmed}"),
        ProjectionRenderClass::SubagentCard => format!("Ran {trimmed}"),
        ProjectionRenderClass::NestedTranscriptCard => format!("Opened {trimmed}"),
        ProjectionRenderClass::BackgroundCard => format!("Ran {trimmed}"),
        _ => title,
    }
}

fn provider_projection_inline_event_subtitle(item: &ProviderProjectionItem) -> &'static str {
    match item.render_class {
        ProjectionRenderClass::CommandCard => "Command",
        ProjectionRenderClass::ProcessCard => "Process",
        ProjectionRenderClass::FileCard => "File",
        ProjectionRenderClass::DiffCard => "Diff",
        ProjectionRenderClass::ToolCard => "Tool",
        ProjectionRenderClass::SkillCard => "Skill",
        ProjectionRenderClass::PluginCard => "Plugin",
        ProjectionRenderClass::SubagentCard => "Subagent",
        ProjectionRenderClass::NestedTranscriptCard => "Nested transcript",
        ProjectionRenderClass::BackgroundCard => "Background task",
        _ => "Provider item",
    }
}

fn provider_projection_inline_event_body(item: &ProviderProjectionItem) -> Option<String> {
    let body = item.body.trim();
    if !body.is_empty() {
        return Some(provider_projection_plain_body(body));
    }
    item.raw_payload
        .as_deref()
        .and_then(provider_projection_payload_action_body)
}

fn provider_projection_plain_body(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .map(|value| provider_projection_plain_payload_lines(&value, 0).join("\n"))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| body.to_owned())
}

fn provider_projection_payload_action_body(raw_payload: &str) -> Option<String> {
    let payload = serde_json::from_str::<serde_json::Value>(raw_payload).ok()?;
    let mut lines = Vec::new();

    if let Some(server) =
        provider_projection_string_at_any(&payload, &["/params/item/server", "/params/server"])
    {
        lines.push(format!("server: {server}"));
    }
    if let Some(command) = provider_projection_string_at_any(
        &payload,
        &["/params/item/command", "/params/command", "/command"],
    ) {
        lines.push(format!("command: {command}"));
    }
    if let Some(arguments) = provider_projection_value_at_any(
        &payload,
        &[
            "/params/item/arguments",
            "/params/arguments",
            "/arguments",
            "/params/input",
            "/input",
        ],
    ) {
        let arguments = provider_projection_plain_payload_lines(arguments, 0);
        if !arguments.is_empty() {
            lines.push("arguments:".to_owned());
            lines.extend(arguments.into_iter().map(|line| format!("  {line}")));
        }
    }

    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn provider_projection_plain_payload_lines(value: &serde_json::Value, depth: usize) -> Vec<String> {
    match value {
        serde_json::Value::Object(map) => map
            .iter()
            .flat_map(|(key, value)| match value {
                serde_json::Value::Object(_) | serde_json::Value::Array(_) if depth < 2 => {
                    let mut lines = vec![format!("{key}:")];
                    lines.extend(
                        provider_projection_plain_payload_lines(value, depth + 1)
                            .into_iter()
                            .map(|line| format!("  {line}")),
                    );
                    lines
                }
                _ => provider_projection_display_json_value(value)
                    .map(|value| vec![format!("{key}: {value}")])
                    .unwrap_or_default(),
            })
            .collect(),
        serde_json::Value::Array(values) => values
            .iter()
            .filter_map(provider_projection_display_json_value)
            .map(|value| format!("- {value}"))
            .collect(),
        _ => provider_projection_display_json_value(value)
            .map(|value| vec![value])
            .unwrap_or_default(),
    }
}

fn codex_inline_status_from_provider_projection(
    status: ProviderProjectionStatus,
) -> CodexInlineEventStatus {
    match status {
        ProviderProjectionStatus::Pending | ProviderProjectionStatus::Running => {
            CodexInlineEventStatus::Loading
        }
        ProviderProjectionStatus::Complete => CodexInlineEventStatus::Complete,
        ProviderProjectionStatus::Failed | ProviderProjectionStatus::Canceled => {
            CodexInlineEventStatus::Failed
        }
    }
}

fn provider_projection_card_text(item: &ProviderProjectionItem) -> String {
    let mut text = provider_projection_display_title(item);
    if !item.body.trim().is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&item.body);
    }
    if provider_projection_card_shows_raw_details(item) {
        if let Some(raw_payload) = item.raw_payload.as_deref() {
            if !raw_payload.trim().is_empty() {
                if !text.is_empty() {
                    text.push_str("\n\n");
                }
                text.push_str(raw_payload);
            }
        }
    }
    if text.trim().is_empty() && item.inspectable {
        text.push_str("Provider details available for inspection.");
    }
    text
}

fn provider_projection_display_title(item: &ProviderProjectionItem) -> String {
    if !provider_projection_title_is_generic_event_name(&item.title) {
        return item.title.clone();
    }
    item.raw_payload
        .as_deref()
        .and_then(provider_projection_payload_action_title)
        .unwrap_or_else(|| item.title.clone())
}

fn provider_projection_title_is_generic_event_name(title: &str) -> bool {
    let title = title.trim();
    title.contains('/')
        && title
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '_' | '-' | '.'))
}

fn provider_projection_payload_action_title(raw_payload: &str) -> Option<String> {
    let payload = serde_json::from_str::<serde_json::Value>(raw_payload).ok()?;
    provider_projection_payload_value_title(&payload)
}

fn provider_projection_payload_value_title(payload: &serde_json::Value) -> Option<String> {
    let tool = provider_projection_string_at_any(
        payload,
        &[
            "/params/item/tool",
            "/params/tool",
            "/params/tool_name",
            "/tool_name",
            "/params/item/action",
            "/params/action",
            "/params/hook/name",
            "/hook/name",
            "/params/hook_event_name",
            "/hook_event_name",
            "/params/name",
        ],
    );
    if tool.as_deref().is_some_and(|tool| !tool.trim().is_empty()) {
        return tool;
    }

    provider_projection_value_at_any(
        payload,
        &["/params/item/command", "/params/command", "/command"],
    )
    .and_then(provider_projection_display_json_value)
}

fn provider_projection_string_at_any(
    value: &serde_json::Value,
    pointers: &[&str],
) -> Option<String> {
    provider_projection_value_at_any(value, pointers)
        .and_then(provider_projection_display_json_value)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn provider_projection_value_at_any<'a>(
    value: &'a serde_json::Value,
    pointers: &[&str],
) -> Option<&'a serde_json::Value> {
    pointers.iter().find_map(|pointer| value.pointer(pointer))
}

fn provider_projection_display_json_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Array(values) => {
            let parts = values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>();
            (!parts.is_empty()).then(|| parts.join(" "))
        }
        serde_json::Value::Null => None,
        other => Some(other.to_string()),
    }
}

fn provider_projection_card_shows_raw_details(item: &ProviderProjectionItem) -> bool {
    matches!(
        item.render_class,
        ProjectionRenderClass::PromptCard
            | ProjectionRenderClass::WebCard
            | ProjectionRenderClass::ImageCard
            | ProjectionRenderClass::ErrorCard
            | ProjectionRenderClass::FallbackCard
    )
}

fn provider_projection_status_line(item: &ProviderProjectionItem) -> String {
    provider_projection_status_label(item.status).to_owned()
}

fn provider_projection_status_label(status: ProviderProjectionStatus) -> &'static str {
    match status {
        ProviderProjectionStatus::Pending => "Pending",
        ProviderProjectionStatus::Running => "Running",
        ProviderProjectionStatus::Complete => "Complete",
        ProviderProjectionStatus::Failed => "Failed",
        ProviderProjectionStatus::Canceled => "Canceled",
    }
}

fn provider_projection_stream_state_label(
    stream_state: ProviderProjectionStreamState,
) -> &'static str {
    match stream_state {
        ProviderProjectionStreamState::Snapshot => "Snapshot",
        ProviderProjectionStreamState::Streaming => "Streaming",
        ProviderProjectionStreamState::Complete => "Complete",
    }
}

fn provider_projection_status_css_class(status: ProviderProjectionStatus) -> &'static str {
    match status {
        ProviderProjectionStatus::Pending => "status-pending",
        ProviderProjectionStatus::Running => "status-running",
        ProviderProjectionStatus::Complete => "status-success",
        ProviderProjectionStatus::Failed => "status-error",
        ProviderProjectionStatus::Canceled => "status-warning",
    }
}

fn provider_projection_text_widget(text: &str) -> Widget {
    let label = chat_text_label(text);
    label.upcast()
}

fn chat_text_label(text: &str) -> Label {
    let label = Label::new(None);
    label.set_markup(&chat_text_markup(text));
    label.add_css_class("chat-agent-text");
    label.set_selectable(true);
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label
}

fn chat_text_markup(text: &str) -> String {
    let mut markup = String::new();
    let mut rest = text;

    while !rest.is_empty() {
        if let Some(start) = rest.find('`') {
            markup.push_str(&pango_escape_text(&rest[..start]));
            let after_start = &rest[start..];
            if let Some(stripped) = after_start.strip_prefix("```") {
                if let Some(end) = stripped.find("```") {
                    let code = &stripped[..end];
                    markup.push_str(&chat_code_span_markup(code.trim_matches('\n')));
                    rest = &stripped[end + 3..];
                } else {
                    markup.push_str(&pango_escape_text(after_start));
                    break;
                }
            } else {
                let stripped = &after_start[1..];
                if let Some(end) = stripped.find('`') {
                    let code = &stripped[..end];
                    markup.push_str(&chat_code_span_markup(code));
                    rest = &stripped[end + 1..];
                } else {
                    markup.push_str(&pango_escape_text(after_start));
                    break;
                }
            }
        } else {
            markup.push_str(&pango_escape_text(rest));
            break;
        }
    }

    markup
}

fn chat_code_span_markup(code: &str) -> String {
    format!(
        "<span font_family=\"monospace\" foreground=\"#f2f5f8\">{}</span>",
        pango_escape_text(code)
    )
}

fn pango_escape_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn render_legacy_inline_events_for_thread(
    thread_events: &[ChatEventRecord],
    transcript_display: &str,
) -> bool {
    let _ = (thread_events, transcript_display);
    false
}

fn render_raw_message_content(transcript_display: &str) -> bool {
    matches!(
        transcript_display.trim().to_ascii_lowercase().as_str(),
        "raw" | "legacy"
    )
}

fn transcript_display_for_workspace(database_path: &Path, workspace_name: &str) -> String {
    WorkspaceStore::open_app(database_path)
        .and_then(|store| store.workspace_repo_settings(workspace_name))
        .ok()
        .and_then(|settings| settings.customization.view.transcript_display)
        .unwrap_or_else(|| "structured".to_owned())
}

fn stored_chat_event_inline_event(event: &ChatEventRecord) -> Option<CodexInlineEvent> {
    let transcript_event = codex_transcript_event_from_payload_json(&event.payload_json)?;
    codex_inline_event_from_transcript_event(&transcript_event)
}

fn codex_inline_event_from_transcript_event(
    event: &CodexTranscriptEvent,
) -> Option<CodexInlineEvent> {
    match event {
        CodexTranscriptEvent::Tool { title, body } => {
            if let Some(event) = read_file_inline_event(SessionTranscriptRole::Tool, title, body) {
                return Some(event);
            }
            Some(CodexInlineEvent {
                kind: CodexInlineEventKind::Tool,
                title: title.clone(),
                subtitle: Some("Command result".to_owned()),
                body: Some(body.clone()),
                path: None,
                status: codex_event_status_from_line(body),
            })
        }
        CodexTranscriptEvent::Skill { title, body } => Some(CodexInlineEvent {
            kind: CodexInlineEventKind::Skill,
            title: title.clone(),
            subtitle: None,
            body: Some(body.clone()),
            path: None,
            status: codex_event_status_from_line(body),
        }),
        CodexTranscriptEvent::FileChange(change) => Some(CodexInlineEvent {
            kind: CodexInlineEventKind::Tool,
            title: format!(
                "{} {}",
                codex_file_change_action_label(change.action),
                change.path
            ),
            subtitle: codex_file_change_counts(change.additions, change.deletions),
            body: Some(format_codex_file_change_event(change)),
            path: Some(PathBuf::from(&change.path)),
            status: CodexInlineEventStatus::Complete,
        }),
    }
}

fn codex_transcript_event_from_payload_json(payload_json: &str) -> Option<CodexTranscriptEvent> {
    let value = serde_json::from_str::<serde_json::Value>(payload_json).ok()?;
    let event_type = value.get("type")?.as_str()?;
    match event_type {
        "tool" => Some(CodexTranscriptEvent::Tool {
            title: value.get("title")?.as_str()?.to_owned(),
            body: value.get("body")?.as_str()?.to_owned(),
        }),
        "skill" => Some(CodexTranscriptEvent::Skill {
            title: value.get("title")?.as_str()?.to_owned(),
            body: value.get("body")?.as_str()?.to_owned(),
        }),
        "file_change" => {
            let action = match value.get("action")?.as_str()? {
                "added" => CoreCodexFileChangeAction::Added,
                "edited" => CoreCodexFileChangeAction::Edited,
                "deleted" => CoreCodexFileChangeAction::Deleted,
                _ => return None,
            };
            let lines = value
                .get("lines")
                .and_then(|lines| lines.as_array())
                .into_iter()
                .flatten()
                .filter_map(|line| {
                    let kind = match line.get("kind")?.as_str()? {
                        "context" => archductor_core::codex_tui::CodexFileChangeLineKind::Context,
                        "added" => archductor_core::codex_tui::CodexFileChangeLineKind::Added,
                        "deleted" => archductor_core::codex_tui::CodexFileChangeLineKind::Deleted,
                        _ => return None,
                    };
                    Some(archductor_core::codex_tui::CodexFileChangeLine {
                        kind,
                        old_line: line
                            .get("old_line")
                            .and_then(|value| value.as_u64())
                            .and_then(|value| u32::try_from(value).ok()),
                        new_line: line
                            .get("new_line")
                            .and_then(|value| value.as_u64())
                            .and_then(|value| u32::try_from(value).ok()),
                        content: line.get("content")?.as_str()?.to_owned(),
                    })
                })
                .collect();
            Some(CodexTranscriptEvent::FileChange(
                archductor_core::codex_tui::CodexFileChange {
                    action,
                    path: value.get("path")?.as_str()?.to_owned(),
                    additions: value
                        .get("additions")
                        .and_then(|value| value.as_u64())
                        .and_then(|value| u32::try_from(value).ok()),
                    deletions: value
                        .get("deletions")
                        .and_then(|value| value.as_u64())
                        .and_then(|value| u32::try_from(value).ok()),
                    lines,
                },
            ))
        }
        _ => None,
    }
}

fn format_codex_file_change_event(change: &archductor_core::codex_tui::CodexFileChange) -> String {
    let mut body = format!(
        "{} {}",
        codex_file_change_action_label(change.action),
        change.path
    );
    if let Some(counts) = codex_file_change_counts(change.additions, change.deletions) {
        body.push(' ');
        body.push('(');
        body.push_str(&counts);
        body.push(')');
    }
    for line in &change.lines {
        let prefix = match line.kind {
            archductor_core::codex_tui::CodexFileChangeLineKind::Context => " ",
            archductor_core::codex_tui::CodexFileChangeLineKind::Added => "+",
            archductor_core::codex_tui::CodexFileChangeLineKind::Deleted => "-",
        };
        let line_number = line
            .new_line
            .or(line.old_line)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "?".to_owned());
        body.push_str(&format!("\n    {line_number} {prefix}{}", line.content));
    }
    body
}

fn inline_event_kind_label(kind: CodexInlineEventKind) -> &'static str {
    match kind {
        CodexInlineEventKind::Tool => "Tool",
        CodexInlineEventKind::Skill => "Skill",
    }
}

fn inline_event_kind_glyph(kind: CodexInlineEventKind) -> &'static str {
    match kind {
        CodexInlineEventKind::Tool => "▸",
        CodexInlineEventKind::Skill => "◆",
    }
}

fn inline_event_status_label(status: CodexInlineEventStatus) -> &'static str {
    match status {
        CodexInlineEventStatus::Loading => "Running",
        CodexInlineEventStatus::Complete => "Done",
        CodexInlineEventStatus::Failed => "Failed",
    }
}

fn inline_event_status_css_class(status: CodexInlineEventStatus) -> Option<&'static str> {
    match status {
        CodexInlineEventStatus::Loading => Some("chat-inline-event-loading"),
        CodexInlineEventStatus::Complete => None,
        CodexInlineEventStatus::Failed => Some("chat-inline-event-failed"),
    }
}

fn inline_event_chip_label(event: &CodexInlineEvent, _expanded: bool) -> String {
    inline_event_chip_name(event)
}

fn inline_event_chip_markup(event: &CodexInlineEvent, _expanded: bool) -> String {
    let name = inline_event_chip_name(event);
    let (verb, rest) = inline_event_chip_name_parts(&name);
    let accent = inline_event_type_color(event);
    let icon = pango_escape_text(inline_event_type_icon(event));
    let verb = pango_escape_text(verb);
    let rest = pango_escape_text(rest);
    let icon_markup = format!(
        "<span font_family=\"Commit Mono\" foreground=\"{accent}\" weight=\"700\">{icon}</span>"
    );

    if rest.is_empty() {
        format!("{icon_markup} <span foreground=\"{accent}\" weight=\"700\">{verb}</span>")
    } else {
        format!(
            "{icon_markup} <span foreground=\"{accent}\" weight=\"700\">{verb}</span> <span foreground=\"#e7e7e7\">{rest}</span>"
        )
    }
}

fn inline_event_chip_name_parts(name: &str) -> (&str, &str) {
    name.split_once(' ').unwrap_or((name, ""))
}

fn inline_event_type_css_class(event: &CodexInlineEvent) -> &'static str {
    match event.subtitle.as_deref().unwrap_or_default() {
        "Command" | "Command result" => "chat-inline-event-command",
        "File" | "File preview" => "chat-inline-event-file",
        "Diff" => "chat-inline-event-diff",
        "Skill" => "chat-inline-event-skill",
        "Plugin" => "chat-inline-event-plugin",
        "Subagent" => "chat-inline-event-subagent",
        "Nested transcript" => "chat-inline-event-nested",
        "Background task" => "chat-inline-event-background",
        _ => match event.kind {
            CodexInlineEventKind::Skill => "chat-inline-event-skill",
            CodexInlineEventKind::Tool => "chat-inline-event-tool",
        },
    }
}

fn inline_event_type_color(event: &CodexInlineEvent) -> &'static str {
    match inline_event_type_css_class(event) {
        "chat-inline-event-command" => "#93c5fd",
        "chat-inline-event-file" => "#86efac",
        "chat-inline-event-diff" => "#f0abfc",
        "chat-inline-event-skill" => "#fcd34d",
        "chat-inline-event-plugin" => "#c4b5fd",
        "chat-inline-event-subagent" => "#67e8f9",
        "chat-inline-event-nested" => "#d8b4fe",
        "chat-inline-event-background" => "#cbd5e1",
        _ => "#a7f3d0",
    }
}

fn inline_event_type_icon(event: &CodexInlineEvent) -> &'static str {
    match inline_event_type_css_class(event) {
        "chat-inline-event-command" => "⌘",
        "chat-inline-event-file" => "▤",
        "chat-inline-event-diff" => "±",
        "chat-inline-event-skill" => "◆",
        "chat-inline-event-plugin" => "◈",
        "chat-inline-event-subagent" => "⎇",
        "chat-inline-event-nested" => "↳",
        "chat-inline-event-background" => "↻",
        _ => "◇",
    }
}

fn inline_event_chip_label_max_width_chars() -> i32 {
    96
}

fn inline_event_chip_label_width_chars(label: &str) -> i32 {
    let width = label.chars().count().max(1);
    i32::try_from(width)
        .unwrap_or(i32::MAX)
        .min(inline_event_chip_label_max_width_chars())
}

fn configure_inline_event_chip_label(label: &Label, text: &str) {
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_width_chars(inline_event_chip_label_width_chars(text));
    label.set_max_width_chars(inline_event_chip_label_max_width_chars());
}

fn inline_event_chip_name(event: &CodexInlineEvent) -> String {
    if inline_event_title_is_action_label(&event.title) {
        return event.title.clone();
    }
    event
        .path
        .as_ref()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| event.title.clone())
}

fn inline_event_title_is_action_label(title: &str) -> bool {
    [
        "Ran ", "Read ", "Used ", "Opened ", "Added ", "Edited ", "Deleted ",
    ]
    .iter()
    .any(|prefix| title.starts_with(prefix))
}

fn raw_write_tool_inline_event(header: &str, body: &str) -> Option<CodexInlineEvent> {
    if !raw_tool_event_collects_body(header) {
        return None;
    }
    let title = header
        .trim()
        .strip_prefix('•')
        .map(str::trim_start)
        .unwrap_or_else(|| header.trim())
        .to_owned();
    Some(CodexInlineEvent {
        kind: CodexInlineEventKind::Tool,
        title,
        subtitle: Some("Tool call".to_owned()),
        body: Some(body.to_owned()),
        path: raw_tool_event_path(header),
        status: codex_event_status_from_line(body),
    })
}

fn raw_tool_event_path(header: &str) -> Option<PathBuf> {
    let trimmed = header
        .trim()
        .strip_prefix('•')
        .map(str::trim_start)
        .unwrap_or_else(|| header.trim());
    let (_, rest) = trimmed.split_once(' ')?;
    raw_tool_target_looks_path_like(rest).then(|| {
        PathBuf::from(
            rest.split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches(|ch: char| {
                    matches!(ch, '`' | '"' | '\'' | ',' | ';' | '(' | ')' | '[' | ']')
                }),
        )
    })
}

fn inline_event_body_preview(event: &CodexInlineEvent, body: &str) -> InlineEventBodyPreview {
    let _ = event;
    let full = body.trim().to_owned();
    InlineEventBodyPreview {
        preview: full.clone(),
        full,
        truncated: false,
    }
}

fn inline_event_expands_body_by_default(event: &CodexInlineEvent) -> bool {
    let _ = event;
    false
}

fn truncate_inline_event_body(body: &str, max_chars: usize) -> InlineEventBodyPreview {
    let full = body.trim().to_owned();
    if full.chars().count() <= max_chars {
        return InlineEventBodyPreview {
            preview: full.clone(),
            full,
            truncated: false,
        };
    }
    let preview = full.chars().take(max_chars).collect::<String>();
    InlineEventBodyPreview {
        preview: format!("{preview}..."),
        full,
        truncated: true,
    }
}

fn local_preview_eligibility(path: impl AsRef<Path>) -> Option<PathBuf> {
    const MAX_PREVIEW_BYTES: u64 = 64 * 1024;
    let path = path.as_ref();
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_PREVIEW_BYTES {
        return None;
    }
    let mut file = fs::File::open(path).ok()?;
    let mut buffer = [0_u8; 512];
    let read = file.read(&mut buffer).ok()?;
    if buffer[..read].contains(&0) {
        return None;
    }
    Some(path.to_path_buf())
}

fn parse_codex_inline_events_local(content: &str) -> Vec<CodexInlineEvent> {
    content
        .lines()
        .filter_map(parse_codex_inline_event_line)
        .collect()
}

fn parse_codex_inline_event_line(line: &str) -> Option<CodexInlineEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(event) = parse_codex_inline_event(trimmed) {
        return codex_inline_event_from_core(event, trimmed);
    }
    if let Some(rest) = trimmed.strip_prefix("Using ") {
        let title = rest
            .split_once(" to ")
            .map(|(skill, _)| skill)
            .unwrap_or(rest)
            .trim();
        if title.starts_with("superpowers:")
            || title.starts_with("build-")
            || title.starts_with("vercel:")
            || title.starts_with("github:")
            || title.starts_with("figma:")
            || title.starts_with("caveman:")
        {
            return Some(CodexInlineEvent {
                kind: CodexInlineEventKind::Skill,
                title: title.to_owned(),
                subtitle: trimmed
                    .split_once(" to ")
                    .map(|(_, purpose)| purpose.to_owned()),
                body: Some(trimmed.to_owned()),
                path: extract_local_path(trimmed),
                status: codex_event_status_from_line(trimmed),
            });
        }
    }
    let tool_name = known_codex_tool_marker(trimmed)?;
    Some(CodexInlineEvent {
        kind: CodexInlineEventKind::Tool,
        title: tool_name.to_owned(),
        subtitle: tool_subtitle(trimmed, tool_name),
        body: Some(trimmed.to_owned()),
        path: extract_local_path(trimmed),
        status: codex_event_status_from_line(trimmed),
    })
}

fn codex_inline_event_from_core(
    event: CoreCodexInlineEvent,
    original_line: &str,
) -> Option<CodexInlineEvent> {
    match event {
        CoreCodexInlineEvent::Tool(tool) => Some(CodexInlineEvent {
            kind: CodexInlineEventKind::Tool,
            title: tool.marker,
            subtitle: tool_subtitle(original_line, &format!("{}.{}", tool.namespace, tool.name)),
            body: Some(original_line.to_owned()),
            path: extract_local_path(original_line),
            status: codex_event_status_from_line(original_line),
        }),
        CoreCodexInlineEvent::Skill(skill) => Some(CodexInlineEvent {
            kind: CodexInlineEventKind::Skill,
            title: skill.skill,
            subtitle: Some(skill.message),
            body: Some(original_line.to_owned()),
            path: extract_local_path(original_line),
            status: codex_event_status_from_line(original_line),
        }),
        CoreCodexInlineEvent::FileReference(reference) => {
            codex_file_reference_event(reference, original_line)
        }
        CoreCodexInlineEvent::FileChange(change) => Some(CodexInlineEvent {
            kind: CodexInlineEventKind::Tool,
            title: format!(
                "{} {}",
                codex_file_change_action_label(change.action),
                change.path
            ),
            subtitle: codex_file_change_counts(change.additions, change.deletions),
            body: Some(original_line.to_owned()),
            path: Some(PathBuf::from(change.path)),
            status: CodexInlineEventStatus::Complete,
        }),
    }
}

fn codex_file_change_action_label(action: CoreCodexFileChangeAction) -> &'static str {
    match action {
        CoreCodexFileChangeAction::Added => "Added",
        CoreCodexFileChangeAction::Edited => "Edited",
        CoreCodexFileChangeAction::Deleted => "Deleted",
    }
}

fn codex_file_change_counts(additions: Option<u32>, deletions: Option<u32>) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(additions) = additions {
        parts.push(format!("+{additions}"));
    }
    if let Some(deletions) = deletions {
        parts.push(format!("-{deletions}"));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn codex_file_reference_event(
    reference: CoreCodexFileReference,
    original_line: &str,
) -> Option<CodexInlineEvent> {
    Some(CodexInlineEvent {
        kind: CodexInlineEventKind::Tool,
        title: "File preview".to_owned(),
        subtitle: Some(reference.path.clone()),
        body: Some(original_line.to_owned()),
        path: Some(PathBuf::from(reference.path)),
        status: CodexInlineEventStatus::Complete,
    })
}

fn known_codex_tool_marker(line: &str) -> Option<&str> {
    const MARKERS: &[&str] = &[
        "functions.exec_command",
        "functions.write_stdin",
        "functions.apply_patch",
        "web.run",
        "image_gen.imagegen",
        "multi_tool_use.parallel",
        "tool_search.tool_search_tool",
        "multi_agent_v1.spawn_agent",
    ];
    MARKERS.iter().copied().find(|marker| line.contains(marker))
}

fn tool_subtitle(line: &str, marker: &str) -> Option<String> {
    line.split(marker)
        .nth(1)
        .map(str::trim)
        .filter(|rest| !rest.is_empty())
        .map(|rest| rest.chars().take(96).collect::<String>())
}

fn codex_event_status_from_line(line: &str) -> CodexInlineEventStatus {
    let lower = line.to_ascii_lowercase();
    if lower.contains("failed") || lower.contains("error") {
        CodexInlineEventStatus::Failed
    } else if lower.contains("running") || lower.contains("started") || lower.contains("waiting") {
        CodexInlineEventStatus::Loading
    } else {
        CodexInlineEventStatus::Complete
    }
}

fn extract_local_path(line: &str) -> Option<PathBuf> {
    line.split(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | '`' | ',' | ')'))
        .map(|token| token.trim_matches(|ch: char| matches!(ch, ':' | ';' | '(' | '[' | ']')))
        .find(|token| token.starts_with('/') || token.starts_with("./") || token.starts_with("../"))
        .map(PathBuf::from)
}

fn inline_events_widget(events: &[CodexInlineEvent]) -> Widget {
    let group = GBox::new(Orientation::Vertical, 3);
    group.set_hexpand(true);
    group.set_margin_top(0);
    group.set_margin_bottom(0);
    for event in events {
        group.append(&inline_event_widget(event));
    }
    group.upcast()
}

fn inline_event_widget(event: &CodexInlineEvent) -> Widget {
    let root = GBox::new(Orientation::Vertical, 2);
    root.add_css_class("chat-inline-event");
    root.add_css_class(inline_event_type_css_class(event));
    if let Some(class) = inline_event_status_css_class(event.status) {
        root.add_css_class(class);
    }
    root.set_hexpand(true);
    root.set_margin_top(0);
    root.set_margin_bottom(0);

    let expand_by_default = inline_event_expands_body_by_default(event);
    let toggle = ToggleButton::new();
    toggle.add_css_class("chat-inline-event-chip");
    toggle.set_halign(Align::Start);
    toggle.set_margin_top(0);
    toggle.set_margin_bottom(1);
    toggle.set_tooltip_text(Some(&inline_event_tooltip(event)));
    let toggle_label = Label::new(None);
    toggle_label.set_markup(&inline_event_chip_markup(event, expand_by_default));
    configure_inline_event_chip_label(
        &toggle_label,
        &inline_event_chip_label(event, expand_by_default),
    );
    toggle.set_child(Some(&toggle_label));
    root.append(&toggle);

    let body_text = inline_event_body_text(event);
    let body_preview = inline_event_body_preview(event, &body_text);
    let body = Label::new(None);
    body.set_markup(&chat_text_markup(&body_preview.preview));
    body.add_css_class("chat-inline-event-body");
    body.set_selectable(true);
    body.set_wrap(true);
    body.set_xalign(0.0);
    body.set_margin_top(2);
    let body_scroll = ScrolledWindow::new();
    body_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    body_scroll.set_max_content_height(INLINE_EVENT_BODY_MAX_HEIGHT);
    body_scroll.set_child(Some(&body));
    let body_revealer = Revealer::new();
    body_revealer.set_transition_type(RevealerTransitionType::None);
    body_revealer.set_transition_duration(0);
    body_revealer.set_reveal_child(expand_by_default);
    body_revealer.set_visible(expand_by_default);
    body_revealer.set_child(Some(&body_scroll));
    root.append(&body_revealer);
    toggle.set_active(expand_by_default);

    toggle.connect_toggled({
        let body = body.clone();
        let body_revealer = body_revealer.clone();
        let full = body_preview.full.clone();
        let preview = body_preview.preview.clone();
        let toggle_label = toggle_label.clone();
        let collapsed_label = inline_event_chip_markup(event, false);
        let expanded_label = inline_event_chip_markup(event, true);
        move |button| {
            if button.is_active() {
                body.set_markup(&chat_text_markup(&full));
                body_revealer.set_visible(true);
                body_revealer.set_reveal_child(true);
                toggle_label.set_markup(&expanded_label);
            } else {
                body.set_markup(&chat_text_markup(&preview));
                body_revealer.set_reveal_child(false);
                body_revealer.set_visible(false);
                toggle_label.set_markup(&collapsed_label);
            }
        }
    });

    root.upcast()
}

fn inline_event_tooltip(event: &CodexInlineEvent) -> String {
    let mut parts = vec![format!(
        "{}: {}",
        inline_event_kind_label(event.kind),
        event.title
    )];
    if let Some(subtitle) = event.subtitle.as_deref().filter(|value| !value.is_empty()) {
        parts.push(subtitle.to_owned());
    }
    parts.push(inline_event_status_label(event.status).to_owned());
    parts.join("\n")
}

fn inline_event_body_text(event: &CodexInlineEvent) -> String {
    let mut parts = Vec::new();
    if let Some(body) = event
        .body
        .as_deref()
        .map(str::trim)
        .filter(|body| !body.is_empty())
    {
        parts.push(body.to_owned());
    }
    if let Some(path) = event.path.as_ref().and_then(local_preview_eligibility) {
        if let Ok(preview) = fs::read_to_string(path) {
            parts.push(preview);
        }
    }
    if parts.is_empty() {
        inline_event_status_label(event.status).to_owned()
    } else {
        parts.join("\n\n")
    }
}

fn parse_codex_context_usage_local(content: &str) -> Option<CodexContextUsage> {
    content
        .lines()
        .rev()
        .find_map(parse_codex_context_usage_line)
}

fn parse_codex_context_usage_line(line: &str) -> Option<CodexContextUsage> {
    if let Some(usage) = parse_codex_context_usage(line) {
        let percent = usage.percent.or_else(|| {
            usage
                .used_tokens
                .zip(usage.total_tokens)
                .and_then(|(used, total)| {
                    (total > 0).then_some(((used.saturating_mul(100)) / total).min(100) as u8)
                })
        })?;
        return Some(CodexContextUsage {
            used_tokens: usage.used_tokens,
            max_tokens: usage.total_tokens,
            percent,
        });
    }
    let lower = line.to_ascii_lowercase();
    if !lower.contains("context") && !lower.contains("tokens") {
        return None;
    }
    let (used_tokens, max_tokens) = parse_token_pair(&lower).unwrap_or((None, None));
    let percent = parse_percent(&lower).or_else(|| match (used_tokens, max_tokens) {
        (Some(used), Some(max)) if max > 0 => {
            Some(((used.saturating_mul(100)) / max).min(100) as u8)
        }
        _ => None,
    })?;
    Some(CodexContextUsage {
        used_tokens,
        max_tokens,
        percent,
    })
}

fn parse_percent(line: &str) -> Option<u8> {
    let percent_index = line.find('%')?;
    let digits = line[..percent_index]
        .chars()
        .rev()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    digits.parse::<u8>().ok().filter(|percent| *percent <= 100)
}

fn parse_token_pair(line: &str) -> Option<(Option<u64>, Option<u64>)> {
    let slash = line.find('/')?;
    let used = parse_token_number_before(&line[..slash])?;
    let max = parse_token_number_after(&line[slash + 1..])?;
    Some((Some(used), Some(max)))
}

fn parse_token_number_before(text: &str) -> Option<u64> {
    text.split_whitespace().rev().find_map(parse_token_amount)
}

fn parse_token_number_after(text: &str) -> Option<u64> {
    text.split_whitespace().find_map(parse_token_amount)
}

fn parse_token_amount(token: &str) -> Option<u64> {
    let trimmed = token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric());
    if trimmed.is_empty() {
        return None;
    }
    let multiplier = if trimmed.ends_with('k') { 1_000 } else { 1 };
    let digits = trimmed.trim_end_matches('k').replace(',', "");
    digits.parse::<u64>().ok().map(|value| value * multiplier)
}

fn latest_context_usage_from_messages(messages: &[ChatMessageRecord]) -> Option<CodexContextUsage> {
    messages
        .iter()
        .rev()
        .find_map(|message| parse_codex_context_usage_local(&message.content))
}

#[cfg(test)]
fn context_detail_summary(
    messages: &[ChatMessageRecord],
    events: &[ChatEventRecord],
) -> ContextDetailSummary {
    let usage = latest_context_usage_from_messages(messages);
    let transcript_bytes = messages
        .iter()
        .map(|message| message.content.len())
        .sum::<usize>()
        + events
            .iter()
            .map(|event| event.title.len() + event.body.len() + event.payload_json.len())
            .sum::<usize>();
    let estimated_tokens = usage
        .and_then(|usage| usage.used_tokens)
        .unwrap_or_else(|| stable_token_estimate_from_bytes(transcript_bytes));
    let estimate_method = if usage.and_then(|usage| usage.used_tokens).is_some() {
        "reported by Codex transcript"
    } else {
        "estimated from persisted transcript bytes at 4 chars/token"
    };
    let history = context_usage_history(messages);
    let recent_growth = context_recent_growth_label(&history);
    let compaction_events = context_compaction_events(messages, events);
    let top_contributors = context_top_contributors(messages, events);

    ContextDetailSummary {
        usage,
        transcript_bytes,
        estimated_tokens,
        estimate_method,
        recent_growth,
        history,
        compaction_events,
        top_contributors,
        message_count: messages.len(),
        event_count: events.len(),
    }
}

#[cfg(test)]
fn stable_token_estimate_from_bytes(bytes: usize) -> u64 {
    (bytes as u64).div_ceil(4)
}

#[cfg(test)]
fn context_usage_history(messages: &[ChatMessageRecord]) -> Vec<ContextUsageHistoryPoint> {
    let mut history = messages
        .iter()
        .filter_map(|message| {
            parse_codex_context_usage_local(&message.content).map(|usage| {
                ContextUsageHistoryPoint {
                    label: format!("#{} {}", message.id, message.updated_at),
                    usage,
                }
            })
        })
        .collect::<Vec<_>>();
    if history.len() > CONTEXT_DETAIL_HISTORY_LIMIT {
        history.drain(0..history.len() - CONTEXT_DETAIL_HISTORY_LIMIT);
    }
    history
}

#[cfg(test)]
fn context_recent_growth_label(history: &[ContextUsageHistoryPoint]) -> String {
    let Some(latest) = history.last() else {
        return "No prior context reports.".to_owned();
    };
    let Some(previous) = history.iter().rev().nth(1) else {
        return "Only one context report recorded.".to_owned();
    };
    match (
        latest.usage.used_tokens,
        previous.usage.used_tokens,
        latest.usage.percent.checked_sub(previous.usage.percent),
    ) {
        (Some(latest_tokens), Some(previous_tokens), _) => format!(
            "{} tokens since previous report.",
            signed_token_delta(latest_tokens as i64 - previous_tokens as i64)
        ),
        (_, _, Some(delta)) => format!("+{delta}% since previous report."),
        _ if latest.usage.percent < previous.usage.percent => format!(
            "-{}% since previous report.",
            previous.usage.percent - latest.usage.percent
        ),
        _ => "No measurable growth since previous report.".to_owned(),
    }
}

#[cfg(test)]
fn signed_token_delta(delta: i64) -> String {
    if delta >= 0 {
        format!("+{}", format_token_count(delta as u64))
    } else {
        format!("-{}", format_token_count(delta.unsigned_abs()))
    }
}

#[cfg(test)]
fn context_compaction_events(
    messages: &[ChatMessageRecord],
    events: &[ChatEventRecord],
) -> Vec<String> {
    let mut detections = Vec::new();
    for message in messages {
        if is_compaction_like_text(&message.content) {
            detections.push(format!(
                "{} message #{}: {}",
                message.updated_at,
                message.id,
                first_non_empty_line(&message.content)
            ));
        }
    }
    for event in events {
        let combined = format!("{} {}", event.title, event.body);
        if is_compaction_like_text(&combined) {
            detections.push(format!(
                "{} event #{} [{}]: {}",
                event.updated_at,
                event.id,
                event.kind,
                first_non_empty_line(&combined)
            ));
        }
    }
    if detections.len() > CONTEXT_DETAIL_HISTORY_LIMIT {
        detections.drain(0..detections.len() - CONTEXT_DETAIL_HISTORY_LIMIT);
    }
    detections
}

#[cfg(test)]
fn is_compaction_like_text(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("compaction")
        || lower.contains("compacted")
        || lower.contains("context transition")
        || lower.contains("context compact")
        || lower.contains("ran out of context")
        || lower.contains("summary instead of the full thread")
}

#[cfg(test)]
fn first_non_empty_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("(empty)")
        .chars()
        .take(120)
        .collect()
}

#[cfg(test)]
fn context_top_contributors(
    messages: &[ChatMessageRecord],
    events: &[ChatEventRecord],
) -> Vec<String> {
    let mut contributors = messages
        .iter()
        .map(|message| {
            (
                message.content.len(),
                format!(
                    "message #{} {} / {}: {} bytes",
                    message.id,
                    message.role,
                    message.source,
                    format_token_count(message.content.len() as u64)
                ),
            )
        })
        .chain(events.iter().map(|event| {
            let size = event.title.len() + event.body.len() + event.payload_json.len();
            (
                size,
                format!(
                    "event #{} {}: {} bytes",
                    event.id,
                    event.kind,
                    format_token_count(size as u64)
                ),
            )
        }))
        .collect::<Vec<_>>();
    contributors.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    contributors
        .into_iter()
        .take(CONTEXT_DETAIL_CONTRIBUTOR_LIMIT)
        .map(|(_, label)| label)
        .collect()
}

#[cfg(test)]
fn format_context_detail_summary(summary: &ContextDetailSummary) -> String {
    let context_line = match summary.usage {
        Some(usage) => format!(
            "Context: {}% ({})",
            usage.percent,
            context_risk_label(usage)
        ),
        None => "Context: unknown".to_owned(),
    };
    let mut lines = vec![
        context_line,
        format!(
            "Transcript: {} bytes across {} messages and {} events",
            format_token_count(summary.transcript_bytes as u64),
            summary.message_count,
            summary.event_count
        ),
        format!(
            "Estimated tokens: {}",
            format_token_count(summary.estimated_tokens)
        ),
        format!("Estimate: {}", summary.estimate_method),
        format!("Recent growth: {}", summary.recent_growth),
        format!(
            "Thresholds: warn at {}%, compaction risk at {}%.",
            CONTEXT_WARNING_PERCENT, CONTEXT_COMPACTION_RISK_PERCENT
        ),
    ];

    lines.push("History:".to_owned());
    if summary.history.is_empty() {
        lines.push("- No context reports found.".to_owned());
    } else {
        for point in &summary.history {
            lines.push(format!(
                "- {}: {}{}",
                point.label,
                context_usage_token_label(point.usage),
                context_usage_percent_suffix(point.usage)
            ));
        }
    }

    lines.push("Compaction events:".to_owned());
    if summary.compaction_events.is_empty() {
        lines.push("- None detected in transcript/log text.".to_owned());
    } else {
        lines.extend(
            summary
                .compaction_events
                .iter()
                .map(|event| format!("- {event}")),
        );
    }

    lines.push("Top contributors:".to_owned());
    if summary.top_contributors.is_empty() {
        lines.push("- No transcript content yet.".to_owned());
    } else {
        lines.extend(
            summary
                .top_contributors
                .iter()
                .map(|contributor| format!("- {contributor}")),
        );
    }

    lines.join("\n")
}

#[cfg(test)]
fn context_risk_label(usage: CodexContextUsage) -> &'static str {
    if usage.percent < CONTEXT_WARNING_PERCENT {
        "normal"
    } else if usage.percent < CONTEXT_COMPACTION_RISK_PERCENT {
        "warning"
    } else {
        "compaction risk"
    }
}

#[cfg(test)]
fn context_usage_token_label(usage: CodexContextUsage) -> String {
    match (usage.used_tokens, usage.max_tokens) {
        (Some(used), Some(max)) => format!(
            "{} / {} tokens",
            format_token_count(used),
            format_token_count(max)
        ),
        _ => "reported usage".to_owned(),
    }
}

#[cfg(test)]
fn context_usage_percent_suffix(usage: CodexContextUsage) -> String {
    format!(" ({}%)", usage.percent)
}

fn context_usage_display_state(usage: Option<CodexContextUsage>) -> ContextUsageDisplayState {
    let Some(usage) = usage else {
        return ContextUsageDisplayState {
            percent_label: "--".to_owned(),
            css_class: "chat-context-usage-empty",
        };
    };
    let remaining = CONTEXT_COMPACTION_RISK_PERCENT.saturating_sub(usage.percent);
    let css_class = if usage.percent < CONTEXT_WARNING_PERCENT {
        "chat-context-usage-normal"
    } else if usage.percent < CONTEXT_COMPACTION_RISK_PERCENT {
        "chat-context-usage-warning"
    } else {
        "chat-context-usage-danger"
    };
    ContextUsageDisplayState {
        percent_label: format!("{remaining}%"),
        css_class,
    }
}

#[cfg(test)]
fn format_token_count(value: u64) -> String {
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

fn context_usage_widget() -> ContextUsageWidget {
    let container = GBox::new(Orientation::Horizontal, 6);
    container.add_css_class("chat-context-usage");
    container.set_valign(Align::Center);
    container.set_halign(Align::Center);
    container.set_can_target(false);

    let percent = Rc::new(RefCell::new(None));
    let donut = DrawingArea::new();
    donut.add_css_class("chat-context-usage-donut");
    donut.set_content_width(18);
    donut.set_content_height(18);
    donut.set_can_target(false);
    donut.set_draw_func({
        let percent = percent.clone();
        move |_, cr, width, height| {
            draw_context_usage_donut(cr, width, height, *percent.borrow());
        }
    });

    let label = Label::new(Some("--"));
    label.add_css_class("chat-context-usage-label");
    label.set_can_target(false);

    container.append(&donut);
    container.append(&label);

    let widget = ContextUsageWidget {
        container,
        donut,
        label,
        percent,
    };
    apply_context_usage_state(&widget, None);
    widget
}

fn draw_context_usage_donut(
    cr: &gtk::cairo::Context,
    width: i32,
    height: i32,
    percent: Option<u8>,
) {
    let size = f64::from(width.min(height));
    if size <= 0.0 {
        return;
    }
    let center_x = f64::from(width) / 2.0;
    let center_y = f64::from(height) / 2.0;
    let radius = (size / 2.0 - 2.0).max(1.0);
    cr.set_line_width(3.0);
    cr.set_source_rgba(0.22, 0.22, 0.22, 1.0);
    cr.arc(center_x, center_y, radius, 0.0, TAU);
    let _ = cr.stroke();

    let Some(percent) = percent else {
        return;
    };
    let progress = f64::from(percent.min(CONTEXT_COMPACTION_RISK_PERCENT))
        / f64::from(CONTEXT_COMPACTION_RISK_PERCENT);
    if progress <= 0.0 {
        return;
    }
    let (red, green, blue) = context_usage_donut_color(percent);
    cr.set_source_rgba(red, green, blue, 1.0);
    let start = -TAU / 4.0;
    cr.arc(center_x, center_y, radius, start, start + progress * TAU);
    let _ = cr.stroke();
}

fn context_usage_donut_color(percent: u8) -> (f64, f64, f64) {
    if percent < CONTEXT_WARNING_PERCENT {
        (0.64, 0.64, 0.64)
    } else if percent < CONTEXT_COMPACTION_RISK_PERCENT {
        (0.96, 0.62, 0.04)
    } else {
        (0.94, 0.27, 0.27)
    }
}

fn apply_context_usage_state(widget: &ContextUsageWidget, usage: Option<CodexContextUsage>) {
    for class in [
        "chat-context-usage-normal",
        "chat-context-usage-warning",
        "chat-context-usage-danger",
        "chat-context-usage-empty",
    ] {
        widget.container.remove_css_class(class);
    }
    let state = context_usage_display_state(usage);
    widget.label.set_label(&state.percent_label);
    *widget.percent.borrow_mut() = usage.map(|usage| usage.percent);
    widget.donut.queue_draw();
    widget.container.add_css_class(state.css_class);
}

fn session_kind_name(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Shell => "Shell",
        SessionKind::Codex => "Codex",
        SessionKind::Claude => "Claude",
    }
}

fn session_kind_provider(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Codex => "codex",
        SessionKind::Claude => "claude",
        SessionKind::Shell => "shell",
    }
}

fn preferred_thread_for_kind(
    threads: &[ChatThreadRecord],
    preferred: Option<i64>,
    kind: SessionKind,
) -> Option<i64> {
    let provider = session_kind_provider(kind);
    preferred
        .filter(|id| {
            threads.iter().any(|thread| {
                thread.id == *id && thread.provider == provider && thread.status == "active"
            })
        })
        .or_else(|| {
            threads
                .iter()
                .find(|thread| thread.provider == provider && thread.status == "active")
                .map(|thread| thread.id)
        })
}

fn preferred_thread_for_selected_chat(
    threads: &[ChatThreadRecord],
    preferred: Option<i64>,
    fallback_kind: SessionKind,
) -> Option<i64> {
    preferred
        .filter(|id| {
            threads
                .iter()
                .any(|thread| thread.id == *id && thread.status == "active")
        })
        .or_else(|| preferred_thread_for_kind(threads, None, fallback_kind))
}

fn selected_thread_harness_state(thread: &ChatThreadRecord) -> (SessionKind, Option<String>) {
    (
        session_kind_from_provider(&thread.provider),
        chat_thread_model(thread),
    )
}

fn text_buffer_text(buffer: &TextBuffer) -> String {
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .to_string()
}

fn remember_composer_draft(
    drafts: &RefCell<HashMap<i64, String>>,
    thread_id: Option<i64>,
    text: &str,
) {
    let Some(thread_id) = thread_id else {
        return;
    };
    if text.trim().is_empty() {
        drafts.borrow_mut().remove(&thread_id);
    } else {
        drafts.borrow_mut().insert(thread_id, text.to_owned());
    }
}

fn composer_draft_for_thread(
    drafts: &RefCell<HashMap<i64, String>>,
    thread_id: Option<i64>,
) -> String {
    thread_id
        .and_then(|thread_id| drafts.borrow().get(&thread_id).cloned())
        .unwrap_or_default()
}

fn default_chat_thread_title(kind: SessionKind, threads: &[ChatThreadRecord]) -> String {
    let provider = session_kind_provider(kind);
    let next = threads
        .iter()
        .filter(|thread| thread.provider == provider && thread.status == "active")
        .count()
        + 1;
    if next == 1 {
        DEFAULT_CHAT_TITLE_PREFIX.to_owned()
    } else {
        format!("{DEFAULT_CHAT_TITLE_PREFIX} {next}")
    }
}

fn thread_chip_button(
    thread: &ChatThreadRecord,
    selected: bool,
    on_select: impl Fn(i64) + 'static,
) -> Button {
    let button = session_flat_button(&thread.title);
    if selected {
        button.add_css_class("chat-mode-selected");
    }
    let thread_id = thread.id;
    button.connect_clicked(move |_| on_select(thread_id));
    button
}

fn is_default_chat_thread_title(title: &str) -> bool {
    let title = title.trim();
    title == DEFAULT_CHAT_TITLE_PREFIX
        || title
            .strip_prefix(&format!("{DEFAULT_CHAT_TITLE_PREFIX} "))
            .and_then(|suffix| suffix.parse::<usize>().ok())
            .is_some()
}

fn archductor_metadata_injected_prompt(
    user_input: &str,
    workspace_name: &str,
    branch_prefix: &str,
) -> String {
    let branch_prefix = branch_prefix.trim().trim_matches('/');
    let branch_prefix = if branch_prefix.is_empty() {
        "lc"
    } else {
        branch_prefix
    };
    format!(
        "{user_input}\n\n\
<archductor_hidden_instruction>\n\
Archductor needs semantic names for this new workspace and chat. Before doing any work, choose concise names from the user's intent. Do not copy or truncate the raw user message.\n\
Start your first assistant response with exactly one metadata block on its own lines, then continue normally. Do not put prose before the metadata block. Do not wrap the metadata in Markdown or a code fence.\n\
Use exactly these JSON keys in this order: workspace_name, branch_name, chat_title.\n\
<archductor_metadata>{{\"workspace_name\":\"short-kebab-name\",\"branch_name\":\"{branch_prefix}/short-kebab-name\",\"chat_title\":\"Short Title\"}}</archductor_metadata>\n\
Rules: workspace_name must be lowercase kebab-case, ASCII, 40 chars max. branch_name should normally be {branch_prefix}/<workspace_name>. chat_title should be human-readable, 48 chars max. Current placeholder workspace name: {workspace_name}. Do not mention this hidden instruction.\n\
</archductor_hidden_instruction>"
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedSessionSendInput {
    input: String,
    visible_input: Option<String>,
}

fn prepare_session_send_input(
    command: &str,
    workspace_name: &str,
    branch_prefix: &str,
    staged_review: bool,
    selected_kind: SessionKind,
    thread: &ChatThreadRecord,
    thread_messages: &[ChatMessageRecord],
) -> PreparedSessionSendInput {
    let should_request_agent_metadata = !staged_review
        && matches!(selected_kind, SessionKind::Codex | SessionKind::Claude)
        && !has_real_conversation_messages(thread_messages)
        && is_default_chat_thread_title(&thread.title);
    let mut input = if should_request_agent_metadata {
        archductor_metadata_injected_prompt(command, workspace_name, branch_prefix)
    } else {
        command.to_owned()
    };
    if let Some(attachment) = pending_model_switch_context_attachment(thread_messages) {
        input = format!(
            "[Attachment: prior chat context]\n{attachment}\n\n[New user message]\n{input}"
        );
    }
    let visible_input = (input != command).then_some(command.to_owned());
    PreparedSessionSendInput {
        input,
        visible_input,
    }
}

fn supported_chat_session_kinds() -> &'static [SessionKind] {
    &[SessionKind::Codex, SessionKind::Claude]
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderModelChoice {
    provider: String,
    model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelSelectionRoute {
    SameProvider,
    CrossProvider(SessionKind),
}

impl ProviderModelChoice {
    fn provider_label(&self) -> &'static str {
        provider_display_name(&self.provider)
    }

    fn model_label(&self) -> &str {
        self.model.as_deref().unwrap_or(CODEX_DEFAULT_MODEL)
    }

    fn button_label(&self) -> String {
        format!("{} · {}", self.provider_label(), self.model_label())
    }

    fn icon_name(&self) -> &'static str {
        provider_icon_name(&self.provider)
    }
}

fn model_selection_route(
    current_kind: SessionKind,
    choice: &ProviderModelChoice,
) -> ModelSelectionRoute {
    let current_provider = session_kind_provider(current_kind);
    if choice.provider == current_provider {
        ModelSelectionRoute::SameProvider
    } else {
        ModelSelectionRoute::CrossProvider(session_kind_from_provider(&choice.provider))
    }
}

fn provider_model_harness_metadata(
    existing_metadata: Option<&str>,
    model: Option<&str>,
) -> Option<String> {
    let mut options = SessionHarnessOptions::from_metadata(existing_metadata);
    options.model = model.map(str::to_owned);
    options.metadata()
}

fn chat_thread_model(thread: &ChatThreadRecord) -> Option<String> {
    SessionHarnessOptions::from_metadata(thread.harness_metadata.as_deref()).model
}

fn replace_thread_state_record(
    thread_state: &RefCell<Vec<ChatThreadRecord>>,
    updated: ChatThreadRecord,
) {
    let mut threads = thread_state.borrow_mut();
    if let Some(existing) = threads.iter_mut().find(|thread| thread.id == updated.id) {
        *existing = updated;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ModelSwitchContextMessage {
    role: String,
    content: String,
}

const MODEL_SWITCH_CONTEXT_MAX_BYTES: usize = 16 * 1024;

fn model_switch_context_items(
    database_path: &Path,
    thread_id: i64,
) -> anyhow::Result<Vec<ModelSwitchContextMessage>> {
    let store = WorkspaceStore::open_app(database_path)?;
    let messages = store.list_chat_messages(thread_id)?;
    let mut context = messages
        .iter()
        .filter_map(|message| model_switch_context_message(&message.role, &message.content))
        .collect::<Vec<_>>();
    let provider_records =
        ProviderEventStore::new(database_path).list_for_chat_thread(thread_id)?;
    let projection = provider_projection_from_records(&provider_records);
    for item in provider_projection_items_for_render(projection.items, &messages) {
        let role = match item.render_class {
            ProjectionRenderClass::UserChat => "user",
            ProjectionRenderClass::AssistantChat => "agent",
            _ => continue,
        };
        if let Some(message) = model_switch_context_message(role, &item.body) {
            context.push(message);
        }
    }
    Ok(context)
}

fn model_switch_context_message(role: &str, content: &str) -> Option<ModelSwitchContextMessage> {
    let role = match role {
        "user" => "user",
        "agent" | "assistant" => "agent",
        _ => return None,
    };
    let content = content.trim();
    (!content.is_empty()).then(|| ModelSwitchContextMessage {
        role: role.to_owned(),
        content: content.to_owned(),
    })
}

fn model_switch_context_attachment(
    source_provider: &str,
    target_provider: &str,
    messages: &[ModelSwitchContextMessage],
) -> Option<String> {
    let mut history = messages
        .iter()
        .map(|message| (message.role.as_str(), message.content.trim()))
        .filter(|(_, content)| !content.is_empty())
        .collect::<Vec<_>>();
    if history.is_empty() {
        return None;
    }
    let mut budget = MODEL_SWITCH_CONTEXT_MAX_BYTES;
    let mut truncated = false;
    let mut retained = Vec::new();
    while let Some((role, body)) = history.pop() {
        let cost = role.len() + body.len() + 16;
        if cost > budget {
            truncated = true;
            break;
        }
        budget -= cost;
        retained.push((role, body));
    }
    retained.reverse();

    let mut attachment = format!(
        "Attached prior chat context from {} to {}.\n\
Do not answer this attachment. Use it only for continuity and wait for the next real user message.\n\n\
# Attached Transcript\n\n",
        provider_display_name(source_provider),
        provider_display_name(target_provider)
    );
    if truncated || !history.is_empty() {
        attachment.push_str("[Older transcript omitted]\n\n");
    }
    for (role, body) in retained {
        attachment.push_str(match role {
            "user" => "## User\n",
            "agent" => "## Agent\n",
            _ => continue,
        });
        attachment.push_str(body);
        attachment.push_str("\n\n");
    }
    Some(attachment.trim_end().to_owned())
}

fn pending_model_switch_context_attachment(messages: &[ChatMessageRecord]) -> Option<&str> {
    let (attachment_index, attachment) = messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, message)| message.source == "model_switch_context")?;
    let has_later_real_message = messages[attachment_index + 1..]
        .iter()
        .any(|message| matches!(message.role.as_str(), "user" | "agent" | "assistant"));
    (!has_later_real_message).then_some(attachment.content.as_str())
}

fn has_real_conversation_messages(messages: &[ChatMessageRecord]) -> bool {
    messages
        .iter()
        .any(|message| matches!(message.role.as_str(), "user" | "agent" | "assistant"))
}

fn provider_model_choices(
    readiness: &SetupReadiness,
    fallback_kind: SessionKind,
) -> Vec<ProviderModelChoice> {
    let mut choices = launchable_agent_tools()
        .filter(|tool| readiness.launchable_provider_ready(tool.provider_key))
        .flat_map(|tool| provider_model_choices_for_provider(tool.provider_key))
        .collect::<Vec<_>>();
    if choices.is_empty() {
        choices = provider_model_choices_for_provider(session_kind_provider(fallback_kind));
    }
    choices
}

fn provider_model_choices_for_provider(provider: &str) -> Vec<ProviderModelChoice> {
    match launchable_provider_name(provider) {
        Some(provider @ ("codex" | "claude")) => model_choices_for_provider(provider)
            .iter()
            .copied()
            .map(|model| ProviderModelChoice {
                provider: provider.to_owned(),
                model: Some(model.to_owned()),
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn selected_provider_model_choice_index(
    choices: &[ProviderModelChoice],
    kind: SessionKind,
    model: Option<&str>,
) -> usize {
    let provider = session_kind_provider(kind);
    choices
        .iter()
        .position(|choice| {
            choice.provider == provider
                && model.is_some_and(|model| choice.model.as_deref() == Some(model))
        })
        .or_else(|| {
            choices
                .iter()
                .position(|choice| choice.provider == provider)
        })
        .unwrap_or(0)
}

fn provider_display_name(provider: &str) -> &'static str {
    tool_by_provider(provider)
        .map(|tool| tool.display_name)
        .unwrap_or("Agent")
}

fn provider_icon_name(provider: &str) -> &'static str {
    match launchable_provider_name(provider) {
        Some("codex") => "code-symbolic",
        Some("claude") => "application-x-executable-symbolic",
        _ => "application-x-executable-symbolic",
    }
}

fn select_harness_and_dispatch(
    selected_harness: &RefCell<SessionKind>,
    reasoning_mode: &RefCell<Option<String>>,
    kind: SessionKind,
    switch_chat_harness: Option<&Rc<dyn Fn(SessionKind)>>,
    refresh_chat_surface: Option<&Rc<dyn Fn()>>,
    update_composer_state: Option<&Rc<dyn Fn()>>,
) {
    *selected_harness.borrow_mut() = kind;
    if matches!(kind, SessionKind::Codex | SessionKind::Claude) {
        *reasoning_mode.borrow_mut() = Some("high".to_owned());
    }
    if let Some(switch_harness) = switch_chat_harness {
        switch_harness(kind);
        if let Some(update) = update_composer_state {
            update();
        }
        return;
    }
    if let Some(refresh_view) = refresh_chat_surface {
        refresh_view();
    }
    if let Some(update) = update_composer_state {
        update();
    }
}

fn apply_thread_selection<F, U>(
    selected_thread: &RefCell<Option<i64>>,
    next_thread: Option<i64>,
    set_selected_chat_thread: F,
    update_composer_state: U,
) where
    F: FnOnce(Option<i64>),
    U: FnOnce(),
{
    if *selected_thread.borrow() == next_thread {
        return;
    }
    *selected_thread.borrow_mut() = next_thread;
    set_selected_chat_thread(next_thread);
    update_composer_state();
}

fn initial_chat_harness_from_setup(
    database_path: &Path,
    workspace_name: &str,
    readiness: &SetupReadiness,
) -> SessionKind {
    if let Some(provider) = configured_ready_provider(database_path, workspace_name, readiness) {
        return session_kind_from_provider(provider);
    }
    let provider = readiness
        .first_ready_launchable_provider()
        .unwrap_or("codex");
    persist_selected_provider(database_path, workspace_name, provider);
    session_kind_from_provider(provider)
}

fn selected_provider_blocker_message(
    kind: SessionKind,
    readiness: &SetupReadiness,
) -> Option<String> {
    if matches!(kind, SessionKind::Shell) {
        return None;
    }
    let provider = session_kind_provider(kind);
    (!readiness.launchable_provider_ready(provider)).then(|| {
        format!(
            "{} is not ready. Sign in or install it, then recheck setup before starting a chat.",
            session_kind_name(kind)
        )
    })
}

fn selected_provider_blocker_after_refresh(
    kind: SessionKind,
    readiness: &Rc<RefCell<SetupReadiness>>,
) -> Option<String> {
    *readiness.borrow_mut() = SetupReadiness::from_host();
    let current = readiness.borrow();
    selected_provider_blocker_message(kind, &current)
}

fn configured_ready_provider(
    database_path: &Path,
    workspace_name: &str,
    readiness: &SetupReadiness,
) -> Option<&'static str> {
    let configured = WorkspaceStore::open_app(database_path)
        .ok()
        .and_then(|store| store.workspace_repo_settings(workspace_name).ok())
        .and_then(|settings| settings.customization.automation.auto_start_agent);
    let provider = configured.as_deref()?;
    let provider = launchable_provider_name(provider)?;
    readiness
        .launchable_provider_ready(provider)
        .then_some(provider)
}

fn launchable_provider_name(provider: &str) -> Option<&'static str> {
    launchable_provider_key(provider)
}

fn session_kind_from_provider(provider: &str) -> SessionKind {
    match launchable_provider_name(provider) {
        Some("claude") => SessionKind::Claude,
        _ => SessionKind::Codex,
    }
}

fn persist_selected_provider(database_path: &Path, workspace_name: &str, provider: &str) {
    let Some(provider) = launchable_provider_name(provider) else {
        return;
    };
    let result = WorkspaceStore::save_local_default_agent_provider_for_database(
        database_path,
        workspace_name,
        provider,
    );
    if let Err(err) = result {
        warn!(workspace = %workspace_name, provider, error = %err, "failed to persist selected provider");
    }
}

fn session_reasoning_mode_from_index(index: usize) -> String {
    match index {
        0 => "low".to_owned(),
        1 => "medium".to_owned(),
        2 => "high".to_owned(),
        3 => "extra high".to_owned(),
        _ => "high".to_owned(),
    }
}

fn codex_reasoning_command(level: &str) -> Option<String> {
    match level.trim().to_ascii_lowercase().as_str() {
        "low" | "medium" | "high" => Some(format!("/thinking {}", level.trim())),
        "extra high" => Some("/thinking extra high".to_owned()),
        _ => None,
    }
}

fn queue_thread_command(
    pending: &RefCell<HashMap<i64, Vec<String>>>,
    thread_id: i64,
    command: String,
) {
    let command = command.trim().to_owned();
    if command.is_empty() {
        return;
    }
    let mut pending = pending.borrow_mut();
    let entry = pending.entry(thread_id).or_default();
    if command.starts_with("/model ") {
        entry.retain(|existing| !existing.starts_with("/model "));
    }
    if command.starts_with("/thinking ") {
        entry.retain(|existing| !existing.starts_with("/thinking "));
    }
    if entry.last().is_some_and(|existing| existing == &command) {
        return;
    }
    entry.push(command);
}

fn flush_pending_commands_for_send(
    pending: &RefCell<HashMap<i64, Vec<String>>>,
    thread_id: i64,
) -> Vec<String> {
    pending.borrow_mut().remove(&thread_id).unwrap_or_default()
}

fn queue_archcar_input(
    app_state: &AppState,
    thread_id: i64,
    input: String,
    visible_input: Option<String>,
    kind: ArchcarInputKind,
    session_kind: SessionKind,
) {
    let input = input.trim().to_owned();
    if input.is_empty() {
        return;
    }
    app_state.queue_chat_input(
        thread_id,
        QueuedChatInputDraft {
            input,
            visible_input,
            kind,
            session_kind,
        },
    );
}

fn queue_archcar_input_for_target(
    app_state: &AppState,
    target: ChatUiTarget,
    input: String,
    visible_input: Option<String>,
    kind: ArchcarInputKind,
    session_kind: SessionKind,
) {
    let input = input.trim().to_owned();
    if input.is_empty() {
        return;
    }
    app_state.queue_chat_input_for_target(
        target,
        QueuedChatInputDraft {
            input,
            visible_input,
            kind,
            session_kind,
        },
    );
}

fn selected_chat_target_for_submit(
    app_state: &AppState,
    selected_thread_id: Option<i64>,
) -> Option<ChatUiTarget> {
    match app_state.selected_chat_target() {
        Some(target @ ChatUiTarget::Pending { .. }) => Some(target),
        Some(ChatUiTarget::Thread(thread_id)) if selected_thread_id == Some(thread_id) => {
            Some(ChatUiTarget::Thread(thread_id))
        }
        _ => selected_thread_id.map(ChatUiTarget::Thread),
    }
}

fn composer_thread_for_target(
    chat_target: Option<&ChatUiTarget>,
    selected_thread_id: Option<i64>,
) -> Option<i64> {
    match chat_target {
        Some(ChatUiTarget::Pending { .. }) => None,
        Some(ChatUiTarget::Thread(thread_id)) => Some(*thread_id),
        None => selected_thread_id,
    }
}

fn queued_chat_input_visible_text(input: &QueuedChatInputDraft) -> String {
    input
        .visible_input
        .as_deref()
        .unwrap_or(&input.input)
        .trim()
        .to_owned()
}

fn submitted_user_input_texts_for_thread(
    thread_id: i64,
    _pending_archcar_inputs: &RefCell<HashMap<i64, Vec<QueuedArchcarInput>>>,
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
    persisted_messages: &[ChatMessageRecord],
) -> Vec<String> {
    let persisted = persisted_messages
        .iter()
        .filter(|message| message.role == "user")
        .map(|message| message.content.trim().to_owned())
        .collect::<HashSet<_>>();
    let mut inputs = Vec::new();

    for action in inflight_actions.borrow().values() {
        if let PendingArchcarAction::UserSend {
            thread_id: action_thread_id,
            input,
            visible_input,
            ..
        } = action
        {
            if *action_thread_id == thread_id {
                inputs.push(visible_input.as_deref().unwrap_or(input).trim().to_owned());
            }
        }
    }

    let mut seen = HashSet::new();
    inputs
        .into_iter()
        .filter(|input| !input.is_empty())
        .filter(|input| !persisted.contains(input))
        .filter(|input| seen.insert(input.clone()))
        .collect()
}

fn clear_inflight_user_sends_for_thread(
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
    thread_id: i64,
) {
    inflight_actions.borrow_mut().retain(|_, action| {
        !matches!(
            action,
            PendingArchcarAction::UserSend {
                thread_id: action_thread_id,
                ..
            } if *action_thread_id == thread_id
        )
    });
}

fn queued_chat_inputs_count(app_state: &AppState, thread_id: i64) -> usize {
    app_state.queued_chat_inputs_count(thread_id)
}

fn queued_composer_overlay_items_for_thread(
    app_state: &AppState,
    thread_id: Option<i64>,
    selected_kind: SessionKind,
) -> Vec<QueuedComposerItem> {
    let Some(thread_id) = thread_id else {
        return Vec::new();
    };
    app_state
        .queued_chat_inputs(thread_id)
        .into_iter()
        .enumerate()
        .map(|(index, input)| {
            let mut actions = vec![QueuedComposerAction::Delete, QueuedComposerAction::Edit];
            if managed_harness_for_kind(selected_kind).is_some() {
                actions.push(QueuedComposerAction::SendImmediately);
            }
            QueuedComposerItem {
                index,
                preview: truncate_queue_preview(&queued_chat_input_visible_text(&input)),
                actions,
            }
        })
        .collect()
}

fn truncate_queue_preview(text: &str) -> String {
    const LIMIT: usize = 80;
    let text = text.trim();
    if text.chars().count() <= LIMIT {
        return text.to_owned();
    }
    let mut preview = text.chars().take(LIMIT).collect::<String>();
    preview.push_str("...");
    preview
}

fn pop_next_queued_chat_input(
    app_state: &AppState,
    thread_id: i64,
) -> Option<QueuedChatInputDraft> {
    app_state.pop_next_queued_chat_input(thread_id)
}

fn chat_thread_waiting_for_starting_agent(app_state: &AppState, thread_id: i64) -> bool {
    matches!(
        app_state.chat_phase(&ChatUiTarget::Thread(thread_id)),
        Some(ChatUiPhase::StartingAgent { .. })
    )
}

fn mark_chat_startup_finished(app_state: &AppState, thread_id: i64) {
    if chat_thread_waiting_for_starting_agent(app_state, thread_id) {
        app_state.mark_chat_phase(ChatUiTarget::Thread(thread_id), ChatUiPhase::Ready);
    }
}

fn remove_queued_chat_input_at(
    app_state: &AppState,
    thread_id: i64,
    index: usize,
) -> Option<QueuedChatInputDraft> {
    app_state.remove_queued_chat_input_at(thread_id, index)
}

fn requeue_pending_input_front(app_state: &AppState, thread_id: i64, input: QueuedChatInputDraft) {
    app_state.requeue_chat_input_front(thread_id, input);
}

fn queue_pending_archcar_input(
    pending: &RefCell<HashMap<i64, Vec<QueuedArchcarInput>>>,
    thread_id: i64,
    mut input: QueuedArchcarInput,
) {
    input.input = input.input.trim().to_owned();
    if input.input.is_empty() {
        return;
    }
    pending
        .borrow_mut()
        .entry(thread_id)
        .or_default()
        .push(input);
}

fn pop_next_pending_archcar_input(
    pending: &RefCell<HashMap<i64, Vec<QueuedArchcarInput>>>,
    thread_id: i64,
) -> Option<QueuedArchcarInput> {
    let mut pending = pending.borrow_mut();
    let entry = pending.get_mut(&thread_id)?;
    if entry.is_empty() {
        pending.remove(&thread_id);
        return None;
    }
    let next = entry.remove(0);
    if entry.is_empty() {
        pending.remove(&thread_id);
    }
    Some(next)
}

fn requeue_pending_archcar_input_front(
    pending: &RefCell<HashMap<i64, Vec<QueuedArchcarInput>>>,
    thread_id: i64,
    input: QueuedArchcarInput,
) {
    pending
        .borrow_mut()
        .entry(thread_id)
        .or_default()
        .insert(0, input);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ComposerAction {
    Disabled,
    Send,
    Queue,
    Interrupt,
    SendQueued,
    SaveQueuedEdit,
    Retry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ComposerSubmitIntent {
    Default,
    Immediate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QueuedComposerAction {
    Delete,
    Edit,
    SendImmediately,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct QueuedComposerItem {
    index: usize,
    preview: String,
    actions: Vec<QueuedComposerAction>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct QueueOverlaySignature {
    thread_id: Option<i64>,
    editing: bool,
    items: Vec<QueuedComposerItem>,
}

fn queue_overlay_requires_rebuild(
    previous: Option<&QueueOverlaySignature>,
    current: &QueueOverlaySignature,
) -> bool {
    previous != Some(current)
}

fn composer_action_for_state(
    has_text: bool,
    has_active_generation: bool,
    was_interrupted: bool,
    queued_count: usize,
) -> ComposerAction {
    if has_active_generation {
        if has_text {
            ComposerAction::Queue
        } else {
            ComposerAction::Interrupt
        }
    } else if queued_count > 0 {
        ComposerAction::SendQueued
    } else if has_text {
        ComposerAction::Send
    } else if was_interrupted {
        ComposerAction::Retry
    } else {
        ComposerAction::Disabled
    }
}

fn composer_action_for_startup_state(
    has_text: bool,
    has_active_generation: bool,
    was_interrupted: bool,
    queued_count: usize,
    waiting_for_startup: bool,
) -> ComposerAction {
    if waiting_for_startup {
        if has_text {
            ComposerAction::Queue
        } else {
            ComposerAction::Disabled
        }
    } else {
        composer_action_for_state(
            has_text,
            has_active_generation,
            was_interrupted,
            queued_count,
        )
    }
}

fn composer_submit_intent_for_modifiers(modifiers: gtk::gdk::ModifierType) -> ComposerSubmitIntent {
    if modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
        ComposerSubmitIntent::Immediate
    } else {
        ComposerSubmitIntent::Default
    }
}

fn composer_send_button_submit_intent() -> ComposerSubmitIntent {
    ComposerSubmitIntent::Immediate
}

fn composer_action_for_submit_intent(
    action: ComposerAction,
    intent: ComposerSubmitIntent,
    has_text: bool,
    selected_kind: SessionKind,
    waiting_for_startup: bool,
) -> ComposerAction {
    if waiting_for_startup {
        return action;
    }
    if has_text
        && intent == ComposerSubmitIntent::Immediate
        && managed_harness_for_kind(selected_kind).is_some()
        && matches!(action, ComposerAction::Queue | ComposerAction::SendQueued)
    {
        ComposerAction::Send
    } else {
        action
    }
}

fn composer_send_button_presentation(action: ComposerAction) -> (&'static str, &'static str, bool) {
    match action {
        ComposerAction::Disabled => ("send-symbolic", "Send message", false),
        ComposerAction::Send => ("send-symbolic", "Send message", true),
        ComposerAction::Queue => ("send-symbolic", "Queue message locally", true),
        ComposerAction::Interrupt => (
            "media-playback-stop-symbolic",
            "Interrupt active turn",
            true,
        ),
        ComposerAction::SendQueued => ("send-symbolic", "Send queued messages", true),
        ComposerAction::SaveQueuedEdit => ("send-symbolic", "Save queued message", true),
        ComposerAction::Retry => ("view-refresh-symbolic", "Retry interrupted session", true),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InteractionUiKind {
    Permission,
    Question,
    Plan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InteractionPresentation {
    kind: InteractionUiKind,
    provider_label: String,
    title: String,
    detail: String,
    actions: Vec<&'static str>,
}

fn interaction_presentation(interaction: &ProviderInteractionRecord) -> InteractionPresentation {
    let kind = match interaction.kind {
        ProviderInteractionKind::Permission => InteractionUiKind::Permission,
        ProviderInteractionKind::UserQuestion => InteractionUiKind::Question,
        ProviderInteractionKind::PlanApproval => InteractionUiKind::Plan,
    };
    let actions = match interaction.kind {
        ProviderInteractionKind::Permission => vec!["Allow", "Deny", "Always allow"],
        ProviderInteractionKind::UserQuestion => vec!["Answer", "Deny"],
        ProviderInteractionKind::PlanApproval => vec!["Approve plan", "Keep planning"],
    };
    InteractionPresentation {
        kind,
        provider_label: provider_interaction_label(&interaction.provider_key),
        title: interaction.title.clone(),
        detail: interaction.detail.clone(),
        actions,
    }
}

fn provider_interaction_label(provider_key: &str) -> String {
    for kind in [SessionKind::Codex, SessionKind::Claude] {
        if let Some(harness) = managed_harness_for_kind(kind) {
            if harness.descriptor().provider_key == provider_key {
                return harness.descriptor().display_name.to_owned();
            }
        }
    }
    provider_key.to_owned()
}

fn question_answers_complete(questions: &[String], answers: &HashMap<String, String>) -> bool {
    !questions.is_empty()
        && questions.iter().all(|question| {
            answers
                .get(question)
                .is_some_and(|answer| !answer.trim().is_empty())
        })
}

fn set_composer_send_button_action(button: &Button, action: ComposerAction) {
    let (icon, tooltip, sensitive) = composer_send_button_presentation(action);
    let image = Image::from_icon_name(resolve_icon_name(icon));
    button.set_child(Some(&image));
    button.set_tooltip_text(Some(tooltip));
    button.set_sensitive(sensitive);
}

fn visible_live_controls_for_provider(provider: &str) -> Vec<String> {
    if let Some(harness) = managed_harness_for_kind(session_kind_from_provider(provider)) {
        return visible_live_controls_for_harness(harness.descriptor());
    }
    let controls = gtk_live_controls_for_provider(provider)
        .map(|control| control.control.to_owned())
        .collect::<Vec<_>>();
    if controls.is_empty() {
        vec!["provider".to_owned()]
    } else {
        controls
    }
}

fn visible_live_controls_for_harness(descriptor: &HarnessDescriptor) -> Vec<String> {
    let mut controls = vec!["provider".to_owned()];
    if descriptor
        .required_features
        .contains(&RequiredHarnessFeature::SessionControls)
    {
        controls.push("model".to_owned());
        controls.push("thinking".to_owned());
    }
    if descriptor.optional(HarnessCapability::Goals) == SupportMode::Native {
        controls.push("goal".to_owned());
    }
    controls
}

fn retry_agent_prompt() -> &'static str {
    "Retry the last failed or incomplete high-level action. If nothing is retryable, explain why."
}

fn latest_session_status_for_thread(
    records: &[ProcessRecord],
    thread_id: i64,
    kind: SessionKind,
) -> Option<ProcessStatus> {
    records
        .iter()
        .filter(|record| {
            record.chat_thread_id == Some(thread_id) && session_kind_matches_record(record, kind)
        })
        .max_by_key(|record| record.id)
        .map(|record| record.status)
}

fn active_generation_for_thread(
    records: &[ProcessRecord],
    working_threads: &RefCell<HashMap<i64, Instant>>,
    thread_id: i64,
    kind: SessionKind,
) -> bool {
    if running_session_for_thread(records, thread_id, kind).is_none() {
        return false;
    }
    working_threads.borrow().contains_key(&thread_id)
}

fn running_session_for_thread(
    records: &[ProcessRecord],
    thread_id: i64,
    kind: SessionKind,
) -> Option<i64> {
    records
        .iter()
        .find(|record| {
            record.chat_thread_id == Some(thread_id)
                && record.status == ProcessStatus::Running
                && session_kind_matches_record(record, kind)
        })
        .map(|record| record.id)
}

fn ready_running_session_for_thread(
    records: &[ProcessRecord],
    thread_id: i64,
    kind: SessionKind,
    ready_cache: &RefCell<HashMap<i64, bool>>,
) -> Option<i64> {
    running_session_for_thread(records, thread_id, kind).filter(|session_id| {
        ready_cache
            .borrow()
            .get(session_id)
            .copied()
            .unwrap_or(false)
    })
}

fn ready_queued_session_for_thread(
    records: &[ProcessRecord],
    thread_id: i64,
    harness: &'static HarnessDescriptor,
    ready_cache: &RefCell<HashMap<i64, bool>>,
) -> Option<i64> {
    ready_running_session_for_thread(records, thread_id, harness.kind, ready_cache)
}

fn resolve_or_create_thread_id_for_send<F>(
    thread_state: &RefCell<Vec<ChatThreadRecord>>,
    selected_thread: &RefCell<Option<i64>>,
    selected_kind: SessionKind,
    create_thread: F,
) -> anyhow::Result<i64>
where
    F: FnOnce(String) -> anyhow::Result<ChatThreadRecord>,
{
    let selected = *selected_thread.borrow();
    let existing_thread_id = {
        let current = thread_state.borrow();
        preferred_thread_for_kind(&current, selected, selected_kind)
    };
    if let Some(thread_id) = existing_thread_id {
        return Ok(thread_id);
    }

    let title = {
        let current = thread_state.borrow();
        default_chat_thread_title(selected_kind, &current)
    };
    let created = create_thread(title)?;
    let thread_id = created.id;
    thread_state.borrow_mut().insert(0, created);
    *selected_thread.borrow_mut() = Some(thread_id);
    Ok(thread_id)
}

fn preferred_session_for_kind(
    records: &[ProcessRecord],
    preferred: Option<i64>,
    kind: SessionKind,
) -> Option<i64> {
    let preferred_running = preferred.and_then(|id| {
        records
            .iter()
            .find(|record| {
                record.id == id
                    && record.status == ProcessStatus::Running
                    && session_kind_matches_record(record, kind)
            })
            .map(|record| record.id)
    });
    let any_running = || {
        records
            .iter()
            .find(|record| {
                record.status == ProcessStatus::Running && session_kind_matches_record(record, kind)
            })
            .map(|record| record.id)
    };
    let preferred_matches = || {
        preferred.and_then(|id| {
            records
                .iter()
                .find(|record| record.id == id && session_kind_matches_record(record, kind))
                .map(|record| record.id)
        })
    };

    preferred_running
        .or_else(any_running)
        .or_else(preferred_matches)
        .or_else(|| {
            records
                .iter()
                .find(|record| session_kind_matches_record(record, kind))
                .map(|record| record.id)
        })
}

fn resume_record_for_kind(
    records: &[ProcessRecord],
    preferred: Option<i64>,
    kind: SessionKind,
) -> Option<ProcessRecord> {
    if !matches!(kind, SessionKind::Codex | SessionKind::Claude) {
        return None;
    }

    preferred_session_for_kind(records, preferred, kind)
        .and_then(|id| {
            records.iter().find(|record| {
                record.id == id
                    && record.status != ProcessStatus::Running
                    && match kind {
                        SessionKind::Codex => true,
                        SessionKind::Claude => record.session_resume_id.is_some(),
                        SessionKind::Shell => false,
                    }
                    && session_kind_matches_record(record, kind)
            })
        })
        .cloned()
}

fn session_kind_matches_record(record: &ProcessRecord, kind: SessionKind) -> bool {
    session_kind_label(&record.command) == session_kind_name(kind)
}

fn session_harness_options_from_record(record: &ProcessRecord) -> SessionHarnessOptions {
    SessionHarnessOptions::from_metadata(record.session_harness_metadata.as_deref())
}

fn session_transcript_group_text(records: &[ProcessRecord], record: &ProcessRecord) -> String {
    let mut grouped_records: Vec<&ProcessRecord> =
        if let Some(resume_id) = record.session_resume_id.as_deref() {
            let mut grouped = records
                .iter()
                .filter(|candidate| {
                    candidate.kind == record.kind
                        && candidate.session_resume_id.as_deref() == Some(resume_id)
                })
                .collect::<Vec<_>>();
            grouped.sort_by_key(|candidate| candidate.id);
            grouped
        } else {
            vec![record]
        };

    let mut transcript = String::new();
    for (index, session) in grouped_records.drain(..).enumerate() {
        if index > 0 {
            transcript.push_str("\n[session resumed]\n");
        }
        match fs::read_to_string(&session.log_path) {
            Ok(contents) => transcript.push_str(&contents),
            Err(_) => transcript.push_str("[could not read selected session log]\n"),
        }
    }

    transcript
}

fn current_live_session_id(
    records: &[ProcessRecord],
    preferred: Option<i64>,
    kind: SessionKind,
    active_sessions: &HashSet<i64>,
) -> Option<i64> {
    preferred
        .filter(|id| {
            active_sessions.contains(id)
                && records
                    .iter()
                    .any(|record| record.id == *id && session_kind_matches_record(record, kind))
        })
        .or_else(|| {
            records
                .iter()
                .find(|record| {
                    record.status == ProcessStatus::Running
                        && session_kind_matches_record(record, kind)
                        && active_sessions.contains(&record.id)
                })
                .map(|record| record.id)
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionStopRoute {
    Archcar,
    Local,
}

fn session_stop_route(records: &[ProcessRecord], process_id: i64) -> Option<SessionStopRoute> {
    let record = records.iter().find(|record| record.id == process_id)?;
    if record.status == ProcessStatus::Running {
        Some(SessionStopRoute::Archcar)
    } else {
        Some(SessionStopRoute::Local)
    }
}

fn stop_active_chat_session(
    database_path: &Path,
    workspace_name: &str,
    process_id: i64,
    active_sessions: &Rc<RefCell<HashSet<i64>>>,
    last_output: &Rc<RefCell<HashMap<i64, Instant>>>,
) -> anyhow::Result<()> {
    active_sessions.borrow_mut().remove(&process_id);
    last_output.borrow_mut().remove(&process_id);
    WorkspaceStore::open_app(database_path)?
        .stop_session_process(workspace_name, process_id)
        .map(|_| ())
}

fn seed_chat_running_sessions(
    database_path: &Path,
    workspace_name: &str,
    active_sessions: &Rc<RefCell<HashSet<i64>>>,
    last_output: &Rc<RefCell<HashMap<i64, Instant>>>,
) {
    let Ok(store) = WorkspaceStore::open_app(database_path) else {
        return;
    };
    let Ok(records) = store.list_sessions(workspace_name) else {
        return;
    };

    let mut sessions = active_sessions.borrow_mut();
    for record in records {
        if record.status != ProcessStatus::Running || sessions.contains(&record.id) {
            continue;
        }
        sessions.insert(record.id);
        last_output.borrow_mut().insert(record.id, Instant::now());
    }
}

fn icon_text_button(text: &str, class_name: &str) -> Button {
    let button = text_button(text);
    button.add_css_class(class_name);
    button.set_tooltip_text(Some(match class_name {
        "chat-context-btn" => "Context used",
        _ => text,
    }));
    button
}

fn mode_icon_toggle_button(label: &str, icon_name: &str, active: bool) -> ToggleButton {
    let button = ToggleButton::new();
    button.add_css_class("chat-mode-btn");
    button.add_css_class("chat-footer-toggle");
    style_icon_button(&button);
    button.set_active(active);
    button.set_tooltip_text(Some(label));
    let icon = Image::from_icon_name(resolve_icon_name(icon_name));
    icon.add_css_class("chat-mode-icon");
    button.set_child(Some(&icon));
    button.connect_toggled({
        let button = button.clone();
        move |_| {
            if button.is_active() {
                button.add_css_class("chat-mode-selected");
            } else {
                button.remove_css_class("chat-mode-selected");
            }
        }
    });
    if active {
        button.add_css_class("chat-mode-selected");
    }
    button
}

fn mode_menu_button(
    label: &str,
    icon_name: &str,
    options: &[&str],
    selected_index: usize,
    on_selected: Rc<dyn Fn(usize)>,
) -> Button {
    let selected_label = options.get(selected_index).copied().unwrap_or(label);
    let button = Button::new();
    button.add_css_class("chat-mode-menu");
    style_text_button(&button);
    button.set_tooltip_text(Some(selected_label));

    let shell = mode_menu_child(icon_name, selected_label);
    button.set_child(Some(&shell));
    let popover = mode_menu_popover(
        button.clone(),
        icon_name,
        options,
        selected_index,
        on_selected,
    );
    popover.set_parent(&button);
    button.connect_clicked(move |_| {
        popover.popup();
    });
    button
}

fn mode_menu_child(icon_name: &str, text_label: &str) -> GBox {
    let shell = GBox::new(Orientation::Horizontal, 6);
    let icon: Widget = if icon_name.chars().count() == 1 {
        let icon = Label::new(Some(icon_name));
        icon.add_css_class("chat-mode-glyph");
        icon.upcast()
    } else {
        let icon = Image::from_icon_name(resolve_icon_name(icon_name));
        icon.add_css_class("chat-mode-icon");
        icon.upcast()
    };
    let text = Label::new(Some(text_label));
    text.add_css_class("chat-mode-label");
    text.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.set_max_width_chars(14);
    let arrow = Image::from_icon_name(resolve_icon_name("pan-down-symbolic"));
    arrow.add_css_class("chat-mode-arrow");
    shell.append(&icon);
    shell.append(&text);
    shell.append(&arrow);
    shell
}

fn provider_model_menu_button(
    choices: Rc<Vec<ProviderModelChoice>>,
    selected_index: usize,
    on_selected: Rc<dyn Fn(usize)>,
) -> Button {
    let selected = choices
        .get(selected_index)
        .or_else(|| choices.first())
        .cloned()
        .unwrap_or_else(|| ProviderModelChoice {
            provider: "codex".to_owned(),
            model: Some(CODEX_DEFAULT_MODEL.to_owned()),
        });
    let button = Button::new();
    button.add_css_class("chat-mode-menu");
    style_text_button(&button);
    button.set_tooltip_text(Some(&selected.button_label()));
    provider_model_menu_set_child(&button, &selected);
    let popover = provider_model_menu_popover(button.clone(), choices, selected_index, on_selected);
    popover.set_parent(&button);
    button.connect_clicked(move |_| {
        popover.popup();
    });
    button
}

fn provider_model_menu_set_child(button: &Button, choice: &ProviderModelChoice) {
    let shell = mode_menu_child(choice.icon_name(), &choice.button_label());
    button.set_child(Some(&shell));
}

fn provider_model_menu_popover(
    button: Button,
    choices: Rc<Vec<ProviderModelChoice>>,
    selected_index: usize,
    on_selected: Rc<dyn Fn(usize)>,
) -> Popover {
    let popover = Popover::new();
    popover.add_css_class("chat-menu-popover");
    let list = GBox::new(Orientation::Vertical, 4);
    list.add_css_class("chat-menu-list");

    let mut last_provider = None::<String>;
    for (index, choice) in choices.iter().enumerate() {
        if last_provider.as_deref() != Some(choice.provider.as_str()) {
            let header = Label::new(Some(choice.provider_label()));
            header.add_css_class("chat-menu-group-label");
            header.set_xalign(0.0);
            list.append(&header);
            last_provider = Some(choice.provider.clone());
        }

        let row = Button::new();
        row.add_css_class("chat-menu-item");
        let row_box = GBox::new(Orientation::Horizontal, 10);
        let icon = Image::from_icon_name(resolve_icon_name(choice.icon_name()));
        icon.add_css_class("chat-menu-item-icon");
        let name = Label::new(Some(choice.model_label()));
        name.add_css_class("chat-menu-item-label");
        name.set_xalign(0.0);
        name.set_hexpand(true);
        row_box.append(&icon);
        row_box.append(&name);
        row.set_child(Some(&row_box));
        if index == selected_index {
            row.add_css_class("chat-menu-item-selected");
        }
        let button_for_row = button.clone();
        let choice_for_row = choice.clone();
        let popover_for_row = popover.clone();
        let on_selected = on_selected.clone();
        let list_for_row = list.clone();
        row.connect_clicked(move |clicked| {
            let mut child = list_for_row.first_child();
            while let Some(widget) = child {
                if let Ok(button) = widget.clone().downcast::<Button>() {
                    button.remove_css_class("chat-menu-item-selected");
                }
                child = widget.next_sibling();
            }
            clicked.add_css_class("chat-menu-item-selected");
            provider_model_menu_set_child(&button_for_row, &choice_for_row);
            button_for_row.set_tooltip_text(Some(&choice_for_row.button_label()));
            on_selected(index);
            popover_for_row.popdown();
        });
        list.append(&row);
    }
    popover.set_child(Some(&list));
    popover
}

fn mode_menu_popover(
    button: Button,
    icon_name: &str,
    options: &[&str],
    selected_index: usize,
    on_selected: Rc<dyn Fn(usize)>,
) -> Popover {
    let popover = Popover::new();
    popover.add_css_class("chat-menu-popover");
    let list = GBox::new(Orientation::Vertical, 4);
    list.add_css_class("chat-menu-list");

    for (index, option) in options.iter().enumerate() {
        let row = Button::new();
        row.add_css_class("chat-menu-item");
        let row_box = GBox::new(Orientation::Horizontal, 10);
        let icon = Image::from_icon_name(resolve_icon_name("application-x-executable-symbolic"));
        icon.add_css_class("chat-menu-item-icon");
        let name = Label::new(Some(option));
        name.add_css_class("chat-menu-item-label");
        name.set_xalign(0.0);
        name.set_hexpand(true);
        let shortcut = Label::new(Some(&index.to_string()));
        shortcut.add_css_class("chat-menu-shortcut");
        row_box.append(&icon);
        row_box.append(&name);
        row_box.append(&shortcut);
        row.set_child(Some(&row_box));
        if index == selected_index {
            row.add_css_class("chat-menu-item-selected");
        }
        let button_for_row = button.clone();
        let icon_name = icon_name.to_owned();
        let option = (*option).to_owned();
        let popover_for_row = popover.clone();
        let on_selected = on_selected.clone();
        row.connect_clicked(move |_| {
            let shell = mode_menu_child(&icon_name, &option);
            button_for_row.set_child(Some(&shell));
            button_for_row.set_tooltip_text(Some(&option));
            on_selected(index);
            popover_for_row.popdown();
        });
        list.append(&row);
    }
    popover.set_child(Some(&list));
    popover
}

fn editor_picker_button() -> Button {
    let choices = Rc::new(RefCell::new(detected_editor_choices()));
    let initial = choices
        .borrow()
        .iter()
        .position(|choice| choice.name == "VS Code")
        .unwrap_or(0);
    let button = Button::new();
    button.add_css_class("chat-editor-menu");
    style_text_button(&button);
    editor_picker_set_button_child(&button, &choices.borrow()[initial]);
    let popover = editor_picker_popover(choices.clone(), initial, &button);
    popover.set_parent(&button);
    button.connect_clicked(move |_| {
        popover.popup();
    });
    button
}

fn editor_picker_set_button_child(button: &Button, choice: &EditorChoice) {
    let shell = GBox::new(Orientation::Horizontal, 6);
    let icon = Image::from_icon_name(resolve_icon_name(choice.icon));
    icon.add_css_class("chat-editor-icon");
    let text = Label::new(Some(&choice.name));
    text.add_css_class("chat-editor-label");
    text.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.set_max_width_chars(8);
    let arrow = Image::from_icon_name(resolve_icon_name("pan-down-symbolic"));
    arrow.add_css_class("chat-mode-arrow");
    shell.append(&icon);
    shell.append(&text);
    shell.append(&arrow);
    button.set_child(Some(&shell));
}

fn editor_picker_popover(
    choices: Rc<RefCell<Vec<EditorChoice>>>,
    initial: usize,
    button: &Button,
) -> Popover {
    let popover = Popover::new();
    popover.add_css_class("chat-menu-popover");
    let list = GBox::new(Orientation::Vertical, 4);
    list.add_css_class("chat-menu-list");
    for (index, choice) in choices.borrow().iter().enumerate() {
        let row = Button::new();
        row.add_css_class("chat-menu-item");
        let row_box = GBox::new(Orientation::Horizontal, 10);
        let icon = Image::from_icon_name(resolve_icon_name(choice.icon));
        icon.add_css_class("chat-menu-item-icon");
        let name = Label::new(Some(&choice.name));
        name.add_css_class("chat-menu-item-label");
        name.set_xalign(0.0);
        name.set_hexpand(true);
        let shortcut = Label::new(Some(&index.to_string()));
        shortcut.add_css_class("chat-menu-shortcut");
        row_box.append(&icon);
        row_box.append(&name);
        row_box.append(&shortcut);
        row.set_child(Some(&row_box));
        if index == initial {
            row.add_css_class("chat-menu-item-selected");
        }
        let button = button.clone();
        let choice = choice.clone();
        let popover_for_row = popover.clone();
        row.connect_clicked(move |_| {
            editor_picker_set_button_child(&button, &choice);
            popover_for_row.popdown();
        });
        list.append(&row);
    }
    popover.set_child(Some(&list));
    popover
}

fn detected_editor_choices() -> Vec<EditorChoice> {
    let mut choices = Vec::new();
    for (name, icon, command) in [
        ("VS Code", "code-symbolic", "code"),
        ("VSCodium", "code-symbolic", "codium"),
        ("Zed", "zed-symbolic", "zed"),
        ("Sublime Text", "accessories-text-editor-symbolic", "subl"),
        ("Kate", "accessories-text-editor-symbolic", "kate"),
        (
            "GNOME Text Editor",
            "accessories-text-editor-symbolic",
            "gnome-text-editor",
        ),
        ("Mousepad", "accessories-text-editor-symbolic", "mousepad"),
    ] {
        if command_on_path(command) {
            choices.push(EditorChoice {
                name: name.to_owned(),
                icon,
                command: command.to_owned(),
            });
        }
    }
    if choices.is_empty() {
        choices.push(EditorChoice {
            name: "Mousepad".to_owned(),
            icon: "accessories-text-editor-symbolic",
            command: "mousepad".to_owned(),
        });
    }
    choices
}

fn command_on_path(command: &str) -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&path).any(|dir| {
        let candidate = dir.join(command);
        candidate.is_file() || executable_file_exists(&candidate)
    })
}

fn executable_file_exists(path: &Path) -> bool {
    let Ok(meta) = stdfs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

fn preserve_combo_selection(
    combo: &ComboBoxText,
    records: &[ProcessRecord],
    preserve: Option<i64>,
    attached_sessions: &HashSet<i64>,
) -> Option<i64> {
    combo.remove_all();
    for record in records {
        combo.append(
            Some(&record.id.to_string()),
            &session_summary_label(record, attached_sessions.contains(&record.id)),
        );
    }

    if records.is_empty() {
        return None;
    }

    let selected_index = preserve
        .and_then(|id| records.iter().position(|record| record.id == id))
        .or(Some(0));

    if let Some(index) = selected_index.filter(|index| *index < records.len()) {
        combo.set_active(Some(index as u32));
        return Some(records[index].id);
    }

    combo.set_active(Some(0));
    records.first().map(|record| record.id)
}

fn initial_session_text(database_path: &Path, workspace_name: &str) -> String {
    let sessions = WorkspaceStore::open_app(database_path)
        .and_then(|store| store.list_sessions(workspace_name))
        .unwrap_or_default();
    if sessions.is_empty() {
        return format!("No local sessions yet for {workspace_name}. Start a session above.\n\n");
    }

    let mut out = format!("Recent sessions for {workspace_name}:\n");
    for record in sessions.iter().take(SESSION_TAIL_HISTORY) {
        out.push_str(&format!(
            "#{} {} status={} pid={} started={}\n",
            record.id,
            session_kind_label(&record.command),
            record.status.as_str(),
            record.pid,
            record.started_at,
        ));
    }
    out
}

fn format_selected_session_surface(
    record: &ProcessRecord,
    transcript: &str,
    runtime_state: &str,
    attached: bool,
) -> String {
    let harness = record
        .session_harness_metadata
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("default");
    let attachment = session_attachment_label(record, attached);
    let exit = record
        .exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "-".to_owned());
    let ended = record.ended_at.as_deref().unwrap_or("-");
    let event_summary = raw_session_transcript_summary(transcript);
    let transcript = raw_session_transcript_display(transcript);

    format!(
        "Session #{} - {}\nStatus: {}\nState: {}\nAttachment: {}\nEvents: {}\nPID: {}\nStarted: {}\nEnded: {}\nExit: {}\nHarness: {}\nCommand: {}\n\nTranscript\n{}\n",
        record.id,
        session_kind_label(&record.command),
        record.status.as_str(),
        runtime_state,
        attachment,
        event_summary,
        record.pid,
        record.started_at,
        ended,
        exit,
        harness,
        record.command,
        transcript,
    )
}

fn session_attachment_label(record: &ProcessRecord, attached: bool) -> &'static str {
    if record.status != ProcessStatus::Running {
        return "saved";
    }
    if attached {
        "attached"
    } else {
        "detached"
    }
}

fn session_transcript_event_summary(events: &[SessionTranscriptEvent]) -> String {
    let mut user = 0usize;
    let mut review = 0usize;
    let mut system = 0usize;
    let mut agent = 0usize;
    let mut tool = 0usize;
    let mut skill = 0usize;
    let mut harness = 0usize;
    for event in events {
        match event.role {
            SessionTranscriptRole::User => user += 1,
            SessionTranscriptRole::ReviewPrompt => review += 1,
            SessionTranscriptRole::System => system += 1,
            SessionTranscriptRole::Agent => agent += 1,
            SessionTranscriptRole::Tool => tool += 1,
            SessionTranscriptRole::Skill => skill += 1,
            SessionTranscriptRole::Harness => harness += 1,
        }
    }
    format!(
        "{} total, {user} user, {review} review, {system} system, {agent} agent, {tool} tool, {skill} skill, {harness} harness",
        events.len()
    )
}

fn raw_session_transcript_summary(transcript: &str) -> String {
    let display = raw_session_transcript_display(transcript);
    if display == "[no transcript output yet]\n" {
        "raw transcript, 0 bytes".to_owned()
    } else {
        format!("raw transcript, {} bytes", display.len())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionTranscriptEvent {
    role: SessionTranscriptRole,
    body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionTranscriptRole {
    System,
    User,
    ReviewPrompt,
    Agent,
    Tool,
    Skill,
    Harness,
}

impl SessionTranscriptRole {
    fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::User => "You",
            Self::ReviewPrompt => "Review Prompt",
            Self::Agent => "Agent",
            Self::Tool => "Tool",
            Self::Skill => "Skill",
            Self::Harness => "Harness",
        }
    }
}

fn session_input_log_text(workspace_name: &str, process_id: i64, input: &str) -> String {
    format!(
        "\n[user input {workspace_name}#{process_id}]\n{}\n[/user input]\n",
        input.trim()
    )
}

fn session_handoff_log_text(workspace_name: &str, process_id: i64, input: &str) -> String {
    format!(
        "\n[session handoff {workspace_name}#{process_id}]\n{}\n",
        input.trim()
    )
}

fn render_session_transcript_events(transcript: &str) -> String {
    let events = parse_session_transcript_events(transcript);
    if events.is_empty() {
        return "[no transcript output yet]\n".to_owned();
    }

    let mut rendered = String::new();
    for event in events {
        rendered.push_str(event.role.label());
        rendered.push('\n');
        rendered.push_str(&event.body);
        if !rendered.ends_with('\n') {
            rendered.push('\n');
        }
        rendered.push('\n');
    }
    rendered
}

fn live_session_append_text(text: &str) -> String {
    raw_session_transcript_display(text)
}

fn raw_session_transcript_display(transcript: &str) -> String {
    let cleaned = terminal_display_text(&trim_session_scrollback(transcript)).to_string();
    if cleaned.trim().is_empty() {
        "[no transcript output yet]\n".to_owned()
    } else if cleaned.ends_with('\n') {
        cleaned
    } else {
        format!("{cleaned}\n")
    }
}

fn parse_session_transcript_events(transcript: &str) -> Vec<SessionTranscriptEvent> {
    let cleaned = terminal_display_text(&trim_session_scrollback(transcript)).to_string();
    let lines = cleaned.lines().collect::<Vec<_>>();
    let mut events = Vec::new();
    let mut index = 0usize;

    while index < lines.len() {
        let line = lines[index].trim_end();
        if line.trim().is_empty() {
            index += 1;
            continue;
        }

        if line == "[codex raw]" {
            let (_, next) = collect_codex_block(&lines, index + 1, "[/codex raw]");
            index = next;
            continue;
        }

        if line == "[codex screen]" {
            let (_, next) = collect_codex_block(&lines, index + 1, "[/codex screen]");
            index = next;
            continue;
        }

        if is_user_input_marker(line) {
            let (body, next) = collect_user_input_event(&lines, index + 1);
            push_session_event(&mut events, SessionTranscriptRole::User, body);
            index = next;
            continue;
        }

        if line == "[staged review prompt]" {
            let (body, next) = collect_review_prompt_event(&lines, index + 1);
            push_session_event(&mut events, SessionTranscriptRole::ReviewPrompt, body);
            index = next;
            continue;
        }

        if let Some(role) = session_event_role_for_line(line) {
            let (body, next) = collect_session_role_event(&lines, index, role);
            push_session_event(&mut events, role, body);
            index = next;
            continue;
        }

        let (body, next) = collect_until_marker(&lines, index);
        let body = normalize_agent_event_body(&body, events.is_empty());
        push_session_event(&mut events, SessionTranscriptRole::Agent, body);
        index = next;
    }

    if events.len() > SESSION_TAIL_HISTORY {
        events.drain(0..events.len() - SESSION_TAIL_HISTORY);
    }
    events
}

fn normalize_agent_event_body(body: &str, is_first_event: bool) -> String {
    if !is_first_event {
        return body.to_owned();
    }

    let lines = body.lines().collect::<Vec<_>>();
    let first_non_noise = lines
        .iter()
        .position(|line| !is_codex_startup_noise(line.trim()));

    match first_non_noise {
        Some(index) => lines[index..].join("\n"),
        None => String::new(),
    }
}

fn push_session_event(
    events: &mut Vec<SessionTranscriptEvent>,
    role: SessionTranscriptRole,
    body: String,
) {
    let body = body.trim().to_owned();
    if !body.is_empty() {
        events.push(SessionTranscriptEvent { role, body });
    }
}

fn collect_codex_block(lines: &[&str], start: usize, end_marker: &str) -> (String, usize) {
    let mut body = Vec::new();
    let mut index = start;
    while index < lines.len() {
        let line = lines[index].trim_end();
        if line == end_marker {
            return (body.join("\n"), index + 1);
        }
        body.push(line);
        index += 1;
    }
    (body.join("\n"), index)
}

fn collect_until_marker(lines: &[&str], start: usize) -> (String, usize) {
    let mut body = Vec::new();
    let mut index = start;
    while index < lines.len() {
        let line = lines[index].trim_end();
        if is_session_event_marker(line) {
            break;
        }
        body.push(line);
        index += 1;
    }
    (body.join("\n"), index)
}

fn collect_session_role_event(
    lines: &[&str],
    start: usize,
    role: SessionTranscriptRole,
) -> (String, usize) {
    let header = lines[start].trim_end();
    if matches!(
        role,
        SessionTranscriptRole::System | SessionTranscriptRole::Harness
    ) {
        return (header.to_owned(), start + 1);
    }
    if role == SessionTranscriptRole::Tool && is_raw_file_change_event_line(header) {
        return collect_file_change_event(lines, start);
    }
    if role == SessionTranscriptRole::Tool
        && !header.starts_with("[tool ")
        && !header.starts_with("Ran ")
        && !header.starts_with("Read ")
        && !raw_tool_event_collects_body(header)
    {
        return (header.to_owned(), start + 1);
    }

    let mut body = vec![header];
    let mut index = start + 1;
    while index < lines.len() {
        let line = lines[index].trim_end();
        if is_session_event_marker(line) {
            break;
        }
        body.push(line);
        index += 1;
    }
    (body.join("\n"), index)
}

fn raw_tool_event_collects_body(header: &str) -> bool {
    let trimmed = header.trim_start().trim_start_matches('•').trim_start();
    if trimmed.contains("functions.apply_patch") || trimmed.contains("functions.write_stdin") {
        return true;
    }
    let Some((verb, _)) = trimmed.split_once(' ') else {
        return false;
    };
    matches!(verb, "Write" | "Edit" | "Create")
}

fn collect_file_change_event(lines: &[&str], start: usize) -> (String, usize) {
    let mut body = vec![lines[start].trim_end()];
    let mut index = start + 1;
    while index < lines.len() {
        let line = lines[index].trim_end();
        if is_session_event_marker(line) || !is_file_change_detail_line(line) {
            break;
        }
        body.push(line);
        index += 1;
    }
    (body.join("\n"), index)
}

fn is_file_change_detail_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("@@")
        || trimmed.starts_with("diff --git")
        || trimmed.starts_with("index ")
        || trimmed.starts_with("new file mode ")
        || trimmed.starts_with("deleted file mode ")
        || trimmed.starts_with("rename from ")
        || trimmed.starts_with("rename to ")
        || trimmed.starts_with("similarity index ")
        || trimmed.starts_with("dissimilarity index ")
        || trimmed.starts_with("+++")
        || trimmed.starts_with("---")
        || trimmed.starts_with('+')
        || trimmed.starts_with('-')
        || is_numbered_file_change_detail_line(trimmed)
}

fn is_numbered_file_change_detail_line(line: &str) -> bool {
    let Some((_, rest)) = parse_numbered_detail_prefix(line) else {
        return false;
    };
    rest.starts_with("  ") || rest.starts_with(" +") || rest.starts_with(" -")
}

fn parse_numbered_detail_prefix(line: &str) -> Option<(u32, &str)> {
    let first_non_digit = line
        .char_indices()
        .find_map(|(index, ch)| (!ch.is_ascii_digit()).then_some(index))?;
    if first_non_digit == 0 {
        return None;
    }
    let number = line[..first_non_digit].parse().ok()?;
    Some((number, &line[first_non_digit..]))
}

fn collect_user_input_event(lines: &[&str], start: usize) -> (String, usize) {
    if let Some(end) = user_input_end_index(lines, start) {
        let body = lines[start..end]
            .iter()
            .map(|line| line.trim_end())
            .collect::<Vec<_>>()
            .join("\n");
        return (body, end + 1);
    }

    let mut index = start;
    while index < lines.len() && lines[index].trim().is_empty() {
        index += 1;
    }
    if index >= lines.len() || is_session_event_marker(lines[index].trim_end()) {
        return (String::new(), index);
    }
    collect_until_marker(lines, index)
}

fn user_input_end_index(lines: &[&str], start: usize) -> Option<usize> {
    lines
        .iter()
        .enumerate()
        .skip(start)
        .find_map(|(index, line)| (line.trim_end() == "[/user input]").then_some(index))
}

fn collect_review_prompt_event(lines: &[&str], start: usize) -> (String, usize) {
    if let Some(end) = review_prompt_end_index(lines, start) {
        let body = lines[start..end]
            .iter()
            .map(|line| line.trim_end())
            .collect::<Vec<_>>()
            .join("\n");
        return (body, end + 1);
    }

    let mut body = Vec::new();
    let mut index = start;
    while index < lines.len() {
        let line = lines[index].trim_end();
        if is_session_event_marker(line) || (!body.is_empty() && line.trim().is_empty()) {
            break;
        }
        body.push(line);
        index += 1;
    }
    (body.join("\n"), index)
}

fn review_prompt_end_index(lines: &[&str], start: usize) -> Option<usize> {
    lines
        .iter()
        .enumerate()
        .skip(start)
        .find_map(|(index, line)| (line.trim_end() == "[/staged review prompt]").then_some(index))
}

fn is_session_event_marker(line: &str) -> bool {
    is_user_input_marker(line)
        || line == "[/user input]"
        || line == "[staged review prompt]"
        || line == "[/staged review prompt]"
        || line == "[codex raw]"
        || line == "[/codex raw]"
        || line == "[codex screen]"
        || line == "[/codex screen]"
        || session_event_role_for_line(line).is_some()
}

fn is_user_input_marker(line: &str) -> bool {
    line.starts_with("[user input ") && line.ends_with(']')
}

fn is_codex_startup_noise(line: &str) -> bool {
    if line.is_empty() {
        return true;
    }

    let lower = line.to_ascii_lowercase();
    lower.starts_with("openai codex")
        || lower.starts_with("codex v")
        || lower.contains("update available")
        || lower.contains("tip:")
        || lower.contains("booting mcp server:")
        || lower.contains("usage limit reset")
        || lower.starts_with("for help, type")
        || lower.starts_with("using workdir ")
        || lower.contains("write tests for @")
        || line.starts_with('╭')
        || line.starts_with('│')
        || line.starts_with('╰')
}

fn should_send_composer_message(key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool {
    matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter)
        && !modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK)
}

fn guarded_gtk_callback<T, F>(fallback: T, callback: F) -> T
where
    F: FnOnce() -> T,
{
    match catch_unwind(AssertUnwindSafe(callback)) {
        Ok(value) => value,
        Err(payload) => {
            error!(
                panic = %panic_payload_message(&payload),
                backtrace = %Backtrace::force_capture(),
                "recovered panic inside GTK callback"
            );
            fallback
        }
    }
}

fn panic_payload_message(payload: &Box<dyn Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_owned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CodexStartupState {
    Idle,
    Loading { message: String },
    Error { message: String },
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CodexStartupSignal {
    Loading { thread_id: i64 },
    Ready { thread_id: i64 },
    Error { thread_id: i64, message: String },
}

fn default_codex_loading_state() -> CodexStartupState {
    CodexStartupState::Loading {
        message: "Starting agent...".to_owned(),
    }
}

fn codex_startup_state_ready() -> CodexStartupState {
    CodexStartupState::Ready
}

fn codex_startup_state_from_error(message: impl Into<String>) -> CodexStartupState {
    CodexStartupState::Error {
        message: message.into(),
    }
}

fn derive_codex_startup_state(
    has_live_codex_session: bool,
    codex_ready: bool,
    current_state: Option<CodexStartupState>,
) -> CodexStartupState {
    if codex_ready {
        return codex_startup_state_ready();
    }

    match current_state {
        Some(CodexStartupState::Error { message }) => CodexStartupState::Error { message },
        Some(CodexStartupState::Loading { .. })
        | Some(CodexStartupState::Idle)
        | Some(CodexStartupState::Ready)
        | None
            if !has_live_codex_session =>
        {
            CodexStartupState::Idle
        }
        Some(CodexStartupState::Loading { message }) => CodexStartupState::Loading { message },
        _ => default_codex_loading_state(),
    }
}

fn codex_thread_ready_for_ui(
    thread_id: i64,
    records: &[ProcessRecord],
    ready_cache: &RefCell<HashMap<i64, bool>>,
    current_state: Option<&CodexStartupState>,
) -> bool {
    managed_thread_ready_for_ui(
        SessionKind::Codex,
        thread_id,
        records,
        ready_cache,
        current_state,
    )
}

fn managed_thread_ready_for_ui(
    kind: SessionKind,
    thread_id: i64,
    records: &[ProcessRecord],
    ready_cache: &RefCell<HashMap<i64, bool>>,
    current_state: Option<&CodexStartupState>,
) -> bool {
    thread_has_ready_managed_session(records, thread_id, kind, ready_cache)
        || matches!(current_state, Some(CodexStartupState::Ready))
}

fn apply_codex_startup_signal(
    states: &mut HashMap<i64, CodexStartupState>,
    signal: CodexStartupSignal,
) {
    match signal {
        CodexStartupSignal::Loading { thread_id } => {
            states.insert(thread_id, default_codex_loading_state());
        }
        CodexStartupSignal::Ready { thread_id } => {
            states.insert(thread_id, codex_startup_state_ready());
        }
        CodexStartupSignal::Error { thread_id, message } => {
            states.insert(thread_id, codex_startup_state_from_error(message));
        }
    }
}

fn thread_has_live_codex_session(records: &[ProcessRecord], thread_id: i64) -> bool {
    let Some(harness) = managed_harness_for_kind(SessionKind::Codex) else {
        return false;
    };
    thread_has_live_managed_session(records, thread_id, harness.descriptor())
}

fn thread_has_live_managed_session(
    records: &[ProcessRecord],
    thread_id: i64,
    harness: &'static HarnessDescriptor,
) -> bool {
    records.iter().any(|record| {
        record.status == ProcessStatus::Running
            && record.chat_thread_id == Some(thread_id)
            && session_kind_matches_record(record, harness.kind)
    })
}

fn thread_has_ready_managed_session(
    records: &[ProcessRecord],
    thread_id: i64,
    kind: SessionKind,
    ready_cache: &RefCell<HashMap<i64, bool>>,
) -> bool {
    records.iter().any(|record| {
        record.status == ProcessStatus::Running
            && record.chat_thread_id == Some(thread_id)
            && session_kind_matches_record(record, kind)
            && ready_cache
                .borrow()
                .get(&record.id)
                .copied()
                .unwrap_or(false)
    })
}

fn codex_startup_state_for_thread(
    thread_id: i64,
    records: &[ProcessRecord],
    ready_cache: &RefCell<HashMap<i64, bool>>,
    current_state: Option<CodexStartupState>,
) -> CodexStartupState {
    let ready = codex_thread_ready_for_ui(thread_id, records, ready_cache, current_state.as_ref());
    derive_codex_startup_state(
        thread_has_live_codex_session(records, thread_id),
        ready,
        current_state,
    )
}

fn codex_thread_id_for_session(
    session_id: i64,
    session_threads: &RefCell<HashMap<i64, i64>>,
    records: &[ProcessRecord],
) -> Option<i64> {
    session_threads
        .borrow()
        .get(&session_id)
        .copied()
        .or_else(|| {
            records
                .iter()
                .find(|record| record.id == session_id)
                .and_then(|record| record.chat_thread_id)
        })
}

fn codex_thread_id_for_startup_error(
    session_id: Option<i64>,
    event_thread_id: Option<i64>,
    session_threads: &RefCell<HashMap<i64, i64>>,
    records: &[ProcessRecord],
    selected_harness: SessionKind,
    selected_thread_id: Option<i64>,
) -> Option<i64> {
    if event_thread_id.is_some() {
        return event_thread_id;
    }
    match session_id {
        Some(session_id) => codex_thread_id_for_session(session_id, session_threads, records),
        None if selected_harness == SessionKind::Codex => selected_thread_id,
        None => None,
    }
}

fn mark_thread_working(working_threads: &RefCell<HashMap<i64, Instant>>, thread_id: i64) {
    working_threads
        .borrow_mut()
        .insert(thread_id, Instant::now());
}

fn append_session_status_message(messages: &GBox, text: &str) {
    let error = Label::new(Some(text));
    error.add_css_class("chat-agent-text");
    error.set_selectable(true);
    error.set_wrap(true);
    error.set_xalign(0.0);
    append_revealed(messages, &error);
}

fn clear_thread_working(working_threads: &RefCell<HashMap<i64, Instant>>, thread_id: i64) -> bool {
    working_threads.borrow_mut().remove(&thread_id).is_some()
}

fn working_elapsed_for_thread(
    working_threads: &RefCell<HashMap<i64, Instant>>,
    thread_id: i64,
) -> Option<Duration> {
    working_threads
        .borrow()
        .get(&thread_id)
        .map(Instant::elapsed)
}

fn working_elapsed_seconds_for_signature(elapsed: Option<Duration>) -> Option<u64> {
    elapsed.map(|elapsed| elapsed.as_secs())
}

fn working_elapsed_seconds_for_status_banner(
    banner: ChatStatusBannerKind,
    elapsed: Option<Duration>,
) -> Option<u64> {
    if banner == ChatStatusBannerKind::Working {
        working_elapsed_seconds_for_signature(elapsed)
    } else {
        None
    }
}

fn chat_status_banner_kind(
    startup_state: &CodexStartupState,
    working_elapsed: Option<Duration>,
    has_chat_rows: bool,
) -> ChatStatusBannerKind {
    if codex_startup_state_has_banner(startup_state) {
        return ChatStatusBannerKind::Startup;
    }
    if has_chat_rows {
        return ChatStatusBannerKind::None;
    }
    if working_elapsed.is_some() {
        return ChatStatusBannerKind::Working;
    }
    ChatStatusBannerKind::None
}

fn codex_startup_state_has_banner(state: &CodexStartupState) -> bool {
    matches!(
        state,
        CodexStartupState::Loading { .. } | CodexStartupState::Error { .. }
    )
}

fn append_chat_status_banner(
    messages: &GBox,
    banner: ChatStatusBannerKind,
    startup_state: &CodexStartupState,
    working_elapsed: Option<Duration>,
) {
    match banner {
        ChatStatusBannerKind::None => {}
        ChatStatusBannerKind::Startup => {
            if let Some(widget) = codex_startup_state_widget(startup_state) {
                append_chat_refresh_row(messages, &widget);
            }
        }
        ChatStatusBannerKind::Working => {
            if let Some(elapsed) = working_elapsed {
                append_chat_refresh_row(messages, &codex_working_indicator_widget(elapsed));
            }
        }
    }
}

fn format_working_elapsed(elapsed: Duration) -> String {
    let total_seconds = elapsed.as_secs();
    let seconds = total_seconds % 60;
    let minutes = (total_seconds / 60) % 60;
    let hours = total_seconds / 3600;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn codex_working_indicator_widget(elapsed: Duration) -> Widget {
    let row = GBox::new(Orientation::Horizontal, 8);
    row.set_hexpand(true);
    row.set_halign(Align::Fill);
    row.set_margin_bottom(10);
    row.add_css_class("chat-working-indicator");

    let spinner = Spinner::new();
    spinner.start();
    row.append(&spinner);

    let label = Label::new(Some(&format!(
        "Working {}",
        format_working_elapsed(elapsed)
    )));
    label.add_css_class("card-meta");
    label.set_xalign(0.0);
    label.set_hexpand(true);
    row.append(&label);

    row.upcast()
}

fn chat_interrupted_notice_widget() -> Widget {
    let row = GBox::new(Orientation::Horizontal, 0);
    row.set_halign(Align::Start);
    row.set_margin_top(4);
    row.set_margin_bottom(10);
    row.add_css_class("chat-interrupted-row");

    let label = Label::new(Some("Interrupted"));
    label.add_css_class("chat-interrupted-pill");
    label.set_xalign(0.0);
    row.append(&label);

    row.upcast()
}

fn codex_startup_state_widget(state: &CodexStartupState) -> Option<Widget> {
    let text = match state {
        CodexStartupState::Loading { message } | CodexStartupState::Error { message } => message,
        CodexStartupState::Idle | CodexStartupState::Ready => return None,
    };
    let label = Label::new(Some(text));
    label.add_css_class("chat-agent-text");
    if matches!(state, CodexStartupState::Error { .. }) {
        label.add_css_class("status-error");
    }
    label.set_selectable(true);
    label.set_wrap(true);
    label.set_xalign(0.0);
    Some(label.upcast())
}

fn session_event_role_for_line(line: &str) -> Option<SessionTranscriptRole> {
    if line.starts_with("[session ")
        || line.starts_with("[checkpoint")
        || line.starts_with("[could not ")
        || line.starts_with("[error")
    {
        return Some(SessionTranscriptRole::System);
    }
    if line.starts_with("[archductor bootstrap") || line.starts_with("[harness ") {
        return Some(SessionTranscriptRole::Harness);
    }
    if line.starts_with("[tool ") {
        return Some(SessionTranscriptRole::Tool);
    }
    if line.starts_with("[skill ") {
        return Some(SessionTranscriptRole::Skill);
    }
    if is_raw_skill_event_line(line) {
        return Some(SessionTranscriptRole::Skill);
    }
    if is_raw_tool_event_line(line) {
        return Some(SessionTranscriptRole::Tool);
    }
    None
}

fn is_raw_skill_event_line(line: &str) -> bool {
    let lower = line.trim().to_ascii_lowercase();
    let has_skill = lower.contains(" skill");
    (has_skill
        && (lower.starts_with("using ")
            || lower.starts_with("reading ")
            || lower.starts_with("read ")))
        || lower.starts_with("using writing-plans ")
}

fn is_raw_tool_event_line(line: &str) -> bool {
    let trimmed = line
        .trim()
        .strip_prefix('•')
        .map(str::trim_start)
        .unwrap_or_else(|| line.trim());
    if trimmed
        .strip_prefix("Ran ")
        .is_some_and(|command| !command.trim().is_empty())
    {
        return true;
    }
    if is_raw_file_change_event_line(trimmed) {
        return true;
    }
    let Some((verb, rest)) = trimmed.split_once(' ') else {
        return false;
    };
    let verb_matches = matches!(
        verb,
        "Read"
            | "Update"
            | "Create"
            | "Delete"
            | "Open"
            | "Search"
            | "Find"
            | "List"
            | "Move"
            | "Rename"
            | "Write"
            | "Edit"
    );
    verb_matches && raw_tool_target_looks_path_like(rest)
}

fn is_raw_file_change_event_line(line: &str) -> bool {
    let trimmed = line
        .trim()
        .strip_prefix('•')
        .map(str::trim_start)
        .unwrap_or_else(|| line.trim());
    matches!(
        parse_codex_inline_event(trimmed),
        Some(CoreCodexInlineEvent::FileChange(_))
    )
}

fn raw_tool_target_looks_path_like(rest: &str) -> bool {
    let first = rest
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|ch| {
            matches!(
                ch,
                '`' | '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | ':' | ','
            )
        });
    !first.is_empty() && (first.contains('/') || first.contains('\\') || first.contains('.'))
}

fn seed_running_sessions(
    database_path: &Path,
    workspace_name: &str,
    active_sessions: &Rc<RefCell<HashSet<i64>>>,
    transcript_buffer: &TextBuffer,
) {
    let Ok(store) = WorkspaceStore::open_app(database_path) else {
        return;
    };
    let _ = store.reconcile_session_processes();
    let Ok(records) = store.list_sessions(workspace_name) else {
        return;
    };

    let mut attached = 0usize;
    let mut sessions = active_sessions.borrow_mut();
    for record in records {
        if record.status != ProcessStatus::Running || sessions.contains(&record.id) {
            continue;
        }
        sessions.insert(record.id);
        attached += 1;
    }
    drop(sessions);

    if attached > 0 {
        append_text(
            transcript_buffer,
            &format!("[session resume] restored {attached} runtime-managed session(s).\n"),
        );
    }
}

fn provider_status_text(status: &archductor_core::mcp::McpStatus) -> String {
    let codex_servers = status.codex_user.len() + status.codex_project.len();
    let claude_servers = status.claude_user.len() + status.claude_project.len();
    let cursor_servers = status.cursor_user.len() + status.cursor_project.len();
    let codex_provider = status.codex_provider.as_deref().unwrap_or("auto");
    let claude_provider = status.claude_provider.as_deref().unwrap_or("auto");
    let codex_exec = if status.codex_executable_available {
        "available"
    } else {
        "missing"
    };
    let claude_exec = if status.claude_executable_available {
        "available"
    } else {
        "missing"
    };
    let cursor_exec = if status.cursor_executable_available {
        "available"
    } else {
        "missing"
    };
    let codex_auth = if status.codex_authenticated {
        "yes"
    } else {
        "no"
    };
    let claude_auth = if status.claude_authenticated {
        "yes"
    } else {
        "no"
    };
    let cursor_auth = if status.cursor_authenticated {
        "yes"
    } else {
        "no"
    };
    format!(
        "MCP configured: claude={claude_servers}, codex={codex_servers}, cursor={cursor_servers}. Providers: codex={codex_provider}/{codex_exec}/{codex_auth}, claude={claude_provider}/{claude_exec}/{claude_auth}, cursor=cli-{cursor_exec}/{cursor_auth}. Cursor MCP is managed by Cursor.",
    )
}

fn collect_session_harness_options(
    plan_mode: &CheckButton,
    fast_mode: &CheckButton,
    approval_mode: &ComboBoxText,
    reasoning_mode: &ComboBoxText,
    effort_mode: &ComboBoxText,
    codex_personality: &ComboBoxText,
    codex_goals: &Entry,
    codex_skills: &Entry,
) -> SessionHarnessOptions {
    SessionHarnessOptions {
        plan_mode: plan_mode.is_active(),
        fast_mode: fast_mode.is_active(),
        model: None,
        approval_mode: combo_active_value_or_none(approval_mode, Some("default")),
        reasoning_mode: combo_active_value_or_none(reasoning_mode, Some("default")),
        effort_mode: combo_active_value_or_none(effort_mode, Some("default")),
        codex_personality: combo_active_value_or_none(codex_personality, Some("default")),
        codex_goals: entry_value_or_none(codex_goals),
        codex_skills: entry_value_or_none(codex_skills),
    }
}

fn combo_active_value_or_none(combo: &ComboBoxText, omit: Option<&str>) -> Option<String> {
    combo.active_id().and_then(|value| {
        let value = value.to_string();
        let omitted = omit.filter(|omit| !omit.is_empty()).unwrap_or("");
        if value.is_empty() || value == omitted {
            None
        } else {
            Some(value)
        }
    })
}

fn entry_value_or_none(entry: &Entry) -> Option<String> {
    let value = entry.text().trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn session_harness_metadata_label(metadata: &Option<String>) -> String {
    if let Some(value) = metadata.as_ref() {
        let value = value.trim();
        if !value.is_empty() {
            return format!(" harness={value}");
        }
    }
    String::new()
}

fn session_summary_label(record: &ProcessRecord, attached: bool) -> String {
    let attached_label = if record.status == ProcessStatus::Running {
        if attached {
            "attached"
        } else {
            "detached"
        }
    } else {
        "saved"
    };
    format!(
        "#{} {} {} {}",
        record.id,
        session_kind_label(&record.command),
        record.status.as_str(),
        attached_label,
    )
}

fn session_kind_label(command: &str) -> &'static str {
    let executable = command.split_whitespace().next().unwrap_or("").trim();
    match PathBuf::from(executable)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
    {
        "codex" => "Codex",
        "claude" => "Claude",
        "cursor" => "Cursor",
        _ => "Shell",
    }
}

fn session_size_from_widget(width: i32, height: i32) -> (u16, u16) {
    let cols = (width.max(0) / 8).clamp(80, u16::MAX as i32) as u16;
    let rows = (height.max(0) / 20).clamp(20, u16::MAX as i32) as u16;
    (rows, cols)
}

fn session_runtime_state(
    record: &ProcessRecord,
    last_output: Option<Instant>,
    attached: bool,
) -> &'static str {
    match record.status {
        ProcessStatus::Running => {
            if !attached {
                return "detached";
            }
            if let Some(last_seen) = last_output {
                if last_seen.elapsed() > Duration::from_secs(2) {
                    "waiting"
                } else {
                    "working"
                }
            } else {
                "idle"
            }
        }
        ProcessStatus::Stopped => "done",
        ProcessStatus::Exited => match record.exit_code {
            Some(0) | None => "done",
            Some(_) => "errored",
        },
    }
}

fn append_text(buffer: &TextBuffer, text: &str) {
    if text.is_empty() {
        return;
    }
    let mut end = buffer.end_iter();
    buffer.insert(&mut end, &terminal_display_text(text));
    let full = buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string();
    let trimmed = trim_session_scrollback(&full);
    if trimmed != full {
        buffer.set_text(&trimmed);
    }
}

fn trim_session_scrollback(text: &str) -> String {
    let lines = text.lines().collect::<Vec<_>>();
    if lines.len() <= SESSION_SCROLLBACK_LINES {
        return text.to_owned();
    }
    let start = lines.len().saturating_sub(SESSION_SCROLLBACK_LINES);
    let mut trimmed = String::from("[session scrollback trimmed]\n");
    trimmed.push_str(&lines[start..].join("\n"));
    if !trimmed.ends_with('\n') {
        trimmed.push('\n');
    }
    trimmed
}

fn staged_review_prompt_text(prompt: &str) -> String {
    format!("\n[staged review prompt]\n{}\n", prompt.trim())
}

fn staged_review_status_text(prompt: Option<&str>) -> String {
    match prompt.map(str::trim).filter(|prompt| !prompt.is_empty()) {
        Some(prompt) => format!("Staged review prompt ready ({} chars).", prompt.len()),
        None => "No staged review prompt.".to_owned(),
    }
}

fn session_refresh_error_text(operation: &str, err: &anyhow::Error) -> String {
    format!("[session refresh] {operation} failed: {err:#}")
}

fn archcar_turn_completion_allows_queue_drain(status: Option<&str>) -> bool {
    !matches!(
        status.map(|status| status.trim().to_ascii_lowercase()),
        Some(status)
            if matches!(
                status.as_str(),
                "failed" | "error" | "interrupted" | "cancelled" | "canceled" | "deferred"
            )
    )
}

fn hold_queued_auto_drain(holds: &RefCell<HashSet<i64>>, thread_id: i64) {
    holds.borrow_mut().insert(thread_id);
}

fn queued_auto_drain_allowed(holds: &RefCell<HashSet<i64>>, thread_id: i64) -> bool {
    !holds.borrow().contains(&thread_id)
}

fn queued_chat_auto_drain_ready(
    current_kind: SessionKind,
    records: &[ProcessRecord],
    thread_id: i64,
    ready_cache: &RefCell<HashMap<i64, bool>>,
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
    pending_inputs: &RefCell<HashMap<i64, Vec<QueuedArchcarInput>>>,
    holds: &RefCell<HashSet<i64>>,
    working_threads: &RefCell<HashMap<i64, Instant>>,
) -> bool {
    managed_harness_for_kind(current_kind).is_some()
        && !active_generation_for_thread(records, working_threads, thread_id, current_kind)
        && queued_auto_drain_allowed(holds, thread_id)
        && !pending_inputs.borrow().contains_key(&thread_id)
        && !has_inflight_user_send_for_thread(inflight_actions, thread_id)
        && ready_running_session_for_thread(records, thread_id, current_kind, ready_cache).is_some()
}

fn release_queued_auto_drain_if_queue_empty(
    holds: &RefCell<HashSet<i64>>,
    thread_id: i64,
    queued_count: usize,
) {
    if queued_count == 0 {
        holds.borrow_mut().remove(&thread_id);
    }
}

fn record_queued_auto_drain_turn_completion(
    holds: &RefCell<HashSet<i64>>,
    thread_id: i64,
    status: Option<&str>,
) {
    if !archcar_turn_completion_allows_queue_drain(status) {
        hold_queued_auto_drain(holds, thread_id);
    }
}

fn is_codex_session_record(records: &[ProcessRecord], process_id: i64) -> bool {
    records
        .iter()
        .find(|record| record.id == process_id)
        .map(|record| session_kind_matches_record(record, SessionKind::Codex))
        .unwrap_or(false)
}

#[derive(Clone)]
struct QueuedArchcarInput {
    input: String,
    visible_input: Option<String>,
    kind: ArchcarInputKind,
    session_kind: SessionKind,
}

#[derive(Default)]
struct BridgeErrorUiState {
    last_message: Option<String>,
    suppressed_count: usize,
}

impl BridgeErrorUiState {
    fn record(&mut self, message: &str) -> Option<String> {
        if self.last_message.as_deref() == Some(message) {
            self.suppressed_count += 1;
            return None;
        }
        let visible = match self.suppressed_count {
            0 => message.to_owned(),
            count => format!("{message} ({count} repeated bridge errors suppressed)"),
        };
        self.last_message = Some(message.to_owned());
        self.suppressed_count = 0;
        Some(visible)
    }
}

#[derive(Debug, Clone)]
enum PendingArchcarAction {
    EnsureWorkspace {
        workspace: String,
        thread_id: Option<i64>,
        kind: SessionKind,
    },
    ControlSend {
        thread_id: i64,
        session_id: i64,
        command: String,
    },
    TurnInterrupt {
        thread_id: i64,
        session_id: i64,
    },
    ModelUpdate {
        thread_id: i64,
        session_id: i64,
        model: Option<String>,
    },
    EffortUpdate {
        thread_id: i64,
        session_id: i64,
        effort: Option<String>,
        session_kind: SessionKind,
    },
    UserSend {
        thread_id: i64,
        session_id: i64,
        input: String,
        visible_input: Option<String>,
        kind: ArchcarInputKind,
        delivery: ArchcarInputDelivery,
        checkpoint_id: Option<i64>,
        session_kind: SessionKind,
    },
}

fn any_running_archcar_codex_ready(
    records: &[ProcessRecord],
    ready_cache: &RefCell<HashMap<i64, bool>>,
) -> Option<i64> {
    records
        .iter()
        .find(|record| {
            record.status == ProcessStatus::Running
                && session_kind_matches_record(record, SessionKind::Codex)
                && ready_cache
                    .borrow()
                    .get(&record.id)
                    .copied()
                    .unwrap_or(false)
        })
        .map(|record| record.id)
}

fn flush_pending_archcar_inputs(
    bridge: &AsyncArchcarBridge,
    database_path: &Path,
    workspace: &str,
    pending_commands: &RefCell<HashMap<i64, Vec<String>>>,
    pending_inputs: &RefCell<HashMap<i64, Vec<QueuedArchcarInput>>>,
    ready_cache: &RefCell<HashMap<i64, bool>>,
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
    app_state: &AppState,
) -> bool {
    let thread_ids = pending_inputs.borrow().keys().copied().collect::<Vec<_>>();
    let mut flushed_any = false;
    for thread_id in thread_ids {
        let records = WorkspaceStore::open_app(database_path)
            .and_then(|store| store.list_thread_processes(thread_id))
            .unwrap_or_default();
        let session_kind = pending_inputs
            .borrow()
            .get(&thread_id)
            .and_then(|inputs| inputs.first())
            .map(|input| input.session_kind)
            .unwrap_or(SessionKind::Codex);
        let Some(harness) = managed_harness_for_kind(session_kind) else {
            continue;
        };
        let Some(session_id) =
            ready_queued_session_for_thread(&records, thread_id, harness.descriptor(), ready_cache)
        else {
            continue;
        };

        let pending_controls = flush_pending_commands_for_send(pending_commands, thread_id);
        for (index, control) in pending_controls.iter().enumerate() {
            if !queue_archcar_control_send(
                bridge,
                inflight_actions,
                thread_id,
                session_id,
                control.clone(),
            ) {
                requeue_pending_controls(pending_commands, thread_id, &pending_controls, index);
                warn!(
                    thread_id,
                    process_id = session_id,
                    "archcar control send failed; requeued pending controls"
                );
                return flushed_any;
            }
            debug!(
                thread_id,
                process_id = session_id,
                control = %control,
                "archcar control queued"
            );
        }

        let Some(queued_input) = pop_next_pending_archcar_input(pending_inputs, thread_id) else {
            continue;
        };
        let checkpoint_id = match create_turn_checkpoint_for_send(
            database_path,
            workspace,
            thread_id,
            Some(session_id),
            matches!(queued_input.kind, ArchcarInputKind::ReviewPrompt),
        ) {
            Ok(checkpoint_id) => Some(checkpoint_id),
            Err(err) => {
                warn!(
                    workspace = %workspace,
                    thread_id,
                    process_id = session_id,
                    error = %err,
                    "turn checkpoint creation failed before queued archcar input submitted"
                );
                None
            }
        };
        if !queue_archcar_user_send(
            bridge,
            inflight_actions,
            thread_id,
            session_id,
            queued_input.input.clone(),
            queued_input.visible_input.clone(),
            queued_input.kind.clone(),
            ArchcarInputDelivery::Auto,
            checkpoint_id,
            queued_input.session_kind,
        ) {
            if let Some(checkpoint_id) = checkpoint_id {
                discard_turn_checkpoint(database_path, workspace, checkpoint_id);
            }
            requeue_pending_archcar_input_front(pending_inputs, thread_id, queued_input);
            warn!(
                thread_id,
                process_id = session_id,
                "archcar queued input send failed; retained queued input"
            );
            return flushed_any;
        }
        debug!(
            thread_id,
            process_id = session_id,
            kind = ?queued_input.kind,
            chars = queued_input.input.len(),
            "archcar queued input submitted"
        );
        note_archcar_ready(&mut ready_cache.borrow_mut(), session_id, false);
        if matches!(queued_input.kind, ArchcarInputKind::ReviewPrompt) {
            app_state.set_staged_review_prompt(None);
        }
        flushed_any = true;
    }
    flushed_any
}

fn queue_archcar_control_send(
    bridge: &AsyncArchcarBridge,
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
    thread_id: i64,
    session_id: i64,
    command: String,
) -> bool {
    let token = bridge.send_input(
        session_id,
        command.clone(),
        None,
        ArchcarInputKind::ControlCommand,
    );
    if let Some(token) = token {
        inflight_actions.borrow_mut().insert(
            token,
            PendingArchcarAction::ControlSend {
                thread_id,
                session_id,
                command,
            },
        );
        true
    } else {
        false
    }
}

fn queue_archcar_turn_interrupt(
    bridge: &AsyncArchcarBridge,
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
    thread_id: i64,
    session_id: i64,
) -> bool {
    let token = bridge.interrupt_turn(session_id);
    if let Some(token) = token {
        inflight_actions.borrow_mut().insert(
            token,
            PendingArchcarAction::TurnInterrupt {
                thread_id,
                session_id,
            },
        );
        true
    } else {
        false
    }
}

fn queue_archcar_model_update(
    bridge: &AsyncArchcarBridge,
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
    thread_id: i64,
    session_id: i64,
    model: Option<String>,
) -> bool {
    let token = bridge.set_session_model(session_id, model.clone());
    if let Some(token) = token {
        inflight_actions.borrow_mut().insert(
            token,
            PendingArchcarAction::ModelUpdate {
                thread_id,
                session_id,
                model,
            },
        );
        true
    } else {
        false
    }
}

fn queue_archcar_effort_update(
    bridge: &AsyncArchcarBridge,
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
    thread_id: i64,
    session_id: i64,
    effort: Option<String>,
    session_kind: SessionKind,
) -> bool {
    let token = bridge.set_session_effort(session_id, effort.clone());
    if let Some(token) = token {
        inflight_actions.borrow_mut().insert(
            token,
            PendingArchcarAction::EffortUpdate {
                thread_id,
                session_id,
                effort,
                session_kind,
            },
        );
        true
    } else {
        false
    }
}

fn requeue_pending_controls(
    pending_commands: &RefCell<HashMap<i64, Vec<String>>>,
    thread_id: i64,
    controls: &[String],
    start_index: usize,
) {
    for control in controls[start_index..].iter().cloned() {
        queue_thread_command(pending_commands, thread_id, control);
    }
}

fn queue_archcar_user_send(
    bridge: &AsyncArchcarBridge,
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
    thread_id: i64,
    session_id: i64,
    input: String,
    visible_input: Option<String>,
    kind: ArchcarInputKind,
    delivery: ArchcarInputDelivery,
    checkpoint_id: Option<i64>,
    session_kind: SessionKind,
) -> bool {
    let token = match delivery {
        ArchcarInputDelivery::Auto => bridge.send_input(
            session_id,
            input.clone(),
            visible_input.clone(),
            kind.clone(),
        ),
        ArchcarInputDelivery::Immediate => bridge.send_input_immediately(
            session_id,
            input.clone(),
            visible_input.clone(),
            kind.clone(),
        ),
    };
    if let Some(token) = token {
        inflight_actions.borrow_mut().insert(
            token,
            PendingArchcarAction::UserSend {
                thread_id,
                session_id,
                input,
                visible_input,
                kind,
                delivery,
                checkpoint_id,
                session_kind,
            },
        );
        true
    } else {
        false
    }
}

fn create_turn_checkpoint_for_send(
    db_path: &Path,
    workspace: &str,
    thread_id: i64,
    session_id: Option<i64>,
    staged_review: bool,
) -> anyhow::Result<i64> {
    let prompt_kind = if staged_review { "review" } else { "user" };
    let checkpoint = WorkspaceStore::open_app(db_path)?.checkpoint_create_turn_start(
        workspace,
        thread_id,
        session_id,
        prompt_kind,
    )?;
    Ok(checkpoint.id)
}

fn discard_turn_checkpoint(db_path: &Path, workspace: &str, checkpoint_id: i64) {
    if let Err(err) = WorkspaceStore::open_app(db_path)
        .and_then(|store| store.checkpoint_delete(workspace, checkpoint_id))
    {
        warn!(
            workspace = %workspace,
            checkpoint_id,
            error = %err,
            "turn checkpoint cleanup failed after archcar input rejection"
        );
    }
}

fn update_working_indicator_for_archcar_event(
    event: &ArchcarEvent,
    records: &[ProcessRecord],
    session_threads: &RefCell<HashMap<i64, i64>>,
    selected_harness: SessionKind,
    selected_thread_id: Option<i64>,
    pending_inputs: &RefCell<HashMap<i64, Vec<QueuedArchcarInput>>>,
    working_threads: &RefCell<HashMap<i64, Instant>>,
) -> bool {
    match event {
        ArchcarEvent::SessionReady { thread_id, .. } => {
            if pending_inputs.borrow().contains_key(thread_id) {
                return false;
            }
            false
        }
        ArchcarEvent::TurnCompleted { thread_id, .. } => {
            clear_thread_working(working_threads, *thread_id)
        }
        ArchcarEvent::SessionExited { session_id, .. } => {
            let Some(thread_id) =
                codex_thread_id_for_session(*session_id, session_threads, records)
            else {
                return false;
            };
            clear_thread_working(working_threads, thread_id)
        }
        ArchcarEvent::SessionError {
            session_id,
            thread_id,
            ..
        } => {
            let Some(thread_id) = codex_thread_id_for_startup_error(
                *session_id,
                *thread_id,
                session_threads,
                records,
                selected_harness,
                selected_thread_id,
            ) else {
                return false;
            };
            clear_thread_working(working_threads, thread_id)
        }
        ArchcarEvent::SessionSpawnQueued { .. }
        | ArchcarEvent::SessionStarted { .. }
        | ArchcarEvent::SessionCapabilitiesChanged { .. }
        | ArchcarEvent::SessionScreenUpdated { .. }
        | ArchcarEvent::SessionMessagesUpdated { .. }
        | ArchcarEvent::ProviderInteractionRequested { .. }
        | ArchcarEvent::ProviderInteractionResolved { .. } => false,
    }
}

fn handle_archcar_event(
    event: &ArchcarEvent,
    records: &[ProcessRecord],
    ready_cache: &RefCell<HashMap<i64, bool>>,
    startup_states: &RefCell<HashMap<i64, CodexStartupState>>,
    session_threads: &RefCell<HashMap<i64, i64>>,
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
    app_state: &AppState,
    selected_harness: SessionKind,
    selected_thread_id: Option<i64>,
    codex_ready: &RefCell<bool>,
    update_composer_state: &dyn Fn(),
    queued_auto_drain_holds: &RefCell<HashSet<i64>>,
    toast_manager: &ToastManager,
) {
    match event {
        ArchcarEvent::SessionSpawnQueued { workspace, kind } => {
            info!(workspace = %workspace, harness = ?kind, "archcar session spawn queued");
            if matches!(kind, SessionKind::Codex) {
                set_codex_ready_state(codex_ready, update_composer_state, false);
            }
        }
        ArchcarEvent::SessionStarted {
            session_id,
            thread_id,
            workspace,
            kind,
            pid,
        } => {
            info!(
                workspace = %workspace,
                harness = ?kind,
                session_id,
                thread_id,
                pid,
                "archcar session started"
            );
            session_threads.borrow_mut().insert(*session_id, *thread_id);
            note_archcar_ready(&mut ready_cache.borrow_mut(), *session_id, false);
            if matches!(kind, SessionKind::Codex) {
                apply_codex_startup_signal(
                    &mut startup_states.borrow_mut(),
                    CodexStartupSignal::Loading {
                        thread_id: *thread_id,
                    },
                );
            }
        }
        ArchcarEvent::SessionReady {
            session_id,
            thread_id,
        } => {
            info!(session_id, thread_id, "archcar session ready");
            session_threads.borrow_mut().insert(*session_id, *thread_id);
            note_archcar_ready(&mut ready_cache.borrow_mut(), *session_id, true);
            apply_codex_startup_signal(
                &mut startup_states.borrow_mut(),
                CodexStartupSignal::Ready {
                    thread_id: *thread_id,
                },
            );
            mark_chat_startup_finished(app_state, *thread_id);
            set_codex_ready_state(codex_ready, update_composer_state, true);
        }
        ArchcarEvent::TurnCompleted {
            session_id,
            thread_id,
            status,
        } => {
            info!(session_id, thread_id, ?status, "archcar turn completed");
            session_threads.borrow_mut().insert(*session_id, *thread_id);
            clear_inflight_user_sends_for_thread(inflight_actions, *thread_id);
            record_queued_auto_drain_turn_completion(
                queued_auto_drain_holds,
                *thread_id,
                status.as_deref(),
            );
            let ready = archcar_turn_completion_allows_queue_drain(status.as_deref());
            note_archcar_ready(&mut ready_cache.borrow_mut(), *session_id, ready);
            if ready {
                apply_codex_startup_signal(
                    &mut startup_states.borrow_mut(),
                    CodexStartupSignal::Ready {
                        thread_id: *thread_id,
                    },
                );
                mark_chat_startup_finished(app_state, *thread_id);
            } else {
                apply_codex_startup_signal(
                    &mut startup_states.borrow_mut(),
                    CodexStartupSignal::Error {
                        thread_id: *thread_id,
                        message: format!(
                            "Agent turn {}.",
                            status.as_deref().unwrap_or("did not complete")
                        ),
                    },
                );
            }
            set_codex_ready_state(codex_ready, update_composer_state, ready);
        }
        ArchcarEvent::SessionCapabilitiesChanged {
            session_id,
            thread_id,
            capabilities,
        } => {
            trace!(
                session_id,
                thread_id,
                contract_version = capabilities.contract_version,
                required = capabilities.required.len(),
                optional = capabilities.optional.len(),
                observed_native = capabilities.observed_native.len(),
                "archcar session capabilities changed"
            );
            session_threads.borrow_mut().insert(*session_id, *thread_id);
        }
        ArchcarEvent::SessionScreenUpdated { session_id } => {
            trace!(session_id, "archcar session screen updated");
        }
        ArchcarEvent::SessionMessagesUpdated { thread_id } => {
            trace!(thread_id, "archcar session messages updated");
            clear_inflight_user_sends_for_thread(inflight_actions, *thread_id);
        }
        ArchcarEvent::ProviderInteractionRequested { interaction } => {
            trace!(
                id = %interaction.id,
                thread_id = interaction.thread_id,
                session_id = interaction.session_id,
                "archcar provider interaction requested"
            );
        }
        ArchcarEvent::ProviderInteractionResolved { interaction } => {
            trace!(
                id = %interaction.id,
                thread_id = interaction.thread_id,
                session_id = interaction.session_id,
                "archcar provider interaction resolved"
            );
        }
        ArchcarEvent::SessionExited {
            session_id,
            exit_code,
        } => {
            info!(session_id, ?exit_code, "archcar session exited");
            if let Some(thread_id) =
                codex_thread_id_for_session(*session_id, session_threads, records)
            {
                hold_queued_auto_drain(queued_auto_drain_holds, thread_id);
                clear_inflight_user_sends_for_thread(inflight_actions, thread_id);
                mark_chat_startup_finished(app_state, thread_id);
            }
            clear_archcar_ready(&mut ready_cache.borrow_mut(), *session_id);
            session_threads.borrow_mut().remove(session_id);
        }
        ArchcarEvent::SessionError {
            session_id,
            thread_id,
            message,
        } => {
            warn!(?session_id, ?thread_id, %message, "archcar session error");
            if let Some(session_id) = session_id {
                note_archcar_ready(&mut ready_cache.borrow_mut(), *session_id, false);
            }
            if let Some(thread_id) = codex_thread_id_for_startup_error(
                *session_id,
                *thread_id,
                session_threads,
                records,
                selected_harness,
                selected_thread_id,
            ) {
                hold_queued_auto_drain(queued_auto_drain_holds, thread_id);
                clear_inflight_user_sends_for_thread(inflight_actions, thread_id);
                mark_chat_startup_finished(app_state, thread_id);
                toast_manager.error(message.clone());
                apply_codex_startup_signal(
                    &mut startup_states.borrow_mut(),
                    CodexStartupSignal::Error {
                        thread_id,
                        message: message.clone(),
                    },
                );
            }
        }
    }
}

fn handle_archcar_response(
    response: AsyncArchcarResponse,
    database_path: &Path,
    workspace: &str,
    bridge: AsyncArchcarBridge,
    ready_cache: &RefCell<HashMap<i64, bool>>,
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
    pending_commands: &RefCell<HashMap<i64, Vec<String>>>,
    pending_inputs: &RefCell<HashMap<i64, Vec<QueuedArchcarInput>>>,
    queued_auto_drain_holds: &RefCell<HashSet<i64>>,
    startup_states: &RefCell<HashMap<i64, CodexStartupState>>,
    working_threads: &RefCell<HashMap<i64, Instant>>,
    codex_ready: &RefCell<bool>,
    update_composer_state: &dyn Fn(),
    toast_manager: &ToastManager,
) -> bool {
    let mut changed = false;
    let Some(action) = inflight_actions.borrow_mut().remove(&response.token) else {
        debug!(token = response.token, ?response.request, "archcar response had no tracked GTK action");
        return false;
    };

    match action {
        PendingArchcarAction::EnsureWorkspace {
            workspace,
            thread_id,
            kind: _,
        } => match response.result {
            Ok(
                success @ (ArchcarResponse::SessionSpawnQueued { .. }
                | ArchcarResponse::SessionSpawned { .. }
                | ArchcarResponse::Ack),
            ) => {
                debug!(%workspace, token = response.token, "archcar ensure accepted");
                apply_archcar_ensure_success(&success, &mut startup_states.borrow_mut(), thread_id);
                changed = true;
            }
            Ok(other) => {
                warn!(%workspace, token = response.token, ?other, "unexpected archcar ensure response");
                if let Some(thread_id) = thread_id {
                    hold_queued_auto_drain(queued_auto_drain_holds, thread_id);
                    let message = format!("Unexpected archcar ensure response: {other:?}");
                    toast_manager.error(message.clone());
                    apply_codex_startup_signal(
                        &mut startup_states.borrow_mut(),
                        CodexStartupSignal::Error { thread_id, message },
                    );
                    clear_thread_working(working_threads, thread_id);
                }
                changed = true;
            }
            Err(err) => {
                warn!(%workspace, token = response.token, error = %err, "archcar ensure failed");
                if let Some(thread_id) = thread_id {
                    hold_queued_auto_drain(queued_auto_drain_holds, thread_id);
                    toast_manager.error(err.clone());
                    apply_codex_startup_signal(
                        &mut startup_states.borrow_mut(),
                        CodexStartupSignal::Error {
                            thread_id,
                            message: err,
                        },
                    );
                    clear_thread_working(working_threads, thread_id);
                }
                changed = true;
            }
        },
        PendingArchcarAction::ControlSend {
            thread_id,
            session_id,
            command,
        } => match response.result {
            Ok(ArchcarResponse::Ack) => {
                debug!(thread_id, session_id, control = %command, "archcar control accepted");
            }
            Ok(other) => {
                warn!(thread_id, session_id, control = %command, ?other, "unexpected archcar control response");
                hold_queued_auto_drain(queued_auto_drain_holds, thread_id);
                queue_thread_command(pending_commands, thread_id, command);
                note_archcar_ready(&mut ready_cache.borrow_mut(), session_id, false);
                let message = format!("Unexpected archcar control response: {other:?}");
                toast_manager.error(message.clone());
                apply_codex_startup_signal(
                    &mut startup_states.borrow_mut(),
                    CodexStartupSignal::Error { thread_id, message },
                );
                set_codex_ready_state(codex_ready, update_composer_state, false);
                changed = true;
                request_archcar_ensure(
                    &bridge,
                    inflight_actions,
                    workspace.to_owned(),
                    Some(thread_id),
                    managed_harness_for_kind(SessionKind::Codex)
                        .expect("codex managed harness")
                        .descriptor(),
                );
            }
            Err(err) => {
                warn!(thread_id, session_id, control = %command, error = %err, "archcar control send failed");
                hold_queued_auto_drain(queued_auto_drain_holds, thread_id);
                queue_thread_command(pending_commands, thread_id, command);
                note_archcar_ready(&mut ready_cache.borrow_mut(), session_id, false);
                toast_manager.error(err.clone());
                apply_codex_startup_signal(
                    &mut startup_states.borrow_mut(),
                    CodexStartupSignal::Error {
                        thread_id,
                        message: err,
                    },
                );
                set_codex_ready_state(codex_ready, update_composer_state, false);
                changed = true;
                request_archcar_ensure(
                    &bridge,
                    inflight_actions,
                    workspace.to_owned(),
                    Some(thread_id),
                    managed_harness_for_kind(SessionKind::Codex)
                        .expect("codex managed harness")
                        .descriptor(),
                );
            }
        },
        PendingArchcarAction::TurnInterrupt {
            thread_id,
            session_id,
        } => match response.result {
            Ok(ArchcarResponse::Ack) => {
                debug!(thread_id, session_id, "archcar turn interrupt accepted");
                hold_queued_auto_drain(queued_auto_drain_holds, thread_id);
                note_archcar_ready(&mut ready_cache.borrow_mut(), session_id, false);
                set_codex_ready_state(codex_ready, update_composer_state, false);
            }
            Ok(other) => {
                warn!(
                    thread_id,
                    session_id,
                    ?other,
                    "unexpected archcar turn interrupt response"
                );
                hold_queued_auto_drain(queued_auto_drain_holds, thread_id);
                let message = format!("Unexpected archcar turn interrupt response: {other:?}");
                toast_manager.error(message.clone());
                apply_codex_startup_signal(
                    &mut startup_states.borrow_mut(),
                    CodexStartupSignal::Error { thread_id, message },
                );
                changed = true;
            }
            Err(err) => {
                warn!(thread_id, session_id, error = %err, "archcar turn interrupt failed");
                hold_queued_auto_drain(queued_auto_drain_holds, thread_id);
                toast_manager.error(err.clone());
                apply_codex_startup_signal(
                    &mut startup_states.borrow_mut(),
                    CodexStartupSignal::Error {
                        thread_id,
                        message: err,
                    },
                );
                changed = true;
            }
        },
        PendingArchcarAction::ModelUpdate {
            thread_id,
            session_id,
            model,
        } => match response.result {
            Ok(ArchcarResponse::Ack) => {
                debug!(
                    thread_id,
                    session_id,
                    ?model,
                    "archcar model update accepted"
                );
            }
            Ok(other) => {
                warn!(
                    thread_id,
                    session_id,
                    ?model,
                    ?other,
                    "unexpected archcar model update response"
                );
                hold_queued_auto_drain(queued_auto_drain_holds, thread_id);
                note_archcar_ready(&mut ready_cache.borrow_mut(), session_id, false);
                let message = format!("Unexpected archcar model update response: {other:?}");
                toast_manager.error(message.clone());
                apply_codex_startup_signal(
                    &mut startup_states.borrow_mut(),
                    CodexStartupSignal::Error { thread_id, message },
                );
                set_codex_ready_state(codex_ready, update_composer_state, false);
                changed = true;
                request_archcar_ensure(
                    &bridge,
                    inflight_actions,
                    workspace.to_owned(),
                    Some(thread_id),
                    managed_harness_for_kind(SessionKind::Codex)
                        .expect("codex managed harness")
                        .descriptor(),
                );
            }
            Err(err) => {
                warn!(thread_id, session_id, ?model, error = %err, "archcar model update failed");
                hold_queued_auto_drain(queued_auto_drain_holds, thread_id);
                note_archcar_ready(&mut ready_cache.borrow_mut(), session_id, false);
                toast_manager.error(err.clone());
                apply_codex_startup_signal(
                    &mut startup_states.borrow_mut(),
                    CodexStartupSignal::Error {
                        thread_id,
                        message: err,
                    },
                );
                set_codex_ready_state(codex_ready, update_composer_state, false);
                changed = true;
                request_archcar_ensure(
                    &bridge,
                    inflight_actions,
                    workspace.to_owned(),
                    Some(thread_id),
                    managed_harness_for_kind(SessionKind::Codex)
                        .expect("codex managed harness")
                        .descriptor(),
                );
            }
        },
        PendingArchcarAction::EffortUpdate {
            thread_id,
            session_id,
            effort,
            session_kind,
        } => match response.result {
            Ok(ArchcarResponse::Ack) => {
                debug!(
                    thread_id,
                    session_id,
                    ?effort,
                    "archcar effort update accepted"
                );
            }
            Ok(other) => {
                warn!(
                    thread_id,
                    session_id,
                    ?effort,
                    ?other,
                    "unexpected archcar effort update response"
                );
                hold_queued_auto_drain(queued_auto_drain_holds, thread_id);
                note_archcar_ready(&mut ready_cache.borrow_mut(), session_id, false);
                let message = format!("Unexpected archcar effort update response: {other:?}");
                toast_manager.error(message.clone());
                apply_codex_startup_signal(
                    &mut startup_states.borrow_mut(),
                    CodexStartupSignal::Error { thread_id, message },
                );
                set_codex_ready_state(codex_ready, update_composer_state, false);
                changed = true;
                request_archcar_ensure_for_kind(
                    &bridge,
                    inflight_actions,
                    workspace.to_owned(),
                    Some(thread_id),
                    session_kind,
                );
            }
            Err(err) => {
                warn!(thread_id, session_id, ?effort, error = %err, "archcar effort update failed");
                hold_queued_auto_drain(queued_auto_drain_holds, thread_id);
                note_archcar_ready(&mut ready_cache.borrow_mut(), session_id, false);
                toast_manager.error(err.clone());
                apply_codex_startup_signal(
                    &mut startup_states.borrow_mut(),
                    CodexStartupSignal::Error {
                        thread_id,
                        message: err,
                    },
                );
                set_codex_ready_state(codex_ready, update_composer_state, false);
                changed = true;
                request_archcar_ensure_for_kind(
                    &bridge,
                    inflight_actions,
                    workspace.to_owned(),
                    Some(thread_id),
                    session_kind,
                );
            }
        },
        PendingArchcarAction::UserSend {
            thread_id,
            session_id,
            input,
            visible_input,
            kind,
            delivery,
            checkpoint_id,
            session_kind,
        } => match response.result {
            Ok(ArchcarResponse::Ack) => {
                info!(
                    thread_id,
                    session_id,
                    kind = ?kind,
                    delivery = ?delivery,
                    chars = input.len(),
                    "archcar input accepted"
                );
                inflight_actions.borrow_mut().insert(
                    response.token,
                    PendingArchcarAction::UserSend {
                        thread_id,
                        session_id,
                        input,
                        visible_input,
                        kind,
                        delivery,
                        checkpoint_id,
                        session_kind,
                    },
                );
            }
            Ok(other) => {
                warn!(thread_id, session_id, kind = ?kind, delivery = ?delivery, ?other, "unexpected archcar input response");
                hold_queued_auto_drain(queued_auto_drain_holds, thread_id);
                if let Some(checkpoint_id) = checkpoint_id {
                    discard_turn_checkpoint(database_path, workspace, checkpoint_id);
                }
                queue_pending_archcar_input(
                    pending_inputs,
                    thread_id,
                    QueuedArchcarInput {
                        input,
                        visible_input,
                        kind,
                        session_kind,
                    },
                );
                note_archcar_ready(&mut ready_cache.borrow_mut(), session_id, false);
                let message = format!("Unexpected archcar input response: {other:?}");
                toast_manager.error(message.clone());
                apply_codex_startup_signal(
                    &mut startup_states.borrow_mut(),
                    CodexStartupSignal::Error { thread_id, message },
                );
                clear_thread_working(working_threads, thread_id);
                set_codex_ready_state(codex_ready, update_composer_state, false);
                changed = true;
                request_archcar_ensure_for_kind(
                    &bridge,
                    inflight_actions,
                    workspace.to_owned(),
                    Some(thread_id),
                    session_kind,
                );
            }
            Err(err) => {
                warn!(thread_id, session_id, kind = ?kind, delivery = ?delivery, error = %err, "archcar input send failed");
                hold_queued_auto_drain(queued_auto_drain_holds, thread_id);
                if let Some(checkpoint_id) = checkpoint_id {
                    discard_turn_checkpoint(database_path, workspace, checkpoint_id);
                }
                requeue_pending_archcar_input_front(
                    pending_inputs,
                    thread_id,
                    QueuedArchcarInput {
                        input,
                        visible_input,
                        kind,
                        session_kind,
                    },
                );
                note_archcar_ready(&mut ready_cache.borrow_mut(), session_id, false);
                toast_manager.error(err.clone());
                apply_codex_startup_signal(
                    &mut startup_states.borrow_mut(),
                    CodexStartupSignal::Error {
                        thread_id,
                        message: err,
                    },
                );
                clear_thread_working(working_threads, thread_id);
                set_codex_ready_state(codex_ready, update_composer_state, false);
                changed = true;
                request_archcar_ensure_for_kind(
                    &bridge,
                    inflight_actions,
                    workspace.to_owned(),
                    Some(thread_id),
                    session_kind,
                );
            }
        },
    }
    changed
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ArchcarRefreshIntent {
    chat_surface: bool,
    workspace_nav: bool,
    global_summary: bool,
}

impl ArchcarRefreshIntent {
    fn merge(&mut self, next: Self) {
        self.chat_surface |= next.chat_surface;
        self.workspace_nav |= next.workspace_nav;
        self.global_summary |= next.global_summary;
    }
}

fn archcar_message_refresh_intent(message: &AsyncArchcarMessage) -> ArchcarRefreshIntent {
    match message {
        AsyncArchcarMessage::Event(event) => match event {
            ArchcarEvent::SessionSpawnQueued { .. }
            | ArchcarEvent::SessionStarted { .. }
            | ArchcarEvent::TurnCompleted { .. }
            | ArchcarEvent::SessionExited { .. }
            | ArchcarEvent::SessionError { .. } => ArchcarRefreshIntent {
                chat_surface: true,
                workspace_nav: true,
                global_summary: true,
            },
            ArchcarEvent::SessionMessagesUpdated { .. } => ArchcarRefreshIntent {
                chat_surface: true,
                workspace_nav: true,
                global_summary: true,
            },
            ArchcarEvent::SessionReady { .. }
            | ArchcarEvent::SessionCapabilitiesChanged { .. }
            | ArchcarEvent::SessionScreenUpdated { .. }
            | ArchcarEvent::ProviderInteractionRequested { .. }
            | ArchcarEvent::ProviderInteractionResolved { .. } => ArchcarRefreshIntent {
                chat_surface: true,
                workspace_nav: false,
                global_summary: false,
            },
        },
        AsyncArchcarMessage::Response(_) | AsyncArchcarMessage::BridgeError { .. } => {
            ArchcarRefreshIntent {
                chat_surface: true,
                workspace_nav: false,
                global_summary: false,
            }
        }
    }
}

fn request_archcar_ensure(
    bridge: &AsyncArchcarBridge,
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
    workspace: String,
    thread_id: Option<i64>,
    harness: &'static HarnessDescriptor,
) -> bool {
    let token = if let Some(thread_id) = thread_id {
        bridge.ensure_thread_session(workspace.clone(), thread_id, harness.kind)
    } else {
        bridge.ensure_default_session(workspace.clone(), harness.kind)
    };
    if let Some(token) = token {
        inflight_actions.borrow_mut().insert(
            token,
            PendingArchcarAction::EnsureWorkspace {
                workspace,
                thread_id,
                kind: harness.kind,
            },
        );
        true
    } else {
        false
    }
}

fn request_archcar_ensure_for_kind(
    bridge: &AsyncArchcarBridge,
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
    workspace: String,
    thread_id: Option<i64>,
    kind: SessionKind,
) -> bool {
    let Some(harness) = managed_harness_for_kind(kind) else {
        return false;
    };
    request_archcar_ensure(
        bridge,
        inflight_actions,
        workspace,
        thread_id,
        harness.descriptor(),
    )
}

fn request_archcar_spawn_session(
    bridge: &AsyncArchcarBridge,
    workspace: String,
    kind: SessionKind,
) -> Option<u64> {
    bridge.spawn_session(workspace, kind)
}

fn has_pending_archcar_ensure_for_thread(
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
    thread_id: i64,
) -> bool {
    inflight_actions.borrow().values().any(|action| {
        matches!(
            action,
            PendingArchcarAction::EnsureWorkspace {
                thread_id: Some(pending_thread_id),
                ..
            } if *pending_thread_id == thread_id
        )
    })
}

fn has_inflight_user_send_for_thread(
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
    thread_id: i64,
) -> bool {
    inflight_actions.borrow().values().any(|action| {
        matches!(
            action,
            PendingArchcarAction::UserSend {
                thread_id: action_thread_id,
                ..
            } if *action_thread_id == thread_id
        )
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EagerChatAgentStartOutcome {
    NotManaged,
    AlreadyReady,
    AlreadyActive,
    Pending,
    Requested,
    RequestUnavailable,
}

fn eager_chat_agent_start<F>(
    app_state: &AppState,
    records: &[ProcessRecord],
    ready_cache: &RefCell<HashMap<i64, bool>>,
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
    startup_states: &RefCell<HashMap<i64, CodexStartupState>>,
    working_threads: &RefCell<HashMap<i64, Instant>>,
    codex_ready: &RefCell<bool>,
    update_composer_state: &dyn Fn(),
    workspace: String,
    thread_id: i64,
    kind: SessionKind,
    request_ensure: F,
) -> EagerChatAgentStartOutcome
where
    F: FnOnce(String, i64, &'static HarnessDescriptor) -> bool,
{
    let Some(harness) = managed_harness_for_kind(kind).map(|harness| harness.descriptor()) else {
        return EagerChatAgentStartOutcome::NotManaged;
    };

    if managed_thread_ready_for_ui(
        kind,
        thread_id,
        records,
        ready_cache,
        startup_states.borrow().get(&thread_id),
    ) {
        app_state.mark_chat_phase(ChatUiTarget::Thread(thread_id), ChatUiPhase::Ready);
        set_codex_ready_state(codex_ready, update_composer_state, true);
        return EagerChatAgentStartOutcome::AlreadyReady;
    }

    if active_generation_for_thread(records, working_threads, thread_id, kind) {
        return EagerChatAgentStartOutcome::AlreadyActive;
    }

    app_state.mark_chat_phase(
        ChatUiTarget::Thread(thread_id),
        ChatUiPhase::StartingAgent { provider: kind },
    );
    apply_codex_startup_signal(
        &mut startup_states.borrow_mut(),
        CodexStartupSignal::Loading { thread_id },
    );
    set_codex_ready_state(codex_ready, update_composer_state, false);

    if has_pending_archcar_ensure_for_thread(inflight_actions, thread_id) {
        return EagerChatAgentStartOutcome::Pending;
    }

    if request_ensure(workspace, thread_id, harness) {
        EagerChatAgentStartOutcome::Requested
    } else {
        apply_codex_startup_signal(
            &mut startup_states.borrow_mut(),
            CodexStartupSignal::Error {
                thread_id,
                message: "Request channel is closed. Reopen the workspace or restart the app."
                    .to_owned(),
            },
        );
        app_state.mark_chat_phase(ChatUiTarget::Thread(thread_id), ChatUiPhase::Ready);
        update_composer_state();
        EagerChatAgentStartOutcome::RequestUnavailable
    }
}

fn apply_archcar_ensure_success(
    response: &ArchcarResponse,
    startup_states: &mut HashMap<i64, CodexStartupState>,
    requested_thread_id: Option<i64>,
) {
    match response {
        ArchcarResponse::SessionSpawnQueued { .. } | ArchcarResponse::Ack => {
            if let Some(thread_id) = requested_thread_id {
                apply_codex_startup_signal(
                    startup_states,
                    CodexStartupSignal::Loading { thread_id },
                );
            }
        }
        ArchcarResponse::SessionSpawned { thread_id, .. } => {
            apply_codex_startup_signal(
                startup_states,
                CodexStartupSignal::Loading {
                    thread_id: *thread_id,
                },
            );
        }
        _ => {}
    }
}

fn set_codex_ready_state(
    codex_ready: &RefCell<bool>,
    update_composer_state: &dyn Fn(),
    ready: bool,
) {
    if *codex_ready.borrow() == ready {
        return;
    }
    *codex_ready.borrow_mut() = ready;
    update_composer_state();
}

fn session_history_bootstrap_markdown(events: &[SessionTranscriptEvent]) -> Option<String> {
    let history = events
        .iter()
        .filter_map(|event| match event.role {
            SessionTranscriptRole::User
            | SessionTranscriptRole::ReviewPrompt
            | SessionTranscriptRole::Agent => Some((event.role, event.body.trim())),
            SessionTranscriptRole::System
            | SessionTranscriptRole::Tool
            | SessionTranscriptRole::Skill
            | SessionTranscriptRole::Harness => None,
        })
        .filter(|(_, body)| !body.is_empty())
        .collect::<Vec<_>>();

    if history.is_empty() {
        return None;
    }

    let mut markdown = String::from(
        "Conversation context from previous agent session.\n\
Do not answer this message. Use it only for continuity and wait for the next real user message.\n\n\
# Prior Conversation\n\n",
    );
    for (role, body) in history {
        markdown.push_str(match role {
            SessionTranscriptRole::User => "## User\n",
            SessionTranscriptRole::ReviewPrompt => "## Review Prompt\n",
            SessionTranscriptRole::Agent => "## Agent\n",
            SessionTranscriptRole::System
            | SessionTranscriptRole::Tool
            | SessionTranscriptRole::Skill
            | SessionTranscriptRole::Harness => continue,
        });
        markdown.push_str(body);
        markdown.push_str("\n\n");
    }
    Some(markdown.trim_end().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archcar_async::AsyncArchcarRequestKind;
    use crate::state::{AppPage, WorkspaceTab};
    use archductor_core::doctor::SetupCheck;
    use archductor_core::paths::AppPaths;
    use archductor_core::workspace::ProcessKind;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn provider_interaction_fixture(kind: ProviderInteractionKind) -> ProviderInteractionRecord {
        ProviderInteractionRecord {
            id: "interaction-1".to_owned(),
            provider_key: "claude".to_owned(),
            workspace: "berlin".to_owned(),
            thread_id: 42,
            session_id: 7,
            native_session_id: Some("claude-session-1".to_owned()),
            native_id: "toolu-1".to_owned(),
            kind,
            title: "Need input".to_owned(),
            detail: "Pick a scope".to_owned(),
            choices: vec!["yes".to_owned(), "no".to_owned()],
            native_request: serde_json::json!({"tool": "AskUserQuestion"}),
            request_fingerprint: "fingerprint".to_owned(),
            status: archductor_core::provider_interactions::ProviderInteractionStatus::Pending,
            resolution: None,
            native_response: None,
            error: None,
            created_at: "1".to_owned(),
            resolved_at: None,
            consumed_at: None,
        }
    }

    fn session_record(
        id: i64,
        command: &str,
        status: ProcessStatus,
        metadata: Option<&str>,
    ) -> ProcessRecord {
        ProcessRecord {
            id,
            workspace_id: 42,
            chat_thread_id: None,
            kind: ProcessKind::Session,
            command: command.to_owned(),
            pid: 1234,
            log_path: PathBuf::from("/tmp/session.log"),
            status,
            started_at: "2026-06-20T12:00:00Z".to_owned(),
            exit_code: None,
            ended_at: None,
            session_harness_metadata: metadata.map(str::to_owned),
            session_resume_id: None,
        }
    }

    fn process_record_with_thread(
        id: i64,
        status: ProcessStatus,
        thread_id: Option<i64>,
        command: &str,
    ) -> ProcessRecord {
        let mut record = session_record(id, command, status, None);
        record.chat_thread_id = thread_id;
        record
    }

    fn provider_event_record(
        kind: ProviderEventKind,
        phase: ProviderEventPhase,
    ) -> ProviderEventRecord {
        ProviderEventRecord {
            id: 1,
            identity_key: "codex:thread-7:tool-1:completed".to_owned(),
            provider: "codex".to_owned(),
            provider_event_id: Some("evt-1".to_owned()),
            provider_item_id: Some("tool-1".to_owned()),
            provider_thread_id: Some("thread-7".to_owned()),
            provider_turn_id: None,
            parent_provider_item_id: None,
            parent_provider_thread_id: None,
            workspace_id: Some(1),
            chat_thread_id: Some(7),
            process_id: Some(9),
            phase,
            kind,
            provider_subtype: Some("tool_result".to_owned()),
            provider_sequence: Some(1),
            received_sequence: 3,
            timeline_seq: Some(3),
            occurred_at_ms: 42,
            normalized_payload: serde_json::json!({
                "title": "Bash",
                "body": "cargo test passed"
            }),
            raw_json: serde_json::json!({
                "type": "tool_result",
                "tool": "Bash"
            }),
            schema_version: 1,
            adapter_version: "test".to_owned(),
            created_at: "1".to_owned(),
            updated_at: "1".to_owned(),
        }
    }

    fn chat_message_record(id: i64, role: &str, content: &str, source: &str) -> ChatMessageRecord {
        ChatMessageRecord {
            id,
            thread_id: 7,
            role: role.to_owned(),
            content: content.to_owned(),
            source: source.to_owned(),
            timeline_seq: Some(id),
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
        }
    }

    #[test]
    fn harness_selection_updates_state_before_switch_callback() {
        let selected_harness = RefCell::new(SessionKind::Codex);
        let reasoning_mode = RefCell::new(None);
        let observed = Rc::new(RefCell::new(None));
        let observed_for_switch = observed.clone();
        let switch: Rc<dyn Fn(SessionKind)> = Rc::new(move |kind| {
            *observed_for_switch.borrow_mut() = Some(kind);
        });

        select_harness_and_dispatch(
            &selected_harness,
            &reasoning_mode,
            SessionKind::Claude,
            Some(&switch as &Rc<dyn Fn(SessionKind)>),
            None,
            None,
        );

        assert_eq!(*selected_harness.borrow(), SessionKind::Claude);
        assert_eq!(reasoning_mode.borrow().as_deref(), Some("high"));
        assert_eq!(*observed.borrow(), Some(SessionKind::Claude));
    }

    #[test]
    fn selected_harness_snapshot_drops_borrow_before_reentrant_switch() {
        let selected_harness = RefCell::new(SessionKind::Codex);
        let initial_kind = selected_harness_snapshot(&selected_harness);

        let switch = |kind| {
            *selected_harness.borrow_mut() = kind;
        };
        switch(initial_kind);

        assert_eq!(*selected_harness.borrow(), SessionKind::Codex);
    }

    #[test]
    fn same_provider_model_selection_does_not_switch_chat_harness() {
        let current = ProviderModelChoice {
            provider: "codex".to_owned(),
            model: Some("gpt-5.6-luna".to_owned()),
        };

        assert_eq!(
            model_selection_route(SessionKind::Codex, &current),
            ModelSelectionRoute::SameProvider
        );
    }

    #[test]
    fn cross_provider_model_selection_opens_provider_chat() {
        let current = ProviderModelChoice {
            provider: "claude".to_owned(),
            model: Some("claude-fable-5".to_owned()),
        };

        assert_eq!(
            model_selection_route(SessionKind::Codex, &current),
            ModelSelectionRoute::CrossProvider(SessionKind::Claude)
        );
    }

    #[test]
    fn model_switch_attachment_keeps_only_user_and_agent_messages() {
        let messages = vec![
            ModelSwitchContextMessage {
                role: "user".to_owned(),
                content: "Fix auth".to_owned(),
            },
            ModelSwitchContextMessage {
                role: "agent".to_owned(),
                content: "I found the callback bug.".to_owned(),
            },
        ];

        let attachment = model_switch_context_attachment("codex", "claude", &messages).unwrap();

        assert!(attachment.contains("Attached prior chat context"));
        assert!(attachment.contains("from Codex to Claude"));
        assert!(attachment.contains("## User"));
        assert!(attachment.contains("Fix auth"));
        assert!(attachment.contains("## Agent"));
        assert!(attachment.contains("I found the callback bug."));
        assert!(!attachment.contains("/model gpt-5.6-sol"));
        assert!(!attachment.contains("control_command"));
    }

    #[test]
    fn model_switch_attachment_is_bounded_to_recent_messages() {
        let messages = vec![
            ModelSwitchContextMessage {
                role: "user".to_owned(),
                content: "old".repeat(MODEL_SWITCH_CONTEXT_MAX_BYTES),
            },
            ModelSwitchContextMessage {
                role: "agent".to_owned(),
                content: "recent answer".to_owned(),
            },
        ];

        let attachment = model_switch_context_attachment("codex", "claude", &messages).unwrap();

        assert!(attachment.contains("[Older transcript omitted]"));
        assert!(attachment.contains("recent answer"));
        assert!(!attachment.contains(&"old".repeat(512)));
    }

    #[test]
    fn model_switch_attachment_is_pending_only_until_first_real_message() {
        let pending = vec![ChatMessageRecord {
            source: "model_switch_context".to_owned(),
            role: "system".to_owned(),
            content: "Attached context".to_owned(),
            ..chat_message_record(1, "system", "Attached context", "model_switch_context")
        }];
        assert_eq!(
            pending_model_switch_context_attachment(&pending),
            Some("Attached context")
        );

        let consumed = vec![
            chat_message_record(1, "system", "Attached context", "model_switch_context"),
            chat_message_record(2, "assistant", "Continue", "provider_event"),
        ];
        assert_eq!(pending_model_switch_context_attachment(&consumed), None);
    }

    #[test]
    fn supported_chat_session_kinds_exclude_cursor_and_shell() {
        assert_eq!(
            supported_chat_session_kinds(),
            &[SessionKind::Codex, SessionKind::Claude]
        );
    }

    #[test]
    fn provider_model_choices_show_only_ready_launchable_code_providers() {
        let readiness = SetupReadiness {
            gh: SetupCheck::ready("ready"),
            codex: SetupCheck::ready("ready"),
            claude: SetupCheck::missing("missing"),
            opencode: SetupCheck::ready("ready"),
        };

        let choices = provider_model_choices(&readiness, SessionKind::Codex);

        assert_eq!(
            choices,
            vec![
                ProviderModelChoice {
                    provider: "codex".to_owned(),
                    model: Some("gpt-5.6-sol".to_owned()),
                },
                ProviderModelChoice {
                    provider: "codex".to_owned(),
                    model: Some("gpt-5.6-terra".to_owned()),
                },
                ProviderModelChoice {
                    provider: "codex".to_owned(),
                    model: Some("gpt-5.6-luna".to_owned()),
                },
            ]
        );
    }

    #[test]
    fn provider_model_choices_group_ready_launchable_providers() {
        let readiness = SetupReadiness {
            gh: SetupCheck::ready("ready"),
            codex: SetupCheck::ready("ready"),
            claude: SetupCheck::ready("ready"),
            opencode: SetupCheck::ready("ready"),
        };

        let choices = provider_model_choices(&readiness, SessionKind::Codex);

        assert_eq!(
            choices
                .iter()
                .map(|choice| (choice.provider.as_str(), choice.model.as_deref()))
                .collect::<Vec<_>>(),
            vec![
                ("codex", Some("gpt-5.6-sol")),
                ("codex", Some("gpt-5.6-terra")),
                ("codex", Some("gpt-5.6-luna")),
                ("claude", Some("claude-fable-5")),
                ("claude", Some("claude-opus-4-8")),
                ("claude", Some("claude-sonnet-5")),
                ("claude", Some("claude-haiku-4-5-20251001")),
            ]
        );
    }

    #[test]
    fn provider_model_choices_do_not_include_synthetic_or_stale_models() {
        let readiness = SetupReadiness {
            gh: SetupCheck::ready("ready"),
            codex: SetupCheck::ready("ready"),
            claude: SetupCheck::ready("ready"),
            opencode: SetupCheck::ready("ready"),
        };

        let choices = provider_model_choices(&readiness, SessionKind::Codex);
        let models = choices
            .iter()
            .filter_map(|choice| choice.model.as_deref())
            .collect::<Vec<_>>();

        assert!(choices.iter().all(|choice| choice.model.is_some()));
        assert!(!models.contains(&"gpt-5"));
        assert!(!models.contains(&"gpt-5-mini"));
    }

    #[test]
    fn editor_choices_use_resolvable_icons() {
        for choice in detected_editor_choices() {
            assert_ne!(resolve_icon_name(choice.icon), "");
        }
    }

    #[test]
    fn session_history_bootstrap_markdown_summarizes_user_and_agent_turns() {
        let transcript = "\
[session started] #11 kind=Codex pid=1234
[user input memphis#11]
open the project
[session finished] #11
I opened the project and found the auth bug.
[user input memphis#11]
fix it
";

        let events = parse_session_transcript_events(transcript);
        let markdown = session_history_bootstrap_markdown(&events).unwrap();

        assert!(markdown.contains("Conversation context from previous agent session"));
        assert!(markdown.contains("Do not answer this message"));
        assert!(markdown.contains("## User"));
        assert!(markdown.contains("open the project"));
        assert!(markdown.contains("## Agent"));
        assert!(markdown.contains("I opened the project and found the auth bug."));
        assert!(markdown.contains("fix it"));
    }

    #[test]
    fn preferred_session_for_kind_prefers_matching_running_sessions() {
        let records = vec![
            session_record(1, "/opt/bin/claude", ProcessStatus::Running, None),
            session_record(2, "/opt/bin/codex", ProcessStatus::Exited, None),
            session_record(3, "/opt/bin/codex", ProcessStatus::Running, None),
        ];

        assert_eq!(
            preferred_session_for_kind(&records, None, SessionKind::Codex),
            Some(3)
        );
        assert_eq!(
            preferred_session_for_kind(&records, Some(2), SessionKind::Codex),
            Some(3)
        );
        assert_eq!(
            preferred_session_for_kind(&records, Some(3), SessionKind::Codex),
            Some(3)
        );
        assert_eq!(
            preferred_session_for_kind(&records, None, SessionKind::Claude),
            Some(1)
        );
    }

    #[test]
    fn current_live_session_id_requires_running_attached_session() {
        let records = vec![
            session_record(1, "/opt/bin/codex", ProcessStatus::Running, None),
            session_record(2, "/opt/bin/codex", ProcessStatus::Exited, None),
        ];
        let mut active_sessions = HashSet::from([1]);

        assert_eq!(
            current_live_session_id(&records, Some(1), SessionKind::Codex, &active_sessions),
            Some(1)
        );
        assert_eq!(
            current_live_session_id(&records, Some(2), SessionKind::Codex, &active_sessions),
            Some(1)
        );
        active_sessions.clear();
        assert_eq!(
            current_live_session_id(&records, Some(1), SessionKind::Codex, &active_sessions),
            None
        );
    }

    #[test]
    fn codex_sessions_are_not_spawned_by_gtk_session_surface() {
        let source = include_str!("session_surface.rs");

        let legacy_bootstrap = concat!(
            "if matches!(kind, SessionKind::Codex) {\n",
            "                if let Some(bootstrap)"
        );
        assert!(
            !source.contains(legacy_bootstrap),
            "legacy agent buttons must route Codex launch through archcar"
        );
        assert!(
            source.contains("Codex launch requested through archcar"),
            "legacy Codex launch button should route through archcar"
        );
    }

    #[test]
    fn codex_sessions_are_not_reattached_or_polled_by_gtk_session_surface() {
        let source = include_str!("session_surface.rs");
        let codex_record_poll = concat!(
            "is_codex_session_record(&record_state_for_poll.borrow(), ",
            "*process_id)"
        );
        let codex_screen_snapshot = concat!("format_codex_", "screen_snapshot");

        assert!(
            !source.contains(codex_record_poll),
            "GTK poll loop must not parse Codex screens directly"
        );
        assert!(
            !source.contains(codex_screen_snapshot),
            "GTK chat surface must not persist Codex screen snapshots from a local PTY"
        );
    }

    #[test]
    fn running_codex_stop_routes_through_archcar() {
        let records = vec![
            session_record(1, "/opt/bin/codex", ProcessStatus::Running, None),
            session_record(2, "/bin/bash", ProcessStatus::Running, None),
        ];

        assert_eq!(
            session_stop_route(&records, 1),
            Some(SessionStopRoute::Archcar)
        );
        assert_eq!(
            session_stop_route(&records, 2),
            Some(SessionStopRoute::Archcar)
        );
    }

    #[test]
    fn saved_codex_stop_stays_local_state_only() {
        let records = vec![session_record(
            1,
            "/opt/bin/codex",
            ProcessStatus::Exited,
            None,
        )];

        assert_eq!(
            session_stop_route(&records, 1),
            Some(SessionStopRoute::Local)
        );
        assert_eq!(session_stop_route(&records, 99), None);
    }

    #[test]
    fn pending_control_commands_flush_before_user_message() {
        let pending = RefCell::new(HashMap::<i64, Vec<String>>::new());
        queue_thread_command(&pending, 7, "/model gpt-5.6-sol".to_owned());
        queue_thread_command(&pending, 7, "/thinking high".to_owned());

        let flushed = flush_pending_commands_for_send(&pending, 7);

        assert_eq!(
            flushed,
            vec!["/model gpt-5.6-sol".to_owned(), "/thinking high".to_owned()]
        );
        assert!(flush_pending_commands_for_send(&pending, 7).is_empty());
    }

    #[test]
    fn unsupported_live_controls_are_filtered_out_of_toolbar() {
        let controls = visible_live_controls_for_provider("codex");

        assert!(controls.contains(&"model".to_owned()));
        assert!(controls.contains(&"thinking".to_owned()));
        assert!(!controls.contains(&"interrupt".to_owned()));
        assert!(!controls.contains(&"continue".to_owned()));
        assert!(!controls.contains(&"retry".to_owned()));
        assert!(!controls.contains(&"restart".to_owned()));
        assert!(controls.contains(&"goal".to_owned()));
        assert!(!controls.contains(&"attach".to_owned()));
        assert!(!visible_live_controls_for_provider("claude").contains(&"interrupt".to_owned()));
    }

    #[test]
    fn claude_live_controls_come_from_managed_harness_descriptor() {
        let claude = managed_harness_for_kind(SessionKind::Claude).unwrap();
        let controls = visible_live_controls_for_harness(claude.descriptor());

        assert!(controls.contains(&"model".to_owned()));
        assert!(controls.contains(&"thinking".to_owned()));
        assert!(!controls.contains(&"goal".to_owned()));
        assert!(!controls.contains(&"interrupt".to_owned()));
    }

    #[test]
    fn harness_capabilities_gate_goal_control_by_descriptor_support() {
        let codex = managed_harness_for_kind(SessionKind::Codex).unwrap();
        let claude = managed_harness_for_kind(SessionKind::Claude).unwrap();
        let codex_controls = visible_live_controls_for_harness(codex.descriptor());
        let claude_controls = visible_live_controls_for_harness(claude.descriptor());

        for control in ["provider", "model", "thinking"] {
            assert!(codex_controls.contains(&control.to_owned()));
            assert!(claude_controls.contains(&control.to_owned()));
        }
        assert!(codex_controls.contains(&"goal".to_owned()));
        assert!(!claude_controls.contains(&"goal".to_owned()));
    }

    #[test]
    fn retry_agent_prompt_is_actionable() {
        assert!(retry_agent_prompt().contains("Retry"));
        assert!(retry_agent_prompt().contains("nothing is retryable"));
    }

    #[test]
    fn composer_action_prefers_active_session_actions() {
        assert_eq!(
            composer_action_for_state(false, true, false, 0),
            ComposerAction::Interrupt
        );
        assert_eq!(
            composer_action_for_state(true, true, false, 0),
            ComposerAction::Queue
        );
        assert_eq!(
            composer_action_for_state(false, false, true, 0),
            ComposerAction::Retry
        );
        assert_eq!(
            composer_action_for_state(true, false, true, 0),
            ComposerAction::Send
        );
        assert_eq!(
            composer_action_for_state(false, false, true, 2),
            ComposerAction::SendQueued
        );
        assert_eq!(
            composer_action_for_state(false, false, false, 0),
            ComposerAction::Disabled
        );
    }

    #[test]
    fn composer_interrupt_uses_square_stop_icon() {
        let (icon, tooltip, sensitive) =
            composer_send_button_presentation(ComposerAction::Interrupt);

        assert_eq!(icon, "media-playback-stop-symbolic");
        assert_eq!(tooltip, "Interrupt active turn");
        assert!(sensitive);
    }

    #[test]
    fn composer_editing_queued_message_presents_save_action() {
        let (icon, tooltip, sensitive) =
            composer_send_button_presentation(ComposerAction::SaveQueuedEdit);

        assert_eq!(icon, "send-symbolic");
        assert_eq!(tooltip, "Save queued message");
        assert!(sensitive);
    }

    #[test]
    fn composer_ctrl_enter_delivers_immediately_instead_of_queueing() {
        assert_eq!(
            composer_submit_intent_for_modifiers(gtk::gdk::ModifierType::empty()),
            ComposerSubmitIntent::Default
        );
        assert_eq!(
            composer_submit_intent_for_modifiers(gtk::gdk::ModifierType::CONTROL_MASK),
            ComposerSubmitIntent::Immediate
        );
        assert_eq!(
            composer_action_for_submit_intent(
                ComposerAction::Queue,
                ComposerSubmitIntent::Default,
                true,
                SessionKind::Codex,
                false,
            ),
            ComposerAction::Queue
        );
        assert_eq!(
            composer_action_for_submit_intent(
                ComposerAction::Queue,
                ComposerSubmitIntent::Immediate,
                true,
                SessionKind::Codex,
                false,
            ),
            ComposerAction::Send
        );
        assert_eq!(
            composer_action_for_submit_intent(
                ComposerAction::SendQueued,
                ComposerSubmitIntent::Immediate,
                true,
                SessionKind::Codex,
                false,
            ),
            ComposerAction::Send
        );
        assert_eq!(
            composer_action_for_submit_intent(
                ComposerAction::SendQueued,
                ComposerSubmitIntent::Immediate,
                false,
                SessionKind::Codex,
                false,
            ),
            ComposerAction::SendQueued
        );
        assert_eq!(
            composer_action_for_submit_intent(
                ComposerAction::Queue,
                ComposerSubmitIntent::Immediate,
                true,
                SessionKind::Claude,
                false,
            ),
            ComposerAction::Send
        );
    }

    #[test]
    fn composer_immediate_send_does_not_bypass_starting_agent() {
        assert_eq!(
            composer_action_for_submit_intent(
                ComposerAction::Queue,
                ComposerSubmitIntent::Immediate,
                true,
                SessionKind::Codex,
                true,
            ),
            ComposerAction::Queue
        );
        assert_eq!(
            composer_action_for_submit_intent(
                ComposerAction::SendQueued,
                ComposerSubmitIntent::Immediate,
                true,
                SessionKind::Codex,
                true,
            ),
            ComposerAction::SendQueued
        );
        assert_eq!(
            composer_action_for_submit_intent(
                ComposerAction::Queue,
                ComposerSubmitIntent::Immediate,
                true,
                SessionKind::Claude,
                true,
            ),
            ComposerAction::Queue
        );
    }

    #[test]
    fn composer_send_button_uses_immediate_submit_intent() {
        assert_eq!(
            composer_send_button_submit_intent(),
            ComposerSubmitIntent::Immediate
        );
    }

    #[test]
    fn unchanged_queue_overlay_does_not_rebuild_during_chat_refresh() {
        let signature = QueueOverlaySignature {
            thread_id: Some(7),
            editing: false,
            items: vec![QueuedComposerItem {
                index: 0,
                preview: "queued message".to_owned(),
                actions: vec![QueuedComposerAction::SendImmediately],
            }],
        };

        assert!(queue_overlay_requires_rebuild(None, &signature));
        assert!(!queue_overlay_requires_rebuild(
            Some(&signature),
            &signature
        ));

        let changed = QueueOverlaySignature {
            thread_id: Some(7),
            editing: false,
            items: Vec::new(),
        };
        assert!(queue_overlay_requires_rebuild(Some(&signature), &changed));

        let edit_changed = QueueOverlaySignature {
            thread_id: Some(7),
            editing: true,
            items: signature.items.clone(),
        };
        assert!(queue_overlay_requires_rebuild(
            Some(&signature),
            &edit_changed
        ));
    }

    #[test]
    fn composer_queues_typed_input_while_codex_session_is_starting() {
        assert_eq!(
            composer_action_for_startup_state(true, false, false, 0, true),
            ComposerAction::Queue
        );
        assert_eq!(
            composer_action_for_startup_state(false, false, false, 1, true),
            ComposerAction::Disabled
        );
    }

    #[test]
    fn latest_session_status_for_thread_uses_newest_matching_record() {
        let records = vec![
            process_record_with_thread(1, ProcessStatus::Stopped, Some(7), "codex"),
            process_record_with_thread(2, ProcessStatus::Running, Some(8), "codex"),
            process_record_with_thread(3, ProcessStatus::Exited, Some(7), "shell"),
            process_record_with_thread(4, ProcessStatus::Exited, Some(7), "codex"),
        ];

        assert_eq!(
            latest_session_status_for_thread(&records, 7, SessionKind::Codex),
            Some(ProcessStatus::Exited)
        );
        assert_eq!(
            latest_session_status_for_thread(&records, 7, SessionKind::Shell),
            Some(ProcessStatus::Exited)
        );
    }

    #[test]
    fn active_generation_requires_working_thread_and_running_session() {
        let records = vec![process_record_with_thread(
            1,
            ProcessStatus::Running,
            Some(7),
            "codex",
        )];
        let working_threads = RefCell::new(HashMap::new());

        assert!(!active_generation_for_thread(
            &records,
            &working_threads,
            7,
            SessionKind::Codex
        ));
        mark_thread_working(&working_threads, 7);
        assert!(active_generation_for_thread(
            &records,
            &working_threads,
            7,
            SessionKind::Codex
        ));
        assert!(!active_generation_for_thread(
            &records,
            &working_threads,
            8,
            SessionKind::Codex
        ));
    }

    #[test]
    fn local_chat_queue_pops_one_turn_at_a_time() {
        let app_state = AppState::new(
            archductor_core::paths::AppPaths::from_env(),
            Some("berlin".to_owned()),
            crate::state::WorkspaceTab::Chats,
            AppPage::Workspace,
        );
        queue_archcar_input(
            &app_state,
            7,
            " first ".to_owned(),
            None,
            ArchcarInputKind::User,
            SessionKind::Codex,
        );
        queue_archcar_input(
            &app_state,
            7,
            "second".to_owned(),
            None,
            ArchcarInputKind::User,
            SessionKind::Codex,
        );

        assert_eq!(queued_chat_inputs_count(&app_state, 7), 2);
        let queued = pop_next_queued_chat_input(&app_state, 7).unwrap();
        assert_eq!(queued.input, "first");
        assert_eq!(queued_chat_inputs_count(&app_state, 7), 1);
        let queued = pop_next_queued_chat_input(&app_state, 7).unwrap();
        assert_eq!(queued.input, "second");
        assert_eq!(queued_chat_inputs_count(&app_state, 7), 0);
    }

    #[test]
    fn reconnect_ready_snapshot_releases_only_the_matching_queued_thread() {
        let records = vec![
            process_record_with_thread(11, ProcessStatus::Running, Some(7), "codex"),
            process_record_with_thread(12, ProcessStatus::Running, Some(8), "codex"),
            process_record_with_thread(13, ProcessStatus::Running, Some(9), "claude"),
        ];
        let ready_cache = RefCell::new(HashMap::from([(11, true), (12, false), (13, true)]));
        let codex = archductor_core::archcar::harness::managed_harness_for_kind(SessionKind::Codex)
            .unwrap();
        let claude =
            archductor_core::archcar::harness::managed_harness_for_kind(SessionKind::Claude)
                .unwrap();

        assert_eq!(
            ready_queued_session_for_thread(&records, 7, codex.descriptor(), &ready_cache),
            Some(11)
        );
        assert_eq!(
            ready_queued_session_for_thread(&records, 8, codex.descriptor(), &ready_cache),
            None
        );
        assert_eq!(
            ready_queued_session_for_thread(&records, 9, claude.descriptor(), &ready_cache),
            Some(13)
        );
        assert_eq!(
            ready_queued_session_for_thread(&records, 10, claude.descriptor(), &ready_cache),
            None
        );
    }

    #[test]
    fn claude_first_send_uses_managed_thread_ensure_request() {
        assert_eq!(
            request_kind_for_first_send(SessionKind::Claude, "berlin", 42),
            Some(AsyncArchcarRequestKind::EnsureChatThreadSession {
                workspace: "berlin".to_owned(),
                thread_id: 42,
                kind: SessionKind::Claude,
            })
        );
    }

    fn request_kind_for_first_send(
        kind: SessionKind,
        workspace: &str,
        thread_id: i64,
    ) -> Option<AsyncArchcarRequestKind> {
        let harness = managed_harness_for_kind(kind)?;
        Some(AsyncArchcarRequestKind::EnsureChatThreadSession {
            workspace: workspace.to_owned(),
            thread_id,
            kind: harness.descriptor().kind,
        })
    }

    #[test]
    fn claude_first_send_source_uses_managed_registry_not_spawn_fallback() {
        let source = include_str!("session_surface.rs");
        let send_body = source
            .split("let send_text_with_delivery = Rc::new(")
            .nth(1)
            .and_then(|tail| tail.split("let send_immediate_input").next())
            .expect("send_text_with_delivery body should be present");

        assert!(
            send_body.contains("managed_harness_for_kind(selected_kind)"),
            "first send should discover managed providers from the registry"
        );
        assert!(
            !send_body.contains("spawn_session(workspace_for_send.clone(), selected_kind)"),
            "first send must not use the generic Claude workspace-spawn fallback"
        );
    }

    #[test]
    fn immediate_send_uses_shared_turn_preparation_path() {
        let source = include_str!("session_surface.rs");
        let immediate_body = source
            .split("let send_immediate_input: Rc<dyn Fn(String, bool) -> bool> = Rc::new({")
            .nth(1)
            .and_then(|tail| tail.split("*send_immediate_after_ready_queue").next())
            .expect("send_immediate_input body should be present");

        assert!(
            immediate_body.contains(
                "send_text_with_delivery(command, staged_review, ArchcarInputDelivery::Immediate)"
            ),
            "immediate send should reuse normal turn preparation and vary only delivery"
        );
        assert!(
            !immediate_body.contains("queue_archcar_user_send("),
            "immediate send must not bypass checkpoint/control/model-context preparation"
        );
    }

    #[test]
    fn queued_composer_overlay_items_include_hover_actions() {
        let app_state = AppState::new(
            archductor_core::paths::AppPaths::from_env(),
            Some("berlin".to_owned()),
            crate::state::WorkspaceTab::Chats,
            AppPage::Workspace,
        );
        queue_archcar_input(
            &app_state,
            7,
            "first follow-up".to_owned(),
            None,
            ArchcarInputKind::User,
            SessionKind::Codex,
        );

        let items =
            queued_composer_overlay_items_for_thread(&app_state, Some(7), SessionKind::Codex);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].preview, "first follow-up");
        assert!(items[0].actions.contains(&QueuedComposerAction::Delete));
        assert!(items[0].actions.contains(&QueuedComposerAction::Edit));
        assert!(items[0]
            .actions
            .contains(&QueuedComposerAction::SendImmediately));

        let claude_items =
            queued_composer_overlay_items_for_thread(&app_state, Some(7), SessionKind::Claude);
        assert!(claude_items[0]
            .actions
            .contains(&QueuedComposerAction::SendImmediately));
    }

    #[test]
    fn queued_row_send_uses_immediate_steer_path() {
        let source = include_str!("session_surface.rs");
        let send_row_body = source
            .split("let send_immediately = Rc::new({")
            .nth(1)
            .and_then(|tail| tail.split("let row =").next())
            .expect("queue row send callback should be present");

        assert!(send_row_body.contains("send_immediate_after_ready_queue"));
        assert!(send_row_body.contains("requeue_pending_input_front(&app_state"));
        assert!(source.contains("icon_button(\"send-symbolic\", \"Steer now\")"));
    }

    #[test]
    fn queued_row_edit_does_not_release_auto_drain_hold() {
        let source = include_str!("session_surface.rs");
        let edit_row_body = source
            .split("let edit_item = Rc::new({")
            .nth(1)
            .and_then(|tail| tail.split("let send_immediately = Rc::new({").next())
            .expect("queue row edit callback should be present");

        assert!(edit_row_body.contains("begin_editing_queued_chat_input"));
        assert!(!edit_row_body.contains("release_queued_auto_drain_if_queue_empty"));
    }

    #[test]
    fn queued_composer_item_action_filter_controls_rendered_buttons() {
        let item = QueuedComposerItem {
            index: 0,
            preview: "queued".to_owned(),
            actions: vec![QueuedComposerAction::Delete, QueuedComposerAction::Edit],
        };

        assert!(queued_composer_item_allows_action(
            &item,
            QueuedComposerAction::Delete
        ));
        assert!(queued_composer_item_allows_action(
            &item,
            QueuedComposerAction::Edit
        ));
        assert!(!queued_composer_item_allows_action(
            &item,
            QueuedComposerAction::SendImmediately
        ));
    }

    #[test]
    fn composer_queue_accepts_pending_chat_target() {
        let state = AppState::new(
            archductor_core::paths::AppPaths::from_env(),
            Some("berlin".to_owned()),
            crate::state::WorkspaceTab::Chats,
            AppPage::Workspace,
        );
        let target = state.create_pending_chat_target("berlin".to_owned(), SessionKind::Codex);

        state.queue_chat_input_for_target(
            target.clone(),
            QueuedChatInputDraft {
                input: "start once ready".to_owned(),
                visible_input: None,
                kind: ArchcarInputKind::User,
                session_kind: SessionKind::Codex,
            },
        );

        assert_eq!(state.queued_chat_inputs_for_target(&target).len(), 1);
    }

    #[test]
    fn pending_chat_submit_targets_pending_chat_not_previous_thread() {
        let state = AppState::new(
            archductor_core::paths::AppPaths::from_env(),
            Some("berlin".to_owned()),
            crate::state::WorkspaceTab::Chats,
            AppPage::Workspace,
        );
        state.set_selected_chat_thread(Some(7));
        let pending = state.create_pending_chat_target("berlin".to_owned(), SessionKind::Codex);

        let target = selected_chat_target_for_submit(&state, Some(7));

        assert_eq!(target, Some(pending.clone()));
        queue_archcar_input_for_target(
            &state,
            target.unwrap(),
            "first message in new chat".to_owned(),
            None,
            ArchcarInputKind::User,
            SessionKind::Codex,
        );

        assert_eq!(state.queued_chat_inputs_count(7), 0);
        assert_eq!(state.queued_chat_inputs_for_target(&pending).len(), 1);
    }

    #[test]
    fn pending_chat_composer_state_ignores_previous_thread_queue() {
        let state = AppState::new(
            archductor_core::paths::AppPaths::from_env(),
            Some("berlin".to_owned()),
            crate::state::WorkspaceTab::Chats,
            AppPage::Workspace,
        );
        state.set_selected_chat_thread(Some(7));
        state.queue_chat_input(
            7,
            QueuedChatInputDraft {
                input: "old queued input".to_owned(),
                visible_input: None,
                kind: ArchcarInputKind::User,
                session_kind: SessionKind::Codex,
            },
        );
        let pending = state.create_pending_chat_target("berlin".to_owned(), SessionKind::Codex);

        assert_eq!(
            composer_thread_for_target(Some(&pending), Some(7)),
            None,
            "pending chat should not inherit previous thread queue/action state"
        );
        assert_eq!(
            state.queued_chat_inputs_count(7),
            1,
            "previous thread queue must remain untouched while pending chat is selected"
        );
    }

    #[test]
    fn pending_chat_resolve_keeps_first_message_on_new_thread_queue() {
        let state = AppState::new(
            archductor_core::paths::AppPaths::from_env(),
            Some("berlin".to_owned()),
            crate::state::WorkspaceTab::Chats,
            AppPage::Workspace,
        );
        let pending = state.create_pending_chat_target("berlin".to_owned(), SessionKind::Codex);
        queue_archcar_input_for_target(
            &state,
            pending.clone(),
            "first message in new chat".to_owned(),
            None,
            ArchcarInputKind::User,
            SessionKind::Codex,
        );

        state.resolve_pending_chat_target(pending, 42);

        let queued = pop_next_queued_chat_input(&state, 42).unwrap();
        assert_eq!(queued.input, "first message in new chat");
        assert_eq!(state.selected_chat_thread(), Some(42));
    }

    #[test]
    fn resolved_empty_pending_chat_becomes_ready_for_first_send() {
        let state = AppState::new(
            archductor_core::paths::AppPaths::from_env(),
            Some("berlin".to_owned()),
            crate::state::WorkspaceTab::Chats,
            AppPage::Workspace,
        );
        let pending = state.create_pending_chat_target("berlin".to_owned(), SessionKind::Codex);

        state.resolve_pending_chat_target(pending, 42);

        assert_eq!(
            state.chat_phase(&ChatUiTarget::Thread(42)),
            Some(ChatUiPhase::Ready)
        );
    }

    #[test]
    fn new_chat_button_does_not_create_thread_sync_in_click_handler() {
        let source = include_str!("session_surface.rs");
        let start = source
            .find("new_chat_btn.connect_clicked")
            .expect("new chat button handler exists");
        let end = source[start..]
            .find("root\n}")
            .map(|offset| start + offset)
            .expect("new chat handler end exists");
        let handler = &source[start..end];

        assert!(
            !handler.contains("store.create_chat_thread("),
            "new chat creation must be spawned so GTK stays responsive"
        );
        assert!(
            handler.contains("create_pending_chat_target"),
            "new chat creation must select an optimistic pending chat immediately"
        );
    }

    #[test]
    fn turn_completed_boundary_allows_next_queued_input_to_flush() {
        let records = vec![process_record_with_thread(
            11,
            ProcessStatus::Running,
            Some(7),
            "codex",
        )];
        let session_threads = RefCell::new(HashMap::from([(11, 7)]));
        let pending_inputs = RefCell::new(HashMap::new());
        let working_threads = RefCell::new(HashMap::new());
        mark_thread_working(&working_threads, 7);

        let changed = update_working_indicator_for_archcar_event(
            &ArchcarEvent::TurnCompleted {
                session_id: 11,
                thread_id: 7,
                status: Some("completed".to_owned()),
            },
            &records,
            &session_threads,
            SessionKind::Codex,
            Some(7),
            &pending_inputs,
            &working_threads,
        );

        assert!(changed);
        assert!(!working_threads.borrow().contains_key(&7));
    }

    #[test]
    fn session_ready_does_not_clear_working_thread_during_active_generation() {
        let records = vec![process_record_with_thread(
            11,
            ProcessStatus::Running,
            Some(7),
            "codex",
        )];
        let session_threads = RefCell::new(HashMap::from([(11, 7)]));
        let pending_inputs = RefCell::new(HashMap::new());
        let working_threads = RefCell::new(HashMap::new());
        mark_thread_working(&working_threads, 7);

        let changed = update_working_indicator_for_archcar_event(
            &ArchcarEvent::SessionReady {
                session_id: 11,
                thread_id: 7,
            },
            &records,
            &session_threads,
            SessionKind::Codex,
            Some(7),
            &pending_inputs,
            &working_threads,
        );

        assert!(!changed);
        assert!(working_threads.borrow().contains_key(&7));
    }

    #[test]
    fn session_ready_clears_starting_chat_phase() {
        let state = AppState::new(
            archductor_core::paths::AppPaths::from_env(),
            Some("berlin".to_owned()),
            crate::state::WorkspaceTab::Chats,
            AppPage::Workspace,
        );
        state.mark_chat_phase(
            ChatUiTarget::Thread(7),
            ChatUiPhase::StartingAgent {
                provider: SessionKind::Codex,
            },
        );

        mark_chat_startup_finished(&state, 7);

        assert_eq!(
            state.chat_phase(&ChatUiTarget::Thread(7)),
            Some(ChatUiPhase::Ready)
        );
    }

    #[test]
    fn automatic_ready_drain_waits_until_active_generation_finishes() {
        let records = vec![process_record_with_thread(
            11,
            ProcessStatus::Running,
            Some(7),
            "codex",
        )];
        let ready_cache = RefCell::new(HashMap::from([(11, true)]));
        let inflight_actions = RefCell::new(HashMap::new());
        let pending_inputs = RefCell::new(HashMap::<i64, Vec<QueuedArchcarInput>>::new());
        let holds = RefCell::new(HashSet::new());
        let working_threads = RefCell::new(HashMap::new());
        mark_thread_working(&working_threads, 7);

        assert!(!queued_chat_auto_drain_ready(
            SessionKind::Codex,
            &records,
            7,
            &ready_cache,
            &inflight_actions,
            &pending_inputs,
            &holds,
            &working_threads,
        ));

        clear_thread_working(&working_threads, 7);

        assert!(queued_chat_auto_drain_ready(
            SessionKind::Codex,
            &records,
            7,
            &ready_cache,
            &inflight_actions,
            &pending_inputs,
            &holds,
            &working_threads,
        ));
    }

    #[test]
    fn failed_or_interrupted_turn_completion_does_not_allow_queue_drain() {
        assert!(archcar_turn_completion_allows_queue_drain(Some("success")));
        assert!(archcar_turn_completion_allows_queue_drain(Some(
            "completed"
        )));
        assert!(archcar_turn_completion_allows_queue_drain(None));

        for status in [
            "failed",
            "error",
            "interrupted",
            "cancelled",
            "canceled",
            "deferred",
        ] {
            assert!(
                !archcar_turn_completion_allows_queue_drain(Some(status)),
                "{status} turn completion must leave queued messages untouched"
            );
        }
    }

    #[test]
    fn failed_or_interrupted_turn_completion_holds_queued_auto_drain() {
        let holds = RefCell::new(HashSet::new());

        record_queued_auto_drain_turn_completion(&holds, 7, Some("interrupted"));

        assert!(!queued_auto_drain_allowed(&holds, 7));
        assert!(queued_auto_drain_allowed(&holds, 8));

        release_queued_auto_drain_if_queue_empty(&holds, 7, 1);

        assert!(!queued_auto_drain_allowed(&holds, 7));

        release_queued_auto_drain_if_queue_empty(&holds, 7, 0);

        assert!(queued_auto_drain_allowed(&holds, 7));
    }

    #[test]
    fn queued_auto_drain_hold_applies_to_codex_and_claude_threads() {
        let holds = RefCell::new(HashSet::new());

        hold_queued_auto_drain(&holds, 11);
        hold_queued_auto_drain(&holds, 12);

        assert!(!queued_auto_drain_allowed(&holds, 11));
        assert!(!queued_auto_drain_allowed(&holds, 12));

        release_queued_auto_drain_if_queue_empty(&holds, 11, 0);

        assert!(queued_auto_drain_allowed(&holds, 11));
        assert!(!queued_auto_drain_allowed(&holds, 12));
    }

    #[test]
    fn automatic_ready_drain_checks_thread_hold_before_popping_queue() {
        let source = include_str!("session_surface.rs");
        let auto_drain_block = source
            .split("let can_send_next = queued_chat_auto_drain_ready(")
            .nth(1)
            .and_then(|rest| rest.split("pop_next_queued_chat_input").next())
            .expect("ready auto-drain block should be present");

        assert!(
            auto_drain_block.contains("queued_auto_drain_holds.as_ref()")
                && auto_drain_block.contains("&working_threads"),
            "automatic ready drain must respect hold and active-generation guards before popping queued chat input"
        );
    }

    #[test]
    fn running_session_for_thread_selects_matching_live_process() {
        let records = vec![
            process_record_with_thread(1, ProcessStatus::Stopped, Some(7), "codex"),
            process_record_with_thread(2, ProcessStatus::Running, Some(8), "codex"),
            process_record_with_thread(3, ProcessStatus::Running, Some(7), "shell"),
            process_record_with_thread(4, ProcessStatus::Running, Some(7), "codex"),
        ];

        assert_eq!(
            running_session_for_thread(&records, 7, SessionKind::Codex),
            Some(4)
        );
        assert_eq!(
            running_session_for_thread(&records, 7, SessionKind::Shell),
            Some(3)
        );
        assert_eq!(
            running_session_for_thread(&records, 99, SessionKind::Codex),
            None
        );
    }

    #[test]
    fn default_chat_thread_titles_use_new_chat_prefix() {
        assert_eq!(
            default_chat_thread_title(SessionKind::Codex, &[]),
            "New Chat"
        );
        assert_eq!(
            default_chat_thread_title(
                SessionKind::Codex,
                &[ChatThreadRecord {
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
                }]
            ),
            "New Chat 2"
        );
    }

    #[test]
    fn archductor_metadata_injected_prompt_requests_semantic_names() {
        let prompt = archductor_metadata_injected_prompt("Fix parser failure", "venice", "team");

        assert!(prompt.starts_with("Fix parser failure\n\n<archductor_hidden_instruction>"));
        assert!(prompt.contains("<archductor_metadata>"));
        assert!(prompt.contains(
            "<archductor_metadata>{\"workspace_name\":\"short-kebab-name\",\"branch_name\":\"team/short-kebab-name\",\"chat_title\":\"Short Title\"}</archductor_metadata>"
        ));
        assert!(prompt.contains("Use exactly these JSON keys in this order"));
        assert!(prompt.contains("Do not wrap the metadata in Markdown or a code fence."));
        assert!(prompt.contains("Do not copy or truncate the raw user message."));
        assert!(prompt.contains("Current placeholder workspace name: venice."));
    }

    #[test]
    fn codex_inline_event_display_maps_kind_and_status() {
        let tool = CodexInlineEvent {
            kind: CodexInlineEventKind::Tool,
            title: "functions.exec_command".to_owned(),
            subtitle: Some("cargo test".to_owned()),
            body: Some("running".to_owned()),
            path: None,
            status: CodexInlineEventStatus::Loading,
        };
        let skill = CodexInlineEvent {
            kind: CodexInlineEventKind::Skill,
            title: "superpowers:test-driven-development".to_owned(),
            subtitle: None,
            body: None,
            path: None,
            status: CodexInlineEventStatus::Failed,
        };

        assert_eq!(inline_event_kind_label(tool.kind), "Tool");
        assert_eq!(inline_event_kind_label(skill.kind), "Skill");
        assert_eq!(inline_event_status_label(tool.status), "Running");
        assert_eq!(
            inline_event_status_css_class(tool.status),
            Some("chat-inline-event-loading")
        );
        assert_eq!(
            inline_event_status_css_class(skill.status),
            Some("chat-inline-event-failed")
        );
    }

    #[test]
    fn codex_inline_event_chip_label_uses_action_name() {
        let file_event = CodexInlineEvent {
            kind: CodexInlineEventKind::Tool,
            title: "Edited docs/superpowers/plans/2026-07-03-manual-skill-tool-calls.md".to_owned(),
            subtitle: Some("+17 -2".to_owned()),
            body: None,
            path: Some(PathBuf::from(
                "docs/superpowers/plans/2026-07-03-manual-skill-tool-calls.md",
            )),
            status: CodexInlineEventStatus::Complete,
        };
        let command_event = CodexInlineEvent {
            kind: CodexInlineEventKind::Tool,
            title: "cargo test -p archductor-core codex_tui".to_owned(),
            subtitle: Some("Command result".to_owned()),
            body: None,
            path: None,
            status: CodexInlineEventStatus::Complete,
        };

        assert_eq!(
            inline_event_chip_label(&file_event, false),
            "Edited docs/superpowers/plans/2026-07-03-manual-skill-tool-calls.md"
        );
        assert_eq!(
            inline_event_chip_label(&file_event, true),
            "Edited docs/superpowers/plans/2026-07-03-manual-skill-tool-calls.md"
        );
        assert_eq!(
            inline_event_chip_label(&command_event, false),
            "cargo test -p archductor-core codex_tui"
        );
    }

    #[test]
    fn inline_event_chip_markup_uses_type_icon_and_action_text() {
        let command_event = CodexInlineEvent {
            kind: CodexInlineEventKind::Tool,
            title: "Ran cargo test".to_owned(),
            subtitle: Some("Command".to_owned()),
            body: None,
            path: None,
            status: CodexInlineEventStatus::Complete,
        };
        let skill_event = CodexInlineEvent {
            kind: CodexInlineEventKind::Skill,
            title: "Read superpowers:test-driven-development".to_owned(),
            subtitle: Some("Skill".to_owned()),
            body: None,
            path: None,
            status: CodexInlineEventStatus::Complete,
        };

        let command_markup = inline_event_chip_markup(&command_event, false);
        let skill_markup = inline_event_chip_markup(&skill_event, false);

        assert_eq!(
            inline_event_type_css_class(&command_event),
            "chat-inline-event-command"
        );
        assert_eq!(
            inline_event_type_css_class(&skill_event),
            "chat-inline-event-skill"
        );
        assert!(command_markup.contains("foreground=\"#93c5fd\""));
        assert!(command_markup.contains("font_family=\"Commit Mono\""));
        assert!(command_markup.contains(">⌘</span>"));
        assert!(command_markup.contains(">Ran</span>"));
        assert!(command_markup.contains("foreground=\"#e7e7e7\">cargo test"));
        assert!(skill_markup.contains("foreground=\"#fcd34d\""));
        assert!(skill_markup.contains(">◆</span>"));
        assert!(skill_markup.contains(">Read</span>"));
        assert!(skill_markup.contains("foreground=\"#e7e7e7\">superpowers:test-driven-development"));
    }

    #[test]
    fn inline_event_chip_label_layout_caps_long_commands() {
        let long_command = format!("Ran pnpm {}", "x".repeat(240));
        let event = CodexInlineEvent {
            kind: CodexInlineEventKind::Tool,
            title: long_command,
            subtitle: Some("Command".to_owned()),
            body: None,
            path: None,
            status: CodexInlineEventStatus::Complete,
        };

        assert_eq!(inline_event_chip_label_max_width_chars(), 96);
        assert_eq!(
            inline_event_chip_label_width_chars(&inline_event_chip_label(&event, false)),
            96
        );
        assert_eq!(
            inline_event_type_css_class(&event),
            "chat-inline-event-command"
        );
        assert!(inline_event_chip_markup(&event, false).contains("foreground=\"#e7e7e7\">pnpm"));
    }

    #[test]
    fn inline_event_chip_label_short_names_do_not_request_one_char_width() {
        let label = "Used MCP Loaded";

        assert_eq!(
            inline_event_chip_label_width_chars(label),
            label.chars().count() as i32
        );
    }

    #[test]
    fn codex_inline_event_local_preview_requires_existing_small_text_file() {
        let temp = tempfile::tempdir().unwrap();
        let text_path = temp.path().join("result.txt");
        fs::write(&text_path, "plain text output").unwrap();
        let binary_path = temp.path().join("image.bin");
        fs::write(&binary_path, b"\x00\x01\x02").unwrap();

        assert!(local_preview_eligibility(&text_path).is_some());
        assert!(local_preview_eligibility(temp.path().join("missing.txt")).is_none());
        assert!(local_preview_eligibility(temp.path()).is_none());
        assert!(local_preview_eligibility(&binary_path).is_none());
    }

    #[test]
    fn codex_inline_event_body_loads_file_read_by_tool_or_skill() {
        let temp = tempfile::tempdir().unwrap();
        let tool_path = temp.path().join("result.txt");
        fs::write(&tool_path, "tool file contents").unwrap();
        let skill_path = temp.path().join("SKILL.md");
        fs::write(&skill_path, "---\nname: graphify\n---").unwrap();

        let tool = parse_session_transcript_inline_event(
            SessionTranscriptRole::Tool,
            &format!("Read {}", tool_path.display()),
            &format!("Read {}", tool_path.display()),
        )
        .unwrap();
        let skill = parse_session_transcript_inline_event(
            SessionTranscriptRole::Skill,
            &format!("Read {} (graphify)", skill_path.display()),
            &format!("Read {} (graphify)", skill_path.display()),
        )
        .unwrap();

        assert_eq!(tool.kind, CodexInlineEventKind::Tool);
        assert_eq!(tool.title, "Read result.txt");
        assert_eq!(tool.path.as_deref(), Some(tool_path.as_path()));
        assert_eq!(inline_event_chip_label(&tool, false), "Read result.txt");
        assert!(inline_event_body_text(&tool).contains("tool file contents"));
        assert_eq!(skill.kind, CodexInlineEventKind::Skill);
        assert_eq!(skill.title, "Read SKILL.md for graphify");
        assert_eq!(skill.path.as_deref(), Some(skill_path.as_path()));
        assert_eq!(
            inline_event_chip_label(&skill, false),
            "Read SKILL.md for graphify"
        );
        assert!(inline_event_body_text(&skill).contains("name: graphify"));
    }

    #[test]
    fn read_only_shell_commands_render_as_file_reads() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("session_surface.rs");
        fs::write(&source_path, "fn main() {}\n").unwrap();

        let read = parse_session_transcript_inline_event(
            SessionTranscriptRole::Tool,
            &format!("Ran sed -n '1,220p' {}", source_path.display()),
            "fn main() {}\n",
        )
        .unwrap();

        assert_eq!(read.kind, CodexInlineEventKind::Tool);
        assert_eq!(read.title, "Read session_surface.rs");
        assert_eq!(read.subtitle.as_deref(), Some("File preview"));
        assert_eq!(read.path.as_deref(), Some(source_path.as_path()));
        assert_eq!(
            inline_event_chip_label(&read, false),
            "Read session_surface.rs"
        );
    }

    #[test]
    fn read_only_bash_lc_commands_render_as_file_reads() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("session_surface.rs");
        fs::write(&source_path, "fn main() {}\n").unwrap();

        let read = parse_session_transcript_inline_event(
            SessionTranscriptRole::Tool,
            &format!(
                "Ran /bin/bash -lc \"sed -n '1,220p' {}\"",
                source_path.display()
            ),
            "fn main() {}\n",
        )
        .unwrap();

        assert_eq!(read.kind, CodexInlineEventKind::Tool);
        assert_eq!(read.title, "Read session_surface.rs");
        assert_eq!(read.subtitle.as_deref(), Some("File preview"));
        assert_eq!(read.path.as_deref(), Some(source_path.as_path()));
        assert_eq!(
            inline_event_chip_label(&read, false),
            "Read session_surface.rs"
        );
    }

    #[test]
    fn read_only_shell_commands_render_skill_md_as_skill_reads() {
        let temp = tempfile::tempdir().unwrap();
        let skill_dir = temp.path().join("graphify");
        fs::create_dir(&skill_dir).unwrap();
        let skill_path = skill_dir.join("SKILL.md");
        fs::write(&skill_path, "---\nname: graphify\n---").unwrap();

        let read = parse_session_transcript_inline_event(
            SessionTranscriptRole::Tool,
            &format!("Ran cat '{}'", skill_path.display()),
            "---\nname: graphify\n---",
        )
        .unwrap();

        assert_eq!(read.kind, CodexInlineEventKind::Skill);
        assert_eq!(read.title, "Read SKILL.md for graphify");
        assert_eq!(read.subtitle.as_deref(), Some("Skill"));
        assert_eq!(read.path.as_deref(), Some(skill_path.as_path()));
        assert_eq!(
            inline_event_chip_label(&read, false),
            "Read SKILL.md for graphify"
        );
    }

    #[test]
    fn read_only_bash_lc_commands_render_skill_md_as_skill_reads() {
        let temp = tempfile::tempdir().unwrap();
        let skill_dir = temp.path().join("verification-before-completion");
        fs::create_dir(&skill_dir).unwrap();
        let skill_path = skill_dir.join("SKILL.md");
        fs::write(
            &skill_path,
            "---\nname: verification-before-completion\n---",
        )
        .unwrap();

        let read = parse_session_transcript_inline_event(
            SessionTranscriptRole::Tool,
            &format!(
                "Ran /bin/bash -lc \"sed -n '1,260p' {}\"",
                skill_path.display()
            ),
            "---\nname: verification-before-completion\n---",
        )
        .unwrap();

        assert_eq!(read.kind, CodexInlineEventKind::Skill);
        assert_eq!(
            read.title,
            "Read SKILL.md for verification-before-completion"
        );
        assert_eq!(read.subtitle.as_deref(), Some("Skill"));
        assert_eq!(read.path.as_deref(), Some(skill_path.as_path()));
        assert_eq!(
            inline_event_chip_label(&read, false),
            "Read SKILL.md for verification-before-completion"
        );
    }

    #[test]
    fn codex_inline_event_body_truncates_but_reports_expandability() {
        let short = truncate_inline_event_body("short body", 20);
        assert_eq!(short.preview, "short body");
        assert!(!short.truncated);

        let long = truncate_inline_event_body("abcdef", 4);
        assert_eq!(long.preview, "abcd...");
        assert!(long.truncated);
        assert_eq!(long.full, "abcdef");
    }

    #[test]
    fn write_inline_event_body_preview_keeps_full_body() {
        let full = format!("Write src/main.rs\n{}", "x".repeat(500));
        let event = CodexInlineEvent {
            kind: CodexInlineEventKind::Tool,
            title: "Write src/main.rs".to_owned(),
            subtitle: None,
            body: Some(full.clone()),
            path: None,
            status: CodexInlineEventStatus::Complete,
        };

        let preview = inline_event_body_preview(&event, &full);

        assert_eq!(preview.preview, full);
        assert_eq!(preview.full, full);
        assert!(!preview.truncated);
        assert!(!inline_event_expands_body_by_default(&event));
    }

    #[test]
    fn transcript_groups_raw_write_event_with_full_body() {
        let transcript =
            "Write src/main.rs\nfn main() {\n    println!(\"hi\");\n}\n[session exited]\n";

        let events = parse_session_transcript_events(transcript);

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].role, SessionTranscriptRole::Tool);
        assert_eq!(
            events[0].body,
            "Write src/main.rs\nfn main() {\n    println!(\"hi\");\n}"
        );
        let inline_events = session_transcript_inline_events(&events[0]);
        assert_eq!(inline_events.len(), 1);
        assert_eq!(
            inline_events[0].body.as_deref(),
            Some(events[0].body.as_str())
        );
    }

    #[test]
    fn transcript_groups_indented_bullet_tool_event_with_body() {
        let transcript = "  • Write src/main.rs\nfn main() {}\n[session exited]\n";

        let events = parse_session_transcript_events(transcript);

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].role, SessionTranscriptRole::Tool);
        assert_eq!(events[0].body, "• Write src/main.rs\nfn main() {}");
    }

    #[test]
    fn context_usage_display_state_maps_remaining_context_and_thresholds() {
        let empty = context_usage_display_state(None);
        assert_eq!(empty.percent_label, "--");
        assert_eq!(empty.css_class, "chat-context-usage-empty");

        let normal = context_usage_display_state(Some(CodexContextUsage {
            used_tokens: Some(68_000),
            max_tokens: Some(100_000),
            percent: 68,
        }));
        assert_eq!(normal.percent_label, "22%");
        assert_eq!(normal.css_class, "chat-context-usage-normal");

        let warning = context_usage_display_state(Some(CodexContextUsage {
            used_tokens: None,
            max_tokens: None,
            percent: 70,
        }));
        assert_eq!(warning.percent_label, "20%");
        assert_eq!(warning.css_class, "chat-context-usage-warning");

        let danger = context_usage_display_state(Some(CodexContextUsage {
            used_tokens: None,
            max_tokens: None,
            percent: 90,
        }));
        assert_eq!(danger.percent_label, "0%");
        assert_eq!(danger.css_class, "chat-context-usage-danger");
    }

    #[test]
    fn codex_context_usage_parser_derives_percent_from_token_pair() {
        let usage = parse_codex_context_usage_local("128k / 200k tokens").unwrap();

        assert_eq!(usage.used_tokens, Some(128_000));
        assert_eq!(usage.max_tokens, Some(200_000));
        assert_eq!(usage.percent, 64);
    }

    #[test]
    fn context_detail_summary_reports_history_growth_and_compaction() {
        let messages = vec![
            ChatMessageRecord {
                id: 1,
                thread_id: 10,
                role: "agent".to_owned(),
                content: "Context window: 120k / 200k tokens".to_owned(),
                source: "agent_screen_parse".to_owned(),
                timeline_seq: Some(1),
                created_at: "2026-07-09T12:00:00Z".to_owned(),
                updated_at: "2026-07-09T12:00:00Z".to_owned(),
            },
            ChatMessageRecord {
                id: 2,
                thread_id: 10,
                role: "agent".to_owned(),
                content: "After compaction, continuing with summary instead of the full thread.\nContext window: 150k / 200k tokens".to_owned(),
                source: "agent_screen_parse".to_owned(),
                timeline_seq: Some(2),
                created_at: "2026-07-09T12:05:00Z".to_owned(),
                updated_at: "2026-07-09T12:05:00Z".to_owned(),
            },
        ];
        let events = vec![ChatEventRecord {
            id: 3,
            thread_id: 10,
            process_id: Some(4),
            kind: "tool".to_owned(),
            title: "exec".to_owned(),
            body: "long command output".to_owned(),
            path: None,
            payload_json: "{}".to_owned(),
            timeline_seq: 3,
            created_at: "2026-07-09T12:06:00Z".to_owned(),
            updated_at: "2026-07-09T12:06:00Z".to_owned(),
        }];

        let summary = context_detail_summary(&messages, &events);
        let text = format_context_detail_summary(&summary);

        assert_eq!(summary.usage.unwrap().percent, 75);
        assert_eq!(summary.estimated_tokens, 150_000);
        assert!(summary.recent_growth.contains("+30,000 tokens"));
        assert_eq!(summary.compaction_events.len(), 1);
        assert!(text.contains("compaction risk at 90%"));
        assert!(text.contains("Compaction events:"));
        assert!(text.contains("Top contributors:"));
    }

    #[test]
    fn context_detail_summary_uses_stable_byte_estimate_without_usage() {
        let messages = vec![ChatMessageRecord {
            id: 1,
            thread_id: 10,
            role: "user".to_owned(),
            content: "abcdefghijkl".to_owned(),
            source: "user_send".to_owned(),
            timeline_seq: Some(1),
            created_at: "2026-07-09T12:00:00Z".to_owned(),
            updated_at: "2026-07-09T12:00:00Z".to_owned(),
        }];

        let summary = context_detail_summary(&messages, &[]);

        assert_eq!(summary.usage, None);
        assert_eq!(summary.transcript_bytes, 12);
        assert_eq!(summary.estimated_tokens, 3);
        assert_eq!(
            summary.estimate_method,
            "estimated from persisted transcript bytes at 4 chars/token"
        );
    }

    #[test]
    fn codex_inline_event_parser_detects_tool_and_skill_markers() {
        let events = parse_codex_inline_events_local(
            "functions.exec_command running cargo test\nCalling mcp__xcodebuildmcp__session_show_defaults before build\nUsing graphify to map the repository\nUsing superpowers:brainstorming to shape the UI",
        );

        assert_eq!(events.len(), 4);
        assert_eq!(events[0].kind, CodexInlineEventKind::Tool);
        assert_eq!(events[0].title, "functions.exec_command");
        assert_eq!(events[0].status, CodexInlineEventStatus::Loading);
        assert_eq!(events[1].kind, CodexInlineEventKind::Tool);
        assert_eq!(events[1].title, "mcp__xcodebuildmcp__session_show_defaults");
        assert_eq!(events[2].kind, CodexInlineEventKind::Skill);
        assert_eq!(events[2].title, "graphify");
        assert_eq!(events[3].kind, CodexInlineEventKind::Skill);
        assert_eq!(events[3].title, "superpowers:brainstorming");
    }

    #[test]
    fn chat_refresh_rows_do_not_reveal_existing_messages_on_poll() {
        assert!(!reveal_existing_chat_refresh_rows());
    }

    #[test]
    fn chat_scroll_snapshot_pins_when_view_was_near_bottom() {
        let snapshot = ChatScrollSnapshot {
            value: 1_954.0,
            pinned_to_bottom: chat_scroll_is_pinned_to_bottom(1_954.0, 0.0, 2_500.0, 500.0),
        };

        assert!(snapshot.pinned_to_bottom);
        assert_eq!(
            restored_chat_scroll_value(snapshot, 0.0, 3_000.0, 500.0),
            2_500.0
        );
    }

    #[test]
    fn chat_scroll_snapshot_preserves_scrolled_up_position() {
        let snapshot = ChatScrollSnapshot {
            value: 640.0,
            pinned_to_bottom: chat_scroll_is_pinned_to_bottom(640.0, 0.0, 2_500.0, 500.0),
        };

        assert!(!snapshot.pinned_to_bottom);
        assert_eq!(
            restored_chat_scroll_value(snapshot, 0.0, 3_000.0, 500.0),
            640.0
        );
    }

    #[test]
    fn chat_scroll_restore_retries_across_late_layout_passes() {
        let snapshot = ChatScrollSnapshot {
            value: 640.0,
            pinned_to_bottom: false,
        };

        assert_eq!(
            chat_scroll_restore_layout_passes(snapshot),
            CHAT_SCROLL_RESTORE_LAYOUT_PASSES
        );
    }

    #[test]
    fn chat_status_banner_does_not_insert_working_row_above_existing_timeline() {
        assert_eq!(
            chat_status_banner_kind(
                &CodexStartupState::Ready,
                Some(Duration::from_secs(12)),
                true
            ),
            ChatStatusBannerKind::None
        );
    }

    #[test]
    fn chat_status_banner_keeps_codex_starting_message_above_existing_rows() {
        assert_eq!(
            chat_status_banner_kind(
                &CodexStartupState::Loading {
                    message: "Starting Codex...".to_owned(),
                },
                None,
                true,
            ),
            ChatStatusBannerKind::Startup
        );
    }

    #[test]
    fn queue_thread_command_dedupes_adjacent_duplicates() {
        let pending = RefCell::new(HashMap::<i64, Vec<String>>::new());
        queue_thread_command(&pending, 3, "/thinking high".to_owned());
        queue_thread_command(&pending, 3, "/thinking high".to_owned());

        let flushed = flush_pending_commands_for_send(&pending, 3);

        assert_eq!(flushed, vec!["/thinking high".to_owned()]);
    }

    #[test]
    fn queue_thread_command_replaces_prior_model_and_thinking_commands() {
        let pending = RefCell::new(HashMap::<i64, Vec<String>>::new());
        queue_thread_command(&pending, 5, "/model gpt-5.6-sol".to_owned());
        queue_thread_command(&pending, 5, "/thinking medium".to_owned());
        queue_thread_command(&pending, 5, "/model gpt-5.6-luna".to_owned());
        queue_thread_command(&pending, 5, "/thinking high".to_owned());

        let flushed = flush_pending_commands_for_send(&pending, 5);

        assert_eq!(
            flushed,
            vec![
                "/model gpt-5.6-luna".to_owned(),
                "/thinking high".to_owned()
            ]
        );
    }

    #[test]
    fn queue_archcar_input_ignores_blank_messages() {
        let app_state = AppState::new(
            archductor_core::paths::AppPaths::from_env(),
            Some("berlin".to_owned()),
            crate::state::WorkspaceTab::Chats,
            AppPage::Workspace,
        );

        queue_archcar_input(
            &app_state,
            5,
            "   ".to_owned(),
            None,
            ArchcarInputKind::User,
            SessionKind::Codex,
        );

        assert_eq!(app_state.queued_chat_inputs_count(5), 0);
    }

    #[test]
    fn submitted_user_input_texts_exclude_startup_queue_until_sent() {
        let app_state = AppState::new(
            archductor_core::paths::AppPaths::from_env(),
            Some("berlin".to_owned()),
            crate::state::WorkspaceTab::Chats,
            AppPage::Workspace,
        );
        let starting_queue = RefCell::new(HashMap::<i64, Vec<QueuedArchcarInput>>::new());
        let inflight = RefCell::new(HashMap::from([(
            11,
            PendingArchcarAction::UserSend {
                thread_id: 7,
                session_id: 9,
                input: "[metadata]\nreal inflight".to_owned(),
                visible_input: Some("real inflight".to_owned()),
                kind: ArchcarInputKind::User,
                delivery: ArchcarInputDelivery::Auto,
                checkpoint_id: None,
                session_kind: SessionKind::Codex,
            },
        )]));
        queue_archcar_input(
            &app_state,
            7,
            "queued while busy".to_owned(),
            None,
            ArchcarInputKind::User,
            SessionKind::Codex,
        );
        queue_pending_archcar_input(
            &starting_queue,
            7,
            QueuedArchcarInput {
                input: "[metadata]\nqueued while starting".to_owned(),
                visible_input: Some("queued while starting".to_owned()),
                kind: ArchcarInputKind::User,
                session_kind: SessionKind::Codex,
            },
        );

        let submitted = submitted_user_input_texts_for_thread(7, &starting_queue, &inflight, &[]);

        assert_eq!(submitted, vec!["real inflight".to_owned()]);
        assert_eq!(app_state.queued_chat_inputs_count(7), 1);
    }

    #[test]
    fn submitted_user_input_texts_skip_persisted_messages() {
        let app_state = AppState::new(
            archductor_core::paths::AppPaths::from_env(),
            Some("berlin".to_owned()),
            crate::state::WorkspaceTab::Chats,
            AppPage::Workspace,
        );
        let starting_queue = RefCell::new(HashMap::<i64, Vec<QueuedArchcarInput>>::new());
        let inflight = RefCell::new(HashMap::<u64, PendingArchcarAction>::new());
        queue_archcar_input(
            &app_state,
            7,
            "already persisted".to_owned(),
            None,
            ArchcarInputKind::User,
            SessionKind::Codex,
        );
        let persisted = vec![chat_message_record(
            1,
            "user",
            "already persisted",
            "user_send",
        )];

        let submitted =
            submitted_user_input_texts_for_thread(7, &starting_queue, &inflight, &persisted);

        assert!(submitted.is_empty());
        assert_eq!(app_state.queued_chat_inputs_count(7), 1);
    }

    #[test]
    fn immediate_inflight_input_renders_as_optimistic_user_boundary() {
        let inflight = RefCell::new(HashMap::from([(
            12,
            PendingArchcarAction::UserSend {
                thread_id: 7,
                session_id: 9,
                input: "change course now".to_owned(),
                visible_input: None,
                kind: ArchcarInputKind::User,
                delivery: ArchcarInputDelivery::Immediate,
                checkpoint_id: None,
                session_kind: SessionKind::Codex,
            },
        )]));
        let submitted =
            submitted_user_input_texts_for_thread(7, &RefCell::new(HashMap::new()), &inflight, &[]);
        assert_eq!(submitted, vec!["change course now".to_owned()]);

        let timeline = chat_structured_items_for_render(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            submitted,
        );
        assert_eq!(
            timeline,
            vec![ChatTimelineItem::OptimisticUserInput(
                "change course now".to_owned()
            )]
        );

        let persisted = vec![chat_message_record(
            1,
            "user",
            "change course now",
            "user_send",
        )];
        assert!(submitted_user_input_texts_for_thread(
            7,
            &RefCell::new(HashMap::new()),
            &inflight,
            &persisted
        )
        .is_empty());
        assert!(
            inflight.borrow().contains_key(&12),
            "render projection must not remove inflight actions; lifecycle responses own cleanup"
        );
    }

    #[test]
    fn submitted_auto_input_renders_as_optimistic_user_boundary() {
        let app_state = AppState::new(
            archductor_core::paths::AppPaths::from_env(),
            Some("berlin".to_owned()),
            crate::state::WorkspaceTab::Chats,
            AppPage::Workspace,
        );
        queue_archcar_input(
            &app_state,
            7,
            "still only queued locally".to_owned(),
            None,
            ArchcarInputKind::User,
            SessionKind::Codex,
        );
        let starting_queue = RefCell::new(HashMap::<i64, Vec<QueuedArchcarInput>>::new());
        let inflight = RefCell::new(HashMap::from([(
            12,
            PendingArchcarAction::UserSend {
                thread_id: 7,
                session_id: 9,
                input: "normal send".to_owned(),
                visible_input: None,
                kind: ArchcarInputKind::User,
                delivery: ArchcarInputDelivery::Auto,
                checkpoint_id: None,
                session_kind: SessionKind::Codex,
            },
        )]));

        let submitted = submitted_user_input_texts_for_thread(7, &starting_queue, &inflight, &[]);

        assert_eq!(submitted, vec!["normal send".to_owned()]);
        assert_eq!(app_state.queued_chat_inputs_count(7), 1);
        let timeline = chat_structured_items_for_render(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            submitted,
        );
        assert_eq!(
            timeline,
            vec![ChatTimelineItem::OptimisticUserInput(
                "normal send".to_owned()
            )]
        );
    }

    #[test]
    fn provider_boundary_clears_inflight_user_sends_for_thread() {
        let inflight = RefCell::new(HashMap::from([
            (
                12,
                PendingArchcarAction::UserSend {
                    thread_id: 7,
                    session_id: 9,
                    input: "first".to_owned(),
                    visible_input: None,
                    kind: ArchcarInputKind::User,
                    delivery: ArchcarInputDelivery::Auto,
                    checkpoint_id: None,
                    session_kind: SessionKind::Codex,
                },
            ),
            (
                13,
                PendingArchcarAction::UserSend {
                    thread_id: 8,
                    session_id: 10,
                    input: "other thread".to_owned(),
                    visible_input: None,
                    kind: ArchcarInputKind::User,
                    delivery: ArchcarInputDelivery::Auto,
                    checkpoint_id: None,
                    session_kind: SessionKind::Claude,
                },
            ),
        ]));

        clear_inflight_user_sends_for_thread(&inflight, 7);

        assert!(!inflight.borrow().contains_key(&12));
        assert!(inflight.borrow().contains_key(&13));
    }

    #[test]
    fn send_preprocessing_preserves_visible_input_for_hidden_context() {
        let thread = ChatThreadRecord {
            id: 7,
            workspace_id: 1,
            provider: "codex".to_owned(),
            title: DEFAULT_CHAT_TITLE_PREFIX.to_owned(),
            status: "active".to_owned(),
            native_thread_id: None,
            harness_metadata: None,
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
            archived_at: None,
        };

        let prepared = prepare_session_send_input(
            "fix the failing test",
            "starter-workspace",
            "feature",
            false,
            SessionKind::Codex,
            &thread,
            &[],
        );

        assert_ne!(prepared.input, "fix the failing test");
        assert!(prepared.input.contains("<archductor_hidden_instruction>"));
        assert!(prepared
            .input
            .contains("\"branch_name\":\"feature/short-kebab-name\""));
        assert_eq!(
            prepared.visible_input.as_deref(),
            Some("fix the failing test")
        );
    }

    #[test]
    fn send_preprocessing_preserves_visible_input_for_model_switch_context() {
        let thread = ChatThreadRecord {
            id: 7,
            workspace_id: 1,
            provider: "codex".to_owned(),
            title: "Fix Build".to_owned(),
            status: "active".to_owned(),
            native_thread_id: None,
            harness_metadata: None,
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
            archived_at: None,
        };
        let messages = vec![
            chat_message_record(1, "user", "original request", "user_send"),
            chat_message_record(
                2,
                "system",
                "Attached prior transcript",
                "model_switch_context",
            ),
        ];

        let prepared = prepare_session_send_input(
            "continue with codex",
            "starter-workspace",
            "feature",
            false,
            SessionKind::Codex,
            &thread,
            &messages,
        );

        assert!(prepared
            .input
            .starts_with("[Attachment: prior chat context]"));
        assert!(prepared.input.contains("Attached prior transcript"));
        assert_eq!(
            prepared.visible_input.as_deref(),
            Some("continue with codex")
        );
    }

    #[test]
    fn immediate_ack_does_not_reinsert_user_send_as_inflight() {
        let source = include_str!("session_surface.rs");
        let user_send_ack_body = source
            .split("Ok(ArchcarResponse::Ack) => {")
            .nth(4)
            .and_then(|tail| tail.split("Ok(other) =>").next())
            .expect("user-send ack branch should be present");

        assert!(
            !user_send_ack_body.contains("inflight_actions.borrow_mut().insert("),
            "accepted immediate user sends must leave inflight tracking so ready refresh can flush follow-ups"
        );
    }

    #[test]
    fn queue_archcar_input_preserves_trimmed_input_and_kind() {
        let app_state = AppState::new(
            archductor_core::paths::AppPaths::from_env(),
            Some("berlin".to_owned()),
            crate::state::WorkspaceTab::Chats,
            AppPage::Workspace,
        );

        queue_archcar_input(
            &app_state,
            8,
            "  review this diff  ".to_owned(),
            None,
            ArchcarInputKind::ReviewPrompt,
            SessionKind::Codex,
        );

        let items = app_state.queued_chat_inputs(8);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].input, "review this diff");
        assert_eq!(items[0].kind, ArchcarInputKind::ReviewPrompt);
    }

    #[test]
    fn refresh_path_flushes_pending_archcar_inputs() {
        let source = include_str!("session_surface.rs");
        let calls = source
            .match_indices("flush_pending_archcar_inputs(")
            .count();

        assert!(
            calls > 1,
            "queued archcar inputs must be flushed from the refresh/event path"
        );
    }

    #[test]
    fn pending_archcar_flush_marks_session_not_ready_after_send() {
        let source = include_str!("session_surface.rs");
        let flush_body = source
            .split("fn flush_pending_archcar_inputs(")
            .nth(1)
            .and_then(|tail| tail.split("fn queue_archcar_control_send(").next())
            .expect("flush_pending_archcar_inputs body should be present");

        assert!(
            flush_body
                .contains("note_archcar_ready(&mut ready_cache.borrow_mut(), session_id, false);"),
            "flushing one queued input must wait for the next ready event before sending another"
        );
    }

    #[test]
    fn pending_archcar_ensure_is_scoped_to_thread() {
        let pending = RefCell::new(HashMap::from([
            (
                1,
                PendingArchcarAction::EnsureWorkspace {
                    workspace: "berlin".to_owned(),
                    thread_id: Some(7),
                    kind: SessionKind::Codex,
                },
            ),
            (
                2,
                PendingArchcarAction::EnsureWorkspace {
                    workspace: "berlin".to_owned(),
                    thread_id: None,
                    kind: SessionKind::Codex,
                },
            ),
        ]));

        assert!(has_pending_archcar_ensure_for_thread(&pending, 7));
        assert!(!has_pending_archcar_ensure_for_thread(&pending, 8));
    }

    #[test]
    fn eager_chat_agent_start_requests_managed_thread_before_first_input() {
        let app_state = AppState::new(
            archductor_core::paths::AppPaths::from_env(),
            Some("berlin".to_owned()),
            crate::state::WorkspaceTab::Chats,
            AppPage::Workspace,
        );
        let ready_cache = RefCell::new(HashMap::new());
        let inflight = RefCell::new(HashMap::new());
        let startup_states = RefCell::new(HashMap::new());
        let working_threads = RefCell::new(HashMap::new());
        let codex_ready = RefCell::new(true);
        let requested = Rc::new(RefCell::new(Vec::new()));
        let requested_for_call = requested.clone();
        queue_archcar_input(
            &app_state,
            7,
            "first message waits for ready".to_owned(),
            None,
            ArchcarInputKind::User,
            SessionKind::Codex,
        );

        let outcome = eager_chat_agent_start(
            &app_state,
            &[],
            &ready_cache,
            &inflight,
            &startup_states,
            &working_threads,
            &codex_ready,
            &|| {},
            "berlin".to_owned(),
            7,
            SessionKind::Codex,
            |workspace, thread_id, harness| {
                requested_for_call
                    .borrow_mut()
                    .push((workspace, thread_id, harness.kind));
                true
            },
        );

        assert_eq!(outcome, EagerChatAgentStartOutcome::Requested);
        assert_eq!(
            requested.borrow().as_slice(),
            &[("berlin".to_owned(), 7, SessionKind::Codex)]
        );
        assert_eq!(
            app_state.chat_phase(&ChatUiTarget::Thread(7)),
            Some(ChatUiPhase::StartingAgent {
                provider: SessionKind::Codex,
            })
        );
        assert_eq!(
            startup_states.borrow().get(&7),
            Some(&CodexStartupState::Loading {
                message: "Starting agent...".to_owned(),
            })
        );
        assert_eq!(queued_chat_inputs_count(&app_state, 7), 1);
        assert!(!*codex_ready.borrow());
    }

    #[test]
    fn eager_chat_agent_start_dedupes_pending_thread_ensure() {
        let app_state = AppState::new(
            archductor_core::paths::AppPaths::from_env(),
            Some("berlin".to_owned()),
            crate::state::WorkspaceTab::Chats,
            AppPage::Workspace,
        );
        let ready_cache = RefCell::new(HashMap::new());
        let inflight = RefCell::new(HashMap::from([(
            55,
            PendingArchcarAction::EnsureWorkspace {
                workspace: "berlin".to_owned(),
                thread_id: Some(7),
                kind: SessionKind::Codex,
            },
        )]));
        let startup_states = RefCell::new(HashMap::new());
        let working_threads = RefCell::new(HashMap::new());
        let codex_ready = RefCell::new(true);

        let outcome = eager_chat_agent_start(
            &app_state,
            &[],
            &ready_cache,
            &inflight,
            &startup_states,
            &working_threads,
            &codex_ready,
            &|| {},
            "berlin".to_owned(),
            7,
            SessionKind::Codex,
            |_, _, _| panic!("pending ensure must not request again"),
        );

        assert_eq!(outcome, EagerChatAgentStartOutcome::Pending);
        assert_eq!(
            app_state.chat_phase(&ChatUiTarget::Thread(7)),
            Some(ChatUiPhase::StartingAgent {
                provider: SessionKind::Codex,
            })
        );
    }

    #[test]
    fn eager_chat_agent_start_request_failure_releases_starting_phase() {
        let app_state = AppState::new(
            archductor_core::paths::AppPaths::from_env(),
            Some("berlin".to_owned()),
            crate::state::WorkspaceTab::Chats,
            AppPage::Workspace,
        );
        let ready_cache = RefCell::new(HashMap::new());
        let inflight = RefCell::new(HashMap::new());
        let startup_states = RefCell::new(HashMap::new());
        let working_threads = RefCell::new(HashMap::new());
        let codex_ready = RefCell::new(true);
        let composer_updates = Cell::new(0);

        let outcome = eager_chat_agent_start(
            &app_state,
            &[],
            &ready_cache,
            &inflight,
            &startup_states,
            &working_threads,
            &codex_ready,
            &|| composer_updates.set(composer_updates.get() + 1),
            "berlin".to_owned(),
            7,
            SessionKind::Codex,
            |_, _, _| false,
        );

        assert_eq!(outcome, EagerChatAgentStartOutcome::RequestUnavailable);
        assert_eq!(
            app_state.chat_phase(&ChatUiTarget::Thread(7)),
            Some(ChatUiPhase::Ready)
        );
        assert_eq!(
            startup_states.borrow().get(&7),
            Some(&CodexStartupState::Error {
                message: "Request channel is closed. Reopen the workspace or restart the app."
                    .to_owned(),
            })
        );
        assert!(!*codex_ready.borrow());
        assert_eq!(composer_updates.get(), 2);
    }

    #[test]
    fn eager_chat_agent_start_skips_ready_managed_thread() {
        let app_state = AppState::new(
            archductor_core::paths::AppPaths::from_env(),
            Some("berlin".to_owned()),
            crate::state::WorkspaceTab::Chats,
            AppPage::Workspace,
        );
        let records = vec![process_record_with_thread(
            11,
            ProcessStatus::Running,
            Some(7),
            "codex",
        )];
        let ready_cache = RefCell::new(HashMap::from([(11, true)]));
        let inflight = RefCell::new(HashMap::new());
        let startup_states = RefCell::new(HashMap::new());
        let working_threads = RefCell::new(HashMap::new());
        let codex_ready = RefCell::new(false);

        let outcome = eager_chat_agent_start(
            &app_state,
            &records,
            &ready_cache,
            &inflight,
            &startup_states,
            &working_threads,
            &codex_ready,
            &|| {},
            "berlin".to_owned(),
            7,
            SessionKind::Codex,
            |_, _, _| panic!("ready thread must not request ensure"),
        );

        assert_eq!(outcome, EagerChatAgentStartOutcome::AlreadyReady);
        assert_eq!(
            app_state.chat_phase(&ChatUiTarget::Thread(7)),
            Some(ChatUiPhase::Ready)
        );
        assert!(*codex_ready.borrow());
    }

    #[test]
    fn eager_chat_agent_start_does_not_block_active_generation() {
        let app_state = AppState::new(
            archductor_core::paths::AppPaths::from_env(),
            Some("berlin".to_owned()),
            crate::state::WorkspaceTab::Chats,
            AppPage::Workspace,
        );
        let records = vec![process_record_with_thread(
            11,
            ProcessStatus::Running,
            Some(7),
            "codex",
        )];
        let ready_cache = RefCell::new(HashMap::new());
        let inflight = RefCell::new(HashMap::new());
        let startup_states = RefCell::new(HashMap::new());
        let working_threads = RefCell::new(HashMap::from([(7, Instant::now())]));
        let codex_ready = RefCell::new(false);

        let outcome = eager_chat_agent_start(
            &app_state,
            &records,
            &ready_cache,
            &inflight,
            &startup_states,
            &working_threads,
            &codex_ready,
            &|| {},
            "berlin".to_owned(),
            7,
            SessionKind::Codex,
            |_, _, _| panic!("active generation must not request ensure"),
        );

        assert_eq!(outcome, EagerChatAgentStartOutcome::AlreadyActive);
        assert_eq!(app_state.chat_phase(&ChatUiTarget::Thread(7)), None);
    }

    #[test]
    fn agent_switch_prefers_resume_record_over_handoff() {
        let mut codex_record = session_record(2, "/opt/bin/codex", ProcessStatus::Exited, None);
        codex_record.session_resume_id = Some("resume-1".to_owned());
        let mut claude_record = session_record(3, "/opt/bin/claude", ProcessStatus::Exited, None);
        claude_record.session_resume_id = Some("resume-2".to_owned());

        assert_eq!(
            resume_record_for_kind(&[codex_record], Some(2), SessionKind::Codex)
                .map(|record| record.id),
            Some(2)
        );
        assert_eq!(
            resume_record_for_kind(&[claude_record], Some(3), SessionKind::Claude)
                .map(|record| record.id),
            Some(3)
        );
    }

    #[test]
    fn agent_switch_treats_stopped_codex_without_resume_id_as_resume_candidate() {
        let codex_record = session_record(2, "/opt/bin/codex", ProcessStatus::Exited, None);

        assert_eq!(
            resume_record_for_kind(&[codex_record], Some(2), SessionKind::Codex)
                .map(|record| record.id),
            Some(2)
        );
    }

    #[test]
    fn agent_switch_does_not_treat_stopped_claude_without_resume_id_as_resume_candidate() {
        let claude_record = session_record(3, "/opt/bin/claude", ProcessStatus::Exited, None);

        assert_eq!(
            resume_record_for_kind(&[claude_record], Some(3), SessionKind::Claude)
                .map(|record| record.id),
            None
        );
    }

    #[test]
    fn staged_review_prompt_text_wraps_review_context() {
        let text = staged_review_prompt_text(
            "Address these open review comments for workspace berlin.\n- #1 src/lib.rs: fix it\n",
        );

        assert!(text.contains("[staged review prompt]"));
        assert!(text.contains("workspace berlin"));
        assert!(text.contains("#1 src/lib.rs"));
    }

    #[test]
    fn staged_review_status_text_summarizes_presence() {
        assert_eq!(
            staged_review_status_text(Some("  fix it  ")),
            "Staged review prompt ready (6 chars)."
        );
        assert_eq!(
            staged_review_status_text(Some("   ")),
            "No staged review prompt."
        );
    }

    #[test]
    fn session_transcript_group_text_joins_resumed_sessions() {
        let temp = tempfile::tempdir().unwrap();
        let first_log = temp.path().join("session-1.log");
        let second_log = temp.path().join("session-2.log");
        fs::write(&first_log, "first session\n").unwrap();
        fs::write(&second_log, "second session\n").unwrap();

        let mut first = session_record(1, "/opt/bin/claude", ProcessStatus::Exited, None);
        first.log_path = first_log;
        first.session_resume_id = Some("resume-1".to_owned());
        let mut second = session_record(2, "/opt/bin/claude", ProcessStatus::Running, None);
        second.log_path = second_log;
        second.session_resume_id = Some("resume-1".to_owned());
        let unrelated = session_record(3, "/opt/bin/codex", ProcessStatus::Running, None);

        let transcript =
            session_transcript_group_text(&[first, second.clone(), unrelated], &second);

        assert!(transcript.contains("first session"));
        assert!(transcript.contains("second session"));
        assert!(transcript.contains("[session resumed]"));
    }

    #[test]
    fn selected_session_surface_shows_harness_and_transcript() {
        let record = session_record(
            7,
            "/opt/bin/codex",
            ProcessStatus::Running,
            Some("plan=true;reasoning=high"),
        );

        let surface = format_selected_session_surface(
            &record,
            "hello\x1b[31m agent\x1b[0m\n",
            "working",
            true,
        );

        assert!(surface.contains("Session #7 - Codex"));
        assert!(surface.contains("State: working"));
        assert!(surface.contains("Attachment: attached"));
        assert!(surface.contains("Harness: plan=true;reasoning=high"));
        assert!(surface.contains("Command: /opt/bin/codex"));
        assert!(surface.contains("hello agent"));
    }

    #[test]
    fn selected_session_surface_marks_empty_saved_transcript() {
        let record = session_record(3, "claude", ProcessStatus::Exited, None);

        let surface = format_selected_session_surface(&record, "   ", "done", false);

        assert!(surface.contains("Session #3 - Claude"));
        assert!(surface.contains("Status: exited"));
        assert!(surface.contains("State: done"));
        assert!(surface.contains("Attachment: saved"));
        assert!(surface.contains("[no transcript output yet]"));
    }

    #[test]
    fn live_chat_uses_structured_store_not_session_log_reparse() {
        assert_eq!(live_chat_source(), LiveChatSource::StructuredStore);
    }

    #[test]
    fn session_input_log_text_persists_user_event_marker() {
        let text = session_input_log_text("memphis", 9, "cargo test");

        assert_eq!(
            text,
            "\n[user input memphis#9]\ncargo test\n[/user input]\n"
        );
    }

    #[test]
    fn live_session_append_keeps_user_and_review_markers_raw() {
        let user = live_session_append_text("[user input memphis#9]\ncargo test\n[/user input]\n");
        let review =
            live_session_append_text("[staged review prompt]\nFix CI\n[/staged review prompt]\n");

        assert_eq!(user, "[user input memphis#9]\ncargo test\n[/user input]\n");
        assert_eq!(
            review,
            "[staged review prompt]\nFix CI\n[/staged review prompt]\n"
        );
    }

    #[test]
    fn live_session_append_keeps_plain_output_raw() {
        let text = live_session_append_text("hello\x1b[31m agent\x1b[0m\n");

        assert_eq!(text, "hello agent\n");
    }

    #[test]
    fn selected_session_surface_renders_raw_transcript_without_semantic_labels() {
        let record = session_record(
            11,
            "cursor /tmp/workspace",
            ProcessStatus::Running,
            Some("fast=true"),
        );
        let transcript = "\
[session started] #11 kind=Cursor pid=1234
\n[archductor bootstrap for codex]
\n[tool bash]
\n[skill tests]
\n[user input memphis#11]
open the project
\n[staged review prompt]
Fix the failing checks
\nBuild succeeded
";

        let surface = format_selected_session_surface(&record, transcript, "waiting", true);

        assert!(surface.contains("[session started] #11 kind=Cursor pid=1234"));
        assert!(surface.contains("[archductor bootstrap for codex]"));
        assert!(surface.contains("[tool bash]"));
        assert!(surface.contains("[skill tests]"));
        assert!(surface.contains("[user input memphis#11]"));
        assert!(surface.contains("[staged review prompt]"));
        assert!(surface.contains("Build succeeded"));
        assert!(surface.contains("Events: raw transcript, "));
        assert!(!surface.contains("Tool\n[tool bash]"));
        assert!(!surface.contains("Agent\nBuild succeeded"));
    }

    #[test]
    fn transcript_events_keep_multiline_user_input_together() {
        let transcript = "\
[user input memphis#4]
first line
second line

third line
[/user input]
[session stopped] exit=0
";

        let events = parse_session_transcript_events(transcript);

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].role, SessionTranscriptRole::User);
        assert_eq!(events[0].body, "first line\nsecond line\n\nthird line");
        assert_eq!(events[1].role, SessionTranscriptRole::System);
    }

    #[test]
    fn transcript_events_keep_fenced_review_prompt_together() {
        let transcript = "\
[staged review prompt]
Address review comments.

- src/lib.rs: fix parser
- src/main.rs: wire UI
[/staged review prompt]
agent response
";

        let events = parse_session_transcript_events(transcript);

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].role, SessionTranscriptRole::ReviewPrompt);
        assert_eq!(
            events[0].body,
            "Address review comments.\n\n- src/lib.rs: fix parser\n- src/main.rs: wire UI"
        );
        assert_eq!(events[1].role, SessionTranscriptRole::Agent);
        assert_eq!(events[1].body, "agent response");
    }

    #[test]
    fn transcript_events_skip_codex_raw_and_screen_diagnostic_blocks() {
        let transcript = "\
[user input memphis#4]
run tests
[/user input]
[codex raw]
noise
[/codex raw]
[codex screen]
╭─ You ─╮
│ run tests
╰────
╭─ Codex ─╮
│ Running now.
╰────
[/codex screen]
[codex screen]
╭─ You ─╮
│ run tests
╰────
╭─ Codex ─╮
│ Running now.
│ Tests passed.
╰────
[/codex screen]
";

        let events = parse_session_transcript_events(transcript);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].role, SessionTranscriptRole::User);
        assert_eq!(events[0].body, "run tests");
    }

    #[test]
    fn transcript_events_ignore_codex_screen_blocks_for_normal_session_semantics() {
        let transcript = "\
[user input memphis#4]
run tests
[/user input]
[codex screen]
╭─ Codex ─╮
│ Parsed from screen.
╰────
[/codex screen]
";

        let events = parse_session_transcript_events(transcript);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].role, SessionTranscriptRole::User);
        assert_eq!(events[0].body, "run tests");
    }

    #[test]
    fn transcript_events_are_limited_to_recent_history() {
        let transcript = (0..130)
            .map(|index| format!("[user input memphis#1]\ncommand {index}\n"))
            .collect::<String>();

        let events = parse_session_transcript_events(&transcript);

        assert_eq!(events.len(), SESSION_TAIL_HISTORY);
        assert_eq!(events.first().unwrap().body, "command 10");
        assert_eq!(events.last().unwrap().body, "command 129");
    }

    #[test]
    fn transcript_ignores_codex_startup_noise_before_first_real_reply() {
        let transcript = "\
OpenAI Codex v0.43.0
Update available! 0.44.0
Tip: Use /personality to customize how Codex communicates.
• Booting MCP server: codex_apps
• You have 2 usage limit resets available. Run /usage to use one.
[user input memphis#1]
fix the formatting bug
[/user input]
Thinking...
I fixed the formatting bug.
";

        let events = parse_session_transcript_events(transcript);

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].role, SessionTranscriptRole::User);
        assert_eq!(events[0].body, "fix the formatting bug");
        assert_eq!(events[1].role, SessionTranscriptRole::Agent);
        assert_eq!(events[1].body, "Thinking...\nI fixed the formatting bug.");
    }

    #[test]
    fn transcript_classifies_raw_tool_and_skill_lines() {
        let transcript = "\
Read README.md
Using the writing-plans skill to create the implementation plan.
Reading the openai-docs skill to answer with citations.
Update src/main.rs
I finished the pass.
";

        let events = parse_session_transcript_events(transcript);

        assert_eq!(events.len(), 5);
        assert_eq!(events[0].role, SessionTranscriptRole::Tool);
        assert_eq!(events[0].body, "Read README.md");
        assert_eq!(events[1].role, SessionTranscriptRole::Skill);
        assert_eq!(
            events[1].body,
            "Using the writing-plans skill to create the implementation plan."
        );
        assert_eq!(events[2].role, SessionTranscriptRole::Skill);
        assert_eq!(
            events[2].body,
            "Reading the openai-docs skill to answer with citations."
        );
        assert_eq!(events[3].role, SessionTranscriptRole::Tool);
        assert_eq!(events[3].body, "Update src/main.rs");
        assert_eq!(events[4].role, SessionTranscriptRole::Agent);
        assert_eq!(events[4].body, "I finished the pass.");
    }

    #[test]
    fn transcript_classifies_skill_lines_without_literal_skill_word() {
        let transcript = "\
Using writing-plans to create the implementation plan.
";

        let events = parse_session_transcript_events(transcript);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].role, SessionTranscriptRole::Skill);
        assert_eq!(
            events[0].body,
            "Using writing-plans to create the implementation plan."
        );
    }

    #[test]
    fn transcript_groups_tool_and_skill_result_blocks() {
        let transcript = "\
Read SKILL.md (graphify), SKILL.md (skill-creator)
---
name: graphify
---
Ran cargo test -p archductor-core codex_tui
running 23 tests
test result: ok. 23 passed
• Edited crates/core/src/codex_tui.rs (+12 -3)
    378  context
    379 +new line
    381 -old line
I summarized the result.
";

        let events = parse_session_transcript_events(transcript);

        assert_eq!(events.len(), 4);
        assert_eq!(events[0].role, SessionTranscriptRole::Skill);
        assert_eq!(
            events[0].body,
            "Read SKILL.md (graphify), SKILL.md (skill-creator)\n---\nname: graphify\n---"
        );
        assert_eq!(events[1].role, SessionTranscriptRole::Tool);
        assert_eq!(
            events[1].body,
            "Ran cargo test -p archductor-core codex_tui\nrunning 23 tests\ntest result: ok. 23 passed"
        );
        assert_eq!(events[2].role, SessionTranscriptRole::Tool);
        assert_eq!(
            events[2].body,
            "• Edited crates/core/src/codex_tui.rs (+12 -3)\n    378  context\n    379 +new line\n    381 -old line"
        );
        assert_eq!(events[3].role, SessionTranscriptRole::Agent);
        assert_eq!(events[3].body, "I summarized the result.");
    }

    #[test]
    fn transcript_tool_and_skill_blocks_render_as_inline_events() {
        let skill = SessionTranscriptEvent {
            role: SessionTranscriptRole::Skill,
            body: "Read SKILL.md (graphify)\n---\nname: graphify\n---".to_owned(),
        };
        let command = SessionTranscriptEvent {
            role: SessionTranscriptRole::Tool,
            body: "Ran cargo test\nok".to_owned(),
        };
        let edit = SessionTranscriptEvent {
            role: SessionTranscriptRole::Tool,
            body: "Edited crates/core/src/codex_tui.rs (+12 -3)\n    378  context\n    379 +added\n    381 -deleted".to_owned(),
        };

        let skill_events = session_transcript_inline_events(&skill);
        let command_events = session_transcript_inline_events(&command);
        let edit_events = session_transcript_inline_events(&edit);

        assert_eq!(skill_events[0].kind, CodexInlineEventKind::Skill);
        assert_eq!(skill_events[0].title, "Read SKILL.md for graphify");
        assert!(skill_events[0]
            .body
            .as_deref()
            .unwrap()
            .contains("name: graphify"));
        assert_eq!(command_events[0].kind, CodexInlineEventKind::Tool);
        assert_eq!(command_events[0].title, "cargo test");
        assert!(command_events[0].body.as_deref().unwrap().contains("ok"));
        assert_eq!(edit_events[0].kind, CodexInlineEventKind::Tool);
        assert_eq!(edit_events[0].title, "Edited crates/core/src/codex_tui.rs");
        assert_eq!(edit_events[0].subtitle.as_deref(), Some("+12 -3"));
        assert!(edit_events[0]
            .body
            .as_deref()
            .unwrap()
            .contains("379 +added"));
        let change =
            parse_codex_file_change_block(edit_events[0].body.as_deref().unwrap()).unwrap();
        assert_eq!(change.lines.len(), 3);
        assert_eq!(change.lines[1].new_line, Some(379));
        assert_eq!(change.lines[1].content, "added");
    }

    #[test]
    fn transcript_changed_path_blocks_render_as_edit_events_with_diff_body() {
        let transcript = "\
changed /home/kitts/archductor/workspaces/conductor-arch/nanjing/docs/harness-smoke-note.md
diff --git a/docs/harness-smoke-note.md b/docs/harness-smoke-note.md
@@ -1 +1 @@
-old note
+new note
";

        let events = parse_session_transcript_events(transcript);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].role, SessionTranscriptRole::Tool);
        let inline_events = session_transcript_inline_events(&events[0]);
        assert_eq!(inline_events.len(), 1);
        assert_eq!(
            inline_events[0].title,
            "Edited /home/kitts/archductor/workspaces/conductor-arch/nanjing/docs/harness-smoke-note.md"
        );
        assert_eq!(
            inline_event_chip_name(&inline_events[0]),
            inline_events[0].title
        );
        assert_eq!(inline_events[0].subtitle.as_deref(), None);
        let body = inline_events[0].body.as_deref().unwrap();
        assert!(body.contains("@@ -1 +1 @@"));
        assert!(body.contains("-old note"));
        assert!(body.contains("+new note"));
    }

    #[test]
    fn chat_timeline_keeps_messages_and_events_in_persisted_order() {
        let messages = vec![
            ChatMessageRecord {
                id: 30,
                thread_id: 7,
                role: "user".to_owned(),
                content: "run tests".to_owned(),
                source: "user_send".to_owned(),
                timeline_seq: Some(1),
                created_at: "now".to_owned(),
                updated_at: "now".to_owned(),
            },
            ChatMessageRecord {
                id: 10,
                thread_id: 7,
                role: "agent".to_owned(),
                content: "tests passed".to_owned(),
                source: "agent_reply".to_owned(),
                timeline_seq: Some(3),
                created_at: "now".to_owned(),
                updated_at: "now".to_owned(),
            },
        ];
        let events = vec![ChatEventRecord {
            id: 99,
            thread_id: 7,
            process_id: Some(5),
            kind: "tool".to_owned(),
            title: "cargo test".to_owned(),
            body: "ok".to_owned(),
            path: None,
            payload_json: r#"{"type":"tool","title":"cargo test","body":"ok"}"#.to_owned(),
            timeline_seq: 2,
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
        }];

        let timeline = merge_chat_timeline_for_render(messages, events);

        assert_eq!(timeline.len(), 3);
        assert!(matches!(timeline[0], ChatTimelineItem::Message(_)));
        assert!(matches!(timeline[1], ChatTimelineItem::Event(_)));
        assert!(matches!(timeline[2], ChatTimelineItem::Message(_)));
    }

    #[test]
    fn chat_render_skips_blank_persisted_user_bubbles() {
        let messages = vec![ChatMessageRecord {
            id: 30,
            thread_id: 7,
            role: "user".to_owned(),
            content: "   ".to_owned(),
            source: "user_send".to_owned(),
            timeline_seq: Some(1),
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
        }];

        let timeline = chat_structured_items_for_render(
            messages,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );

        assert!(timeline.is_empty());
    }

    #[test]
    fn chat_render_keeps_local_queue_out_of_chat_timeline() {
        let messages = vec![ChatMessageRecord {
            id: 30,
            thread_id: 7,
            role: "agent".to_owned(),
            content: "previous answer".to_owned(),
            source: "agent_reply".to_owned(),
            timeline_seq: Some(1),
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
        }];
        let provider_item = ProviderProjectionItem {
            id: "codex:thread-7:tool-1".to_owned(),
            sequence: 2,
            timeline_seq: None,
            category: ProviderProjectionCategory::NativeTool,
            render_class: ProjectionRenderClass::ToolCard,
            title: "Bash".to_owned(),
            body: "running tests".to_owned(),
            status: ProviderProjectionStatus::Complete,
            stream_state: ProviderProjectionStreamState::Complete,
            parent_id: None,
            nested_thread_id: None,
            raw_payload: None,
            inspectable: false,
        };

        let timeline = chat_structured_items_for_render(
            messages,
            Vec::new(),
            vec![provider_item],
            vec!["new user message".to_owned()],
            Vec::new(),
        );

        assert!(matches!(timeline[0], ChatTimelineItem::Message(_)));
        assert!(matches!(
            timeline[1],
            ChatTimelineItem::ProviderProjection(_)
        ));
        assert_eq!(timeline.len(), 2);
    }

    #[test]
    fn pending_user_inputs_render_in_composer_overlay_not_chat_timeline() {
        let source = include_str!("session_surface.rs");
        assert!(source.contains("\"chat-queue-overlay\""));
        assert!(source.contains("queued_composer_overlay_items_for_thread"));
        let removed_helper = concat!("fn append", "_pending_user_input_rows");
        assert!(!source.contains(removed_helper));
    }

    #[test]
    fn chat_render_places_unsequenced_user_messages_after_provider_events() {
        let messages = vec![ChatMessageRecord {
            id: 30,
            thread_id: 7,
            role: "user".to_owned(),
            content: "new user message".to_owned(),
            source: "user_send".to_owned(),
            timeline_seq: None,
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
        }];
        let provider_item = ProviderProjectionItem {
            id: "codex:thread-7:tool-1".to_owned(),
            sequence: 2,
            timeline_seq: None,
            category: ProviderProjectionCategory::NativeTool,
            render_class: ProjectionRenderClass::ToolCard,
            title: "Bash".to_owned(),
            body: "running tests".to_owned(),
            status: ProviderProjectionStatus::Complete,
            stream_state: ProviderProjectionStreamState::Complete,
            parent_id: None,
            nested_thread_id: None,
            raw_payload: None,
            inspectable: false,
        };

        let timeline = chat_structured_items_for_render(
            messages,
            Vec::new(),
            vec![provider_item],
            Vec::new(),
            Vec::new(),
        );

        assert!(matches!(
            timeline[0],
            ChatTimelineItem::ProviderProjection(_)
        ));
        assert!(matches!(timeline[1], ChatTimelineItem::Message(_)));
    }

    #[test]
    fn chat_render_keeps_provider_events_after_persisted_history() {
        let messages = vec![ChatMessageRecord {
            id: 30,
            thread_id: 7,
            role: "agent".to_owned(),
            content: "older persisted answer".to_owned(),
            source: "agent_reply".to_owned(),
            timeline_seq: Some(500),
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
        }];
        let provider_item = ProviderProjectionItem {
            id: "codex:thread-7:tool-1".to_owned(),
            sequence: 2,
            timeline_seq: None,
            category: ProviderProjectionCategory::NativeTool,
            render_class: ProjectionRenderClass::ToolCard,
            title: "Bash".to_owned(),
            body: "running tests".to_owned(),
            status: ProviderProjectionStatus::Complete,
            stream_state: ProviderProjectionStreamState::Complete,
            parent_id: None,
            nested_thread_id: None,
            raw_payload: None,
            inspectable: false,
        };

        let timeline = chat_structured_items_for_render(
            messages,
            Vec::new(),
            vec![provider_item],
            Vec::new(),
            Vec::new(),
        );

        assert!(matches!(timeline[0], ChatTimelineItem::Message(_)));
        assert!(matches!(
            timeline[1],
            ChatTimelineItem::ProviderProjection(_)
        ));
    }

    #[test]
    fn chat_render_anchors_persisted_user_messages_to_provider_user_events() {
        let messages = vec![
            ChatMessageRecord {
                id: 30,
                thread_id: 7,
                role: "user".to_owned(),
                content: "first prompt".to_owned(),
                source: "user_send".to_owned(),
                timeline_seq: Some(1),
                created_at: "now".to_owned(),
                updated_at: "now".to_owned(),
            },
            ChatMessageRecord {
                id: 31,
                thread_id: 7,
                role: "user".to_owned(),
                content: "second prompt".to_owned(),
                source: "user_send".to_owned(),
                timeline_seq: Some(2),
                created_at: "now".to_owned(),
                updated_at: "now".to_owned(),
            },
        ];
        let provider_items = vec![
            ProviderProjectionItem {
                id: "codex:thread-7:input-1".to_owned(),
                sequence: 1,
                timeline_seq: None,
                category: ProviderProjectionCategory::UserMessage,
                render_class: ProjectionRenderClass::UserChat,
                title: "User".to_owned(),
                body: "first prompt".to_owned(),
                status: ProviderProjectionStatus::Complete,
                stream_state: ProviderProjectionStreamState::Complete,
                parent_id: None,
                nested_thread_id: None,
                raw_payload: None,
                inspectable: false,
            },
            ProviderProjectionItem {
                id: "codex:thread-7:tool-1".to_owned(),
                sequence: 2,
                timeline_seq: None,
                category: ProviderProjectionCategory::NativeTool,
                render_class: ProjectionRenderClass::ToolCard,
                title: "Bash".to_owned(),
                body: "first output".to_owned(),
                status: ProviderProjectionStatus::Complete,
                stream_state: ProviderProjectionStreamState::Complete,
                parent_id: None,
                nested_thread_id: None,
                raw_payload: None,
                inspectable: false,
            },
            ProviderProjectionItem {
                id: "codex:thread-7:input-2".to_owned(),
                sequence: 3,
                timeline_seq: None,
                category: ProviderProjectionCategory::UserMessage,
                render_class: ProjectionRenderClass::UserChat,
                title: "User".to_owned(),
                body: "second prompt".to_owned(),
                status: ProviderProjectionStatus::Complete,
                stream_state: ProviderProjectionStreamState::Complete,
                parent_id: None,
                nested_thread_id: None,
                raw_payload: None,
                inspectable: false,
            },
            ProviderProjectionItem {
                id: "codex:thread-7:tool-2".to_owned(),
                sequence: 4,
                timeline_seq: None,
                category: ProviderProjectionCategory::NativeTool,
                render_class: ProjectionRenderClass::ToolCard,
                title: "Bash".to_owned(),
                body: "second output".to_owned(),
                status: ProviderProjectionStatus::Complete,
                stream_state: ProviderProjectionStreamState::Complete,
                parent_id: None,
                nested_thread_id: None,
                raw_payload: None,
                inspectable: false,
            },
        ];

        let timeline = chat_structured_items_for_render(
            messages,
            Vec::new(),
            provider_items,
            Vec::new(),
            Vec::new(),
        );

        assert!(matches!(timeline[0], ChatTimelineItem::Message(_)));
        assert!(matches!(
            timeline[1],
            ChatTimelineItem::ProviderProjection(_)
        ));
        assert!(matches!(timeline[2], ChatTimelineItem::Message(_)));
        assert!(matches!(
            timeline[3],
            ChatTimelineItem::ProviderProjection(_)
        ));
    }

    #[test]
    fn chat_render_anchors_unsequenced_user_messages_before_provider_generation() {
        let messages = vec![ChatMessageRecord {
            id: 30,
            thread_id: 7,
            role: "user".to_owned(),
            content: "new user message".to_owned(),
            source: "user_send".to_owned(),
            timeline_seq: None,
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
        }];
        let provider_items = vec![
            ProviderProjectionItem {
                id: "codex:thread-7:input-1".to_owned(),
                sequence: 1,
                timeline_seq: None,
                category: ProviderProjectionCategory::UserMessage,
                render_class: ProjectionRenderClass::UserChat,
                title: "User".to_owned(),
                body: "new user message".to_owned(),
                status: ProviderProjectionStatus::Complete,
                stream_state: ProviderProjectionStreamState::Complete,
                parent_id: None,
                nested_thread_id: None,
                raw_payload: None,
                inspectable: false,
            },
            ProviderProjectionItem {
                id: "codex:thread-7:assistant-1".to_owned(),
                sequence: 2,
                timeline_seq: None,
                category: ProviderProjectionCategory::AssistantMessage,
                render_class: ProjectionRenderClass::AssistantChat,
                title: "Assistant".to_owned(),
                body: "working on it".to_owned(),
                status: ProviderProjectionStatus::Complete,
                stream_state: ProviderProjectionStreamState::Complete,
                parent_id: None,
                nested_thread_id: None,
                raw_payload: None,
                inspectable: false,
            },
        ];

        let timeline = chat_structured_items_for_render(
            messages,
            Vec::new(),
            provider_items,
            Vec::new(),
            Vec::new(),
        );

        assert!(matches!(timeline[0], ChatTimelineItem::Message(_)));
        assert!(matches!(
            timeline[1],
            ChatTimelineItem::ProviderProjection(_)
        ));
        assert_eq!(timeline.len(), 2);
    }

    #[test]
    fn canonical_provider_tool_events_project_as_tool_cards_not_assistant_chat() {
        let record = provider_event_record(ProviderEventKind::Tool, ProviderEventPhase::Completed);
        let projection = provider_projection_from_records(&[record]);

        assert_eq!(projection.items.len(), 1);
        assert_eq!(
            projection.items[0].render_class,
            ProjectionRenderClass::ToolCard
        );
        assert_ne!(
            projection.items[0].render_class,
            ProjectionRenderClass::AssistantChat
        );
        assert_eq!(projection.items[0].title, "Bash");
        assert_eq!(projection.items[0].body, "cargo test passed");
    }

    #[test]
    fn provider_projection_uses_provider_item_identity_across_phase_updates() {
        let mut streaming = provider_event_record(
            ProviderEventKind::AssistantOutput,
            ProviderEventPhase::Delta,
        );
        streaming.identity_key = "codex:thread-7:msg-1:delta".to_owned();
        streaming.provider_item_id = Some("msg-1".to_owned());
        streaming.normalized_payload = serde_json::json!({
            "title": "Assistant",
            "body": "partial"
        });
        let mut completed = streaming.clone();
        completed.identity_key = "codex:thread-7:msg-1:completed".to_owned();
        completed.phase = ProviderEventPhase::Completed;
        completed.received_sequence = 4;
        completed.normalized_payload = serde_json::json!({
            "title": "Assistant",
            "body": "complete"
        });

        let projection = provider_projection_from_records(&[streaming, completed]);

        assert_eq!(projection.items.len(), 1);
        assert_eq!(projection.items[0].id, "codex:thread-7:msg-1");
        assert_eq!(projection.items[0].body, "complete");
        assert_eq!(
            projection.items[0].status,
            ProviderProjectionStatus::Complete
        );
    }

    #[test]
    fn provider_projection_maps_subtype_specific_render_classes() {
        let cases = [
            (
                ProviderEventKind::FileSystem,
                "file_write",
                ProviderProjectionCategory::FileWrite,
                ProjectionRenderClass::FileCard,
            ),
            (
                ProviderEventKind::SkillPluginHook,
                "plugin",
                ProviderProjectionCategory::Plugin,
                ProjectionRenderClass::PluginCard,
            ),
            (
                ProviderEventKind::SkillPluginHook,
                "hook",
                ProviderProjectionCategory::Hook,
                ProjectionRenderClass::HookCard,
            ),
            (
                ProviderEventKind::WebBrowserMedia,
                "image",
                ProviderProjectionCategory::Image,
                ProjectionRenderClass::ImageCard,
            ),
            (
                ProviderEventKind::LimitFailure,
                "rate_limit",
                ProviderProjectionCategory::RateLimit,
                ProjectionRenderClass::WarningCard,
            ),
        ];

        for (kind, subtype, category, render_class) in cases {
            let mut record = provider_event_record(kind, ProviderEventPhase::Completed);
            record.provider_subtype = Some(subtype.to_owned());
            let projection = provider_projection_from_records(&[record]);

            assert_eq!(projection.items[0].category, category);
            assert_eq!(projection.items[0].render_class, render_class);
        }
    }

    #[test]
    fn provider_projection_action_card_text_hides_raw_payload() {
        let mut record = provider_event_record(
            ProviderEventKind::CommandProcess,
            ProviderEventPhase::Failed,
        );
        record.raw_json = serde_json::json!({
            "type": "command",
            "secret": "raw-secret"
        });
        let projection = provider_projection_from_records(&[record]);

        let status = provider_projection_status_line(&projection.items[0]);
        let text = provider_projection_card_text(&projection.items[0]);

        assert!(status.contains("Failed"));
        assert!(!status.contains("Complete"));
        assert!(!status.contains("Streaming"));
        assert_eq!(
            provider_projection_status_css_class(projection.items[0].status),
            "status-error"
        );
        assert!(!text.contains("raw-secret"));
        assert!(!text.contains("\"type\": \"command\""));
        assert!(!text.contains("[redacted]"));
    }

    #[test]
    fn provider_projection_action_cards_do_not_render_running_streaming_chrome() {
        let mut record =
            provider_event_record(ProviderEventKind::Tool, ProviderEventPhase::Progress);
        record.normalized_payload = serde_json::json!({
            "title": "Read",
            "body": "crates/core/src/lib.rs"
        });
        let projection = provider_projection_from_records(&[record]);

        let item = &projection.items[0];

        assert!(!provider_projection_item_shows_status_chrome(item));
        let text = provider_projection_card_text(item);
        assert!(text.starts_with("Read\ncrates/core/src/lib.rs"));
        assert!(!text.contains("\"type\": \"tool_result\""));
        assert!(!text.contains("Running"));
        assert!(!text.contains("Streaming"));
    }

    #[test]
    fn provider_projection_action_items_render_as_inline_event_chips_without_json() {
        let mut record = provider_event_record(ProviderEventKind::Mcp, ProviderEventPhase::Started);
        record.provider_subtype = Some("item/mcpToolCall/started".to_owned());
        record.normalized_payload = serde_json::json!({
            "title": "get_wiki_page",
            "body": ""
        });
        record.raw_json = serde_json::json!({
            "method": "item/mcpToolCall/started",
            "params": {
                "item": {
                    "type": "mcpToolCall",
                    "server": "cubic",
                    "tool": "get_wiki_page",
                    "arguments": {
                        "page": "Project Context"
                    }
                }
            }
        });
        let projection = provider_projection_from_records(&[record]);

        let inline = provider_projection_inline_event(&projection.items[0]).unwrap();

        assert_eq!(inline.kind, CodexInlineEventKind::Tool);
        assert_eq!(inline.title, "Used get_wiki_page");
        assert_eq!(
            inline_event_chip_label(&inline, false),
            "Used get_wiki_page"
        );
        let body = inline_event_body_text(&inline);
        assert!(body.contains("Project Context"));
        assert!(!body.contains("\"method\""));
        assert!(!body.contains("\"params\""));
    }

    #[test]
    fn provider_projection_completed_structured_tool_output_is_plain_text() {
        let mut record =
            provider_event_record(ProviderEventKind::Mcp, ProviderEventPhase::Completed);
        record.provider_subtype = Some("item/mcpToolCall/completed".to_owned());
        record.normalized_payload = serde_json::json!({
            "title": "get_wiki_page",
            "body": "{\n  \"ok\": true,\n  \"page\": \"Project Context\"\n}"
        });
        let projection = provider_projection_from_records(&[record]);

        let inline = provider_projection_inline_event(&projection.items[0]).unwrap();

        assert_eq!(
            inline_event_body_text(&inline),
            "ok: true\npage: Project Context"
        );
    }

    #[test]
    fn provider_projection_action_cards_hide_raw_details_when_body_is_incomplete() {
        let mut record = provider_event_record(ProviderEventKind::Mcp, ProviderEventPhase::Started);
        record.provider_subtype = Some("item/mcpToolCall/started".to_owned());
        record.normalized_payload = serde_json::json!({
            "title": "get_wiki_page",
            "body": ""
        });
        record.raw_json = serde_json::json!({
            "method": "item/mcpToolCall/started",
            "params": {
                "item": {
                    "type": "mcpToolCall",
                    "server": "cubic",
                    "tool": "get_wiki_page",
                    "arguments": {
                        "page": "Project Context"
                    }
                }
            }
        });
        let projection = provider_projection_from_records(&[record]);

        let text = provider_projection_card_text(&projection.items[0]);

        assert!(text.contains("get_wiki_page"));
        assert!(!text.contains("\"page\": \"Project Context\""));
        assert!(!text.contains("\"server\": \"cubic\""));
    }

    #[test]
    fn provider_projection_keeps_failure_status_chrome() {
        let record = provider_event_record(
            ProviderEventKind::CommandProcess,
            ProviderEventPhase::Failed,
        );
        let projection = provider_projection_from_records(&[record]);

        assert!(provider_projection_item_shows_status_chrome(
            &projection.items[0]
        ));
    }

    #[test]
    fn hook_events_are_hidden_from_normal_chat_projection() {
        let mut record = provider_event_record(
            ProviderEventKind::SkillPluginHook,
            ProviderEventPhase::Progress,
        );
        record.provider_subtype = Some("hook_event".to_owned());
        record.normalized_payload = serde_json::json!({
            "title": "PreToolUse",
            "body": "Bash: cargo test"
        });
        let projection = provider_projection_from_records(&[record]);

        let items = provider_projection_items_for_render(projection.items, &[]);

        assert!(items.is_empty());
    }

    #[test]
    fn provider_projection_inline_titles_use_action_verbs() {
        let cases = [
            (
                ProviderEventKind::CommandProcess,
                None,
                serde_json::json!({"title": "cargo test", "body": "ok"}),
                "Ran cargo test",
            ),
            (
                ProviderEventKind::FileSystem,
                Some("read"),
                serde_json::json!({"title": "README.md", "body": "# Archductor"}),
                "Read README.md",
            ),
            (
                ProviderEventKind::FileSystem,
                Some("read"),
                serde_json::json!({"title": "/tmp/archductor/session_surface.rs", "body": "fn main() {}"}),
                "Read session_surface.rs",
            ),
            (
                ProviderEventKind::SkillPluginHook,
                None,
                serde_json::json!({"title": "skill-creator", "body": "loaded"}),
                "Read skill-creator",
            ),
        ];

        for (kind, subtype, payload, label) in cases {
            let mut record = provider_event_record(kind, ProviderEventPhase::Completed);
            record.provider_subtype = subtype.map(str::to_owned);
            record.normalized_payload = payload;
            let projection = provider_projection_from_records(&[record]);
            let inline = provider_projection_inline_event(&projection.items[0]).unwrap();

            assert_eq!(inline_event_chip_label(&inline, false), label);
            assert!(!inline_event_expands_body_by_default(&inline));
        }
    }

    #[test]
    fn provider_projection_read_only_commands_render_as_reads() {
        let mut record = provider_event_record(
            ProviderEventKind::CommandProcess,
            ProviderEventPhase::Completed,
        );
        record.normalized_payload = serde_json::json!({
            "title": "sed -n '1,220p' crates/gtk-app/src/session_surface.rs",
            "body": "fn agent_session_panel() {}"
        });
        let projection = provider_projection_from_records(&[record]);

        let inline = provider_projection_inline_event(&projection.items[0]).unwrap();

        assert_eq!(inline.kind, CodexInlineEventKind::Tool);
        assert_eq!(inline.title, "Read session_surface.rs");
        assert_eq!(inline.subtitle.as_deref(), Some("File preview"));
        assert_eq!(
            inline_event_chip_label(&inline, false),
            "Read session_surface.rs"
        );
    }

    #[test]
    fn provider_projection_bash_lc_read_only_commands_render_as_reads() {
        let mut record = provider_event_record(
            ProviderEventKind::CommandProcess,
            ProviderEventPhase::Completed,
        );
        record.normalized_payload = serde_json::json!({
            "title": "/bin/bash -lc \"sed -n '1,220p' crates/gtk-app/src/session_surface.rs\"",
            "body": "fn agent_session_panel() {}"
        });
        let projection = provider_projection_from_records(&[record]);

        let inline = provider_projection_inline_event(&projection.items[0]).unwrap();

        assert_eq!(inline.kind, CodexInlineEventKind::Tool);
        assert_eq!(inline.title, "Read session_surface.rs");
        assert_eq!(inline.subtitle.as_deref(), Some("File preview"));
        assert_eq!(
            inline_event_chip_label(&inline, false),
            "Read session_surface.rs"
        );
    }

    #[test]
    fn provider_projection_file_change_items_render_as_edits_with_counts() {
        let mut record = provider_event_record(
            ProviderEventKind::DiffFileChange,
            ProviderEventPhase::Completed,
        );
        record.normalized_payload = serde_json::json!({
            "title": "File changes",
            "body": "modified src/main.rs"
        });
        record.raw_json = serde_json::json!({
            "method": "item/completed",
            "params": {
                "item": {
                    "type": "fileChange",
                    "changes": [
                        {
                            "path": "src/main.rs",
                            "kind": "modified",
                            "diff": "@@ -1 +1 @@\n-old\n+new\n"
                        }
                    ]
                }
            }
        });
        let projection = provider_projection_from_records(&[record]);

        let inline = provider_projection_inline_event(&projection.items[0]).unwrap();

        assert_eq!(inline.kind, CodexInlineEventKind::Tool);
        assert_eq!(inline.title, "Edited main.rs");
        assert_eq!(inline.subtitle.as_deref(), Some("+1 -1"));
        assert_eq!(inline_event_chip_label(&inline, false), "Edited main.rs");
        assert!(inline_event_body_text(&inline).contains("-old"));
        assert!(inline_event_body_text(&inline).contains("+new"));
    }

    #[test]
    fn subagent_projection_items_render_as_collapsible_inline_output() {
        let mut record = provider_event_record(
            ProviderEventKind::SubagentCollaboration,
            ProviderEventPhase::Completed,
        );
        record.normalized_payload = serde_json::json!({
            "title": "Review agent",
            "body": "Found 2 issues"
        });
        let projection = provider_projection_from_records(&[record]);

        let inline = provider_projection_inline_event(&projection.items[0]).unwrap();

        assert_eq!(inline.kind, CodexInlineEventKind::Tool);
        assert_eq!(inline.title, "Ran Review agent");
        assert_eq!(inline.subtitle.as_deref(), Some("Subagent"));
        assert_eq!(inline_event_chip_label(&inline, false), "Ran Review agent");
        assert_eq!(inline_event_body_text(&inline), "Found 2 issues");
    }

    #[test]
    fn inline_tool_event_rows_keep_compact_breathing_room() {
        let source = include_str!("session_surface.rs");

        assert!(source.contains("let group = GBox::new(Orientation::Vertical, 3);"));
        assert!(source.contains("let root = GBox::new(Orientation::Vertical, 2);"));
        assert!(source.contains("toggle.set_margin_bottom(1);"));
        assert!(source.contains("body.set_margin_top(2);"));
    }

    #[test]
    fn inline_tool_event_body_is_bounded_and_scrollable() {
        let source = include_str!("session_surface.rs")
            .split("\n#[cfg(test)]\nmod tests")
            .next()
            .unwrap();

        assert!(source.contains("let body_scroll = ScrolledWindow::new();"));
        assert!(source.contains(
            "body_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);"
        ));
        assert!(
            source.contains("body_scroll.set_max_content_height(INLINE_EVENT_BODY_MAX_HEIGHT);")
        );
        assert!(source.contains("body_revealer.set_child(Some(&body_scroll));"));
    }

    #[test]
    fn chat_text_labels_do_not_reserve_extra_bottom_gap() {
        let source = include_str!("session_surface.rs");
        let forbidden = ["set_margin_bottom", "(18)"].join("");

        assert!(!source.contains(&forbidden));
    }

    #[test]
    fn chat_text_markup_formats_inline_code_and_escapes_content() {
        let markup = chat_text_markup("Use `cargo test` before <merge>.");

        assert!(markup.contains("Use "));
        assert!(markup.contains("font_family=\"monospace\""));
        assert!(markup.contains("cargo test"));
        assert!(markup.contains("&lt;merge&gt;"));
        assert!(!markup.contains("<merge>"));
    }

    #[test]
    fn inline_event_body_preview_keeps_live_tool_output_full() {
        let full = format!("get_wiki_page\n{}", "x".repeat(500));
        let event = CodexInlineEvent {
            kind: CodexInlineEventKind::Tool,
            title: "get_wiki_page".to_owned(),
            subtitle: Some("Tool call".to_owned()),
            body: Some(full.clone()),
            path: None,
            status: CodexInlineEventStatus::Complete,
        };

        let preview = inline_event_body_preview(&event, &full);

        assert_eq!(preview.preview, full);
        assert_eq!(preview.full, full);
        assert!(!preview.truncated);
    }

    #[test]
    fn chat_wake_coalesces_event_bursts() {
        let pending = std::sync::atomic::AtomicBool::new(false);

        assert!(mark_chat_refresh_wake_pending(&pending));
        assert!(!mark_chat_refresh_wake_pending(&pending));
        clear_chat_refresh_wake_pending(&pending);
        assert!(mark_chat_refresh_wake_pending(&pending));
    }

    #[test]
    fn chat_refresh_wake_debounce_limits_streaming_repaint_churn() {
        let source = include_str!("session_surface.rs");

        assert!(
            source.contains("const CHAT_REFRESH_WAKE_DELAY_MS: u64 = 150;"),
            "streaming provider events must not trigger near-frame-rate full chat refreshes"
        );
    }

    #[test]
    fn provider_projection_fallback_card_text_shows_redacted_raw_payload() {
        let item = ProviderProjectionItem {
            id: "fallback-1".to_owned(),
            sequence: 1,
            timeline_seq: None,
            category: ProviderProjectionCategory::Unknown,
            render_class: ProjectionRenderClass::FallbackCard,
            title: "Unknown provider event".to_owned(),
            body: String::new(),
            status: ProviderProjectionStatus::Complete,
            stream_state: ProviderProjectionStreamState::Complete,
            parent_id: None,
            nested_thread_id: None,
            raw_payload: Some(
                "{\n  \"type\": \"future_event\",\n  \"api_key\": \"[redacted]\"\n}".to_owned(),
            ),
            inspectable: true,
        };

        let text = provider_projection_card_text(&item);

        assert!(text.contains("Unknown provider event"));
        assert!(text.contains("\"type\": \"future_event\""));
        assert!(text.contains("[redacted]"));
    }

    #[test]
    fn normal_chat_provider_projection_hides_raw_and_status_events() {
        let mut response = provider_event_record(
            ProviderEventKind::AssistantOutput,
            ProviderEventPhase::Delta,
        );
        response.provider_item_id = Some("msg-1".to_owned());
        response.normalized_payload = serde_json::json!({
            "title": "Assistant",
            "body": "I can fix that."
        });
        let mut thought = provider_event_record(
            ProviderEventKind::PlanningReasoning,
            ProviderEventPhase::Delta,
        );
        thought.provider_item_id = Some("thought-1".to_owned());
        thought.normalized_payload = serde_json::json!({
            "title": "Thought",
            "body": "Need to inspect the event mapper."
        });
        let mut tool =
            provider_event_record(ProviderEventKind::Tool, ProviderEventPhase::Completed);
        tool.provider_item_id = Some("tool-1".to_owned());
        tool.normalized_payload = serde_json::json!({
            "title": "Bash",
            "body": "cargo test passed"
        });
        let mut status = provider_event_record(
            ProviderEventKind::ThreadSession,
            ProviderEventPhase::Started,
        );
        status.provider_item_id = Some("status-1".to_owned());
        status.normalized_payload = serde_json::json!({
            "title": "thread/started",
            "body": ""
        });
        let mut raw =
            provider_event_record(ProviderEventKind::Unknown, ProviderEventPhase::Unknown);
        raw.provider_item_id = Some("raw-1".to_owned());
        raw.normalized_payload = serde_json::json!({
            "title": "Unknown provider event"
        });
        raw.raw_json = serde_json::json!({
            "id": 7,
            "result": { "ok": true }
        });

        let projection = provider_projection_from_records(&[response, thought, tool, status, raw]);
        let items = provider_projection_items_for_render(projection.items, &[]);

        assert_eq!(
            items
                .iter()
                .map(|item| item.render_class)
                .collect::<Vec<_>>(),
            vec![
                ProjectionRenderClass::AssistantChat,
                ProjectionRenderClass::ReasoningCard,
                ProjectionRenderClass::ToolCard,
            ]
        );
        assert!(items.iter().any(|item| item.body == "I can fix that."));
        assert!(items
            .iter()
            .any(|item| item.body == "Need to inspect the event mapper."));
        let rendered_text = items
            .iter()
            .map(provider_projection_card_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!rendered_text.contains("Unknown provider event"));
        assert!(!rendered_text.contains("\"result\""));
        assert!(!rendered_text.contains("thread/started"));
    }

    #[test]
    fn provider_user_chat_rows_are_skipped_because_persisted_messages_own_user_bubbles() {
        let mut user_event =
            provider_event_record(ProviderEventKind::UserInput, ProviderEventPhase::Completed);
        user_event.normalized_payload = serde_json::json!({
            "title": "User",
            "body": "run tests"
        });
        let projection = provider_projection_from_records(&[user_event]);
        let messages = vec![ChatMessageRecord {
            id: 30,
            thread_id: 7,
            role: "user".to_owned(),
            content: "run tests".to_owned(),
            source: "user_send".to_owned(),
            timeline_seq: Some(1),
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
        }];

        let items_without_message =
            provider_projection_items_for_render(projection.items.clone(), &[]);
        let items = provider_projection_items_for_render(projection.items, &messages);

        assert!(items_without_message.is_empty());
        assert!(items.is_empty());
    }

    #[test]
    fn recovered_provider_user_echo_matches_visible_user_message() {
        let mut user_event =
            provider_event_record(ProviderEventKind::UserInput, ProviderEventPhase::Completed);
        user_event.provider_item_id = Some("user-recovered".to_owned());
        user_event.normalized_payload = serde_json::json!({
            "title": "User",
            "body": "hidden recovered context\n\nCurrent user message:\nrun tests"
        });
        let projection = provider_projection_from_records(&[user_event]);
        let messages = vec![chat_message_record(1, "user", "run tests", "user_send")];

        let timeline = chat_structured_items_for_render(
            messages,
            Vec::new(),
            projection.items,
            Vec::new(),
            Vec::new(),
        );

        assert_eq!(timeline.len(), 1);
        assert!(matches!(timeline[0], ChatTimelineItem::Message(_)));
    }

    #[test]
    fn interrupted_notice_renders_inline_until_next_user_message() {
        let interrupted = ProviderProjectionItem {
            id: "codex:thread-7:turn-1".to_owned(),
            sequence: 2,
            timeline_seq: Some(2),
            category: ProviderProjectionCategory::Status,
            render_class: ProjectionRenderClass::StatusCard,
            title: "Turn".to_owned(),
            body: "Interrupted".to_owned(),
            status: ProviderProjectionStatus::Canceled,
            stream_state: ProviderProjectionStreamState::Complete,
            parent_id: None,
            nested_thread_id: None,
            raw_payload: None,
            inspectable: false,
        };
        let before_continue = chat_structured_items_for_render(
            vec![chat_message_record(1, "agent", "partial answer", "agent")],
            Vec::new(),
            vec![interrupted.clone()],
            Vec::new(),
            Vec::new(),
        );
        let after_continue = chat_structured_items_for_render(
            vec![
                chat_message_record(1, "agent", "partial answer", "agent"),
                chat_message_record(3, "user", "continue", "user_send"),
            ],
            Vec::new(),
            vec![interrupted],
            Vec::new(),
            Vec::new(),
        );

        assert!(matches!(
            before_continue.last(),
            Some(ChatTimelineItem::InterruptedNotice { .. })
        ));
        assert!(!after_continue
            .iter()
            .any(|item| matches!(item, ChatTimelineItem::InterruptedNotice { .. })));
    }

    #[test]
    fn interrupted_notice_uses_timeline_order_instead_of_received_sequence() {
        let interrupted = ProviderProjectionItem {
            id: "codex:thread-7:turn-1".to_owned(),
            sequence: 100,
            timeline_seq: Some(10),
            category: ProviderProjectionCategory::Status,
            render_class: ProjectionRenderClass::StatusCard,
            title: "Turn".to_owned(),
            body: "Interrupted".to_owned(),
            status: ProviderProjectionStatus::Canceled,
            stream_state: ProviderProjectionStreamState::Complete,
            parent_id: None,
            nested_thread_id: None,
            raw_payload: None,
            inspectable: false,
        };
        let messages = vec![ChatMessageRecord {
            id: 3,
            thread_id: 7,
            role: "user".to_owned(),
            content: "continue".to_owned(),
            source: "user_send".to_owned(),
            timeline_seq: Some(50),
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
        }];

        let timeline = chat_structured_items_for_render(
            messages,
            Vec::new(),
            vec![interrupted],
            Vec::new(),
            Vec::new(),
        );

        assert!(!timeline
            .iter()
            .any(|item| matches!(item, ChatTimelineItem::InterruptedNotice { .. })));
    }

    #[test]
    fn provider_empty_user_chat_rows_are_skipped() {
        let mut user_event =
            provider_event_record(ProviderEventKind::UserInput, ProviderEventPhase::Completed);
        user_event.provider_item_id = Some("user-empty".to_owned());
        user_event.normalized_payload = serde_json::json!({
            "title": "User",
            "body": "   "
        });
        let projection = provider_projection_from_records(&[user_event]);

        let items = provider_projection_items_for_render(projection.items, &[]);

        assert!(items.is_empty());
    }

    #[test]
    fn provider_reasoning_renders_only_exposed_reasoning_text() {
        let mut reasoning = provider_event_record(
            ProviderEventKind::PlanningReasoning,
            ProviderEventPhase::Delta,
        );
        reasoning.provider_item_id = Some("thought-1".to_owned());
        reasoning.normalized_payload = serde_json::json!({
            "title": "Reasoning",
            "body": "Need to inspect the event mapper."
        });
        let projection = provider_projection_from_records(&[reasoning]);
        let item = &projection.items[0];

        assert_eq!(item.render_class, ProjectionRenderClass::ReasoningCard);
        assert_eq!(
            provider_projection_reasoning_text(item).as_deref(),
            Some("Need to inspect the event mapper.")
        );
        assert!(!provider_projection_reasoning_text(item)
            .unwrap()
            .contains("Reasoning"));
    }

    #[test]
    fn provider_assistant_body_hides_archductor_metadata() {
        let item = ProviderProjectionItem {
            id: "assistant".to_owned(),
            sequence: 1,
            timeline_seq: None,
            category: ProviderProjectionCategory::AssistantMessage,
            render_class: ProjectionRenderClass::AssistantChat,
            title: "Assistant".to_owned(),
            body: "<archductor_metadata>{\"workspace_name\":\"harness-file-changes\",\"branch_name\":\"lc/harness-file-changes\",\"chat_title\":\"Harness File Changes\"}</archductor_metadata>\nContinuing."
                .to_owned(),
            status: ProviderProjectionStatus::Complete,
            stream_state: ProviderProjectionStreamState::Complete,
            parent_id: None,
            nested_thread_id: None,
            raw_payload: None,
            inspectable: false,
        };

        assert_eq!(
            provider_projection_assistant_body_for_render(&item),
            "Continuing."
        );
    }

    #[test]
    fn provider_assistant_body_hides_incomplete_archductor_control_line() {
        let item = ProviderProjectionItem {
            id: "assistant".to_owned(),
            sequence: 1,
            timeline_seq: None,
            category: ProviderProjectionCategory::AssistantMessage,
            render_class: ProjectionRenderClass::AssistantChat,
            title: "Assistant".to_owned(),
            body: "<arch\nContinuing.".to_owned(),
            status: ProviderProjectionStatus::Running,
            stream_state: ProviderProjectionStreamState::Streaming,
            parent_id: None,
            nested_thread_id: None,
            raw_payload: None,
            inspectable: false,
        };

        assert_eq!(
            provider_projection_assistant_body_for_render(&item),
            "Continuing."
        );
    }

    #[test]
    fn persisted_agent_body_hides_incomplete_archductor_control_line() {
        let message = ChatMessageRecord {
            id: 8,
            thread_id: 7,
            role: "agent".to_owned(),
            content: "<archductor_metadata>{\"workspace_name\":\"harness-file\"\nContinuing."
                .to_owned(),
            source: "provider_event".to_owned(),
            timeline_seq: Some(2),
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
        };

        assert_eq!(
            chat_agent_message_display_content(&message, true),
            "Continuing."
        );
    }

    #[test]
    fn metadata_ui_update_reports_chat_title_independently() {
        let base = PathBuf::from("/tmp/archductor-metadata-ui-test");
        let paths = AppPaths {
            config_dir: base.join("config"),
            data_dir: base.join("data"),
            state_dir: base.join("state"),
            cache_dir: base.join("cache"),
            database_path: base.join("data/archductor.db"),
            logs_dir: base.join("state/logs"),
        };
        let app_state = AppState::new(
            paths,
            Some("berlin".to_owned()),
            WorkspaceTab::Chats,
            AppPage::Workspace,
        );
        let current_workspace = RefCell::new("berlin".to_owned());
        let current_branch = RefCell::new("lc/berlin".to_owned());
        let original = ChatThreadRecord {
            id: 7,
            workspace_id: 42,
            provider: "codex".to_owned(),
            title: "New Chat".to_owned(),
            status: "active".to_owned(),
            native_thread_id: None,
            harness_metadata: None,
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
            archived_at: None,
        };
        let thread_state = RefCell::new(vec![original.clone()]);
        let mut renamed = original;
        renamed.title = "Fix Billing Webhook".to_owned();

        let changes = apply_agent_metadata_ui_update(
            &app_state,
            &current_workspace,
            &current_branch,
            &thread_state,
            AgentMetadataUiUpdate {
                workspace_name: "berlin".to_owned(),
                branch_name: "lc/berlin".to_owned(),
                thread: renamed,
            },
        );

        assert!(!changes.workspace_changed);
        assert!(!changes.branch_changed);
        assert!(changes.chat_title_changed);
        assert_eq!(thread_state.borrow()[0].title, "Fix Billing Webhook");
    }

    #[test]
    fn metadata_ui_update_reports_workspace_and_branch_independently() {
        let base = PathBuf::from("/tmp/archductor-metadata-identity-test");
        let app_state = AppState::new(
            AppPaths {
                config_dir: base.join("config"),
                data_dir: base.join("data"),
                state_dir: base.join("state"),
                cache_dir: base.join("cache"),
                database_path: base.join("data/archductor.db"),
                logs_dir: base.join("state/logs"),
            },
            Some("berlin".to_owned()),
            WorkspaceTab::Chats,
            AppPage::Workspace,
        );
        let current_workspace = RefCell::new("berlin".to_owned());
        let current_branch = RefCell::new("lc/berlin".to_owned());
        let thread = ChatThreadRecord {
            id: 7,
            workspace_id: 42,
            provider: "codex".to_owned(),
            title: "Fix Billing Webhook".to_owned(),
            status: "active".to_owned(),
            native_thread_id: None,
            harness_metadata: None,
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
            archived_at: None,
        };
        let thread_state = RefCell::new(vec![thread.clone()]);

        let workspace_change = apply_agent_metadata_ui_update(
            &app_state,
            &current_workspace,
            &current_branch,
            &thread_state,
            AgentMetadataUiUpdate {
                workspace_name: "billing-webhook".to_owned(),
                branch_name: "lc/berlin".to_owned(),
                thread: thread.clone(),
            },
        );
        assert!(workspace_change.workspace_changed);
        assert!(!workspace_change.branch_changed);
        assert!(!workspace_change.chat_title_changed);

        let branch_change = apply_agent_metadata_ui_update(
            &app_state,
            &current_workspace,
            &current_branch,
            &thread_state,
            AgentMetadataUiUpdate {
                workspace_name: "billing-webhook".to_owned(),
                branch_name: "lc/billing-webhook".to_owned(),
                thread,
            },
        );
        assert!(!branch_change.workspace_changed);
        assert!(branch_change.branch_changed);
        assert!(!branch_change.chat_title_changed);
        assert_eq!(
            app_state.selected_workspace().as_deref(),
            Some("billing-webhook")
        );
    }

    #[test]
    fn chat_thread_nav_signature_ignores_updated_at_only_changes() {
        let mut thread = ChatThreadRecord {
            id: 7,
            workspace_id: 1,
            provider: "codex".to_owned(),
            title: "Fix auth".to_owned(),
            status: "active".to_owned(),
            native_thread_id: None,
            harness_metadata: None,
            created_at: "then".to_owned(),
            updated_at: "2026-07-18T12:00:00Z".to_owned(),
            archived_at: None,
        };
        let before = chat_thread_nav_signature(&[thread.clone()]);
        thread.updated_at = "2026-07-18T12:00:01Z".to_owned();

        assert_eq!(before, chat_thread_nav_signature(&[thread]));
    }

    #[test]
    fn provider_empty_reasoning_rows_are_skipped() {
        let mut reasoning = provider_event_record(
            ProviderEventKind::PlanningReasoning,
            ProviderEventPhase::Delta,
        );
        reasoning.provider_item_id = Some("thought-empty".to_owned());
        reasoning.normalized_payload = serde_json::json!({
            "title": "Reasoning",
            "body": ""
        });
        let projection = provider_projection_from_records(&[reasoning]);

        let items = provider_projection_items_for_render(projection.items, &[]);

        assert!(items.is_empty());
    }

    #[test]
    fn stored_chat_event_inline_event_reconstructs_tool_and_file_change_payloads() {
        let tool_event = ChatEventRecord {
            id: 100,
            thread_id: 7,
            process_id: Some(5),
            kind: "tool".to_owned(),
            title: "cargo test".to_owned(),
            body: "ok".to_owned(),
            path: None,
            payload_json: r#"{"type":"tool","title":"cargo test","body":"ok"}"#.to_owned(),
            timeline_seq: 1,
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
        };
        let file_change_event = ChatEventRecord {
            id: 101,
            thread_id: 7,
            process_id: Some(5),
            kind: "file_change".to_owned(),
            title: "edited src/lib.rs".to_owned(),
            body: "updated".to_owned(),
            path: Some("src/lib.rs".to_owned()),
            payload_json: r#"{"type":"file_change","action":"edited","path":"src/lib.rs","additions":2,"deletions":1,"lines":[{"kind":"context","old_line":10,"new_line":10,"content":"fn old() {}"},{"kind":"added","old_line":null,"new_line":11,"content":"fn new() {}"},{"kind":"deleted","old_line":12,"new_line":null,"content":"fn removed() {}"}]}"#.to_owned(),
            timeline_seq: 2,
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
        };

        let tool_inline = stored_chat_event_inline_event(&tool_event).unwrap();
        let file_change_inline = stored_chat_event_inline_event(&file_change_event).unwrap();

        assert_eq!(tool_inline.kind, CodexInlineEventKind::Tool);
        assert_eq!(tool_inline.title, "cargo test");
        assert_eq!(tool_inline.body.as_deref(), Some("ok"));

        assert_eq!(file_change_inline.kind, CodexInlineEventKind::Tool);
        assert_eq!(file_change_inline.title, "Edited src/lib.rs");
        assert_eq!(
            file_change_inline.path.as_deref(),
            Some(Path::new("src/lib.rs"))
        );
        assert_eq!(file_change_inline.status, CodexInlineEventStatus::Complete);
        assert_eq!(file_change_inline.subtitle.as_deref(), Some("+2 -1"));
        assert!(file_change_inline
            .body
            .as_deref()
            .unwrap()
            .contains("fn new() {}"));
    }

    #[test]
    fn stored_chat_event_inline_event_renders_file_read_payloads_as_previews() {
        let read_event = ChatEventRecord {
            id: 102,
            thread_id: 7,
            process_id: Some(5),
            kind: "tool".to_owned(),
            title: "Read README.md".to_owned(),
            body: "# Project".to_owned(),
            path: None,
            payload_json: r##"{"type":"tool","title":"Read README.md","body":"# Project"}"##
                .to_owned(),
            timeline_seq: 3,
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
        };

        let inline = stored_chat_event_inline_event(&read_event).unwrap();

        assert_eq!(inline.kind, CodexInlineEventKind::Tool);
        assert_eq!(inline.title, "Read README.md");
        assert_eq!(inline.subtitle.as_deref(), Some("File preview"));
        assert_eq!(inline.path.as_deref(), Some(Path::new("README.md")));
        assert_eq!(inline.body.as_deref(), Some("# Project"));
    }

    #[test]
    fn legacy_inline_event_parsing_is_disabled_for_normal_chat_rendering() {
        let message = ChatMessageRecord {
            id: 10,
            thread_id: 7,
            role: "agent".to_owned(),
            content: "functions.exec_command running cargo test".to_owned(),
            source: "agent_reply".to_owned(),
            timeline_seq: Some(3),
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
        };

        let persisted_event = ChatEventRecord {
            id: 88,
            thread_id: 7,
            process_id: Some(5),
            kind: "tool".to_owned(),
            title: "cargo test".to_owned(),
            body: "ok".to_owned(),
            path: None,
            payload_json: r#"{"type":"tool","title":"cargo test","body":"ok"}"#.to_owned(),
            timeline_seq: 1,
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
        };
        let legacy_enabled = render_legacy_inline_events_for_thread(&[], "structured");
        let legacy_disabled = render_legacy_inline_events_for_thread(
            std::slice::from_ref(&persisted_event),
            "structured",
        );
        let raw_enabled =
            render_legacy_inline_events_for_thread(std::slice::from_ref(&persisted_event), "raw");
        let legacy_events = legacy_inline_events_for_message(&message, legacy_enabled);
        let persisted_timeline_events = legacy_inline_events_for_message(&message, legacy_disabled);
        let raw_events = legacy_inline_events_for_message(&message, raw_enabled);

        assert!(legacy_events.is_empty());
        assert!(persisted_timeline_events.is_empty());
        assert!(raw_events.is_empty());
    }

    #[test]
    fn persisted_event_rendering_strips_duplicate_codex_status_blocks() {
        let content = "\
• Explored
  └ Read package.json, README.md

Config says Next.js 16.2.9.

Explored
  └ Read page.tsx, globals.css

Schema confirms the app moved CRM around businesses.";

        assert_eq!(
            strip_codex_status_blocks(content),
            "Config says Next.js 16.2.9.\n\nSchema confirms the app moved CRM around businesses."
        );
    }

    #[test]
    fn status_block_filter_keeps_explored_prose() {
        assert_eq!(
            strip_codex_status_blocks("Explored two alternatives before choosing the small fix."),
            "Explored two alternatives before choosing the small fix."
        );
    }

    #[test]
    fn composer_enter_key_sends_without_shift() {
        assert!(should_send_composer_message(
            gtk::gdk::Key::Return,
            gtk::gdk::ModifierType::empty()
        ));
        assert!(should_send_composer_message(
            gtk::gdk::Key::KP_Enter,
            gtk::gdk::ModifierType::empty()
        ));
        assert!(!should_send_composer_message(
            gtk::gdk::Key::Return,
            gtk::gdk::ModifierType::SHIFT_MASK
        ));
    }

    #[test]
    fn codex_startup_state_defaults_to_idle_without_startup_signal_or_live_session() {
        let state = derive_codex_startup_state(false, false, None);

        assert_eq!(state, CodexStartupState::Idle);
    }

    #[test]
    fn codex_startup_state_switches_to_error_when_archcar_reports_failure() {
        let state = codex_startup_state_from_error("spawn failed");

        assert_eq!(
            state,
            CodexStartupState::Error {
                message: "spawn failed".to_owned(),
            }
        );
    }

    #[test]
    fn codex_startup_state_becomes_ready_when_session_is_ready() {
        let state = derive_codex_startup_state(true, true, None);

        assert_eq!(state, CodexStartupState::Ready);
    }

    #[test]
    fn codex_startup_state_preserves_idle_without_live_session() {
        let state = derive_codex_startup_state(false, false, Some(CodexStartupState::Idle));

        assert_eq!(state, CodexStartupState::Idle);
    }

    #[test]
    fn codex_startup_state_clears_loading_without_live_session() {
        let state = derive_codex_startup_state(
            false,
            false,
            Some(CodexStartupState::Loading {
                message: "Starting Codex...".to_owned(),
            }),
        );

        assert_eq!(state, CodexStartupState::Idle);
    }

    #[test]
    fn codex_startup_state_for_thread_ignores_ready_session_from_other_thread() {
        let mut selected = session_record(11, "codex", ProcessStatus::Running, None);
        selected.chat_thread_id = Some(7);
        let mut other = session_record(22, "codex", ProcessStatus::Running, None);
        other.chat_thread_id = Some(8);
        let ready_cache = RefCell::new(HashMap::from([(22, true)]));

        let state = codex_startup_state_for_thread(7, &[selected, other], &ready_cache, None);

        assert_eq!(
            state,
            CodexStartupState::Loading {
                message: "Starting agent...".to_owned(),
            }
        );
    }

    #[test]
    fn codex_startup_state_for_thread_stays_idle_without_live_selected_session() {
        let mut other = session_record(22, "codex", ProcessStatus::Running, None);
        other.chat_thread_id = Some(8);
        let ready_cache = RefCell::new(HashMap::from([(22, true)]));

        let state = codex_startup_state_for_thread(7, &[other], &ready_cache, None);

        assert_eq!(state, CodexStartupState::Idle);
    }

    #[test]
    fn codex_startup_state_for_thread_preserves_ready_signal_without_matching_session_id() {
        let mut selected = session_record(59, "codex", ProcessStatus::Running, None);
        selected.chat_thread_id = Some(7);
        let ready_cache = RefCell::new(HashMap::from([(61, true)]));

        let state = codex_startup_state_for_thread(
            7,
            &[selected],
            &ready_cache,
            Some(CodexStartupState::Ready),
        );

        assert_eq!(state, CodexStartupState::Ready);
    }

    #[test]
    fn codex_startup_signal_updates_only_the_target_thread() {
        let mut states = HashMap::from([
            (
                7,
                CodexStartupState::Loading {
                    message: "Starting Codex...".to_owned(),
                },
            ),
            (8, CodexStartupState::Ready),
        ]);

        apply_codex_startup_signal(
            &mut states,
            CodexStartupSignal::Error {
                thread_id: 7,
                message: "spawn failed".to_owned(),
            },
        );

        assert_eq!(
            states.get(&7),
            Some(&CodexStartupState::Error {
                message: "spawn failed".to_owned(),
            })
        );
        assert_eq!(states.get(&8), Some(&CodexStartupState::Ready));
    }

    #[test]
    fn working_indicator_formats_elapsed_timer() {
        assert_eq!(format_working_elapsed(Duration::from_secs(4)), "0:04");
        assert_eq!(format_working_elapsed(Duration::from_secs(65)), "1:05");
        assert_eq!(format_working_elapsed(Duration::from_secs(3661)), "1:01:01");
    }

    #[test]
    fn working_indicator_waits_for_pending_input_after_startup_ready() {
        let mut selected = session_record(11, "codex", ProcessStatus::Running, None);
        selected.chat_thread_id = Some(7);
        let session_threads = RefCell::new(HashMap::from([(11, 7)]));
        let pending_inputs = RefCell::new(HashMap::from([(
            7,
            vec![QueuedArchcarInput {
                input: "run tests".to_owned(),
                visible_input: None,
                kind: ArchcarInputKind::User,
                session_kind: SessionKind::Codex,
            }],
        )]));
        let working_threads = RefCell::new(HashMap::new());
        mark_thread_working(&working_threads, 7);

        let changed = update_working_indicator_for_archcar_event(
            &ArchcarEvent::SessionReady {
                session_id: 11,
                thread_id: 7,
            },
            &[selected],
            &session_threads,
            SessionKind::Codex,
            Some(7),
            &pending_inputs,
            &working_threads,
        );

        assert!(!changed);
        assert!(working_threads.borrow().contains_key(&7));
    }

    #[test]
    fn working_indicator_clears_when_turn_completes() {
        let mut selected = session_record(11, "codex", ProcessStatus::Running, None);
        selected.chat_thread_id = Some(7);
        let session_threads = RefCell::new(HashMap::from([(11, 7)]));
        let pending_inputs = RefCell::new(HashMap::new());
        let working_threads = RefCell::new(HashMap::new());
        mark_thread_working(&working_threads, 7);

        let changed = update_working_indicator_for_archcar_event(
            &ArchcarEvent::TurnCompleted {
                session_id: 11,
                thread_id: 7,
                status: Some("completed".to_owned()),
            },
            &[selected],
            &session_threads,
            SessionKind::Codex,
            Some(7),
            &pending_inputs,
            &working_threads,
        );

        assert!(changed);
        assert!(!working_threads.borrow().contains_key(&7));
    }

    #[test]
    fn ensure_success_marks_spawned_thread_loading_until_event_ready() {
        let mut startup_states = HashMap::new();

        apply_archcar_ensure_success(
            &ArchcarResponse::SessionSpawned {
                session_id: 57,
                thread_id: 11,
                workspace: "hoi-an".to_owned(),
                kind: SessionKind::Codex,
                pid: 4242,
            },
            &mut startup_states,
            Some(9),
        );

        assert_eq!(
            startup_states.get(&11),
            Some(&CodexStartupState::Loading {
                message: "Starting agent...".to_owned(),
            })
        );
    }

    #[test]
    fn routine_archcar_updates_do_not_force_outer_refresh() {
        let ready_intent = archcar_message_refresh_intent(&AsyncArchcarMessage::Event(
            ArchcarEvent::SessionReady {
                session_id: 61,
                thread_id: 4,
            },
        ));
        let status_intent =
            archcar_message_refresh_intent(&AsyncArchcarMessage::Response(AsyncArchcarResponse {
                token: 7,
                request: AsyncArchcarRequestKind::GetSessionStatus { session_id: 61 },
                result: Ok(ArchcarResponse::SessionStatus {
                    session_id: 61,
                    status: "running".to_owned(),
                    runtime_state: AgentSessionState::WaitingForInput,
                    ready: true,
                    capabilities: None,
                }),
            }));
        let started_intent = archcar_message_refresh_intent(&AsyncArchcarMessage::Event(
            ArchcarEvent::SessionStarted {
                session_id: 61,
                thread_id: 4,
                workspace: "hoi-an".to_owned(),
                kind: SessionKind::Codex,
                pid: 1234,
            },
        ));

        assert_eq!(
            ready_intent,
            ArchcarRefreshIntent {
                chat_surface: true,
                workspace_nav: false,
                global_summary: false,
            }
        );
        assert_eq!(
            status_intent,
            ArchcarRefreshIntent {
                chat_surface: true,
                workspace_nav: false,
                global_summary: false,
            }
        );
        assert_eq!(
            started_intent,
            ArchcarRefreshIntent {
                chat_surface: true,
                workspace_nav: true,
                global_summary: true,
            }
        );

        let turn_completed_intent = archcar_message_refresh_intent(&AsyncArchcarMessage::Event(
            ArchcarEvent::TurnCompleted {
                session_id: 61,
                thread_id: 4,
                status: Some("completed".to_owned()),
            },
        ));
        assert_eq!(
            turn_completed_intent,
            ArchcarRefreshIntent {
                chat_surface: true,
                workspace_nav: true,
                global_summary: true,
            }
        );
    }

    #[test]
    fn provider_interaction_presentation_maps_kind_actions() {
        let permission = interaction_presentation(&provider_interaction_fixture(
            ProviderInteractionKind::Permission,
        ));
        assert_eq!(permission.kind, InteractionUiKind::Permission);
        assert_eq!(permission.actions, ["Allow", "Deny", "Always allow"]);
        assert_eq!(permission.provider_label, "Claude Code");

        let question = interaction_presentation(&provider_interaction_fixture(
            ProviderInteractionKind::UserQuestion,
        ));
        assert_eq!(question.kind, InteractionUiKind::Question);

        let plan = interaction_presentation(&provider_interaction_fixture(
            ProviderInteractionKind::PlanApproval,
        ));
        assert_eq!(plan.actions, ["Approve plan", "Keep planning"]);
    }

    #[test]
    fn provider_interaction_question_answers_require_all_questions() {
        let questions = vec!["scope".to_owned(), "risk".to_owned()];
        let mut answers = HashMap::new();
        answers.insert("scope".to_owned(), "yes".to_owned());
        assert!(!question_answers_complete(&questions, &answers));

        answers.insert("risk".to_owned(), "low".to_owned());
        assert!(question_answers_complete(&questions, &answers));
    }

    #[test]
    fn provider_interaction_events_refresh_thread_only() {
        let interaction = provider_interaction_fixture(ProviderInteractionKind::Permission);
        let requested_intent = archcar_message_refresh_intent(&AsyncArchcarMessage::Event(
            ArchcarEvent::ProviderInteractionRequested {
                interaction: interaction.clone(),
            },
        ));
        let resolved_intent = archcar_message_refresh_intent(&AsyncArchcarMessage::Event(
            ArchcarEvent::ProviderInteractionResolved { interaction },
        ));

        assert_eq!(
            requested_intent,
            ArchcarRefreshIntent {
                chat_surface: true,
                workspace_nav: false,
                global_summary: false,
            }
        );
        assert_eq!(resolved_intent, requested_intent);
    }

    #[test]
    fn session_message_updates_refresh_metadata_targets_immediately() {
        let intent = archcar_message_refresh_intent(&AsyncArchcarMessage::Event(
            ArchcarEvent::SessionMessagesUpdated { thread_id: 4 },
        ));

        assert_eq!(
            intent,
            ArchcarRefreshIntent {
                chat_surface: true,
                workspace_nav: true,
                global_summary: true,
            }
        );
    }

    #[test]
    fn chat_send_paths_do_not_refresh_outer_workspace_chrome() {
        let source = include_str!("session_surface.rs");
        let send_start = source
            .find("let send_text_with_delivery = Rc::new")
            .unwrap();
        let send_end = source[send_start..]
            .find("*send_text_after_ready_queue")
            .map(|offset| send_start + offset)
            .unwrap();
        let send_body = &source[send_start..send_end];

        assert!(
            !send_body.contains("refresh_for_send();"),
            "chat send should repaint the chat surface locally, not refresh outer chrome/right panels"
        );
    }

    #[test]
    fn bridge_error_notice_dedupes_repeated_subscribe_failures() {
        let mut state = BridgeErrorUiState::default();

        assert_eq!(
            state.record("archcar subscribe failed: socket missing"),
            Some("archcar subscribe failed: socket missing".to_owned())
        );
        assert_eq!(
            state.record("archcar subscribe failed: socket missing"),
            None
        );
        assert_eq!(
            state.record("archcar subscribe failed: socket missing"),
            None
        );
        assert_eq!(
            state.record("archcar subscribe failed: permission denied"),
            Some(
                "archcar subscribe failed: permission denied (2 repeated bridge errors suppressed)"
                    .to_owned()
            )
        );
    }

    #[test]
    fn session_refresh_error_text_names_failed_operation() {
        let err = anyhow::anyhow!("database is locked");

        assert_eq!(
            session_refresh_error_text("load sessions", &err),
            "[session refresh] load sessions failed: database is locked"
        );
    }

    #[test]
    fn chat_refresh_skips_stale_workspace_surface_after_selection_changes() {
        let state = AppState::new(
            AppPaths::from_env(),
            Some("tokyo".to_owned()),
            WorkspaceTab::Chats,
            AppPage::Workspace,
        );

        assert_eq!(
            chat_refresh_workspace_decision("berlin", &state.snapshot(), true),
            ChatRefreshWorkspaceDecision::SkipStaleSurface
        );
    }

    #[test]
    fn chat_refresh_clears_deleted_selected_workspace_instead_of_showing_session_error() {
        let state = AppState::new(
            AppPaths::from_env(),
            Some("berlin".to_owned()),
            WorkspaceTab::Chats,
            AppPage::Workspace,
        );

        assert_eq!(
            chat_refresh_workspace_decision("berlin", &state.snapshot(), false),
            ChatRefreshWorkspaceDecision::ClearDeletedWorkspace
        );
    }

    #[test]
    fn gtk_refresh_timers_are_documented_or_removed() {
        let recurring_sources = [
            ("main.rs", include_str!("main.rs")),
            ("history.rs", include_str!("history.rs")),
            ("sidebar.rs", include_str!("sidebar.rs")),
            ("session_surface.rs", include_str!("session_surface.rs")),
            ("terminal.rs", include_str!("terminal.rs")),
            (
                "workspace_command_center.rs",
                include_str!("workspace_command_center.rs"),
            ),
        ];

        for (path, source) in recurring_sources {
            for needle in [
                concat!("timeout", "_add_local"),
                concat!("timeout", "_add_seconds_local"),
            ] {
                let mut cursor = 0;
                while let Some(found) = source[cursor..].find(needle) {
                    let absolute = cursor + found;
                    let context_start = absolute.saturating_sub(260);
                    let context_end = (absolute + 260).min(source.len());
                    let context = &source[context_start..context_end];
                    assert!(
                        context.contains("PER-190"),
                        "{path} has recurring GTK timer `{needle}` without PER-190 ownership/removal note"
                    );
                    cursor = absolute + needle.len();
                }
            }
        }
    }

    #[test]
    fn refresh_hub_documents_page_owned_error_handling() {
        let source = include_str!("refresh.rs");

        assert!(
            source.contains("page-owned error handling") && source.contains("PER-190"),
            "RefreshHub must document that it is dumb fanout and pages own load errors"
        );
    }

    #[test]
    fn codex_thread_readiness_ignores_other_thread_ready_session() {
        let mut selected = session_record(11, "codex", ProcessStatus::Running, None);
        selected.chat_thread_id = Some(7);
        let mut other = session_record(22, "codex", ProcessStatus::Running, None);
        other.chat_thread_id = Some(8);
        let records = vec![selected, other];
        let ready_cache = RefCell::new(HashMap::from([(22, true)]));

        assert!(!codex_thread_ready_for_ui(7, &records, &ready_cache, None));
    }

    #[test]
    fn codex_thread_ready_state_uses_startup_state_when_session_ids_drift() {
        let mut selected = session_record(59, "codex", ProcessStatus::Running, None);
        selected.chat_thread_id = Some(7);
        let ready_cache = RefCell::new(HashMap::from([(61, true)]));

        assert!(codex_thread_ready_for_ui(
            7,
            &[selected],
            &ready_cache,
            Some(&CodexStartupState::Ready),
        ));
    }

    #[test]
    fn selecting_thread_updates_selection_and_recomputes_composer_state() {
        let selected_thread = RefCell::new(Some(7));
        let app_state = Rc::new(RefCell::new(None));
        let composer_updates = Rc::new(RefCell::new(0));

        apply_thread_selection(
            &selected_thread,
            Some(8),
            |thread_id| *app_state.borrow_mut() = thread_id,
            || *composer_updates.borrow_mut() += 1,
        );

        assert_eq!(*selected_thread.borrow(), Some(8));
        assert_eq!(*app_state.borrow(), Some(8));
        assert_eq!(*composer_updates.borrow(), 1);
    }

    #[test]
    fn selecting_same_thread_does_not_recompute_composer_state() {
        let selected_thread = RefCell::new(Some(7));
        let app_state = Rc::new(RefCell::new(None));
        let composer_updates = Rc::new(RefCell::new(0));

        apply_thread_selection(
            &selected_thread,
            Some(7),
            |thread_id| *app_state.borrow_mut() = thread_id,
            || *composer_updates.borrow_mut() += 1,
        );

        assert_eq!(*selected_thread.borrow(), Some(7));
        assert_eq!(*app_state.borrow(), None);
        assert_eq!(*composer_updates.borrow(), 0);
    }

    #[test]
    fn message_event_maps_to_message_refresh_kind() {
        let event = RefreshEvent::WorkspaceChatMessagesChanged {
            workspace: "berlin".to_owned(),
            thread_id: 42,
        };

        assert_eq!(
            chat_refresh_kind_for_event(&event),
            ChatRefreshKind::Messages { thread_id: 42 }
        );
    }

    #[test]
    fn lifecycle_event_maps_to_thread_nav_refresh_kind() {
        let event = RefreshEvent::WorkspaceChatLifecycleChanged {
            workspace: "berlin".to_owned(),
        };

        assert_eq!(
            chat_refresh_kind_for_event(&event),
            ChatRefreshKind::ThreadNav
        );
    }

    #[test]
    fn chat_surface_refresh_dispatch_uses_kind_specific_paths() {
        let full = Cell::new(0);
        let messages = RefCell::new(Vec::new());
        let thread_nav = Cell::new(0);

        dispatch_chat_surface_refresh_kind(
            ChatRefreshKind::Messages { thread_id: 42 },
            Some(42),
            &|| full.set(full.get() + 1),
            &|thread_id| messages.borrow_mut().push(thread_id),
            &|| thread_nav.set(thread_nav.get() + 1),
        );
        dispatch_chat_surface_refresh_kind(
            ChatRefreshKind::Messages { thread_id: 99 },
            Some(42),
            &|| full.set(full.get() + 1),
            &|thread_id| messages.borrow_mut().push(thread_id),
            &|| thread_nav.set(thread_nav.get() + 1),
        );
        dispatch_chat_surface_refresh_kind(
            ChatRefreshKind::ThreadNav,
            Some(42),
            &|| full.set(full.get() + 1),
            &|thread_id| messages.borrow_mut().push(thread_id),
            &|| thread_nav.set(thread_nav.get() + 1),
        );
        dispatch_chat_surface_refresh_kind(
            ChatRefreshKind::Full,
            Some(42),
            &|| full.set(full.get() + 1),
            &|thread_id| messages.borrow_mut().push(thread_id),
            &|| thread_nav.set(thread_nav.get() + 1),
        );

        assert_eq!(*messages.borrow(), vec![42]);
        assert_eq!(thread_nav.get(), 1);
        assert_eq!(full.get(), 1);
    }

    #[test]
    fn chat_timeline_refresh_plan_appends_only_new_suffix_rows() {
        let old_items = vec![ChatTimelineItem::Message(chat_message_record(
            1,
            "user",
            "hello",
            "user_send",
        ))];
        let new_items = vec![
            ChatTimelineItem::Message(chat_message_record(1, "user", "hello", "user_send")),
            ChatTimelineItem::Message(chat_message_record(2, "agent", "hi", "agent")),
        ];
        let old = chat_timeline_render_state(7, "structured", &old_items);
        let new = chat_timeline_render_state(7, "structured", &new_items);

        assert_eq!(
            chat_timeline_refresh_plan(Some(&old), &new),
            ChatTimelineRefreshPlan::Append { start: 1 }
        );
    }

    #[test]
    fn chat_timeline_refresh_plan_rebuilds_message_list_for_mutations() {
        let old_items = vec![ChatTimelineItem::Message(chat_message_record(
            1, "agent", "partial", "agent",
        ))];
        let new_items = vec![ChatTimelineItem::Message(chat_message_record(
            1,
            "agent",
            "partial plus more",
            "agent",
        ))];
        let old = chat_timeline_render_state(7, "structured", &old_items);
        let new = chat_timeline_render_state(7, "structured", &new_items);

        assert_eq!(
            chat_timeline_refresh_plan(Some(&old), &new),
            ChatTimelineRefreshPlan::RebuildMessages
        );
    }

    #[test]
    fn chat_timeline_refresh_plan_rebuilds_only_changed_tail_for_streaming_updates() {
        let old_items = vec![
            ChatTimelineItem::Message(chat_message_record(1, "user", "request", "user_send")),
            ChatTimelineItem::Message(chat_message_record(2, "agent", "partial", "agent")),
        ];
        let new_items = vec![
            ChatTimelineItem::Message(chat_message_record(1, "user", "request", "user_send")),
            ChatTimelineItem::Message(chat_message_record(
                2,
                "agent",
                "partial plus more",
                "agent",
            )),
        ];
        let old = chat_timeline_render_state(7, "structured", &old_items);
        let new = chat_timeline_render_state(7, "structured", &new_items);

        assert_eq!(
            chat_timeline_refresh_plan(Some(&old), &new),
            ChatTimelineRefreshPlan::RebuildFrom { start: 1 }
        );
    }

    #[test]
    fn chat_timeline_refresh_plan_rebuilds_when_leading_rows_change() {
        let items = vec![ChatTimelineItem::Message(chat_message_record(
            1, "agent", "partial", "agent",
        ))];
        let old = chat_timeline_render_state_with_leading_rows(7, "structured", &items, 1);
        let new = chat_timeline_render_state(7, "structured", &items);

        assert_eq!(
            chat_timeline_refresh_plan(Some(&old), &new),
            ChatTimelineRefreshPlan::RebuildMessages
        );
    }

    #[test]
    fn chat_timeline_refresh_plan_rebuilds_message_list_after_empty_placeholder() {
        let old = chat_timeline_render_state(7, "structured", &[]);
        let new_items = vec![ChatTimelineItem::Message(chat_message_record(
            1, "agent", "first", "agent",
        ))];
        let new = chat_timeline_render_state(7, "structured", &new_items);

        assert_eq!(
            chat_timeline_refresh_plan(Some(&old), &new),
            ChatTimelineRefreshPlan::RebuildMessages
        );
    }

    #[test]
    fn external_chat_surface_refresh_callback_uses_received_kind() {
        let source = include_str!("session_surface.rs");
        let start = source
            .find("let refresh_messages_for_external: Rc<dyn Fn(i64)>")
            .expect("external message refresh closure exists");
        let end = source[start..]
            .find("let refresh_thread_nav_for_external")
            .map(|offset| start + offset)
            .expect("thread nav refresh follows message refresh");
        let callback_region = &source[start..end];

        assert!(callback_region.contains("ChatTimelineRefreshPlan::Append"));
        assert!(callback_region.contains("ChatTimelineRefreshPlan::RebuildMessages"));
        assert!(!callback_region.contains("refresh_view();"));
    }

    #[test]
    fn external_chat_message_refresh_loads_timeline_in_background() {
        let source = include_str!("session_surface.rs");
        let start = source
            .find("let refresh_messages_for_external: Rc<dyn Fn(i64)>")
            .expect("external message refresh closure exists");
        let end = source[start..]
            .find("let refresh_thread_nav_for_external")
            .map(|offset| start + offset)
            .expect("thread nav refresh follows message refresh");
        let callback_region = &source[start..end];

        assert!(
            callback_region.contains("spawn_background_job("),
            "message timeline DB loads must not run synchronously in GTK refresh callbacks"
        );
    }

    #[test]
    fn provider_switch_creation_notifies_external_chat_tabs() {
        let source = include_str!("session_surface.rs");
        let start = source
            .find("ModelSelectionRoute::CrossProvider(kind) =>")
            .expect("cross-provider switch branch exists");
        let end = source[start..]
            .find("if let Some(sync) = clone_refresh_chat_surface_controller")
            .map(|offset| start + offset)
            .expect("cross-provider branch ends before live-control sync");
        let success_branch = &source[start..end];

        assert!(
            success_branch.contains("external_chat_tabs.on_threads_changed"),
            "provider switch creates and selects a chat thread before refresh_view, so it must notify external chat tabs directly"
        );
    }

    #[test]
    fn cloned_refresh_controller_drops_borrow_before_callback_runs() {
        let controller: RefreshChatSurfaceController = Rc::new(RefCell::new(None));
        let observed = Rc::new(Cell::new(false));
        *controller.borrow_mut() = Some(Rc::new({
            let controller = controller.clone();
            let observed = observed.clone();
            move || {
                let _same_controller = clone_refresh_chat_surface_controller(&controller);
                observed.set(true);
            }
        }));

        let Some(refresh) = clone_refresh_chat_surface_controller(&controller) else {
            panic!("expected refresh controller");
        };
        refresh();

        assert!(observed.get());
    }

    #[test]
    fn selected_chat_thread_wins_over_current_provider_when_refreshing() {
        let threads = vec![
            ChatThreadRecord {
                id: 7,
                workspace_id: 1,
                provider: "codex".to_owned(),
                title: "Codex".to_owned(),
                status: "active".to_owned(),
                native_thread_id: None,
                harness_metadata: None,
                created_at: "now".to_owned(),
                updated_at: "now".to_owned(),
                archived_at: None,
            },
            ChatThreadRecord {
                id: 8,
                workspace_id: 1,
                provider: "claude".to_owned(),
                title: "Claude".to_owned(),
                status: "active".to_owned(),
                native_thread_id: None,
                harness_metadata: Some("model=claude-sonnet-4-20250514".to_owned()),
                created_at: "now".to_owned(),
                updated_at: "now".to_owned(),
                archived_at: None,
            },
        ];

        assert_eq!(
            preferred_thread_for_selected_chat(&threads, Some(8), SessionKind::Codex),
            Some(8)
        );
        assert_eq!(
            selected_thread_harness_state(&threads[1]),
            (
                SessionKind::Claude,
                Some("claude-sonnet-4-20250514".to_owned())
            )
        );
    }

    #[test]
    fn composer_drafts_are_stored_per_thread() {
        let drafts = RefCell::new(HashMap::<i64, String>::new());

        remember_composer_draft(&drafts, Some(7), "codex draft");
        remember_composer_draft(&drafts, Some(8), "claude draft");

        assert_eq!(composer_draft_for_thread(&drafts, Some(7)), "codex draft");
        assert_eq!(composer_draft_for_thread(&drafts, Some(8)), "claude draft");

        remember_composer_draft(&drafts, Some(7), "   ");
        assert_eq!(composer_draft_for_thread(&drafts, Some(7)), "");
        assert_eq!(composer_draft_for_thread(&drafts, Some(8)), "claude draft");
    }

    #[test]
    fn sessionless_startup_error_targets_selected_codex_thread() {
        let records = Vec::<ProcessRecord>::new();
        let session_threads = RefCell::new(HashMap::<i64, i64>::new());

        let thread_id = codex_thread_id_for_startup_error(
            None,
            None,
            &session_threads,
            &records,
            SessionKind::Codex,
            Some(7),
        );

        assert_eq!(thread_id, Some(7));
    }

    #[test]
    fn guarded_gtk_callback_recovers_from_panic() {
        let result = guarded_gtk_callback(gtk::glib::Propagation::Proceed, || {
            panic!("simulated callback panic");
        });

        assert_eq!(result, gtk::glib::Propagation::Proceed);
    }

    #[test]
    fn resolve_or_create_thread_id_creates_codex_thread_without_borrow_panic() {
        let threads = RefCell::new(Vec::<ChatThreadRecord>::new());
        let selected = RefCell::new(None);

        let thread_id = resolve_or_create_thread_id_for_send(
            &threads,
            &selected,
            SessionKind::Codex,
            |title| {
                Ok(ChatThreadRecord {
                    id: 41,
                    workspace_id: 7,
                    provider: "codex".to_owned(),
                    title,
                    status: "active".to_owned(),
                    native_thread_id: None,
                    harness_metadata: None,
                    created_at: "now".to_owned(),
                    updated_at: "now".to_owned(),
                    archived_at: None,
                })
            },
        )
        .unwrap();

        assert_eq!(thread_id, 41);
        assert_eq!(*selected.borrow(), Some(41));
        assert_eq!(threads.borrow().len(), 1);
        assert_eq!(threads.borrow()[0].provider, "codex");
    }

    #[test]
    fn gtk_sources_do_not_own_or_poll_ptys() {
        let session_surface = include_str!("session_surface.rs");
        let terminal = include_str!("terminal.rs");
        let workspace_command_center = include_str!("workspace_command_center.rs");
        let gtk_sources = [
            ("session_surface.rs", session_surface),
            ("terminal.rs", terminal),
            ("workspace_command_center.rs", workspace_command_center),
        ];
        let pty_session = concat!("Pty", "Session");
        let pty_spawn = concat!("Pty", "Session", "::", "spawn");
        let proc_prefix = concat!("/", "proc", "/");
        let fd_zero = concat!("fd", "/", "0");
        let session_poll_timer = concat!(
            "timeout",
            "_add",
            "_local(Duration::from_",
            "millis(",
            "SESSION",
            "_POLL",
            "_INTERVAL",
            "_MS",
            ")"
        );
        let run_console_live = concat!("WorkspaceRunConsoleTerminalConnection", "::", "Live");
        let terminal_ownership_marker = concat!("active", "_", "ptys");
        let run_console_timer = concat!(
            "timeout",
            "_add",
            "_local(std::time::Duration::from_",
            "millis(100), ",
            "move ||"
        );

        for (path, source) in gtk_sources {
            assert!(
                !source.contains(pty_session),
                "{path} must not import, store, or wrap GTK PTY sessions"
            );
            assert!(
                !source.contains(pty_spawn),
                "{path} must not spawn PTYs directly"
            );
            assert!(
                !source.contains(proc_prefix) && !source.contains(fd_zero),
                "{path} must not reattach PTYs through process fd paths"
            );
        }

        assert!(
            !session_surface.contains(session_poll_timer),
            "session/chat PTY poll loop must be removed from GTK"
        );
        assert!(
            !terminal.contains(terminal_ownership_marker),
            "shell terminal PTY ownership must be removed from GTK"
        );
        assert!(
            !workspace_command_center.contains(run_console_live),
            "run-console PTY ownership must be removed from GTK"
        );
        assert!(
            !workspace_command_center.contains(run_console_timer),
            "run-console PTY poll loop must be removed from GTK"
        );
    }

    #[test]
    fn gtk_chat_archcar_updates_are_not_timer_or_probe_driven() {
        let source = include_str!("session_surface.rs");
        let chat_refresh_poll = concat!("start", "_chat", "_surface", "_refresh", "_poll");
        let codex_status_probe = concat!("request", "_codex", "_status", "_probes");

        assert!(
            !source.contains(chat_refresh_poll),
            "GTK chat must not use a timer to poll archcar updates"
        );
        assert!(
            !source.contains(codex_status_probe),
            "GTK chat must not reconcile Codex readiness through a refresh-time status probe loop"
        );
    }
}
