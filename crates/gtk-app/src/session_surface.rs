use gtk::prelude::*;
use gtk::{
    Align, Box as GBox, Button, CheckButton, ComboBoxText, Entry, EventControllerKey, GestureClick,
    Image, Label, Orientation, Overlay, Popover, Revealer, RevealerTransitionType, ScrolledWindow,
    Spinner, TextBuffer, TextView, ToggleButton, Widget,
};
use linux_archductor_core::archcar::protocol::{ArchcarEvent, ArchcarInputKind, ArchcarResponse};
use linux_archductor_core::codex_tui::{
    merge_screen_messages, parse_codex_context_usage, parse_codex_file_change_block,
    parse_codex_inline_event, parse_codex_screen_messages,
    CodexFileChangeAction as CoreCodexFileChangeAction,
    CodexFileReference as CoreCodexFileReference, CodexInlineEvent as CoreCodexInlineEvent,
    CodexTranscriptEvent, ScreenMessage, ScreenMessageRole,
};
#[cfg(test)]
use linux_archductor_core::session_state::AgentSessionState;
use linux_archductor_core::workspace::{
    ChatEventRecord, ChatMessageRecord, ChatThreadRecord, ProcessRecord, ProcessStatus,
    SessionHarnessOptions, SessionKind, WorkspaceStore,
};
use std::any::Any;
use std::backtrace::Backtrace;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use std::{env, fs as stdfs};
use tracing::{debug, error, info, trace, warn};

use crate::archcar_async::{
    clear_archcar_ready, note_archcar_ready, AsyncArchcarBridge, AsyncArchcarMessage,
    AsyncArchcarResponse,
};
use crate::buttons::{
    icon_button, resolve_icon_name, style_icon_button, style_text_button, text_button,
};
use crate::motion::{append_revealed, clear_box};
use crate::state::AppState;
use crate::terminal::terminal_display_text;

const SESSION_SCROLLBACK_LINES: usize = 2_000;
const SESSION_TAIL_HISTORY: usize = 120;
const DEFAULT_CHAT_TITLE_PREFIX: &str = "New Chat";
const REVEAL_EXISTING_CHAT_REFRESH_ROWS: bool = false;
static NEXT_CHAT_WAKE_ID: AtomicUsize = AtomicUsize::new(1);

thread_local! {
    static CHAT_WAKE_REGISTRY: RefCell<HashMap<usize, Rc<dyn Fn()>>> = RefCell::new(HashMap::new());
}

pub type ExternalThreadSelectionController = Rc<RefCell<Option<Rc<dyn Fn(Option<i64>)>>>>;
type RefreshChatSurfaceController = Rc<RefCell<Option<Rc<dyn Fn()>>>>;
type SwitchChatHarnessController = Rc<RefCell<Option<Rc<dyn Fn(SessionKind)>>>>;

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ContextUsageDisplayState {
    percent_label: String,
    css_class: &'static str,
    tooltip: String,
}

type ChatRenderRecordSignature = (i64, Option<i64>, ProcessStatus, Option<i32>, Option<String>);
type ChatRenderThreadSignature = (i64, String, String, String, String);
type ChatRenderMessageSignature = (i64, String, Option<i64>, String, String);
type ChatRenderEventSignature = (i64, String, i64, String, String, usize);

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
    render_state: &'static str,
    runtime_summary: Option<String>,
}

#[derive(Clone)]
pub struct ExternalChatTabs {
    pub on_threads_changed: Rc<dyn Fn(Vec<ChatThreadRecord>, Option<i64>)>,
    pub selection_controller: ExternalThreadSelectionController,
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
    external_chat_tabs: Option<ExternalChatTabs>,
) -> GBox {
    let root = GBox::new(Orientation::Vertical, 0);
    root.add_css_class("chat-surface");
    root.set_vexpand(true);
    root.set_hexpand(true);

    if include_header {
        root.append(&session_header_row(
            repository_name,
            branch_name,
            collapse_sidebar.clone(),
        ));
    }

    let selected_harness = Rc::new(RefCell::new(SessionKind::Codex));
    let selected_model = Rc::new(RefCell::new(None::<String>));
    let reasoning_mode = Rc::new(RefCell::new(Some("high".to_owned())));
    let thread_state = Rc::new(RefCell::new(Vec::<ChatThreadRecord>::new()));
    let selected_thread: Rc<RefCell<Option<i64>>> =
        Rc::new(RefCell::new(app_state.selected_chat_thread()));
    let pending_commands = Rc::new(RefCell::new(HashMap::<i64, Vec<String>>::new()));
    let pending_archcar_inputs =
        Rc::new(RefCell::new(HashMap::<i64, Vec<QueuedArchcarInput>>::new()));
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

    let interface_btn = mode_menu_button("Codex", "code-symbolic", &["Codex", "Claude"], 0, {
        let selected_harness = selected_harness.clone();
        let reasoning_mode = reasoning_mode.clone();
        let refresh_chat_surface = refresh_chat_surface.clone();
        let switch_chat_harness = switch_chat_harness.clone();
        let selected_model = selected_model.clone();
        let pending_commands = pending_commands.clone();
        let selected_thread = selected_thread.clone();
        let sync_live_controls = sync_live_controls.clone();
        Rc::new(move |index| {
            let kind = session_kind_from_index(index);
            select_harness_and_dispatch(
                selected_harness.as_ref(),
                reasoning_mode.as_ref(),
                kind,
                switch_chat_harness.borrow().as_ref(),
                refresh_chat_surface.borrow().as_ref(),
                None,
            );
            *selected_model.borrow_mut() = None;
            if kind != SessionKind::Codex {
                if let Some(thread_id) = *selected_thread.borrow() {
                    pending_commands.borrow_mut().remove(&thread_id);
                }
            }
            if let Some(sync) = sync_live_controls.borrow().as_ref().cloned() {
                sync();
            }
        })
    });
    let model_btn = mode_menu_button("Default", "M", &["Default", "gpt-5", "gpt-5-mini"], 0, {
        let selected_model = selected_model.clone();
        let selected_harness = selected_harness.clone();
        let selected_thread = selected_thread.clone();
        let pending_commands = pending_commands.clone();
        Rc::new(move |index| {
            let model = session_model_from_index(index);
            *selected_model.borrow_mut() = model.clone();
            if *selected_harness.borrow() == SessionKind::Codex {
                if let (Some(thread_id), Some(command)) = (
                    *selected_thread.borrow(),
                    codex_model_command(model.as_deref()),
                ) {
                    queue_thread_command(&pending_commands, thread_id, command);
                }
            }
        })
    });
    let thinking_btn =
        mode_menu_button("High", "◔", &["Low", "Medium", "High", "Extra high"], 2, {
            let reasoning_mode = reasoning_mode.clone();
            let selected_harness = selected_harness.clone();
            let selected_thread = selected_thread.clone();
            let pending_commands = pending_commands.clone();
            Rc::new(move |index| {
                let level = session_reasoning_mode_from_index(index);
                *reasoning_mode.borrow_mut() = Some(level.clone());
                if *selected_harness.borrow() == SessionKind::Codex {
                    if let (Some(thread_id), Some(command)) =
                        (*selected_thread.borrow(), codex_reasoning_command(&level))
                    {
                        queue_thread_command(&pending_commands, thread_id, command);
                    }
                }
            })
        });
    thinking_btn.add_css_class("chat-thinking-menu");

    left_group.append(&interface_btn);
    left_group.append(&model_btn);
    left_group.append(&thinking_btn);

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
    right_group.append(&context_usage);
    right_group.append(&send_btn);

    toolbar.append(&left_group);
    toolbar.append(&right_group);

    let sync_live_controls_fn: Rc<dyn Fn()> = Rc::new({
        let selected_harness = selected_harness.clone();
        let model_btn = model_btn.clone();
        let thinking_btn = thinking_btn.clone();
        move || {
            let controls = visible_live_controls_for_provider(session_kind_provider(
                *selected_harness.borrow(),
            ));
            model_btn.set_visible(controls.iter().any(|control| control == "model"));
            thinking_btn.set_visible(controls.iter().any(|control| control == "thinking"));
        }
    });
    *sync_live_controls.borrow_mut() = Some(sync_live_controls_fn.clone());
    sync_live_controls_fn();

    composer_box.append(&input_shell);
    composer_box.append(&toolbar);
    composer_wrap.append(&composer_box);
    chat_overlay.add_overlay(&composer_wrap);
    chat_overlay.set_measure_overlay(&composer_wrap, false);

    let record_state = Rc::new(RefCell::new(Vec::<ProcessRecord>::new()));
    let last_render_signature = Rc::new(RefCell::new(None::<ChatRenderSignature>));
    let buffer = input_view.buffer();
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
        Rc::new(move || {
            let start = buffer_for_update.start_iter();
            let end = buffer_for_update.end_iter();
            let text = buffer_for_update.text(&start, &end, true);
            let has_text = !text.as_str().trim().is_empty();
            let ready = if *selected_harness.borrow() == SessionKind::Codex {
                composer_ready_for_codex_thread(
                    *selected_thread.borrow(),
                    &record_state.borrow(),
                    archcar_ready_cache.as_ref(),
                    codex_startup_states.as_ref(),
                )
            } else {
                true
            };
            if !ready {
                placeholder.set_text("Codex is starting...");
                placeholder.set_visible(true);
                send_btn.set_sensitive(false);
                send_btn.remove_css_class("chat-send-btn-active");
            } else {
                placeholder.set_text("Ask to make changes, @mention files, or run /commands");
                placeholder.set_visible(!has_text);
                send_btn.set_sensitive(has_text);
                if has_text {
                    send_btn.add_css_class("chat-send-btn-active");
                } else {
                    send_btn.remove_css_class("chat-send-btn-active");
                }
            }
            input_view.queue_draw();
        })
    };
    buffer.connect_changed({
        let update = update_composer_state.clone();
        move |_| update()
    });
    update_composer_state();

    let selected_session: Rc<RefCell<Option<i64>>> = Rc::new(RefCell::new(None));
    let active_sessions: Rc<RefCell<HashSet<i64>>> = Rc::new(RefCell::new(HashSet::new()));
    let last_output = Rc::new(RefCell::new(HashMap::<i64, Instant>::new()));
    let refresh_chat_surface_for_view = refresh_chat_surface.clone();

    let refresh_view = {
        let database_path = database_path.clone();
        let workspace = _workspace_name.to_owned();
        let messages = messages.clone();
        let thread_row = thread_row.clone();
        let record_state = record_state.clone();
        let thread_state = thread_state.clone();
        let selected_session = selected_session.clone();
        let selected_thread = selected_thread.clone();
        let last_render_signature = last_render_signature.clone();
        let selected_harness = selected_harness.clone();
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
        let external_chat_tabs = external_chat_tabs.clone();
        let context_usage = context_usage.clone();
        Rc::new(move || {
            debug!(workspace = %workspace, "chat refresh_view start");
            let (loaded, loaded_threads) = match WorkspaceStore::open(database_path.clone())
                .and_then(|store| {
                    let sessions = store.list_sessions(&workspace)?;
                    let threads = store.list_chat_threads(&workspace)?;
                    Ok((sessions, threads))
                }) {
                Ok(loaded) => loaded,
                Err(err) => {
                    error!(workspace = %workspace, error = %err, "chat refresh_view failed to load workspace state");
                    clear_box(&thread_row);
                    clear_box(&messages);
                    apply_context_usage_state(&context_usage, None);
                    let label =
                        Label::new(Some(&session_refresh_error_text("load sessions", &err)));
                    label.add_css_class("chat-agent-text");
                    label.set_selectable(true);
                    label.set_wrap(true);
                    label.set_xalign(0.0);
                    append_chat_refresh_row(&messages, &label);
                    return;
                }
            };
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
                workspace = %workspace,
                session_count = record_state.borrow().len(),
                thread_count = thread_state.borrow().len(),
                "chat refresh_view loaded workspace state"
            );

            let current_kind = *selected_harness.borrow();
            let selected_thread_id = *selected_thread.borrow();
            let mut archcar_changed = false;
            while let Some(message) = archcar_bridge.try_recv() {
                match message {
                    AsyncArchcarMessage::Event(event) => {
                        let _ = update_working_indicator_for_archcar_event(
                            &event,
                            &record_state.borrow(),
                            archcar_session_threads.as_ref(),
                            current_kind,
                            selected_thread_id,
                            pending_archcar_inputs.as_ref(),
                            working_threads.as_ref(),
                        );
                        handle_archcar_event(
                            &event,
                            &record_state.borrow(),
                            archcar_ready_cache.as_ref(),
                            codex_startup_states.as_ref(),
                            archcar_session_threads.as_ref(),
                            current_kind,
                            selected_thread_id,
                            codex_ready.as_ref(),
                            update_composer_for_view.as_ref(),
                        );
                        archcar_changed = true;
                    }
                    AsyncArchcarMessage::Response(response) => {
                        archcar_changed |= handle_archcar_response(
                            response,
                            &workspace,
                            archcar_bridge.clone(),
                            archcar_ready_cache.as_ref(),
                            inflight_archcar_actions.as_ref(),
                            pending_commands.as_ref(),
                            pending_archcar_inputs.as_ref(),
                            codex_startup_states.as_ref(),
                            working_threads.as_ref(),
                            codex_ready.as_ref(),
                            update_composer_for_view.as_ref(),
                        );
                    }
                    AsyncArchcarMessage::BridgeError { message } => {
                        if let Some(visible_message) =
                            bridge_error_state.borrow_mut().record(&message)
                        {
                            warn!(workspace = %workspace, error = %visible_message, "archcar bridge error");
                            if current_kind == SessionKind::Codex {
                                if let Some(thread_id) = selected_thread_id {
                                    apply_codex_startup_signal(
                                        &mut codex_startup_states.borrow_mut(),
                                        CodexStartupSignal::Error {
                                            thread_id,
                                            message: visible_message,
                                        },
                                    );
                                    clear_thread_working(working_threads.as_ref(), thread_id);
                                    set_codex_ready_state(
                                        codex_ready.as_ref(),
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
                let _ = flush_pending_archcar_inputs(
                    &archcar_bridge,
                    &database_path,
                    pending_commands.as_ref(),
                    pending_archcar_inputs.as_ref(),
                    archcar_ready_cache.as_ref(),
                    inflight_archcar_actions.as_ref(),
                    &app_state,
                );
                update_composer_for_view();
            }

            let preferred_thread = *selected_thread.borrow();
            let active_thread = {
                let current = thread_state.borrow();
                preferred_thread_for_kind(&current, preferred_thread, current_kind)
            };
            apply_thread_selection(
                selected_thread.as_ref(),
                active_thread,
                |thread_id| app_state.set_selected_chat_thread(thread_id),
                || update_composer_for_view(),
            );
            if let Some(external_chat_tabs) = external_chat_tabs.as_ref() {
                (external_chat_tabs.on_threads_changed)(
                    thread_state.borrow().clone(),
                    *selected_thread.borrow(),
                );
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
                            move |thread_id| {
                                apply_thread_selection(
                                    selected_thread.as_ref(),
                                    Some(thread_id),
                                    |selected| app_state.set_selected_chat_thread(selected),
                                    || update_composer_state(),
                                );
                                if let Some(refresh_view) =
                                    refresh_chat_surface.borrow().as_ref().cloned()
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
                    let startup_state = if current_kind == SessionKind::Codex {
                        codex_startup_state_for_thread(
                            thread_id,
                            &current,
                            archcar_ready_cache.as_ref(),
                            codex_startup_states.borrow().get(&thread_id).cloned(),
                        )
                    } else {
                        CodexStartupState::Idle
                    };
                    let working_elapsed =
                        working_elapsed_for_thread(working_threads.as_ref(), thread_id);
                    if current_kind == SessionKind::Codex
                        && thread_has_live_codex_session(&current, thread_id)
                        && !matches!(startup_state, CodexStartupState::Error { .. })
                        && !codex_thread_ready_for_ui(
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
                        );
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
                            let (thread_messages, thread_events) = match WorkspaceStore::open(
                                database_path.clone(),
                            )
                            .and_then(|store| {
                                let messages = store.list_chat_messages(thread_id)?;
                                let events = store.list_chat_events(thread_id)?;
                                Ok((messages, events))
                            }) {
                                Ok(timeline) => timeline,
                                Err(err) => {
                                    error!(workspace = %workspace, thread_id, error = %err, "chat refresh_view failed to load thread timeline");
                                    let label = Label::new(Some(&session_refresh_error_text(
                                        "load chat timeline",
                                        &err,
                                    )));
                                    label.add_css_class("chat-agent-text");
                                    label.set_selectable(true);
                                    label.set_wrap(true);
                                    label.set_xalign(0.0);
                                    clear_box(&messages);
                                    *last_render_signature.borrow_mut() = None;
                                    apply_context_usage_state(&context_usage, None);
                                    append_chat_refresh_row(&messages, &label);
                                    return;
                                }
                            };
                            let render_legacy_inline_events =
                                render_legacy_inline_events_for_thread(&thread_events);
                            debug!(
                                workspace = %workspace,
                                thread_id,
                                thread_message_count = thread_messages.len(),
                                thread_timeline_count = thread_messages.len() + thread_events.len(),
                                render_legacy_inline_events,
                                "chat refresh_view loaded persisted chat timeline"
                            );
                            if !thread_messages.is_empty() || !thread_events.is_empty() {
                                let signature = chat_render_signature(
                                    current_kind,
                                    Some(thread_id),
                                    active_record,
                                    startup_state.clone(),
                                    working_elapsed_seconds_for_signature(working_elapsed),
                                    &current,
                                    &thread_state.borrow(),
                                    &thread_messages,
                                    &thread_events,
                                    "timeline",
                                    runtime_summary.clone(),
                                );
                                if chat_render_is_unchanged(
                                    last_render_signature.as_ref(),
                                    signature,
                                ) {
                                    return;
                                }
                                clear_box(&messages);
                                apply_context_usage_state(
                                    &context_usage,
                                    latest_context_usage_from_messages(&thread_messages),
                                );
                                if let Some(elapsed) = working_elapsed {
                                    append_chat_refresh_row(
                                        &messages,
                                        &codex_working_indicator_widget(elapsed),
                                    );
                                } else if let Some(widget) =
                                    codex_startup_state_widget(&startup_state)
                                {
                                    append_chat_refresh_row(&messages, &widget);
                                }
                                let timeline = merge_chat_timeline_for_render(
                                    thread_messages.clone(),
                                    thread_events,
                                );
                                for item in timeline {
                                    match item {
                                        ChatTimelineItem::Message(message) => {
                                            if let Some(widget) = chat_message_widget(
                                                &message,
                                                render_legacy_inline_events,
                                            ) {
                                                append_chat_refresh_row(&messages, &widget);
                                            }
                                        }
                                        ChatTimelineItem::Event(event) => {
                                            append_chat_refresh_row(
                                                &messages,
                                                &chat_event_widget(&event),
                                            );
                                        }
                                    }
                                }
                                return;
                            }
                        }
                    }

                    let signature = chat_render_signature(
                        current_kind,
                        Some(thread_id),
                        active_record,
                        startup_state.clone(),
                        working_elapsed_seconds_for_signature(working_elapsed),
                        &current,
                        &thread_state.borrow(),
                        &[],
                        &[],
                        "empty",
                        runtime_summary.clone(),
                    );
                    if chat_render_is_unchanged(last_render_signature.as_ref(), signature) {
                        return;
                    }
                    clear_box(&messages);
                    apply_context_usage_state(&context_usage, None);
                    if let Some(elapsed) = working_elapsed {
                        append_chat_refresh_row(
                            &messages,
                            &codex_working_indicator_widget(elapsed),
                        );
                    } else if let Some(widget) = codex_startup_state_widget(&startup_state) {
                        append_chat_refresh_row(&messages, &widget);
                    }
                    let empty = Label::new(Some("No messages yet."));
                    empty.add_css_class("chat-agent-text");
                    empty.set_selectable(true);
                    empty.set_wrap(true);
                    empty.set_xalign(0.0);
                    append_chat_refresh_row(&messages, &empty);
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
                        "no_thread",
                        None,
                    );
                    if chat_render_is_unchanged(last_render_signature.as_ref(), signature) {
                        return;
                    }
                    clear_box(&messages);
                    apply_context_usage_state(&context_usage, None);
                    let prompt = format!(
                        "No {} chat yet. Create one or send a message to start one.",
                        session_kind_name(current_kind)
                    );
                    append_chat_refresh_row(&messages, &chat_user_bubble(&prompt));
                }
            }
            debug!(workspace = %workspace, "chat refresh_view complete");
        }) as Rc<dyn Fn()>
    };
    *refresh_chat_surface.borrow_mut() = Some(refresh_view.clone());
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
    let workspace_for_send = _workspace_name.to_owned();
    let selected_harness_for_send = selected_harness.clone();
    let thread_state_for_send = thread_state.clone();
    let selected_thread_for_send = selected_thread.clone();
    let pending_commands_for_send = pending_commands.clone();
    let pending_archcar_inputs_for_send = pending_archcar_inputs.clone();
    let active_sessions_for_send = active_sessions.clone();
    let selected_session_for_send = selected_session.clone();
    let record_state_for_send = record_state.clone();
    let refresh_view_for_send = refresh_view.clone();
    let refresh_for_send = refresh.clone();
    let app_state_for_send = app_state.clone();
    let messages_for_send = messages.clone();
    let archcar_bridge_for_send = archcar_bridge.clone();
    let archcar_ready_cache_for_send = archcar_ready_cache.clone();
    let inflight_archcar_actions_for_send = inflight_archcar_actions.clone();
    let working_threads_for_send = working_threads.clone();
    let codex_ready_for_send = codex_ready.clone();
    let codex_startup_states_for_send = codex_startup_states.clone();
    let update_composer_for_send = update_composer_state.clone();
    let send_text = Rc::new(move |text: String, staged_review: bool| {
        let command = text.trim().to_owned();
        if command.is_empty() {
            return;
        }
        let mut workspace_for_send = workspace_for_send.clone();
        let mut auto_renamed_workspace = false;
        if !staged_review {
            match WorkspaceStore::open(db_for_send.clone()).and_then(|store| {
                store.apply_first_message_workspace_naming(&workspace_for_send, &command)
            }) {
                Ok(Some(workspace)) => {
                    auto_renamed_workspace = workspace.name != workspace_for_send;
                    let workspace_name = workspace.name.clone();
                    let branch = workspace.branch.clone();
                    workspace_for_send = workspace_name.clone();
                    app_state_for_send.set_selected_workspace(Some(workspace_name));
                    info!(
                        workspace = %workspace_for_send,
                        branch = %branch,
                        "applied first-message workspace naming"
                    );
                }
                Ok(None) => {}
                Err(err) => {
                    warn!(
                        workspace = %workspace_for_send,
                        error = %err,
                        "failed to apply first-message workspace naming"
                    );
                }
            }
        }
        let selected_kind = *selected_harness_for_send.borrow();
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
            if let Ok(store) = WorkspaceStore::open(db_for_send.clone()) {
                records = store.list_sessions(&workspace_for_send).unwrap_or_default();
                debug!(
                    workspace = %workspace_for_send,
                    harness = ?selected_kind,
                    session_records = records.len(),
                    "session send stage: loaded record state from store"
                );
            }
        }
        let thread_id = match resolve_or_create_thread_id_for_send(
            thread_state_for_send.as_ref(),
            selected_thread_for_send.as_ref(),
            selected_kind,
            |title| {
                WorkspaceStore::open(db_for_send.clone()).and_then(|store| {
                    store.create_chat_thread(
                        &workspace_for_send,
                        session_kind_provider(selected_kind),
                        &title,
                        None,
                    )
                })
            },
        ) {
            Ok(thread_id) => {
                app_state_for_send.set_selected_chat_thread(Some(thread_id));
                thread_id
            }
            Err(err) => {
                let error = Label::new(Some(&format!("[chat thread] {err:#}")));
                error.add_css_class("chat-agent-text");
                error.set_selectable(true);
                error.set_wrap(true);
                error.set_xalign(0.0);
                append_revealed(&messages_for_send, &error);
                return;
            }
        };
        debug!(
            workspace = %workspace_for_send,
            harness = ?selected_kind,
            thread_id,
            "session send stage: resolved thread"
        );
        let thread = match WorkspaceStore::open(db_for_send.clone())
            .and_then(|store| store.get_chat_thread_record(thread_id))
        {
            Ok(thread) => thread,
            Err(err) => {
                let error = Label::new(Some(&format!("[chat thread] {err:#}")));
                error.add_css_class("chat-agent-text");
                error.set_selectable(true);
                error.set_wrap(true);
                error.set_xalign(0.0);
                append_revealed(&messages_for_send, &error);
                return;
            }
        };
        let should_rename_thread = WorkspaceStore::open(db_for_send.clone())
            .and_then(|store| store.list_chat_messages(thread_id))
            .map(|messages| messages.is_empty() && is_default_chat_thread_title(&thread.title))
            .unwrap_or(false);
        if should_rename_thread {
            if let Some(title) = summarize_chat_title_from_opening_message(&command) {
                if let Ok(store) = WorkspaceStore::open(db_for_send.clone()) {
                    let _ = store.update_chat_thread_title(thread_id, &title);
                }
            }
        }
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
        if matches!(selected_kind, SessionKind::Codex) {
            let running_record = thread_records
                .iter()
                .find(|record| {
                    record.status == ProcessStatus::Running
                        && session_kind_matches_record(record, SessionKind::Codex)
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
                    for control in pending_controls {
                        queue_archcar_control_send(
                            &archcar_bridge_for_send,
                            inflight_archcar_actions_for_send.as_ref(),
                            thread_id,
                            record.id,
                            control,
                        );
                    }
                    let kind = if staged_review {
                        ArchcarInputKind::ReviewPrompt
                    } else {
                        ArchcarInputKind::User
                    };
                    if !queue_archcar_user_send(
                        &archcar_bridge_for_send,
                        inflight_archcar_actions_for_send.as_ref(),
                        thread_id,
                        record.id,
                        command.clone(),
                        kind.clone(),
                    ) {
                        append_session_status_message(
                            &messages_for_send,
                            "[archcar] Request channel is closed. Reopen the workspace or restart the app.",
                        );
                        return;
                    }
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
                    if auto_renamed_workspace {
                        refresh_for_send();
                    } else {
                        refresh_view_for_send();
                        refresh_for_send();
                    }
                    return;
                }
            }

            if request_archcar_ensure(
                &archcar_bridge_for_send,
                inflight_archcar_actions_for_send.as_ref(),
                workspace_for_send.clone(),
                Some(thread_id),
            ) {
                queue_archcar_input(
                    &pending_archcar_inputs_for_send,
                    thread_id,
                    command.clone(),
                    if staged_review {
                        ArchcarInputKind::ReviewPrompt
                    } else {
                        ArchcarInputKind::User
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
                return;
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
            if auto_renamed_workspace {
                refresh_for_send();
            } else {
                refresh_view_for_send();
                refresh_for_send();
            }
            return;
        }
        let running_record = thread_records
            .iter()
            .find(|record| {
                record.status == ProcessStatus::Running
                    && session_kind_matches_record(record, selected_kind)
            })
            .cloned();

        let Some(record) = running_record else {
            let token =
                archcar_bridge_for_send.spawn_session(workspace_for_send.clone(), selected_kind);
            if let Some(token) = token {
                inflight_archcar_actions_for_send.borrow_mut().insert(
                    token,
                    PendingArchcarAction::EnsureWorkspace {
                        workspace: workspace_for_send.clone(),
                        thread_id: Some(thread_id),
                    },
                );
            }
            if let Ok(writer) = WorkspaceStore::open(db_for_send.clone()) {
                let _ =
                    writer.append_chat_message(thread_id, "user", &command, "queued_before_spawn");
            }
            let queued = Label::new(Some(
                "[session start] Runtime session requested through archcar. Send again once the session is ready.",
            ));
            queued.add_css_class("chat-agent-text");
            queued.set_selectable(true);
            queued.set_wrap(true);
            queued.set_xalign(0.0);
            append_revealed(&messages_for_send, &queued);
            if auto_renamed_workspace {
                refresh_for_send();
            } else {
                refresh_view_for_send();
                refresh_for_send();
            }
            return;
        };

        let process_id = record.id;
        *selected_session_for_send.borrow_mut() = Some(process_id);
        app_state_for_send.set_selected_agent_session(Some(process_id));
        active_sessions_for_send.borrow_mut().insert(process_id);

        for control in flush_pending_commands_for_send(&pending_commands_for_send, thread_id) {
            queue_archcar_control_send(
                &archcar_bridge_for_send,
                inflight_archcar_actions_for_send.as_ref(),
                thread_id,
                process_id,
                control,
            );
        }
        let input_kind = if staged_review {
            ArchcarInputKind::ReviewPrompt
        } else {
            ArchcarInputKind::User
        };
        queue_archcar_user_send(
            &archcar_bridge_for_send,
            inflight_archcar_actions_for_send.as_ref(),
            thread_id,
            process_id,
            command.clone(),
            input_kind,
        );
        if staged_review {
            app_state_for_send.set_staged_review_prompt(None);
        }
        if auto_renamed_workspace {
            refresh_for_send();
        } else {
            refresh_view_for_send();
        }
    });

    {
        let db_for_switch = database_path.clone();
        let workspace_for_switch = _workspace_name.to_owned();
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
        let switch_action = Rc::new(move |next_kind: SessionKind| {
            let (records, threads) =
                match WorkspaceStore::open(db_for_switch.clone()).map(|store| {
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
                        let error = Label::new(Some(&format!("[session switch] {err:#}")));
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
                || update_composer_for_switch(),
            );
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
        switch_action(*selected_harness.borrow());
    }

    if let Some(external_chat_tabs) = external_chat_tabs.as_ref() {
        let selected_thread = selected_thread.clone();
        let refresh_chat_surface = refresh_chat_surface.clone();
        let app_state = app_state.clone();
        let update_composer_state = update_composer_state.clone();
        *external_chat_tabs.selection_controller.borrow_mut() = Some(Rc::new(move |thread_id| {
            apply_thread_selection(
                selected_thread.as_ref(),
                thread_id,
                |selected| app_state.set_selected_chat_thread(selected),
                || update_composer_state(),
            );
            if let Some(refresh_view) = refresh_chat_surface.borrow().as_ref().cloned() {
                refresh_view();
            }
        }));
    }

    if let Some(prompt) = app_state.take_pending_chat_prompt() {
        let send_text = send_text.clone();
        gtk::glib::idle_add_local_once(move || {
            (send_text)(prompt, false);
        });
    }

    let send_text_for_button = send_text.clone();
    let composer_keybind = EventControllerKey::new();
    composer_keybind.connect_key_pressed({
        let send_text = send_text.clone();
        let buffer = buffer.clone();
        move |_, keyval, _, modifiers| {
            guarded_gtk_callback(gtk::glib::Propagation::Proceed, || {
                if !should_send_composer_message(keyval, modifiers) {
                    return gtk::glib::Propagation::Proceed;
                }

                let command = buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), true)
                    .to_string();
                if command.trim().is_empty() {
                    return gtk::glib::Propagation::Stop;
                }

                buffer.set_text("");
                (send_text)(command, false);
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
        let send_text = send_text_for_button.clone();
        let buffer = buffer.clone();
        move |_| {
            let command = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), true)
                .to_string();
            if command.trim().is_empty() {
                return;
            }
            buffer.set_text("");
            (send_text)(command, false);
        }
    });
    new_chat_btn.connect_clicked({
        let database_path = database_path.clone();
        let workspace_name = _workspace_name.to_owned();
        let selected_harness = selected_harness.clone();
        let thread_state = thread_state.clone();
        let selected_thread = selected_thread.clone();
        let app_state = app_state.clone();
        let refresh_view = refresh_view.clone();
        let update_composer_state = update_composer_state.clone();
        move |_| {
            let kind = *selected_harness.borrow();
            let title = default_chat_thread_title(kind, &thread_state.borrow());
            match WorkspaceStore::open(database_path.clone()).and_then(|store| {
                store.create_chat_thread(
                    &workspace_name,
                    session_kind_provider(kind),
                    &title,
                    None,
                )
            }) {
                Ok(thread) => {
                    thread_state.borrow_mut().insert(0, thread.clone());
                    apply_thread_selection(
                        selected_thread.as_ref(),
                        Some(thread.id),
                        |thread_id| app_state.set_selected_chat_thread(thread_id),
                        || update_composer_state(),
                    );
                    refresh_view();
                }
                Err(err) => {
                    error!(workspace = %workspace_name, error = %err, "failed to create chat thread");
                }
            }
        }
    });
    input_view.buffer().connect_changed({
        let update = update_composer_state.clone();
        move |_| update()
    });

    root
}

pub(crate) fn session_header_row(
    repository_name: &str,
    branch_name: &str,
    collapse_sidebar: Rc<dyn Fn()>,
) -> GBox {
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
    header
}

fn chat_user_bubble(text: &str) -> GBox {
    let row = GBox::new(Orientation::Horizontal, 0);
    row.set_halign(gtk::Align::Fill);
    row.set_hexpand(true);
    row.add_css_class("chat-user-row");

    let bubble = Label::new(Some(text));
    bubble.add_css_class("chat-user-bubble");
    bubble.set_selectable(true);
    bubble.set_wrap(true);
    bubble.set_xalign(0.0);
    bubble.set_hexpand(true);
    bubble.set_halign(gtk::Align::Fill);
    row.append(&bubble);
    row
}

fn append_chat_refresh_row<W: IsA<Widget>>(container: &GBox, child: &W) {
    if reveal_existing_chat_refresh_rows() {
        append_revealed(container, child);
    } else {
        container.append(child);
    }
}

fn reveal_existing_chat_refresh_rows() -> bool {
    REVEAL_EXISTING_CHAT_REFRESH_ROWS
}

fn install_archcar_wake(root: &GBox, bridge: &AsyncArchcarBridge, refresh_view: Rc<dyn Fn()>) {
    let wake_id = NEXT_CHAT_WAKE_ID.fetch_add(1, Ordering::Relaxed);
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
        let root_ref = root_ref.clone();
        main_context.invoke(move || {
            if root_ref.upgrade().is_none() {
                CHAT_WAKE_REGISTRY.with(|registry| {
                    registry.borrow_mut().remove(&wake_id);
                });
                return;
            }
            let refresh =
                CHAT_WAKE_REGISTRY.with(|registry| registry.borrow().get(&wake_id).cloned());
            if let Some(refresh) = refresh {
                refresh();
            }
        });
    });
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
    let label = Label::new(Some(&format!("{}\n{}", event.role.label(), event.body)));
    label.add_css_class("chat-agent-text");
    label.set_selectable(true);
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_margin_bottom(18);
    label.upcast()
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
    let title = match role {
        SessionTranscriptRole::Skill => skill_name_from_read_header(header)
            .map(str::to_owned)
            .unwrap_or_else(|| path.display().to_string()),
        _ => format!("Read {}", path.display()),
    };
    Some(CodexInlineEvent {
        kind: match role {
            SessionTranscriptRole::Skill => CodexInlineEventKind::Skill,
            _ => CodexInlineEventKind::Tool,
        },
        title,
        subtitle: Some("File preview".to_owned()),
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
            (message.timeline_seq.unwrap_or(message.id), 0, message.id)
        }
        ChatTimelineItem::Event(event) => (event.timeline_seq, 1, event.id),
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
        render_state,
        runtime_summary,
    }
}

fn chat_message_widget(
    message: &ChatMessageRecord,
    render_legacy_inline_events: bool,
) -> Option<Widget> {
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
            let content = chat_agent_message_display_content(message, render_legacy_inline_events);
            if content.trim().is_empty() {
                return None;
            }
            let label = Label::new(Some(&content));
            label.add_css_class("chat-agent-text");
            label.set_selectable(true);
            label.set_wrap(true);
            label.set_xalign(0.0);
            label.set_hexpand(true);
            label.set_margin_bottom(18);
            Some(label.upcast())
        }
    }
}

fn chat_agent_message_display_content(
    message: &ChatMessageRecord,
    render_legacy_inline_events: bool,
) -> String {
    if render_legacy_inline_events {
        message.content.clone()
    } else {
        strip_codex_status_blocks(&message.content)
    }
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
        .unwrap_or_else(|| {
            let label = Label::new(Some(&event.body));
            label.add_css_class("chat-agent-text");
            label.set_selectable(true);
            label.set_wrap(true);
            label.set_xalign(0.0);
            label.set_hexpand(true);
            label.set_margin_bottom(18);
            label.upcast()
        })
}

fn render_legacy_inline_events_for_thread(thread_events: &[ChatEventRecord]) -> bool {
    thread_events.is_empty()
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
                        "context" => {
                            linux_archductor_core::codex_tui::CodexFileChangeLineKind::Context
                        }
                        "added" => linux_archductor_core::codex_tui::CodexFileChangeLineKind::Added,
                        "deleted" => {
                            linux_archductor_core::codex_tui::CodexFileChangeLineKind::Deleted
                        }
                        _ => return None,
                    };
                    Some(linux_archductor_core::codex_tui::CodexFileChangeLine {
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
                linux_archductor_core::codex_tui::CodexFileChange {
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

fn format_codex_file_change_event(
    change: &linux_archductor_core::codex_tui::CodexFileChange,
) -> String {
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
            linux_archductor_core::codex_tui::CodexFileChangeLineKind::Context => " ",
            linux_archductor_core::codex_tui::CodexFileChangeLineKind::Added => "+",
            linux_archductor_core::codex_tui::CodexFileChangeLineKind::Deleted => "-",
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

fn inline_event_chip_label(event: &CodexInlineEvent, expanded: bool) -> String {
    let marker = if expanded { '-' } else { '+' };
    format!("{marker} {}", inline_event_chip_name(event))
}

fn inline_event_chip_name(event: &CodexInlineEvent) -> String {
    event
        .path
        .as_ref()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| event.title.clone())
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
    let group = GBox::new(Orientation::Vertical, 8);
    group.set_hexpand(true);
    group.set_margin_bottom(18);
    for event in events {
        group.append(&inline_event_widget(event));
    }
    group.upcast()
}

fn inline_event_widget(event: &CodexInlineEvent) -> Widget {
    let root = GBox::new(Orientation::Vertical, 8);
    root.add_css_class("chat-inline-event");
    if let Some(class) = inline_event_status_css_class(event.status) {
        root.add_css_class(class);
    }
    root.set_hexpand(true);

    let toggle = ToggleButton::with_label(&inline_event_chip_label(event, false));
    toggle.add_css_class("chat-inline-event-chip");
    toggle.set_halign(Align::Start);
    toggle.set_tooltip_text(Some(&inline_event_tooltip(event)));
    root.append(&toggle);

    let body_text = inline_event_body_text(event);
    let body_preview = truncate_inline_event_body(&body_text, 320);
    let body = Label::new(Some(&body_preview.preview));
    body.add_css_class("chat-inline-event-body");
    body.set_selectable(true);
    body.set_wrap(true);
    body.set_xalign(0.0);
    body.set_margin_top(2);
    let body_revealer = Revealer::new();
    body_revealer.set_transition_type(RevealerTransitionType::SlideDown);
    body_revealer.set_transition_duration(180);
    body_revealer.set_reveal_child(false);
    body_revealer.set_child(Some(&body));
    root.append(&body_revealer);

    toggle.connect_toggled({
        let body = body.clone();
        let body_revealer = body_revealer.clone();
        let full = body_preview.full.clone();
        let preview = body_preview.preview.clone();
        let collapsed_label = inline_event_chip_label(event, false);
        let expanded_label = inline_event_chip_label(event, true);
        move |button| {
            if button.is_active() {
                body.set_text(&full);
                body_revealer.set_reveal_child(true);
                button.set_label(&expanded_label);
            } else {
                body.set_text(&preview);
                body_revealer.set_reveal_child(false);
                button.set_label(&collapsed_label);
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

fn context_usage_display_state(usage: Option<CodexContextUsage>) -> ContextUsageDisplayState {
    let Some(usage) = usage else {
        return ContextUsageDisplayState {
            percent_label: "--".to_owned(),
            css_class: "chat-context-usage-empty",
            tooltip: "Context usage unknown".to_owned(),
        };
    };
    let css_class = match usage.percent {
        0..=69 => "chat-context-usage-normal",
        70..=89 => "chat-context-usage-warning",
        _ => "chat-context-usage-danger",
    };
    let tooltip = match (usage.used_tokens, usage.max_tokens) {
        (Some(used), Some(max)) => format!(
            "Context usage: {} / {} tokens ({}%)",
            format_token_count(used),
            format_token_count(max),
            usage.percent
        ),
        _ => format!("Context usage: {}%", usage.percent),
    };
    ContextUsageDisplayState {
        percent_label: format!("{}%", usage.percent),
        css_class,
        tooltip,
    }
}

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

fn context_usage_widget() -> Label {
    let label = Label::new(Some("--"));
    label.add_css_class("chat-context-usage");
    apply_context_usage_state(&label, None);
    label
}

fn apply_context_usage_state(label: &Label, usage: Option<CodexContextUsage>) {
    for class in [
        "chat-context-usage-normal",
        "chat-context-usage-warning",
        "chat-context-usage-danger",
        "chat-context-usage-empty",
    ] {
        label.remove_css_class(class);
    }
    let state = context_usage_display_state(usage);
    label.set_text(&state.percent_label);
    label.set_tooltip_text(Some(&state.tooltip));
    label.add_css_class(state.css_class);
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

fn summarize_chat_title_from_opening_message(message: &str) -> Option<String> {
    let collapsed = message.split_whitespace().collect::<Vec<_>>().join(" ");
    let collapsed = collapsed.trim();
    if collapsed.is_empty() {
        return None;
    }

    let mut title = collapsed
        .split_whitespace()
        .take(6)
        .collect::<Vec<_>>()
        .join(" ");
    if title.chars().count() > 48 {
        title = title.chars().take(48).collect::<String>();
    }
    let title = title
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ':' | ';' | ',' | '.'))
        .trim()
        .to_owned();
    (!title.is_empty()).then_some(title)
}

fn supported_chat_session_kinds() -> &'static [SessionKind] {
    &[SessionKind::Codex, SessionKind::Claude]
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

fn session_kind_from_index(index: usize) -> SessionKind {
    match index {
        1 => SessionKind::Claude,
        _ => SessionKind::Codex,
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

fn session_model_from_index(index: usize) -> Option<String> {
    match index {
        1 => Some("gpt-5".to_owned()),
        2 => Some("gpt-5-mini".to_owned()),
        _ => None,
    }
}

fn codex_model_command(model: Option<&str>) -> Option<String> {
    let model = model?.trim();
    (!model.is_empty()).then(|| format!("/model {model}"))
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
    pending: &RefCell<HashMap<i64, Vec<QueuedArchcarInput>>>,
    thread_id: i64,
    input: String,
    kind: ArchcarInputKind,
) {
    let input = input.trim().to_owned();
    if input.is_empty() {
        return;
    }
    pending
        .borrow_mut()
        .entry(thread_id)
        .or_default()
        .push(QueuedArchcarInput { input, kind });
}

fn visible_live_controls_for_provider(provider: &str) -> Vec<String> {
    match provider {
        "codex" => vec![
            "provider".to_owned(),
            "model".to_owned(),
            "thinking".to_owned(),
        ],
        "claude" => vec!["provider".to_owned()],
        "shell" => vec!["provider".to_owned()],
        _ => vec!["provider".to_owned()],
    }
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
    WorkspaceStore::open(database_path)?
        .stop_session_process(workspace_name, process_id)
        .map(|_| ())
}

fn seed_chat_running_sessions(
    database_path: &Path,
    workspace_name: &str,
    active_sessions: &Rc<RefCell<HashSet<i64>>>,
    last_output: &Rc<RefCell<HashMap<i64, Instant>>>,
) {
    let Ok(store) = WorkspaceStore::open(database_path) else {
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
    let arrow = Image::from_icon_name(resolve_icon_name("pan-down-symbolic"));
    arrow.add_css_class("chat-mode-arrow");
    shell.append(&icon);
    shell.append(&text);
    shell.append(&arrow);
    shell
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
    let sessions = WorkspaceStore::open(database_path)
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
    let events = parse_session_transcript_events(transcript);
    let event_summary = session_transcript_event_summary(&events);
    let transcript = render_session_transcript_events(transcript);

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
    let rendered = render_session_transcript_events(text);
    if rendered == "[no transcript output yet]\n" {
        String::new()
    } else {
        rendered
    }
}

fn parse_session_transcript_events(transcript: &str) -> Vec<SessionTranscriptEvent> {
    let cleaned = terminal_display_text(&trim_session_scrollback(transcript)).to_string();
    let lines = cleaned.lines().collect::<Vec<_>>();
    let mut events = Vec::new();
    let mut codex_agent_messages = Vec::<ScreenMessage>::new();
    let mut codex_agent_event_indices = Vec::<usize>::new();
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
            let (screen, next) = collect_codex_block(&lines, index + 1, "[/codex screen]");
            let parsed = parse_codex_screen_messages(&screen)
                .into_iter()
                .filter(|message| message.role == ScreenMessageRole::Agent)
                .collect::<Vec<_>>();
            sync_codex_agent_events(
                &mut events,
                &mut codex_agent_messages,
                &mut codex_agent_event_indices,
                &parsed,
            );
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

fn sync_codex_agent_events(
    events: &mut Vec<SessionTranscriptEvent>,
    codex_agent_messages: &mut Vec<ScreenMessage>,
    codex_agent_event_indices: &mut Vec<usize>,
    incoming: &[ScreenMessage],
) {
    let previous = codex_agent_messages.clone();
    merge_screen_messages(codex_agent_messages, incoming);
    let shared = previous.len().min(codex_agent_messages.len());

    for index in 0..shared {
        if previous[index].content != codex_agent_messages[index].content {
            if let Some(event_index) = codex_agent_event_indices.get(index).copied() {
                if let Some(event) = events.get_mut(event_index) {
                    event.body = codex_agent_messages[index].content.clone();
                }
            }
        }
    }

    for message in codex_agent_messages.iter().skip(previous.len()) {
        events.push(SessionTranscriptEvent {
            role: SessionTranscriptRole::Agent,
            body: message.content.clone(),
        });
        codex_agent_event_indices.push(events.len() - 1);
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
        message: "Starting Codex...".to_owned(),
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
    thread_has_ready_codex_session(records, thread_id, ready_cache)
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
    records.iter().any(|record| {
        record.status == ProcessStatus::Running
            && record.chat_thread_id == Some(thread_id)
            && session_kind_matches_record(record, SessionKind::Codex)
    })
}

fn thread_has_ready_codex_session(
    records: &[ProcessRecord],
    thread_id: i64,
    ready_cache: &RefCell<HashMap<i64, bool>>,
) -> bool {
    records.iter().any(|record| {
        record.status == ProcessStatus::Running
            && record.chat_thread_id == Some(thread_id)
            && session_kind_matches_record(record, SessionKind::Codex)
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
    let trimmed = line.trim();
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
    ["Added ", "Edited ", "Deleted "].iter().any(|prefix| {
        trimmed
            .strip_prefix(prefix)
            .is_some_and(raw_tool_target_looks_path_like)
    })
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
    let Ok(store) = WorkspaceStore::open(database_path) else {
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

fn provider_status_text(status: &linux_archductor_core::mcp::McpStatus) -> String {
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
    kind: ArchcarInputKind,
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
    },
    ControlSend {
        thread_id: i64,
        session_id: i64,
        command: String,
    },
    UserSend {
        thread_id: i64,
        session_id: i64,
        input: String,
        kind: ArchcarInputKind,
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
    pending_commands: &RefCell<HashMap<i64, Vec<String>>>,
    pending_inputs: &RefCell<HashMap<i64, Vec<QueuedArchcarInput>>>,
    ready_cache: &RefCell<HashMap<i64, bool>>,
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
    app_state: &AppState,
) -> bool {
    let thread_ids = pending_inputs.borrow().keys().copied().collect::<Vec<_>>();
    let mut flushed_any = false;
    for thread_id in thread_ids {
        let records = WorkspaceStore::open(database_path)
            .and_then(|store| store.list_thread_processes(thread_id))
            .unwrap_or_default();
        let Some(record) = records.into_iter().find(|record| {
            record.status == ProcessStatus::Running
                && session_kind_matches_record(record, SessionKind::Codex)
        }) else {
            continue;
        };
        if !ready_cache
            .borrow()
            .get(&record.id)
            .copied()
            .unwrap_or(false)
        {
            debug!(
                thread_id,
                process_id = record.id,
                "archcar session not ready; deferring queued inputs"
            );
            continue;
        }

        let pending_controls = flush_pending_commands_for_send(pending_commands, thread_id);
        for control in pending_controls {
            queue_archcar_control_send(
                bridge,
                inflight_actions,
                thread_id,
                record.id,
                control.clone(),
            );
            debug!(
                thread_id,
                process_id = record.id,
                control = %control,
                "archcar control queued"
            );
        }

        let queued = pending_inputs
            .borrow_mut()
            .remove(&thread_id)
            .unwrap_or_default();
        for queued_input in queued {
            queue_archcar_user_send(
                bridge,
                inflight_actions,
                thread_id,
                record.id,
                queued_input.input.clone(),
                queued_input.kind.clone(),
            );
            debug!(
                thread_id,
                process_id = record.id,
                kind = ?queued_input.kind,
                chars = queued_input.input.len(),
                "archcar queued input submitted"
            );
            if matches!(queued_input.kind, ArchcarInputKind::ReviewPrompt) {
                app_state.set_staged_review_prompt(None);
            }
            flushed_any = true;
        }
    }
    flushed_any
}

fn queue_archcar_control_send(
    bridge: &AsyncArchcarBridge,
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
    thread_id: i64,
    session_id: i64,
    command: String,
) {
    let token = bridge.send_input(
        session_id,
        command.clone(),
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
    }
}

fn queue_archcar_user_send(
    bridge: &AsyncArchcarBridge,
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
    thread_id: i64,
    session_id: i64,
    input: String,
    kind: ArchcarInputKind,
) -> bool {
    let token = bridge.send_input(session_id, input.clone(), kind.clone());
    if let Some(token) = token {
        inflight_actions.borrow_mut().insert(
            token,
            PendingArchcarAction::UserSend {
                thread_id,
                session_id,
                input,
                kind,
            },
        );
        true
    } else {
        false
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
        | ArchcarEvent::SessionScreenUpdated { .. }
        | ArchcarEvent::SessionMessagesUpdated { .. } => false,
    }
}

fn handle_archcar_event(
    event: &ArchcarEvent,
    records: &[ProcessRecord],
    ready_cache: &RefCell<HashMap<i64, bool>>,
    startup_states: &RefCell<HashMap<i64, CodexStartupState>>,
    session_threads: &RefCell<HashMap<i64, i64>>,
    selected_harness: SessionKind,
    selected_thread_id: Option<i64>,
    codex_ready: &RefCell<bool>,
    update_composer_state: &dyn Fn(),
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
            set_codex_ready_state(codex_ready, update_composer_state, true);
        }
        ArchcarEvent::SessionScreenUpdated { session_id } => {
            trace!(session_id, "archcar session screen updated");
        }
        ArchcarEvent::SessionMessagesUpdated { thread_id } => {
            trace!(thread_id, "archcar session messages updated");
        }
        ArchcarEvent::SessionExited {
            session_id,
            exit_code,
        } => {
            info!(session_id, ?exit_code, "archcar session exited");
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
    workspace: &str,
    bridge: AsyncArchcarBridge,
    ready_cache: &RefCell<HashMap<i64, bool>>,
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
    pending_commands: &RefCell<HashMap<i64, Vec<String>>>,
    pending_inputs: &RefCell<HashMap<i64, Vec<QueuedArchcarInput>>>,
    startup_states: &RefCell<HashMap<i64, CodexStartupState>>,
    working_threads: &RefCell<HashMap<i64, Instant>>,
    codex_ready: &RefCell<bool>,
    update_composer_state: &dyn Fn(),
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
                    apply_codex_startup_signal(
                        &mut startup_states.borrow_mut(),
                        CodexStartupSignal::Error {
                            thread_id,
                            message: format!("Unexpected archcar ensure response: {other:?}"),
                        },
                    );
                    clear_thread_working(working_threads, thread_id);
                }
                changed = true;
            }
            Err(err) => {
                warn!(%workspace, token = response.token, error = %err, "archcar ensure failed");
                if let Some(thread_id) = thread_id {
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
                queue_thread_command(pending_commands, thread_id, command);
                note_archcar_ready(&mut ready_cache.borrow_mut(), session_id, false);
                apply_codex_startup_signal(
                    &mut startup_states.borrow_mut(),
                    CodexStartupSignal::Error {
                        thread_id,
                        message: format!("Unexpected archcar control response: {other:?}"),
                    },
                );
                set_codex_ready_state(codex_ready, update_composer_state, false);
                changed = true;
                request_archcar_ensure(
                    &bridge,
                    inflight_actions,
                    workspace.to_owned(),
                    Some(thread_id),
                );
            }
            Err(err) => {
                warn!(thread_id, session_id, control = %command, error = %err, "archcar control send failed");
                queue_thread_command(pending_commands, thread_id, command);
                note_archcar_ready(&mut ready_cache.borrow_mut(), session_id, false);
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
                );
            }
        },
        PendingArchcarAction::UserSend {
            thread_id,
            session_id,
            input,
            kind,
        } => match response.result {
            Ok(ArchcarResponse::Ack) => {
                info!(
                    thread_id,
                    session_id,
                    kind = ?kind,
                    chars = input.len(),
                    "archcar input accepted"
                );
            }
            Ok(other) => {
                warn!(thread_id, session_id, kind = ?kind, ?other, "unexpected archcar input response");
                queue_archcar_input(pending_inputs, thread_id, input, kind);
                note_archcar_ready(&mut ready_cache.borrow_mut(), session_id, false);
                apply_codex_startup_signal(
                    &mut startup_states.borrow_mut(),
                    CodexStartupSignal::Error {
                        thread_id,
                        message: format!("Unexpected archcar input response: {other:?}"),
                    },
                );
                clear_thread_working(working_threads, thread_id);
                set_codex_ready_state(codex_ready, update_composer_state, false);
                changed = true;
                request_archcar_ensure(
                    &bridge,
                    inflight_actions,
                    workspace.to_owned(),
                    Some(thread_id),
                );
            }
            Err(err) => {
                warn!(thread_id, session_id, kind = ?kind, error = %err, "archcar input send failed");
                queue_archcar_input(pending_inputs, thread_id, input, kind);
                note_archcar_ready(&mut ready_cache.borrow_mut(), session_id, false);
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
                request_archcar_ensure(
                    &bridge,
                    inflight_actions,
                    workspace.to_owned(),
                    Some(thread_id),
                );
            }
        },
    }
    changed
}

fn archcar_message_refresh_scope(message: &AsyncArchcarMessage) -> (bool, bool) {
    match message {
        AsyncArchcarMessage::Event(event) => match event {
            ArchcarEvent::SessionSpawnQueued { .. }
            | ArchcarEvent::SessionStarted { .. }
            | ArchcarEvent::SessionExited { .. }
            | ArchcarEvent::SessionError { .. } => (true, true),
            ArchcarEvent::SessionReady { .. }
            | ArchcarEvent::SessionScreenUpdated { .. }
            | ArchcarEvent::SessionMessagesUpdated { .. } => (true, false),
        },
        AsyncArchcarMessage::Response(_) => (false, false),
        AsyncArchcarMessage::BridgeError { .. } => (true, false),
    }
}

fn request_archcar_ensure(
    bridge: &AsyncArchcarBridge,
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
    workspace: String,
    thread_id: Option<i64>,
) -> bool {
    let token = if let Some(thread_id) = thread_id {
        bridge.ensure_thread_session(workspace.clone(), thread_id, SessionKind::Codex)
    } else {
        bridge.ensure_default_session(workspace.clone(), SessionKind::Codex)
    };
    if let Some(token) = token {
        inflight_actions.borrow_mut().insert(
            token,
            PendingArchcarAction::EnsureWorkspace {
                workspace,
                thread_id,
            },
        );
        true
    } else {
        false
    }
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

fn composer_ready_for_input(has_live_codex_session: bool, codex_ready: bool) -> bool {
    !has_live_codex_session || codex_ready
}

fn composer_ready_for_codex_thread(
    selected_thread_id: Option<i64>,
    records: &[ProcessRecord],
    ready_cache: &RefCell<HashMap<i64, bool>>,
    startup_states: &RefCell<HashMap<i64, CodexStartupState>>,
) -> bool {
    let Some(thread_id) = selected_thread_id else {
        return true;
    };
    composer_ready_for_input(
        thread_has_live_codex_session(records, thread_id),
        codex_thread_ready_for_ui(
            thread_id,
            records,
            ready_cache,
            startup_states.borrow().get(&thread_id),
        ),
    )
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
    use linux_archductor_core::workspace::ProcessKind;
    use std::collections::HashMap;
    use std::path::PathBuf;

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

    #[test]
    fn session_kind_from_index_maps_harness_menu_order() {
        assert_eq!(session_kind_from_index(0), SessionKind::Codex);
        assert_eq!(session_kind_from_index(1), SessionKind::Claude);
        assert_eq!(session_kind_from_index(2), SessionKind::Codex);
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
    fn supported_chat_session_kinds_exclude_cursor_and_shell() {
        assert_eq!(
            supported_chat_session_kinds(),
            &[SessionKind::Codex, SessionKind::Claude]
        );
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
        queue_thread_command(&pending, 7, "/model gpt-5".to_owned());
        queue_thread_command(&pending, 7, "/thinking high".to_owned());

        let flushed = flush_pending_commands_for_send(&pending, 7);

        assert_eq!(
            flushed,
            vec!["/model gpt-5".to_owned(), "/thinking high".to_owned()]
        );
        assert!(flush_pending_commands_for_send(&pending, 7).is_empty());
    }

    #[test]
    fn unsupported_live_controls_are_filtered_out_of_toolbar() {
        let controls = visible_live_controls_for_provider("codex");

        assert!(controls.contains(&"model".to_owned()));
        assert!(controls.contains(&"thinking".to_owned()));
        assert!(!controls.contains(&"goal".to_owned()));
        assert!(!controls.contains(&"attach".to_owned()));
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
    fn summarize_chat_title_from_opening_message_compacts_and_truncates() {
        assert_eq!(
            summarize_chat_title_from_opening_message(
                "  Fix   the parser failure in main.rs and add coverage  "
            )
            .as_deref(),
            Some("Fix the parser failure in main.rs")
        );
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
    fn codex_inline_event_chip_label_uses_plus_minus_and_compact_name() {
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
            title: "cargo test -p linux-archductor-core codex_tui".to_owned(),
            subtitle: Some("Command result".to_owned()),
            body: None,
            path: None,
            status: CodexInlineEventStatus::Complete,
        };

        assert_eq!(
            inline_event_chip_label(&file_event, false),
            "+ 2026-07-03-manual-skill-tool-calls.md"
        );
        assert_eq!(
            inline_event_chip_label(&file_event, true),
            "- 2026-07-03-manual-skill-tool-calls.md"
        );
        assert_eq!(
            inline_event_chip_label(&command_event, false),
            "+ cargo test -p linux-archductor-core codex_tui"
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
        assert_eq!(tool.path.as_deref(), Some(tool_path.as_path()));
        assert!(inline_event_body_text(&tool).contains("tool file contents"));
        assert_eq!(skill.kind, CodexInlineEventKind::Skill);
        assert_eq!(skill.path.as_deref(), Some(skill_path.as_path()));
        assert!(inline_event_body_text(&skill).contains("name: graphify"));
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
    fn context_usage_display_state_maps_percent_and_tooltips() {
        let empty = context_usage_display_state(None);
        assert_eq!(empty.percent_label, "--");
        assert_eq!(empty.css_class, "chat-context-usage-empty");
        assert_eq!(empty.tooltip, "Context usage unknown");

        let normal = context_usage_display_state(Some(CodexContextUsage {
            used_tokens: Some(68_000),
            max_tokens: Some(100_000),
            percent: 68,
        }));
        assert_eq!(normal.percent_label, "68%");
        assert_eq!(normal.css_class, "chat-context-usage-normal");
        assert!(normal.tooltip.contains("68,000 / 100,000 tokens"));

        assert_eq!(
            context_usage_display_state(Some(CodexContextUsage {
                used_tokens: None,
                max_tokens: None,
                percent: 70,
            }))
            .css_class,
            "chat-context-usage-warning"
        );
        assert_eq!(
            context_usage_display_state(Some(CodexContextUsage {
                used_tokens: None,
                max_tokens: None,
                percent: 90,
            }))
            .css_class,
            "chat-context-usage-danger"
        );
    }

    #[test]
    fn codex_context_usage_parser_derives_percent_from_token_pair() {
        let usage = parse_codex_context_usage_local("128k / 200k tokens").unwrap();

        assert_eq!(usage.used_tokens, Some(128_000));
        assert_eq!(usage.max_tokens, Some(200_000));
        assert_eq!(usage.percent, 64);
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
        queue_thread_command(&pending, 5, "/model gpt-5".to_owned());
        queue_thread_command(&pending, 5, "/thinking medium".to_owned());
        queue_thread_command(&pending, 5, "/model gpt-5-mini".to_owned());
        queue_thread_command(&pending, 5, "/thinking high".to_owned());

        let flushed = flush_pending_commands_for_send(&pending, 5);

        assert_eq!(
            flushed,
            vec!["/model gpt-5-mini".to_owned(), "/thinking high".to_owned()]
        );
    }

    #[test]
    fn queue_archcar_input_ignores_blank_messages() {
        let pending = RefCell::new(HashMap::<i64, Vec<QueuedArchcarInput>>::new());

        queue_archcar_input(&pending, 5, "   ".to_owned(), ArchcarInputKind::User);

        assert!(pending.borrow().is_empty());
    }

    #[test]
    fn queue_archcar_input_preserves_trimmed_input_and_kind() {
        let pending = RefCell::new(HashMap::<i64, Vec<QueuedArchcarInput>>::new());

        queue_archcar_input(
            &pending,
            8,
            "  review this diff  ".to_owned(),
            ArchcarInputKind::ReviewPrompt,
        );

        let queued = pending.borrow();
        let items = queued.get(&8).unwrap();
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
    fn pending_archcar_ensure_is_scoped_to_thread() {
        let pending = RefCell::new(HashMap::from([
            (
                1,
                PendingArchcarAction::EnsureWorkspace {
                    workspace: "berlin".to_owned(),
                    thread_id: Some(7),
                },
            ),
            (
                2,
                PendingArchcarAction::EnsureWorkspace {
                    workspace: "berlin".to_owned(),
                    thread_id: None,
                },
            ),
        ]));

        assert!(has_pending_archcar_ensure_for_thread(&pending, 7));
        assert!(!has_pending_archcar_ensure_for_thread(&pending, 8));
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
    fn live_session_append_renders_user_and_review_events_without_markers() {
        let user = live_session_append_text("[user input memphis#9]\ncargo test\n[/user input]\n");
        let review =
            live_session_append_text("[staged review prompt]\nFix CI\n[/staged review prompt]\n");

        assert_eq!(user, "You\ncargo test\n\n");
        assert_eq!(review, "Review Prompt\nFix CI\n\n");
    }

    #[test]
    fn live_session_append_renders_plain_output_as_agent_event() {
        let text = live_session_append_text("hello\x1b[31m agent\x1b[0m\n");

        assert_eq!(text, "Agent\nhello agent\n\n");
    }

    #[test]
    fn selected_session_surface_renders_labeled_transcript_events() {
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

        assert!(surface.contains("System\n[session started] #11 kind=Cursor pid=1234"));
        assert!(surface.contains("Harness\n[archductor bootstrap for codex]"));
        assert!(surface.contains("Tool\n[tool bash]"));
        assert!(surface.contains("Skill\n[skill tests]"));
        assert!(surface.contains("You\nopen the project"));
        assert!(surface.contains("Review Prompt\nFix the failing checks"));
        assert!(surface.contains("Agent\nBuild succeeded"));
        assert!(surface.contains(
            "Events: 7 total, 1 user, 1 review, 1 system, 1 agent, 1 tool, 1 skill, 1 harness"
        ));
        assert!(!surface.contains("[user input memphis#11]"));
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
    fn transcript_events_parse_codex_screen_blocks_without_raw_duplication() {
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

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].role, SessionTranscriptRole::User);
        assert_eq!(events[0].body, "run tests");
        assert_eq!(events[1].role, SessionTranscriptRole::Agent);
        assert_eq!(events[1].body, "Running now.\nTests passed.");
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
Ran cargo test -p linux-archductor-core codex_tui
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
            "Ran cargo test -p linux-archductor-core codex_tui\nrunning 23 tests\ntest result: ok. 23 passed"
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
        assert_eq!(skill_events[0].title, "graphify");
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
    fn legacy_inline_event_parsing_only_runs_without_persisted_events() {
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

        let legacy_enabled = render_legacy_inline_events_for_thread(&[]);
        let legacy_disabled = render_legacy_inline_events_for_thread(&[ChatEventRecord {
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
        }]);
        let legacy_events = legacy_inline_events_for_message(&message, legacy_enabled);
        let persisted_timeline_events = legacy_inline_events_for_message(&message, legacy_disabled);

        assert_eq!(legacy_events.len(), 1);
        assert!(persisted_timeline_events.is_empty());
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
                message: "Starting Codex...".to_owned(),
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
                kind: ArchcarInputKind::User,
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
    fn working_indicator_clears_when_generation_is_ready() {
        let mut selected = session_record(11, "codex", ProcessStatus::Running, None);
        selected.chat_thread_id = Some(7);
        let session_threads = RefCell::new(HashMap::from([(11, 7)]));
        let pending_inputs = RefCell::new(HashMap::new());
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
                message: "Starting Codex...".to_owned(),
            })
        );
    }

    #[test]
    fn routine_archcar_updates_do_not_force_outer_refresh() {
        let ready_scope = archcar_message_refresh_scope(&AsyncArchcarMessage::Event(
            ArchcarEvent::SessionReady {
                session_id: 61,
                thread_id: 4,
            },
        ));
        let status_scope =
            archcar_message_refresh_scope(&AsyncArchcarMessage::Response(AsyncArchcarResponse {
                token: 7,
                request: AsyncArchcarRequestKind::GetSessionStatus { session_id: 61 },
                result: Ok(ArchcarResponse::SessionStatus {
                    session_id: 61,
                    status: "running".to_owned(),
                    runtime_state: AgentSessionState::WaitingForInput,
                    ready: true,
                }),
            }));
        let started_scope = archcar_message_refresh_scope(&AsyncArchcarMessage::Event(
            ArchcarEvent::SessionStarted {
                session_id: 61,
                thread_id: 4,
                workspace: "hoi-an".to_owned(),
                kind: SessionKind::Codex,
                pid: 1234,
            },
        ));

        assert_eq!(ready_scope, (true, false));
        assert_eq!(status_scope, (false, false));
        assert_eq!(started_scope, (true, true));
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
    fn composer_ready_for_selected_thread_ignores_other_thread_ready_session() {
        let mut selected = session_record(11, "codex", ProcessStatus::Running, None);
        selected.chat_thread_id = Some(7);
        let mut other = session_record(22, "codex", ProcessStatus::Running, None);
        other.chat_thread_id = Some(8);
        let ready_cache = RefCell::new(HashMap::from([(22, true)]));
        let startup_states = RefCell::new(HashMap::<i64, CodexStartupState>::new());

        let ready = composer_ready_for_codex_thread(
            Some(7),
            &[selected, other],
            &ready_cache,
            &startup_states,
        );

        assert!(!ready);
    }

    #[test]
    fn composer_ready_for_selected_thread_uses_thread_ready_state_when_session_ids_drift() {
        let mut selected = session_record(59, "codex", ProcessStatus::Running, None);
        selected.chat_thread_id = Some(7);
        let ready_cache = RefCell::new(HashMap::from([(61, true)]));
        let startup_states = RefCell::new(HashMap::from([(7, CodexStartupState::Ready)]));

        let ready =
            composer_ready_for_codex_thread(Some(7), &[selected], &ready_cache, &startup_states);

        assert!(ready);
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
    fn composer_stays_ready_when_no_live_codex_session_exists() {
        assert!(composer_ready_for_input(false, false));
        assert!(!composer_ready_for_input(true, false));
        assert!(composer_ready_for_input(true, true));
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
