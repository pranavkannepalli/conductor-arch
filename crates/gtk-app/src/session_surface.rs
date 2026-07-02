use gtk::prelude::*;
use gtk::{
    Align, Box as GBox, Button, CheckButton, ComboBoxText, Entry, EventControllerKey, Image, Label,
    Orientation, Overlay, Popover, ScrolledWindow, Spinner, TextBuffer, TextView, ToggleButton,
    Widget,
};
use linux_archductor_core::archcar::protocol::{ArchcarEvent, ArchcarInputKind, ArchcarResponse};
use linux_archductor_core::codex_tui::{
    detect_directory_trust_prompt, merge_screen_messages, parse_codex_context_usage,
    parse_codex_inline_event, parse_codex_screen_messages,
    CodexFileReference as CoreCodexFileReference, CodexInlineEvent as CoreCodexInlineEvent,
    ScreenMessage, ScreenMessageRole,
};
use linux_archductor_core::pty::PtySession;
use linux_archductor_core::workspace::{
    ChatMessageRecord, ChatThreadRecord, ProcessRecord, ProcessStatus, SessionHarnessOptions,
    SessionKind, WorkspaceStore,
};
use std::any::Any;
use std::backtrace::Backtrace;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::{env, fs as stdfs};
use tracing::{debug, error, info, trace, warn};

use crate::archcar_async::{
    clear_archcar_ready, note_archcar_ready, AsyncArchcarBridge, AsyncArchcarMessage,
    AsyncArchcarRequestKind, AsyncArchcarResponse,
};
use crate::buttons::{
    icon_button, resolve_icon_name, style_icon_button, style_text_button, text_button,
};
use crate::state::AppState;
use crate::terminal::terminal_display_text;

const SESSION_SCROLLBACK_LINES: usize = 2_000;
const SESSION_TAIL_HISTORY: usize = 120;
const SESSION_POLL_INTERVAL_MS: u64 = 100;
const SESSION_RECONCILE_EVERY_TICKS: u32 = 10;
const DEFAULT_CHAT_TITLE_PREFIX: &str = "New Chat";

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ContextUsageDisplayState {
    percent_label: String,
    css_class: &'static str,
    tooltip: String,
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
    let pending_session_inputs = Rc::new(RefCell::new(
        HashMap::<i64, Vec<DeferredSessionInput>>::new(),
    ));
    let pending_archcar_inputs =
        Rc::new(RefCell::new(HashMap::<i64, Vec<QueuedArchcarInput>>::new()));
    let codex_ready: Rc<RefCell<bool>> = Rc::new(RefCell::new(true));
    let codex_startup_states = Rc::new(RefCell::new(HashMap::<i64, CodexStartupState>::new()));
    let codex_session_threads = Rc::new(RefCell::new(HashMap::<i64, i64>::new()));
    let archcar_bridge = AsyncArchcarBridge::new(app_state.paths.clone());
    let archcar_ready_cache = Rc::new(RefCell::new(HashMap::<i64, bool>::new()));
    let inflight_archcar_actions =
        Rc::new(RefCell::new(HashMap::<u64, PendingArchcarAction>::new()));
    let pending_archcar_status = Rc::new(RefCell::new(HashMap::<i64, u64>::new()));
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
    root.append(&scroll);

    // ── Composer ─────────────────────────────────────────────────────
    let composer_wrap = GBox::new(Orientation::Vertical, 0);
    composer_wrap.add_css_class("chat-composer");

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
    input_scroll.set_propagate_natural_width(false);
    input_scroll.add_css_class("chat-input-scroll");
    input_scroll.set_child(Some(&input_view));
    let input_shell = GBox::new(Orientation::Horizontal, 8);
    input_shell.set_hexpand(true);
    let input_overlay = Overlay::new();
    input_overlay.set_hexpand(true);
    input_overlay.set_child(Some(&input_scroll));
    let placeholder = Label::new(Some(
        "Ask to make changes, @mention files, or run /commands",
    ));
    placeholder.add_css_class("chat-placeholder");
    placeholder.set_halign(Align::Start);
    placeholder.set_valign(Align::Start);
    placeholder.set_margin_start(18);
    placeholder.set_margin_top(18);
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

    let interface_btn = mode_menu_button(
        "Codex",
        "code-symbolic",
        &["Codex", "Claude", "Shell"],
        0,
        {
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
        },
    );
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
    root.append(&composer_wrap);

    let record_state = Rc::new(RefCell::new(Vec::<ProcessRecord>::new()));
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
    let active_sessions: Rc<RefCell<HashMap<i64, SessionConnection>>> =
        Rc::new(RefCell::new(HashMap::new()));
    let last_output = Rc::new(RefCell::new(HashMap::<i64, Instant>::new()));
    let last_screen = Rc::new(RefCell::new(HashMap::<i64, String>::new()));
    let trust_answered = Rc::new(RefCell::new(HashSet::<i64>::new()));
    let last_size = Rc::new(RefCell::new(None::<(u16, u16)>));
    let reconcile_tick = Rc::new(RefCell::new(0_u32));
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
        let selected_harness = selected_harness.clone();
        let active_sessions = active_sessions.clone();
        let last_output = last_output.clone();
        let app_state = app_state.clone();
        let app_state_for_thread_select = app_state.clone();
        let codex_startup_states = codex_startup_states.clone();
        let archcar_ready_cache = archcar_ready_cache.clone();
        let update_composer_for_view = update_composer_state.clone();
        let external_chat_tabs = external_chat_tabs.clone();
        let context_usage = context_usage.clone();
        Rc::new(move || {
            debug!(workspace = %workspace, "chat refresh_view start");
            let (loaded, loaded_threads) = WorkspaceStore::open(database_path.clone())
                .map(|store| {
                    (
                        store.list_sessions(&workspace).unwrap_or_default(),
                        store.list_chat_threads(&workspace).unwrap_or_default(),
                    )
                })
                .unwrap_or_default();
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

            while let Some(child) = thread_row.first_child() {
                thread_row.remove(&child);
            }
            {
                let current = thread_state.borrow();
                let selected = *selected_thread.borrow();
                let current_kind = *selected_harness.borrow();
                let provider = session_kind_provider(current_kind);
                let mut visible_threads = current
                    .iter()
                    .filter(|thread| provider == thread.provider)
                    .cloned()
                    .collect::<Vec<_>>();
                if visible_threads.is_empty() {
                    let empty = Label::new(Some("No chats yet for this provider."));
                    empty.add_css_class("card-meta");
                    empty.set_xalign(0.0);
                    thread_row.append(&empty);
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
                        thread_row.append(&button);
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

            while let Some(child) = messages.first_child() {
                messages.remove(&child);
            }
            apply_context_usage_state(&context_usage, None);

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
                        let label = Label::new(Some("No chat selected."));
                        label.add_css_class("chat-agent-text");
                        label.set_wrap(true);
                        label.set_xalign(0.0);
                        messages.append(&label);
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
                    let runtime_summary = record.as_ref().map(|record| {
                        let attached_sessions = active_sessions
                            .borrow()
                            .keys()
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
                    if let Some(widget) = codex_startup_status_widget(&startup_state) {
                        messages.append(&widget);
                    }

                    let thread_messages = WorkspaceStore::open(database_path.clone())
                        .and_then(|store| store.list_chat_messages(thread_id))
                        .unwrap_or_default();
                    debug!(
                        workspace = %workspace,
                        thread_id,
                        thread_message_count = thread_messages.len(),
                        "chat refresh_view loaded thread messages"
                    );
                    if !thread_messages.is_empty() {
                        apply_context_usage_state(
                            &context_usage,
                            latest_context_usage_from_messages(&thread_messages),
                        );
                        for message in thread_messages {
                            messages.append(&chat_message_widget(&message));
                        }
                        return;
                    }

                    let empty = Label::new(Some(&runtime_summary.unwrap_or_else(|| {
                        "No messages yet. Send a message to start.".to_owned()
                    })));
                    empty.add_css_class("chat-agent-text");
                    empty.set_wrap(true);
                    empty.set_xalign(0.0);
                    messages.append(&empty);
                }
                (None, _) => {
                    let prompt = format!(
                        "No {} chat yet. Create one or send a message to start one.",
                        session_kind_name(current_kind)
                    );
                    messages.append(&chat_user_bubble(&prompt));
                }
            }
            debug!(workspace = %workspace, "chat refresh_view complete");
        }) as Rc<dyn Fn()>
    };
    *refresh_chat_surface.borrow_mut() = Some(refresh_view.clone());

    seed_chat_running_sessions(
        &database_path,
        _workspace_name,
        &active_sessions,
        &last_output,
    );
    let refresh_session_surface = refresh_view.clone();
    refresh_session_surface();

    let db_for_refresh = database_path.clone();
    let workspace_for_refresh = _workspace_name.to_owned();
    let active_sessions_for_refresh = active_sessions.clone();
    let selected_session_for_refresh = selected_session.clone();
    let record_state_for_poll = record_state.clone();
    let last_output_for_refresh = last_output.clone();
    let last_screen_for_refresh = last_screen.clone();
    let trust_answered_for_refresh = trust_answered.clone();
    let pending_session_inputs_for_refresh = pending_session_inputs.clone();
    let pending_archcar_inputs_for_refresh = pending_archcar_inputs.clone();
    let last_size_for_refresh = last_size.clone();
    let reconcile_tick_for_refresh = reconcile_tick.clone();
    let refresh_view_for_poll = refresh_view.clone();
    let refresh_for_poll = refresh.clone();
    let messages_for_resize = messages.clone();
    let codex_ready_for_poll = codex_ready.clone();
    let update_composer_for_poll = update_composer_state.clone();
    let archcar_bridge_for_poll = archcar_bridge.clone();
    let archcar_ready_cache_for_poll = archcar_ready_cache.clone();
    let inflight_archcar_actions_for_poll = inflight_archcar_actions.clone();
    let pending_archcar_status_for_poll = pending_archcar_status.clone();
    let pending_commands_for_poll = pending_commands.clone();
    let app_state_for_poll = app_state.clone();
    let codex_startup_states_for_poll = codex_startup_states.clone();
    let codex_session_threads_for_poll = codex_session_threads.clone();
    let selected_harness_for_poll = selected_harness.clone();
    let selected_thread_for_poll = selected_thread.clone();
    glib::timeout_add_local(Duration::from_millis(SESSION_POLL_INTERVAL_MS), move || {
        if messages_for_resize.root().is_none() {
            info!(workspace = %workspace_for_refresh, "session surface poll stopped after widget detach");
            return glib::ControlFlow::Break;
        }
        let size = session_size_from_widget(
            messages_for_resize.allocated_width(),
            messages_for_resize.allocated_height(),
        );
        let should_resize = match *last_size_for_refresh.borrow() {
            Some(previous) => previous != size,
            None => true,
        };
        *last_size_for_refresh.borrow_mut() = Some(size);

        let mut ended = Vec::<i64>::new();
        let mut should_refresh_view = false;
        let mut should_refresh_outer = false;
        while let Some(message) = archcar_bridge_for_poll.try_recv() {
            let (refresh_view_after_message, refresh_outer_after_message) =
                archcar_message_refresh_scope(&message);
            match message {
                AsyncArchcarMessage::Event(event) => {
                    handle_archcar_event(
                        &event,
                        &record_state_for_poll.borrow(),
                        archcar_ready_cache_for_poll.as_ref(),
                        codex_startup_states_for_poll.as_ref(),
                        codex_session_threads_for_poll.as_ref(),
                        *selected_harness_for_poll.borrow(),
                        *selected_thread_for_poll.borrow(),
                        codex_ready_for_poll.as_ref(),
                        update_composer_for_poll.as_ref(),
                    );
                }
                AsyncArchcarMessage::Response(response) => {
                    handle_archcar_response(
                        response,
                        &db_for_refresh,
                        &workspace_for_refresh,
                        archcar_bridge_for_poll.clone(),
                        &record_state_for_poll.borrow(),
                        archcar_ready_cache_for_poll.as_ref(),
                        pending_archcar_status_for_poll.as_ref(),
                        inflight_archcar_actions_for_poll.as_ref(),
                        pending_commands_for_poll.as_ref(),
                        pending_archcar_inputs_for_refresh.as_ref(),
                        codex_startup_states_for_poll.as_ref(),
                        codex_session_threads_for_poll.as_ref(),
                        codex_ready_for_poll.as_ref(),
                        update_composer_for_poll.as_ref(),
                        &app_state_for_poll,
                    );
                }
                AsyncArchcarMessage::BridgeError { message } => {
                    warn!(%message, "async archcar bridge error");
                }
            }
            should_refresh_view |= refresh_view_after_message;
            should_refresh_outer |= refresh_outer_after_message;
        }
        if flush_pending_archcar_inputs(
            &archcar_bridge_for_poll,
            &db_for_refresh,
            pending_commands_for_poll.as_ref(),
            pending_archcar_inputs_for_refresh.as_ref(),
            archcar_ready_cache_for_poll.as_ref(),
            inflight_archcar_actions_for_poll.as_ref(),
            &app_state_for_poll,
        ) {
            should_refresh_view = true;
            should_refresh_outer = true;
        }
        request_codex_status_probes(
            &archcar_bridge_for_poll,
            &record_state_for_poll.borrow(),
            pending_archcar_status_for_poll.as_ref(),
            inflight_archcar_actions_for_poll.as_ref(),
        );
        if let Some(process_id) = any_running_archcar_codex_ready(
            &record_state_for_poll.borrow(),
            archcar_ready_cache_for_poll.as_ref(),
        ) {
            if !*codex_ready_for_poll.borrow() {
                info!(
                    process_id,
                    "reconciled codex ready state from archcar session status"
                );
                set_codex_ready_state(
                    codex_ready_for_poll.as_ref(),
                    update_composer_for_poll.as_ref(),
                    true,
                );
                should_refresh_view = true;
                should_refresh_outer = true;
            }
        }
        {
            let mut sessions = active_sessions_for_refresh.borrow_mut();
            for (process_id, session) in sessions.iter_mut() {
                let output = session.read_available();
                if !output.is_empty() {
                    trace!(process_id, bytes = output.len(), "session chunk received");
                    let is_codex =
                        is_codex_session_record(&record_state_for_poll.borrow(), *process_id);
                    let _ = WorkspaceStore::open(db_for_refresh.clone()).and_then(|store| {
                        if is_codex {
                            store.append_session_process_output(
                                *process_id,
                                &format_codex_raw_output(&output),
                            )
                        } else {
                            store.append_session_process_output(*process_id, &output)
                        }
                    });
                    let is_selected = *selected_session_for_refresh.borrow() == Some(*process_id);
                    if is_selected {
                        should_refresh_view = true;
                    }
                    last_output_for_refresh
                        .borrow_mut()
                        .insert(*process_id, Instant::now());
                }

                if is_codex_session_record(&record_state_for_poll.borrow(), *process_id) {
                    if let Some(screen) = session.visible_screen_text() {
                        let changed = last_screen_for_refresh
                            .borrow()
                            .get(process_id)
                            .map(|previous| previous != &screen)
                            .unwrap_or(!screen.is_empty());
                        if !screen.is_empty() && changed {
                            let answered = trust_answered_for_refresh.borrow().contains(process_id);
                            if !answered
                                && detect_directory_trust_prompt(&screen)
                                && session.send_line("1").is_ok()
                            {
                                trust_answered_for_refresh.borrow_mut().insert(*process_id);
                            }
                            let _ =
                                WorkspaceStore::open(db_for_refresh.clone()).and_then(|store| {
                                    store.append_session_process_output(
                                        *process_id,
                                        &format_codex_screen_snapshot(&screen),
                                    )
                                });
                            log_codex_screen_snapshot("chat-poll", *process_id, &screen);
                            if codex_screen_ready_for_input(&screen) {
                                if !*codex_ready_for_poll.borrow() {
                                    *codex_ready_for_poll.borrow_mut() = true;
                                    update_composer_for_poll();
                                }
                                if let Some(thread_id) = codex_thread_id_for_session(
                                    *process_id,
                                    codex_session_threads_for_poll.as_ref(),
                                    &record_state_for_poll.borrow(),
                                ) {
                                    apply_codex_startup_signal(
                                        &mut codex_startup_states_for_poll.borrow_mut(),
                                        CodexStartupSignal::Ready { thread_id },
                                    );
                                }
                                flush_deferred_session_inputs(
                                    pending_session_inputs_for_refresh.as_ref(),
                                    &db_for_refresh,
                                    &workspace_for_refresh,
                                    *process_id,
                                    session,
                                );
                            }
                            last_screen_for_refresh
                                .borrow_mut()
                                .insert(*process_id, screen);
                            let is_selected =
                                *selected_session_for_refresh.borrow() == Some(*process_id);
                            if is_selected {
                                should_refresh_view = true;
                            }
                        }
                    }
                }

                if should_resize {
                    if let Err(err) = session.resize(size.0, size.1) {
                        warn!(
                            process_id,
                            rows = size.0,
                            cols = size.1,
                            error = %err,
                            "session resize failed"
                        );
                        let err_label =
                            Label::new(Some(&format!("[session resize error] {err:#}")));
                        err_label.add_css_class("chat-agent-text");
                        err_label.set_wrap(true);
                        err_label.set_xalign(0.0);
                        messages_for_resize.append(&err_label);
                    }
                }

                match session.has_exited() {
                    Ok(true) => ended.push(*process_id),
                    Ok(false) => {}
                    Err(err) => {
                        warn!(process_id, error = %err, "session poll failed");
                        let err_label = Label::new(Some(&format!("[session poll error] {err:#}")));
                        err_label.add_css_class("chat-agent-text");
                        err_label.set_wrap(true);
                        err_label.set_xalign(0.0);
                        messages_for_resize.append(&err_label);
                    }
                }
            }
        }

        for process_id in ended {
            info!(process_id, "session exited");
            active_sessions_for_refresh.borrow_mut().remove(&process_id);
            let _ = WorkspaceStore::open(db_for_refresh.clone())
                .and_then(|store| store.mark_session_process_exited(process_id, None));
            codex_session_threads_for_poll
                .borrow_mut()
                .remove(&process_id);
            last_output_for_refresh.borrow_mut().remove(&process_id);
            last_screen_for_refresh.borrow_mut().remove(&process_id);
            trust_answered_for_refresh.borrow_mut().remove(&process_id);
            if active_sessions_for_refresh.borrow().is_empty() {
                *codex_ready_for_poll.borrow_mut() = false;
                update_composer_for_poll();
            }
            update_composer_for_poll();
            should_refresh_view = true;
            should_refresh_outer = true;
        }

        *reconcile_tick_for_refresh.borrow_mut() += 1;
        if (*reconcile_tick_for_refresh.borrow()).is_multiple_of(SESSION_RECONCILE_EVERY_TICKS) {
            if let Ok(reconciled) = WorkspaceStore::open(db_for_refresh.clone())
                .and_then(|store| store.reconcile_session_processes())
            {
                if reconciled
                    .iter()
                    .any(|process| process.status != ProcessStatus::Running)
                {
                    for process in reconciled {
                        active_sessions_for_refresh.borrow_mut().remove(&process.id);
                        last_output_for_refresh.borrow_mut().remove(&process.id);
                        last_screen_for_refresh.borrow_mut().remove(&process.id);
                        trust_answered_for_refresh.borrow_mut().remove(&process.id);
                    }
                    update_composer_for_poll();
                    should_refresh_view = true;
                    should_refresh_outer = true;
                }
            }
        }

        if should_refresh_view {
            debug!(
                workspace = %workspace_for_refresh,
                "chat poll entering refresh_view"
            );
            refresh_view_for_poll();
            debug!(
                workspace = %workspace_for_refresh,
                "chat poll finished refresh_view"
            );
        }
        if should_refresh_outer {
            refresh_for_poll();
        }
        glib::ControlFlow::Continue
    });

    let db_for_send = database_path.clone();
    let workspace_for_send = _workspace_name.to_owned();
    let selected_harness_for_send = selected_harness.clone();
    let reasoning_mode_for_send = reasoning_mode.clone();
    let thread_state_for_send = thread_state.clone();
    let selected_thread_for_send = selected_thread.clone();
    let pending_commands_for_send = pending_commands.clone();
    let pending_session_inputs_for_send = pending_session_inputs.clone();
    let pending_archcar_inputs_for_send = pending_archcar_inputs.clone();
    let active_sessions_for_send = active_sessions.clone();
    let selected_session_for_send = selected_session.clone();
    let record_state_for_send = record_state.clone();
    let last_output_for_send = last_output.clone();
    let refresh_view_for_send = refresh_view.clone();
    let refresh_for_send = refresh.clone();
    let app_state_for_send = app_state.clone();
    let messages_for_send = messages.clone();
    let messages_for_send_size = messages.clone();
    let archcar_bridge_for_send = archcar_bridge.clone();
    let archcar_ready_cache_for_send = archcar_ready_cache.clone();
    let inflight_archcar_actions_for_send = inflight_archcar_actions.clone();
    let codex_ready_for_send = codex_ready.clone();
    let codex_startup_states_for_send = codex_startup_states.clone();
    let update_composer_for_send = update_composer_state.clone();
    let send_text = Rc::new(move |text: String, staged_review: bool| {
        let command = text.trim().to_owned();
        if command.is_empty() {
            return;
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
        let current_harness = SessionHarnessOptions {
            plan_mode: false,
            fast_mode: false,
            approval_mode: None,
            reasoning_mode: reasoning_mode_for_send.borrow().clone(),
            effort_mode: None,
            codex_personality: None,
            codex_goals: None,
            codex_skills: None,
        };
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
                error.set_wrap(true);
                error.set_xalign(0.0);
                messages_for_send.append(&error);
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
                error.set_wrap(true);
                error.set_xalign(0.0);
                messages_for_send.append(&error);
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
                    queue_archcar_user_send(
                        &archcar_bridge_for_send,
                        inflight_archcar_actions_for_send.as_ref(),
                        thread_id,
                        record.id,
                        command.clone(),
                        kind.clone(),
                    );
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
                    refresh_for_send();
                    return;
                }
            }

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
            info!(
                workspace = %workspace_for_send,
                thread_id,
                running_record = running_record.as_ref().map(|record| record.id),
                staged_review,
                chars = command.len(),
                "queued archcar input while codex session is starting or absent"
            );
            let token = archcar_bridge_for_send
                .ensure_default_session(workspace_for_send.clone(), SessionKind::Codex);
            if let Some(token) = token {
                inflight_archcar_actions_for_send.borrow_mut().insert(
                    token,
                    PendingArchcarAction::EnsureWorkspace {
                        workspace: workspace_for_send.clone(),
                        thread_id: Some(thread_id),
                    },
                );
            }
            if staged_review {
                app_state_for_send.set_staged_review_prompt(None);
            }
            refresh_view_for_send();
            refresh_for_send();
            return;
        }
        if let Some(record) = selected_record.as_ref() {
            if record.status == ProcessStatus::Running
                && session_kind_matches_record(record, selected_kind)
                && !active_sessions_for_send.borrow().contains_key(&record.id)
            {
                if let Ok(session) = SessionConnection::try_reattach_running(record.pid) {
                    info!(
                        workspace = %workspace_for_send,
                        process_id = record.id,
                        pid = record.pid,
                        harness = ?selected_kind,
                        "reattached running session"
                    );
                    active_sessions_for_send
                        .borrow_mut()
                        .insert(record.id, session);
                }
            }
        }
        let live_session_id = current_live_session_id(
            &thread_records,
            *selected_session_for_send.borrow(),
            selected_kind,
            &active_sessions_for_send.borrow(),
        );
        debug!(
            workspace = %workspace_for_send,
            harness = ?selected_kind,
            thread_id,
            live_session_id,
            "session send stage: resolved live session"
        );
        let launch_session = |harness: SessionHarnessOptions,
                              resume_session_id: Option<String>|
         -> Option<i64> {
            debug!(
                workspace = %workspace_for_send,
                harness = ?selected_kind,
                thread_id,
                resume_session_id,
                "session send stage: building launch"
            );
            let launch = match WorkspaceStore::open(db_for_send.clone()).and_then(|store| {
                if let Some(resume_session_id) = resume_session_id.as_deref() {
                    store.session_launch_with_options_and_resume(
                        &workspace_for_send,
                        selected_kind,
                        harness,
                        Some(resume_session_id),
                    )
                } else {
                    store.session_launch_with_options(&workspace_for_send, selected_kind, harness)
                }
            }) {
                Ok(launch) => launch,
                Err(err) => {
                    error!(
                        workspace = %workspace_for_send,
                        harness = ?selected_kind,
                        error = %err,
                        "failed to build session launch"
                    );
                    let error = Label::new(Some(&format!("[session start] {err:#}")));
                    error.add_css_class("chat-agent-text");
                    error.set_wrap(true);
                    error.set_xalign(0.0);
                    messages_for_send.append(&error);
                    return None;
                }
            };
            debug!(
                workspace = %workspace_for_send,
                harness = ?selected_kind,
                thread_id,
                program = %launch.program.display(),
                arg_count = launch.args.len(),
                env_count = launch.env.len(),
                "session send stage: launch built"
            );

            if matches!(selected_kind, SessionKind::Codex) {
                error!(
                    workspace = %workspace_for_send,
                    thread_id,
                    "codex launch reached generic frontend pty path; refusing launch"
                );
                let error = Label::new(Some(
                    "[session start] Codex must be launched by archcar, not the GTK PTY path.",
                ));
                error.add_css_class("chat-agent-text");
                error.set_wrap(true);
                error.set_xalign(0.0);
                messages_for_send.append(&error);
                return None;
            }

            let (rows, cols) = session_size_from_widget(
                messages_for_send_size.allocated_width(),
                messages_for_send_size.allocated_height(),
            );

            let mut pty = match PtySession::spawn(
                launch.program.clone(),
                launch.args.clone(),
                &launch.cwd,
                launch.env.clone(),
                rows,
                cols,
            ) {
                Ok(session) => session,
                Err(err) => {
                    error!(
                        workspace = %workspace_for_send,
                        harness = ?selected_kind,
                        error = %err,
                        "failed to spawn session pty"
                    );
                    let error = Label::new(Some(&format!(
                        "[session start] {:?} launch failed: {err:#}",
                        selected_kind
                    )));
                    error.add_css_class("chat-agent-text");
                    error.set_wrap(true);
                    error.set_xalign(0.0);
                    messages_for_send.append(&error);
                    return None;
                }
            };
            debug!(
                workspace = %workspace_for_send,
                harness = ?selected_kind,
                thread_id,
                rows,
                cols,
                "session send stage: pty spawned"
            );

            let pid = match pty.process_id() {
                Some(pid) => pid,
                None => {
                    error!(
                        workspace = %workspace_for_send,
                        harness = ?selected_kind,
                        "session pty missing process id"
                    );
                    let _ = pty.stop();
                    let error = Label::new(Some("[session start] failed to allocate process id."));
                    error.add_css_class("chat-agent-text");
                    error.set_wrap(true);
                    error.set_xalign(0.0);
                    messages_for_send.append(&error);
                    return None;
                }
            };
            debug!(
                workspace = %workspace_for_send,
                harness = ?selected_kind,
                thread_id,
                pid,
                "session send stage: resolved child pid"
            );

            let session_record = match WorkspaceStore::open(db_for_send.clone()).and_then(|store| {
                store.record_session_process_for_thread(
                    &workspace_for_send,
                    thread_id,
                    &launch,
                    pid,
                )
            }) {
                Ok(record) => record,
                Err(err) => {
                    error!(
                        workspace = %workspace_for_send,
                        pid,
                        harness = ?selected_kind,
                        error = %err,
                        "failed to record session process"
                    );
                    let _ = pty.stop();
                    let error = Label::new(Some(&format!(
                        "[session start] failed to record process: {err:#}"
                    )));
                    error.add_css_class("chat-agent-text");
                    error.set_wrap(true);
                    error.set_xalign(0.0);
                    messages_for_send.append(&error);
                    return None;
                }
            };
            debug!(
                workspace = %workspace_for_send,
                harness = ?selected_kind,
                thread_id,
                process_id = session_record.id,
                pid,
                "session send stage: recorded session process"
            );

            active_sessions_for_send
                .borrow_mut()
                .insert(session_record.id, SessionConnection::Live(pty));
            info!(
                workspace = %workspace_for_send,
                process_id = session_record.id,
                pid,
                harness = ?selected_kind,
                resumed = resume_session_id.is_some(),
                session_log_path = %session_record.log_path.display(),
                "session launched"
            );
            last_output_for_send
                .borrow_mut()
                .insert(session_record.id, Instant::now());
            *selected_session_for_send.borrow_mut() = Some(session_record.id);
            app_state_for_send.set_selected_agent_session(Some(session_record.id));
            Some(session_record.id)
        };

        let mut launched_new_session = false;
        let process_id = if let Some(process_id) = live_session_id {
            *selected_session_for_send.borrow_mut() = Some(process_id);
            app_state_for_send.set_selected_agent_session(Some(process_id));
            process_id
        } else if let Some(record) = selected_record.as_ref() {
            let record_kind = session_kind_matches_record(record, selected_kind);
            if record_kind && record.status == ProcessStatus::Running {
                let error = Label::new(Some(&format!(
                    "[session send] Session #{} is running but not attached.",
                    record.id
                )));
                error.add_css_class("chat-agent-text");
                error.set_wrap(true);
                error.set_xalign(0.0);
                messages_for_send.append(&error);
                return;
            } else if record_kind && record.status != ProcessStatus::Running {
                let mut harness = session_harness_options_from_record(record);
                if harness.is_empty() {
                    harness = current_harness.clone();
                }
                let resume_id = match selected_kind {
                    SessionKind::Codex => thread.native_thread_id.clone(),
                    SessionKind::Claude => record.session_resume_id.clone(),
                    SessionKind::Shell => None,
                };
                if let Some(process_id) = launch_session(harness, resume_id) {
                    launched_new_session = true;
                    process_id
                } else {
                    return;
                }
            } else if let Some(process_id) = launch_session(current_harness.clone(), None) {
                launched_new_session = true;
                process_id
            } else {
                return;
            }
        } else if let Some(process_id) = launch_session(current_harness.clone(), None) {
            launched_new_session = true;
            process_id
        } else {
            return;
        };
        debug!(
            workspace = %workspace_for_send,
            harness = ?selected_kind,
            thread_id,
            process_id,
            launched_new_session,
            "session send stage: resolved destination process"
        );

        if launched_new_session && matches!(selected_kind, SessionKind::Codex) {
            let deferred_inputs = {
                let mut deferred =
                    flush_pending_commands_for_send(&pending_commands_for_send, thread_id)
                        .into_iter()
                        .map(|command| DeferredSessionInput::Control { thread_id, command })
                        .collect::<Vec<_>>();
                deferred.push(DeferredSessionInput::User {
                    thread_id,
                    command: command.clone(),
                    staged_review,
                });
                deferred
            };
            if let Ok(writer) = WorkspaceStore::open(db_for_send.clone()) {
                let _ = writer.append_chat_message(thread_id, "user", &command, "user_send");
            }
            pending_session_inputs_for_send
                .borrow_mut()
                .entry(process_id)
                .or_default()
                .extend(deferred_inputs);
            debug!(
                workspace = %workspace_for_send,
                harness = ?selected_kind,
                thread_id,
                process_id,
                "session send stage: queued deferred inputs until codex is ready"
            );
            if staged_review {
                app_state_for_send.set_staged_review_prompt(None);
            }
            refresh_view_for_send();
            if launched_new_session {
                refresh_for_send();
            }
            return;
        }

        {
            let mut sessions = active_sessions_for_send.borrow_mut();
            let Some(session) = sessions.get_mut(&process_id) else {
                warn!(
                    workspace = %workspace_for_send,
                    process_id,
                    harness = ?selected_kind,
                    "session missing from active attachments"
                );
                let error = Label::new(Some(&format!(
                    "[session send] Session #{process_id} is detached from this UI."
                )));
                error.add_css_class("chat-agent-text");
                error.set_wrap(true);
                error.set_xalign(0.0);
                messages_for_send.append(&error);
                return;
            };
            debug!(
                workspace = %workspace_for_send,
                harness = ?selected_kind,
                thread_id,
                process_id,
                "session send stage: acquired live session attachment"
            );
            for control in flush_pending_commands_for_send(&pending_commands_for_send, thread_id) {
                if let Err(err) = session.send_line(&control) {
                    error!(
                        workspace = %workspace_for_send,
                        process_id,
                        harness = ?selected_kind,
                        control = %control,
                        error = %err,
                        "failed to write pending control command"
                    );
                    let error = Label::new(Some(&format!(
                        "[session control error for #{process_id}] {err:#}"
                    )));
                    error.add_css_class("chat-agent-text");
                    error.set_wrap(true);
                    error.set_xalign(0.0);
                    messages_for_send.append(&error);
                    return;
                }
                if let Ok(writer) = WorkspaceStore::open(db_for_send.clone()) {
                    let _ = writer.append_chat_message(
                        thread_id,
                        "system",
                        &control,
                        "control_command",
                    );
                }
            }
            debug!(
                workspace = %workspace_for_send,
                harness = ?selected_kind,
                thread_id,
                process_id,
                "session send stage: flushed pending commands"
            );
            if let Ok(writer) = WorkspaceStore::open(db_for_send.clone()) {
                let _ = writer.append_chat_message(thread_id, "user", &command, "user_send");
            }
            debug!(
                workspace = %workspace_for_send,
                harness = ?selected_kind,
                thread_id,
                process_id,
                "session send stage: persisted user chat message"
            );
            if let Err(err) = session.send_line(&command) {
                error!(
                    workspace = %workspace_for_send,
                    process_id,
                    harness = ?selected_kind,
                    error = %err,
                    "failed to write command to session"
                );
                let error = Label::new(Some(&format!(
                    "[session send error for #{process_id}] {err:#}"
                )));
                error.add_css_class("chat-agent-text");
                error.set_wrap(true);
                error.set_xalign(0.0);
                messages_for_send.append(&error);
                return;
            }
            debug!(
                workspace = %workspace_for_send,
                harness = ?selected_kind,
                thread_id,
                process_id,
                "session send stage: wrote user command to session"
            );
        }

        let log_text = if staged_review {
            staged_review_prompt_text(&command)
        } else {
            session_input_log_text(&workspace_for_send, process_id, &command)
        };
        if let Ok(writer) = WorkspaceStore::open(db_for_send.clone()) {
            let _ = writer.append_session_process_output(process_id, &log_text);
        }
        debug!(
            workspace = %workspace_for_send,
            process_id,
            harness = ?selected_kind,
            staged_review,
            logged_bytes = log_text.len(),
            "session input appended to transcript"
        );
        last_output_for_send
            .borrow_mut()
            .insert(process_id, Instant::now());
        if staged_review {
            app_state_for_send.set_staged_review_prompt(None);
        }
        refresh_view_for_send();
        if launched_new_session {
            refresh_for_send();
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
                        error.set_wrap(true);
                        error.set_xalign(0.0);
                        messages_for_switch.append(&error);
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
    repo_label.set_hexpand(true);
    repo_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    repo_label.set_width_chars(1);
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

fn session_transcript_event_widget(event: &SessionTranscriptEvent) -> Widget {
    match event.role {
        SessionTranscriptRole::User | SessionTranscriptRole::ReviewPrompt => {
            chat_user_bubble(&event.body).upcast()
        }
        _ => {
            let label = Label::new(Some(&format!("{}\n{}", event.role.label(), event.body)));
            label.add_css_class("chat-agent-text");
            label.set_selectable(true);
            label.set_wrap(true);
            label.set_xalign(0.0);
            label.set_hexpand(true);
            label.set_margin_bottom(18);
            label.upcast()
        }
    }
}

fn chat_message_widget(message: &ChatMessageRecord) -> Widget {
    match message.role.as_str() {
        "user" => chat_user_bubble(&message.content).upcast(),
        "system" => {
            let label = Label::new(Some(&message.content));
            label.add_css_class("card-meta");
            label.set_selectable(true);
            label.set_wrap(true);
            label.set_xalign(0.0);
            label.set_margin_bottom(12);
            label.upcast()
        }
        _ => {
            let inline_events = parse_codex_inline_events_local(&message.content);
            if !inline_events.is_empty() {
                return inline_events_widget(&inline_events);
            }
            let label = Label::new(Some(&message.content));
            label.add_css_class("chat-agent-text");
            label.set_selectable(true);
            label.set_wrap(true);
            label.set_xalign(0.0);
            label.set_hexpand(true);
            label.set_margin_bottom(18);
            label.upcast()
        }
    }
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
    }
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

    let header = GBox::new(Orientation::Horizontal, 8);
    header.add_css_class("chat-inline-event-header");
    header.set_hexpand(true);

    let glyph = Label::new(Some(inline_event_kind_glyph(event.kind)));
    glyph.add_css_class("chat-inline-event-meta");
    header.append(&glyph);

    let text = GBox::new(Orientation::Vertical, 2);
    text.set_hexpand(true);
    let title = Label::new(Some(&format!(
        "{} · {}",
        inline_event_kind_label(event.kind),
        event.title
    )));
    title.add_css_class("chat-inline-event-title");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&title);
    let meta_text = event
        .subtitle
        .as_deref()
        .unwrap_or_else(|| inline_event_status_label(event.status));
    let meta = Label::new(Some(meta_text));
    meta.add_css_class("chat-inline-event-meta");
    meta.set_xalign(0.0);
    meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&meta);
    header.append(&text);

    let toggle = ToggleButton::with_label("Show");
    toggle.add_css_class("chat-toolbar-btn");
    header.append(&toggle);
    root.append(&header);

    let body_text = inline_event_body_text(event);
    let body_preview = truncate_inline_event_body(&body_text, 320);
    let body = Label::new(Some(&body_preview.preview));
    body.add_css_class("chat-inline-event-body");
    body.set_selectable(true);
    body.set_wrap(true);
    body.set_xalign(0.0);
    body.set_visible(false);
    root.append(&body);

    toggle.connect_toggled({
        let body = body.clone();
        let full = body_preview.full.clone();
        let preview = body_preview.preview.clone();
        move |button| {
            if button.is_active() {
                body.set_text(&full);
                body.set_visible(true);
                button.set_label("Hide");
            } else {
                body.set_text(&preview);
                body.set_visible(false);
                button.set_label("Show");
            }
        }
    });

    root.upcast()
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
            threads
                .iter()
                .any(|thread| thread.id == *id && thread.provider == provider)
        })
        .or_else(|| {
            threads
                .iter()
                .find(|thread| thread.provider == provider)
                .map(|thread| thread.id)
        })
}

fn default_chat_thread_title(kind: SessionKind, threads: &[ChatThreadRecord]) -> String {
    let provider = session_kind_provider(kind);
    let next = threads
        .iter()
        .filter(|thread| thread.provider == provider)
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
    &[SessionKind::Codex, SessionKind::Claude, SessionKind::Shell]
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
    *selected_thread.borrow_mut() = next_thread;
    set_selected_chat_thread(next_thread);
    update_composer_state();
}

fn session_kind_from_index(index: usize) -> SessionKind {
    match index {
        1 => SessionKind::Claude,
        2 => SessionKind::Shell,
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
    active_sessions: &HashMap<i64, SessionConnection>,
) -> Option<i64> {
    preferred
        .filter(|id| {
            active_sessions.contains_key(id)
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
                        && active_sessions.contains_key(&record.id)
                })
                .map(|record| record.id)
        })
}

fn stop_active_chat_session(
    database_path: &Path,
    workspace_name: &str,
    process_id: i64,
    active_sessions: &Rc<RefCell<HashMap<i64, SessionConnection>>>,
    last_output: &Rc<RefCell<HashMap<i64, Instant>>>,
) -> anyhow::Result<()> {
    if let Some(mut session) = active_sessions.borrow_mut().remove(&process_id) {
        session.stop()?;
    }
    last_output.borrow_mut().remove(&process_id);
    WorkspaceStore::open(database_path)?
        .stop_session_process(workspace_name, process_id)
        .map(|_| ())
}

fn seed_chat_running_sessions(
    database_path: &Path,
    workspace_name: &str,
    active_sessions: &Rc<RefCell<HashMap<i64, SessionConnection>>>,
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
        if record.status != ProcessStatus::Running || sessions.contains_key(&record.id) {
            continue;
        }
        if session_kind_matches_record(&record, SessionKind::Codex) {
            continue;
        }
        if let Ok(session) = SessionConnection::try_reattach_running(record.pid) {
            sessions.insert(record.id, session);
            last_output.borrow_mut().insert(record.id, Instant::now());
        }
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

#[allow(dead_code)]
fn agent_session_panel_impl(
    database_path: PathBuf,
    workspace_name: &str,
    app_state: AppState,
    refresh: impl Fn() + Clone + 'static,
) -> GBox {
    let root = GBox::new(Orientation::Vertical, 0);
    root.add_css_class("agent-panel");
    root.add_css_class("session-surface");

    // ── Status labels ────────────────────────────────────────────
    let provider_status = Label::new(Some("Loading provider and MCP status..."));
    provider_status.add_css_class("card-meta");
    provider_status.set_xalign(0.0);
    provider_status.set_wrap(true);

    let active_status = Label::new(Some("No active session."));
    active_status.add_css_class("card-meta");
    active_status.set_xalign(0.0);

    // ── Config options (collapsed by default) ────────────────────
    let harness_row = GBox::new(Orientation::Horizontal, 8);
    let plan_mode = CheckButton::with_label("Plan mode");
    let fast_mode = CheckButton::with_label("Fast mode");
    let approval_mode = ComboBoxText::new();
    approval_mode.append(Some("default"), "Approvals: default");
    approval_mode.append(Some("ask"), "Approvals: ask");
    approval_mode.append(Some("never"), "Approvals: never");
    approval_mode.set_active(Some(0));
    let reasoning_mode = ComboBoxText::new();
    reasoning_mode.append(Some("default"), "Reasoning: default");
    reasoning_mode.append(Some("low"), "Reasoning: low");
    reasoning_mode.append(Some("medium"), "Reasoning: medium");
    reasoning_mode.append(Some("high"), "Reasoning: high");
    reasoning_mode.set_active(Some(0));
    let effort_mode = ComboBoxText::new();
    effort_mode.append(Some("default"), "Effort: default");
    effort_mode.append(Some("low"), "Effort: low");
    effort_mode.append(Some("medium"), "Effort: medium");
    effort_mode.append(Some("high"), "Effort: high");
    effort_mode.set_active(Some(0));
    plan_mode.set_tooltip_text(Some("Plan mode applies to session startup."));
    fast_mode.set_tooltip_text(Some("Fast mode applies to session startup."));
    approval_mode.set_tooltip_text(Some("Tool approval preference for this session."));
    reasoning_mode.set_tooltip_text(Some("Reasoning preference for this session."));
    effort_mode.set_tooltip_text(Some("Effort preference for this session."));
    harness_row.append(&plan_mode);
    harness_row.append(&fast_mode);
    harness_row.append(&approval_mode);
    harness_row.append(&reasoning_mode);
    harness_row.append(&effort_mode);

    let codex_row = GBox::new(Orientation::Horizontal, 8);
    let codex_personality = ComboBoxText::new();
    codex_personality.append(Some("default"), "Personality: default");
    codex_personality.append(Some("friendly"), "Personality: friendly");
    codex_personality.append(Some("pragmatic"), "Personality: pragmatic");
    codex_personality.append(Some("none"), "Personality: none");
    codex_personality.set_active(Some(0));
    let codex_goals = Entry::new();
    codex_goals.set_placeholder_text(Some("Codex goals"));
    let codex_skills = Entry::new();
    codex_skills.set_placeholder_text(Some("Codex skills"));
    codex_personality.set_tooltip_text(Some("Codex personality for session startup."));
    codex_goals.set_tooltip_text(Some("Codex goals for this session."));
    codex_skills.set_tooltip_text(Some("Codex skills for this session."));
    codex_row.append(&codex_personality);
    codex_row.append(&codex_goals);
    codex_row.append(&codex_skills);

    // ── Launch buttons ───────────────────────────────────────────
    let mut agent_buttons: Vec<(Button, SessionKind)> = Vec::new();
    for (label, kind) in [
        ("Shell", SessionKind::Shell),
        ("Codex", SessionKind::Codex),
        ("Claude", SessionKind::Claude),
    ] {
        let button = text_button(label);
        match kind {
            SessionKind::Codex | SessionKind::Claude => button.add_css_class("suggested-action"),
            SessionKind::Shell => button.add_css_class("flat-action"),
        }
        let tooltip = match kind {
            SessionKind::Shell => "Open a PTY shell inside this workspace.",
            SessionKind::Codex => "Open a PTY Codex session inside this workspace.",
            SessionKind::Claude => "Open a PTY Claude session inside this workspace.",
        };
        button.set_tooltip_text(Some(tooltip));
        agent_buttons.push((button, kind));
    }

    // ── Session selector + controls ──────────────────────────────
    let sessions_combo = ComboBoxText::new();
    sessions_combo.set_hexpand(true);
    let refresh_btn = session_flat_button("Refresh");
    let stop_btn = session_destructive_button("Stop session");
    stop_btn.set_tooltip_text(Some("Stop the selected running session."));

    // ── Transcript ───────────────────────────────────────────────
    let transcript = TextView::new();
    transcript.set_editable(false);
    transcript.set_monospace(true);
    transcript.add_css_class("history-view");
    transcript.add_css_class("session-transcript");
    transcript.set_vexpand(true);
    let transcript_buffer = transcript.buffer();
    let transcript_text = initial_session_text(&database_path, workspace_name);
    transcript_buffer.set_text(&transcript_text);
    let transcript_scroll = ScrolledWindow::new();
    transcript_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    transcript_scroll.set_vexpand(true);
    transcript_scroll.set_child(Some(&transcript));

    // ── Input row ────────────────────────────────────────────────
    let input_row = session_action_row();
    let input = Entry::new();
    input.set_placeholder_text(Some("Type session input..."));
    input.set_hexpand(true);
    let send_btn = text_button("Send");
    send_btn.add_css_class("suggested-action");
    send_btn.set_tooltip_text(Some("Send a line to the selected session."));
    input_row.append(&input);
    input_row.append(&send_btn);

    // ── Staged review row ────────────────────────────────────────
    let staged_review_row = session_action_row();
    let staged_review_status = Label::new(Some("No staged review prompt."));
    staged_review_status.add_css_class("card-meta");
    staged_review_status.set_xalign(0.0);
    staged_review_status.set_wrap(true);
    staged_review_status.set_hexpand(true);
    let send_review_btn = session_secondary_button("Send review prompt");
    send_review_btn.set_tooltip_text(Some(
        "Send the staged review prompt to the selected session.",
    ));
    staged_review_row.append(&staged_review_status);
    staged_review_row.append(&send_review_btn);

    // ── Checkpoint row ───────────────────────────────────────────
    let checkpoint_row = session_action_row();
    let checkpoint_message = Entry::new();
    checkpoint_message.set_placeholder_text(Some("Checkpoint message (optional)"));
    checkpoint_message.set_hexpand(true);
    let checkpoint_btn = session_secondary_button("Create checkpoint");
    checkpoint_btn.set_tooltip_text(Some(
        "Create workspace checkpoint tied to selected session.",
    ));
    checkpoint_row.append(&checkpoint_message);
    checkpoint_row.append(&checkpoint_btn);

    // ── Layout: top bar → spawn bar → options → transcript → input ──

    // Top bar: active session selector + stop + refresh
    let top_bar = session_action_row();
    top_bar.append(&sessions_combo);
    top_bar.append(&refresh_btn);
    top_bar.append(&stop_btn);

    // Spawn bar: pick what to launch
    let spawn_bar = session_action_row();
    let spawn_label = Label::new(Some("New:"));
    spawn_label.add_css_class("detail-label");
    spawn_bar.append(&spawn_label);
    for (button, _) in &agent_buttons {
        spawn_bar.append(button);
    }

    // Options expander (collapsed by default): harness config + checkpoint
    let options_inner = GBox::new(Orientation::Vertical, 6);
    options_inner.set_margin_top(4);
    options_inner.set_margin_bottom(4);
    options_inner.append(&harness_row);
    options_inner.append(&codex_row);
    options_inner.append(&provider_status);
    options_inner.append(&checkpoint_row);
    let options_expander = gtk::Expander::new(Some("Session options"));
    options_expander.set_child(Some(&options_inner));
    options_expander.set_expanded(false);

    root.append(&top_bar);
    root.append(&spawn_bar);
    root.append(&options_expander);
    root.append(&active_status);
    root.append(&transcript_scroll);
    root.append(&staged_review_row);
    root.append(&input_row);

    let record_state = Rc::new(RefCell::new(Vec::<ProcessRecord>::new()));
    let selected_session: Rc<RefCell<Option<i64>>> =
        Rc::new(RefCell::new(app_state.selected_agent_session()));
    let active_sessions: Rc<RefCell<HashMap<i64, SessionConnection>>> =
        Rc::new(RefCell::new(HashMap::new()));
    let last_output = Rc::new(RefCell::new(HashMap::<i64, Instant>::new()));
    let last_size = Rc::new(RefCell::new(None::<(u16, u16)>));
    let reconcile_tick = Rc::new(RefCell::new(0_u32));

    let refresh_view = {
        let db_path = database_path.clone();
        let workspace = workspace_name.to_owned();
        let combo = sessions_combo.clone();
        let selected = selected_session.clone();
        let records = record_state.clone();
        let status = active_status.clone();
        let transcript_buffer = transcript_buffer.clone();
        let provider_status = provider_status.clone();
        let active_sessions = active_sessions.clone();
        let last_output = last_output.clone();
        let app_state = app_state.clone();
        Rc::new(move || {
            let loaded = WorkspaceStore::open(db_path.clone())
                .and_then(|store| store.list_sessions(&workspace))
                .unwrap_or_default();

            {
                let mut current = records.borrow_mut();
                current.clear();
                current.extend(loaded);
            }

            let preserved = *selected.borrow();
            let attached_sessions = active_sessions
                .borrow()
                .keys()
                .copied()
                .collect::<HashSet<_>>();
            let active_record = {
                let current = records.borrow();
                preserve_combo_selection(&combo, &current, preserved, &attached_sessions)
            };
            *selected.borrow_mut() = active_record;
            app_state.set_selected_agent_session(active_record);

            match active_record {
                Some(process_id) => {
                    let current = records.borrow();
                    let Some(record) = current.iter().find(|record| record.id == process_id) else {
                        status.set_text("No session selected.");
                        transcript_buffer.set_text("No session selected.");
                        return;
                    };
                    let attached = attached_sessions.contains(&process_id);
                    let last_seen = last_output.borrow().get(&process_id).copied();
                    let runtime_state = session_runtime_state(
                        record,
                        if record.status == ProcessStatus::Running {
                            last_seen
                        } else {
                            None
                        },
                        attached,
                    );
                    let contents = fs::read_to_string(&record.log_path)
                        .unwrap_or_else(|_| "[could not read selected session log]\n".to_string());
                    transcript_buffer.set_text(&format_selected_session_surface(
                        record,
                        &contents,
                        runtime_state,
                        attached,
                    ));
                    status.set_text(&format!(
                        "Selected #{}: {} (status={}, state={}, pid={}, started={}{})",
                        process_id,
                        session_kind_label(&record.command),
                        record.status.as_str(),
                        runtime_state,
                        record.pid,
                        record.started_at,
                        session_harness_metadata_label(&record.session_harness_metadata),
                    ));
                }
                None => {
                    status.set_text("No session selected.");
                    transcript_buffer
                        .set_text("No local sessions yet. Start Shell/Codex/Claude above.");
                }
            }

            if let Ok(store) = WorkspaceStore::open(db_path.clone()) {
                if let Ok(mcp_status) = store.mcp_status(&workspace) {
                    provider_status.set_text(&provider_status_text(&mcp_status));
                }
            }
            staged_review_status.set_text(&staged_review_status_text(
                app_state.staged_review_prompt().as_deref(),
            ));
        }) as Rc<dyn Fn()>
    };

    refresh_view();
    seed_running_sessions(
        &database_path,
        workspace_name,
        &active_sessions,
        &transcript_buffer,
    );
    refresh_view();

    let db_for_refresh = database_path.clone();
    let refresh_view_for_btn = refresh_view.clone();
    let active_sessions_for_refresh = active_sessions.clone();
    let transcript_buffer_for_refresh = transcript_buffer.clone();
    let workspace_for_refresh = workspace_name.to_owned();
    let refresh_after_refresh = refresh.clone();
    refresh_btn.connect_clicked(move |_| {
        let _ = WorkspaceStore::open(db_for_refresh.clone())
            .and_then(|store| store.reconcile_session_processes())
            .map(|_| ());
        seed_running_sessions(
            &db_for_refresh,
            &workspace_for_refresh,
            &active_sessions_for_refresh,
            &transcript_buffer_for_refresh,
        );
        refresh_view_for_btn();
        refresh_after_refresh();
    });

    let db_for_send = database_path.clone();
    let workspace_for_send = workspace_name.to_owned();
    let active_sessions_send = active_sessions.clone();
    let selected_send = selected_session.clone();
    let last_output_send = last_output.clone();
    let transcript_buffer_send = transcript_buffer.clone();
    let refresh_view_send = refresh_view.clone();
    let app_state_for_send = app_state.clone();
    let send_text = Rc::new({
        move |text: String, staged_review: bool| {
            let Some(process_id) = *selected_send.borrow() else {
                append_text(
                    &transcript_buffer_send,
                    "[session send] No session selected; cannot send input.\n",
                );
                return;
            };
            let command = text.trim().to_owned();
            if command.is_empty() {
                return;
            }
            {
                let mut sessions = active_sessions_send.borrow_mut();
                let Some(session) = sessions.get_mut(&process_id) else {
                    append_text(
                        &transcript_buffer_send,
                        &format!(
                            "[session send] Session #{process_id} is detached from this UI.\n"
                        ),
                    );
                    return;
                };
                if let Err(err) = session.send_line(&command) {
                    append_text(
                        &transcript_buffer_send,
                        &format!("[session send error for #{process_id}] {err:#}\n"),
                    );
                    return;
                }
            }
            let log_text = if staged_review {
                staged_review_prompt_text(&command)
            } else {
                session_input_log_text(&workspace_for_send, process_id, &command)
            };
            if let Ok(writer) = WorkspaceStore::open(db_for_send.clone()) {
                let _ = writer.append_session_process_output(process_id, &log_text);
            }
            last_output_send
                .borrow_mut()
                .insert(process_id, Instant::now());
            append_text(
                &transcript_buffer_send,
                &live_session_append_text(&log_text),
            );
            if staged_review {
                app_state_for_send.set_staged_review_prompt(None);
            }
            refresh_view_send();
        }
    });
    send_btn.connect_clicked({
        let send_text = send_text.clone();
        let input = input.clone();
        move |_| {
            let command = input.text().to_string();
            if command.trim().is_empty() {
                return;
            }
            input.set_text("");
            (send_text)(command, false);
        }
    });
    input.connect_activate({
        let send_text = send_text.clone();
        let input = input.clone();
        move |_| {
            let command = input.text().to_string();
            if command.trim().is_empty() {
                return;
            }
            input.set_text("");
            (send_text)(command, false);
        }
    });
    send_review_btn.connect_clicked({
        let send_text = send_text.clone();
        let app_state = app_state.clone();
        move |_| {
            let Some(prompt) = app_state.staged_review_prompt() else {
                return;
            };
            (send_text)(prompt, true);
        }
    });

    let db_for_stop = database_path.clone();
    let workspace_for_stop = workspace_name.to_owned();
    let active_sessions_stop = active_sessions.clone();
    let selected_session_stop = selected_session.clone();
    let last_output_stop = last_output.clone();
    let transcript_buffer_stop = transcript_buffer.clone();
    let refresh_view_stop = refresh_view.clone();
    let refresh_stop = refresh.clone();
    stop_btn.connect_clicked(move |_| {
        let Some(process_id) = *selected_session_stop.borrow() else {
            append_text(
                &transcript_buffer_stop,
                "[session stop] No session selected.\n",
            );
            return;
        };

        if let Some(mut session) = active_sessions_stop.borrow_mut().remove(&process_id) {
            if let Err(err) = session.stop() {
                append_text(
                    &transcript_buffer_stop,
                    &format!("[session stop local #{process_id}] {err:#}\n"),
                );
            }
        }

        last_output_stop.borrow_mut().remove(&process_id);
        let result = WorkspaceStore::open(db_for_stop.clone())
            .and_then(|store| store.stop_session_process(&workspace_for_stop, process_id));
        match result {
            Ok(process) => append_text(
                &transcript_buffer_stop,
                &format!(
                    "[session stop] #{process_id} status={} pid={}\n",
                    process.status.as_str(),
                    process.pid
                ),
            ),
            Err(err) => append_text(
                &transcript_buffer_stop,
                &format!("[session stop #{process_id}] {err:#}\n"),
            ),
        }
        refresh_view_stop();
        refresh_stop();
    });

    let selected_checkpoint = selected_session.clone();
    let db_for_checkpoint = database_path.clone();
    let workspace_for_checkpoint = workspace_name.to_owned();
    let transcript_buffer_checkpoint = transcript_buffer.clone();
    let checkpoint_message_field = checkpoint_message.clone();
    checkpoint_btn.connect_clicked(move |_| {
        let Some(process_id) = *selected_checkpoint.borrow() else {
            append_text(
                &transcript_buffer_checkpoint,
                "[session checkpoint] Select a session to attach checkpoint metadata.\n",
            );
            return;
        };
        let mut message = checkpoint_message_field.text().to_string();
        if message.trim().is_empty() {
            message = format!("Session {process_id} checkpoint");
        }
        match WorkspaceStore::open(db_for_checkpoint.clone()).and_then(|store| {
            store.checkpoint_create(&workspace_for_checkpoint, &message, Some(process_id))
        }) {
            Ok(checkpoint) => append_text(
                &transcript_buffer_checkpoint,
                &format!(
                    "[checkpoint] created #{}: {}\n",
                    checkpoint.id, checkpoint.message
                ),
            ),
            Err(err) => append_text(
                &transcript_buffer_checkpoint,
                &format!("[checkpoint error] {err:#}\n"),
            ),
        }
    });

    sessions_combo.connect_changed({
        let selected_for_change = selected_session.clone();
        let app_state = app_state.clone();
        let refresh_view = refresh_view.clone();
        move |combo| {
            let selected = combo
                .active_id()
                .and_then(|value| value.as_str().parse::<i64>().ok());
            *selected_for_change.borrow_mut() = selected;
            app_state.set_selected_agent_session(selected);
            refresh_view();
        }
    });

    for (button, kind) in agent_buttons {
        let db_path_for_launch = database_path.clone();
        let workspace_for_launch = workspace_name.to_owned();
        let active_sessions_for_launch = active_sessions.clone();
        let refresh_view_for_launch = refresh_view.clone();
        let last_output_for_launch = last_output.clone();
        let transcript_for_launch = transcript.clone();
        let buffer_for_launch = transcript_buffer.clone();
        let refresh_for_launch = refresh.clone();
        let selected_for_launch = selected_session.clone();
        let app_state_for_launch = app_state.clone();
        let plan_mode_for_launch = plan_mode.clone();
        let fast_mode_for_launch = fast_mode.clone();
        let approval_mode_for_launch = approval_mode.clone();
        let reasoning_mode_for_launch = reasoning_mode.clone();
        let effort_mode_for_launch = effort_mode.clone();
        let codex_personality_for_launch = codex_personality.clone();
        let codex_goals_for_launch = codex_goals.clone();
        let codex_skills_for_launch = codex_skills.clone();
        button.connect_clicked(move |_| {
            let launch_options = collect_session_harness_options(
                &plan_mode_for_launch,
                &fast_mode_for_launch,
                &approval_mode_for_launch,
                &reasoning_mode_for_launch,
                &effort_mode_for_launch,
                &codex_personality_for_launch,
                &codex_goals_for_launch,
                &codex_skills_for_launch,
            );
            let launch = match WorkspaceStore::open(db_path_for_launch.clone()).and_then(|store| {
                store.session_launch_with_options(&workspace_for_launch, kind, launch_options)
            }) {
                Ok(launch) => launch,
                Err(err) => {
                    append_text(&buffer_for_launch, &format!("[session start] {err:#}\n"));
                    return;
                }
            };

            let (rows, cols) = session_size_from_widget(
                transcript_for_launch.allocated_width(),
                transcript_for_launch.allocated_height(),
            );

            let mut pty = match PtySession::spawn(
                launch.program.clone(),
                launch.args.clone(),
                &launch.cwd,
                launch.env.clone(),
                rows,
                cols,
            ) {
                Ok(session) => session,
                Err(err) => {
                    append_text(
                        &buffer_for_launch,
                        &format!("[session start] {kind:?} launch failed: {err:#}\n"),
                    );
                    return;
                }
            };

            if matches!(kind, SessionKind::Codex) {
                if let Some(bootstrap) = launch.env_value("ARCHDUCTOR_SESSION_BOOTSTRAP") {
                    if let Err(err) = pty.write(&(bootstrap.to_owned() + "\n")) {
                        let _ = pty.stop();
                        append_text(
                            &buffer_for_launch,
                            &format!("[session start] failed to send Codex bootstrap: {err:#}\n"),
                        );
                        return;
                    }
                }
            }

            let pid = match pty.process_id() {
                Some(pid) => pid,
                None => {
                    let _ = pty.stop();
                    append_text(
                        &buffer_for_launch,
                        "[session start] failed to allocate process id.\n",
                    );
                    return;
                }
            };

            let session_record = match WorkspaceStore::open(db_path_for_launch.clone())
                .and_then(|store| store.record_session_process(&workspace_for_launch, &launch, pid))
            {
                Ok(record) => record,
                Err(err) => {
                    let _ = pty.stop();
                    append_text(
                        &buffer_for_launch,
                        &format!("[session start] failed to record process: {err:#}\n"),
                    );
                    return;
                }
            };

            active_sessions_for_launch
                .borrow_mut()
                .insert(session_record.id, SessionConnection::Live(pty));
            last_output_for_launch
                .borrow_mut()
                .insert(session_record.id, Instant::now());
            info!(
                workspace = %workspace_for_launch,
                process_id = session_record.id,
                pid,
                harness = ?kind,
                session_log_path = %session_record.log_path.display(),
                "session launched from legacy session surface"
            );
            append_text(
                &buffer_for_launch,
                &format!(
                    "[session started] #{} kind={} pid={}\n",
                    session_record.id,
                    session_kind_label(&session_record.command),
                    pid
                ),
            );
            *selected_for_launch.borrow_mut() = Some(session_record.id);
            app_state_for_launch.set_selected_agent_session(Some(session_record.id));
            refresh_view_for_launch();
            refresh_for_launch();
        });
    }

    let db_for_poll = database_path.clone();
    let active_sessions_for_poll = active_sessions.clone();
    let transcript_buffer_for_poll = transcript_buffer.clone();
    let transcript_for_poll = transcript.clone();
    let selected_for_poll = selected_session.clone();
    let last_output_for_poll = last_output.clone();
    let record_state_for_poll = record_state.clone();
    let last_screen_for_poll = Rc::new(RefCell::new(HashMap::<i64, String>::new()));
    let trust_answered_for_poll = Rc::new(RefCell::new(HashSet::<i64>::new()));
    let last_size_for_poll = last_size.clone();
    let reconcile_tick_for_poll = reconcile_tick.clone();
    let refresh_view_for_poll = refresh_view.clone();
    let refresh_for_poll = refresh.clone();
    glib::timeout_add_local(Duration::from_millis(SESSION_POLL_INTERVAL_MS), move || {
        let size = session_size_from_widget(
            transcript_for_poll.allocated_width(),
            transcript_for_poll.allocated_height(),
        );
        let should_resize = match *last_size_for_poll.borrow() {
            Some(previous) => previous != size,
            None => true,
        };
        *last_size_for_poll.borrow_mut() = Some(size);

        let mut ended = Vec::<i64>::new();
        let mut should_refresh_view = false;
        let mut should_refresh_outer = false;
        {
            let mut sessions = active_sessions_for_poll.borrow_mut();
            for (process_id, session) in sessions.iter_mut() {
                let output = session.read_available();
                if !output.is_empty() {
                    let is_codex =
                        is_codex_session_record(&record_state_for_poll.borrow(), *process_id);
                    let _ = WorkspaceStore::open(db_for_poll.clone()).and_then(|store| {
                        if is_codex {
                            store.append_session_process_output(
                                *process_id,
                                &format_codex_raw_output(&output),
                            )
                        } else {
                            store.append_session_process_output(*process_id, &output)
                        }
                    });
                    let is_selected = *selected_for_poll.borrow() == Some(*process_id);
                    if is_selected && !is_codex {
                        append_text(
                            &transcript_buffer_for_poll,
                            &live_session_append_text(&output),
                        );
                    }
                    last_output_for_poll
                        .borrow_mut()
                        .insert(*process_id, Instant::now());
                    if is_selected {
                        should_refresh_view = true;
                    }
                }

                if is_codex_session_record(&record_state_for_poll.borrow(), *process_id) {
                    if let Some(screen) = session.visible_screen_text() {
                        let changed = last_screen_for_poll
                            .borrow()
                            .get(process_id)
                            .map(|previous| previous != &screen)
                            .unwrap_or(!screen.is_empty());
                        if !screen.is_empty() && changed {
                            let answered = trust_answered_for_poll.borrow().contains(process_id);
                            if !answered
                                && detect_directory_trust_prompt(&screen)
                                && session.send_line("1").is_ok()
                            {
                                trust_answered_for_poll.borrow_mut().insert(*process_id);
                            }
                            let _ = WorkspaceStore::open(db_for_poll.clone()).and_then(|store| {
                                store.append_session_process_output(
                                    *process_id,
                                    &format_codex_screen_snapshot(&screen),
                                )
                            });
                            log_codex_screen_snapshot("legacy-poll", *process_id, &screen);
                            last_screen_for_poll
                                .borrow_mut()
                                .insert(*process_id, screen);
                            if *selected_for_poll.borrow() == Some(*process_id) {
                                should_refresh_view = true;
                            }
                        }
                    }
                }

                if should_resize {
                    if let Err(err) = session.resize(size.0, size.1) {
                        append_text(
                            &transcript_buffer_for_poll,
                            &format!("[session resize error] {err:#}\n"),
                        );
                    }
                }

                match session.has_exited() {
                    Ok(true) => ended.push(*process_id),
                    Ok(false) => {}
                    Err(err) => append_text(
                        &transcript_buffer_for_poll,
                        &format!("[session poll error] {err:#}\n"),
                    ),
                }
            }
        }

        for process_id in ended {
            let was_active = *selected_for_poll.borrow() == Some(process_id);
            active_sessions_for_poll.borrow_mut().remove(&process_id);
            let _ = WorkspaceStore::open(db_for_poll.clone())
                .and_then(|store| store.mark_session_process_exited(process_id, None));
            last_output_for_poll.borrow_mut().remove(&process_id);
            last_screen_for_poll.borrow_mut().remove(&process_id);
            trust_answered_for_poll.borrow_mut().remove(&process_id);
            if was_active {
                append_text(
                    &transcript_buffer_for_poll,
                    &live_session_append_text(&format!("[session finished] #{process_id}\n")),
                );
            }
            should_refresh_view = true;
            should_refresh_outer = true;
        }

        *reconcile_tick_for_poll.borrow_mut() += 1;
        if (*reconcile_tick_for_poll.borrow()).is_multiple_of(SESSION_RECONCILE_EVERY_TICKS) {
            if let Ok(reconciled) = WorkspaceStore::open(db_for_poll.clone())
                .and_then(|store| store.reconcile_session_processes())
            {
                if reconciled
                    .iter()
                    .any(|process| process.status != ProcessStatus::Running)
                {
                    for process in reconciled {
                        active_sessions_for_poll.borrow_mut().remove(&process.id);
                        last_output_for_poll.borrow_mut().remove(&process.id);
                        last_screen_for_poll.borrow_mut().remove(&process.id);
                        trust_answered_for_poll.borrow_mut().remove(&process.id);
                    }
                    should_refresh_view = true;
                    should_refresh_outer = true;
                }
            }
        }

        if should_refresh_view {
            refresh_view_for_poll();
        }
        if should_refresh_outer {
            refresh_for_poll();
        }
        glib::ControlFlow::Continue
    });

    root
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
            push_session_event(&mut events, role, line.to_owned());
            index += 1;
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
    session_threads: &RefCell<HashMap<i64, i64>>,
    records: &[ProcessRecord],
    selected_harness: SessionKind,
    selected_thread_id: Option<i64>,
) -> Option<i64> {
    match session_id {
        Some(session_id) => codex_thread_id_for_session(session_id, session_threads, records),
        None if selected_harness == SessionKind::Codex => selected_thread_id,
        None => None,
    }
}

fn should_render_codex_startup_card(state: &CodexStartupState) -> bool {
    matches!(
        state,
        CodexStartupState::Loading { .. } | CodexStartupState::Error { .. }
    )
}

fn codex_startup_status_widget(state: &CodexStartupState) -> Option<Widget> {
    if !should_render_codex_startup_card(state) {
        return None;
    }

    let row = GBox::new(Orientation::Horizontal, 10);
    row.set_hexpand(true);
    row.set_halign(Align::Fill);
    row.set_margin_bottom(12);

    match state {
        CodexStartupState::Loading { message } => {
            let spinner = Spinner::new();
            spinner.start();
            row.append(&spinner);

            let label = Label::new(Some(message));
            label.add_css_class("card-meta");
            label.set_wrap(true);
            label.set_xalign(0.0);
            label.set_hexpand(true);
            row.append(&label);
        }
        CodexStartupState::Error { message } => {
            let icon = Image::from_icon_name(resolve_icon_name("dialog-error-symbolic"));
            row.append(&icon);

            let label = Label::new(Some(&format!("Codex failed to start. {message}")));
            label.add_css_class("chat-agent-text");
            label.set_wrap(true);
            label.set_xalign(0.0);
            label.set_hexpand(true);
            row.append(&label);
        }
        CodexStartupState::Idle | CodexStartupState::Ready => return None,
    }

    Some(row.upcast())
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
    active_sessions: &Rc<RefCell<HashMap<i64, SessionConnection>>>,
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
    let mut detached = 0usize;
    let mut sessions = active_sessions.borrow_mut();
    for record in records {
        if record.status != ProcessStatus::Running || sessions.contains_key(&record.id) {
            continue;
        }
        match SessionConnection::try_reattach_running(record.pid) {
            Ok(session) => {
                sessions.insert(record.id, session);
                attached += 1;
            }
            Err(_) => detached += 1,
        }
    }
    drop(sessions);

    if attached > 0 {
        append_text(
            transcript_buffer,
            &format!("[session resume] reattached {attached} running session(s).\n"),
        );
    }
    if detached > 0 {
        append_text(
            transcript_buffer,
            &format!("[session resume] {detached} running session(s) are visible but detached.\n"),
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

fn is_codex_session_record(records: &[ProcessRecord], process_id: i64) -> bool {
    records
        .iter()
        .find(|record| record.id == process_id)
        .map(|record| session_kind_matches_record(record, SessionKind::Codex))
        .unwrap_or(false)
}

fn format_codex_raw_output(raw: &str) -> String {
    format!("[codex raw]\n{raw}\n[/codex raw]\n")
}

fn format_codex_screen_snapshot(screen: &str) -> String {
    format!(
        "[codex screen]\n{}\n[/codex screen]\n",
        screen.trim_end_matches('\n')
    )
}

enum DeferredSessionInput {
    Control {
        thread_id: i64,
        command: String,
    },
    User {
        thread_id: i64,
        command: String,
        staged_review: bool,
    },
}

#[derive(Clone)]
struct QueuedArchcarInput {
    input: String,
    kind: ArchcarInputKind,
}

#[derive(Debug, Clone)]
enum PendingArchcarAction {
    EnsureWorkspace {
        workspace: String,
        thread_id: Option<i64>,
    },
    StatusProbe {
        session_id: i64,
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

fn log_codex_screen_snapshot(source: &str, process_id: i64, screen: &str) {
    debug!(
        source,
        process_id,
        rendered_screen = %screen.trim_end_matches('\n'),
        "codex rendered screen snapshot"
    );
    crate::logger::write_pty_screen_snapshot(source, process_id, screen);
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
) {
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
            message,
        } => {
            warn!(?session_id, %message, "archcar session error");
            if let Some(session_id) = session_id {
                note_archcar_ready(&mut ready_cache.borrow_mut(), *session_id, false);
            }
            if let Some(thread_id) = codex_thread_id_for_startup_error(
                *session_id,
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
    database_path: &Path,
    workspace: &str,
    bridge: AsyncArchcarBridge,
    records: &[ProcessRecord],
    ready_cache: &RefCell<HashMap<i64, bool>>,
    pending_status: &RefCell<HashMap<i64, u64>>,
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
    pending_commands: &RefCell<HashMap<i64, Vec<String>>>,
    pending_inputs: &RefCell<HashMap<i64, Vec<QueuedArchcarInput>>>,
    startup_states: &RefCell<HashMap<i64, CodexStartupState>>,
    session_threads: &RefCell<HashMap<i64, i64>>,
    codex_ready: &RefCell<bool>,
    update_composer_state: &dyn Fn(),
    app_state: &AppState,
) {
    let Some(action) = inflight_actions
        .borrow_mut()
        .remove(&response.token)
        .or_else(|| fallback_action_from_archcar_response(&response))
    else {
        debug!(token = response.token, ?response.request, "archcar response had no tracked GTK action");
        return;
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
                let followup_session = apply_archcar_ensure_success(
                    &success,
                    &mut startup_states.borrow_mut(),
                    &mut session_threads.borrow_mut(),
                    thread_id,
                );
                if let Some(session_id) = followup_session {
                    request_archcar_status_probe(
                        &bridge,
                        pending_status,
                        inflight_actions,
                        session_id,
                    );
                }
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
                }
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
                }
            }
        },
        PendingArchcarAction::StatusProbe { session_id } => {
            pending_status.borrow_mut().remove(&session_id);
            match response.result {
                Ok(ArchcarResponse::SessionStatus { ready, .. }) => {
                    note_archcar_ready(&mut ready_cache.borrow_mut(), session_id, ready);
                    if ready {
                        if let Some(thread_id) =
                            codex_thread_id_for_session(session_id, session_threads, records)
                        {
                            apply_codex_startup_signal(
                                &mut startup_states.borrow_mut(),
                                CodexStartupSignal::Ready { thread_id },
                            );
                        }
                    }
                    if ready && !*codex_ready.borrow() {
                        info!(
                            session_id,
                            "reconciled codex ready state from async status probe"
                        );
                        set_codex_ready_state(codex_ready, update_composer_state, true);
                    }
                }
                Ok(other) => {
                    warn!(session_id, ?other, "unexpected archcar status response");
                }
                Err(err) => {
                    note_archcar_ready(&mut ready_cache.borrow_mut(), session_id, false);
                    warn!(session_id, error = %err, "archcar status probe failed");
                    if let Some(thread_id) =
                        codex_thread_id_for_session(session_id, session_threads, records)
                    {
                        apply_codex_startup_signal(
                            &mut startup_states.borrow_mut(),
                            CodexStartupSignal::Error {
                                thread_id,
                                message: err,
                            },
                        );
                    }
                }
            }
        }
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
                request_archcar_ensure(
                    bridge,
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
                request_archcar_ensure(
                    bridge,
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
                set_codex_ready_state(codex_ready, update_composer_state, false);
                request_archcar_ensure(
                    bridge,
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
                set_codex_ready_state(codex_ready, update_composer_state, false);
                request_archcar_ensure(
                    bridge,
                    inflight_actions,
                    workspace.to_owned(),
                    Some(thread_id),
                );
            }
        },
    }

    let _ = app_state;
    let _ = database_path;
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
        AsyncArchcarMessage::Response(_) => (true, false),
        AsyncArchcarMessage::BridgeError { .. } => (true, false),
    }
}

fn fallback_action_from_archcar_response(
    response: &AsyncArchcarResponse,
) -> Option<PendingArchcarAction> {
    match response.request {
        AsyncArchcarRequestKind::GetSessionStatus { session_id } => {
            Some(PendingArchcarAction::StatusProbe { session_id })
        }
        _ => None,
    }
}

fn request_archcar_ensure(
    bridge: AsyncArchcarBridge,
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
    workspace: String,
    thread_id: Option<i64>,
) {
    if let Some(token) = bridge.ensure_default_session(workspace.clone(), SessionKind::Codex) {
        inflight_actions.borrow_mut().insert(
            token,
            PendingArchcarAction::EnsureWorkspace {
                workspace,
                thread_id,
            },
        );
    }
}

fn request_archcar_status_probe(
    bridge: &AsyncArchcarBridge,
    pending_status: &RefCell<HashMap<i64, u64>>,
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
    session_id: i64,
) {
    if pending_status.borrow().contains_key(&session_id) {
        return;
    }
    let Some(token) = bridge.get_session_status(session_id) else {
        return;
    };
    pending_status.borrow_mut().insert(session_id, token);
    inflight_actions
        .borrow_mut()
        .insert(token, PendingArchcarAction::StatusProbe { session_id });
}

fn request_codex_status_probes(
    bridge: &AsyncArchcarBridge,
    records: &[ProcessRecord],
    pending_status: &RefCell<HashMap<i64, u64>>,
    inflight_actions: &RefCell<HashMap<u64, PendingArchcarAction>>,
) {
    for record in records.iter().filter(|record| {
        record.status == ProcessStatus::Running
            && session_kind_matches_record(record, SessionKind::Codex)
    }) {
        request_archcar_status_probe(bridge, pending_status, inflight_actions, record.id);
    }
}

fn apply_archcar_ensure_success(
    response: &ArchcarResponse,
    startup_states: &mut HashMap<i64, CodexStartupState>,
    session_threads: &mut HashMap<i64, i64>,
    requested_thread_id: Option<i64>,
) -> Option<i64> {
    match response {
        ArchcarResponse::SessionSpawnQueued { .. } | ArchcarResponse::Ack => {
            if let Some(thread_id) = requested_thread_id {
                apply_codex_startup_signal(
                    startup_states,
                    CodexStartupSignal::Loading { thread_id },
                );
            }
            None
        }
        ArchcarResponse::SessionSpawned {
            session_id,
            thread_id,
            ..
        } => {
            session_threads.insert(*session_id, *thread_id);
            apply_codex_startup_signal(
                startup_states,
                CodexStartupSignal::Loading {
                    thread_id: *thread_id,
                },
            );
            Some(*session_id)
        }
        _ => None,
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

fn codex_screen_ready_for_input(screen: &str) -> bool {
    let trimmed = screen.trim();
    !trimmed.is_empty()
        && trimmed.contains('›')
        && !detect_directory_trust_prompt(screen)
        && !screen.contains("Booting MCP server")
        && !screen.contains("Starting MCP servers")
        && !screen.contains("model:       loading")
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

fn flush_deferred_session_inputs(
    pending_inputs: &RefCell<HashMap<i64, Vec<DeferredSessionInput>>>,
    database_path: &Path,
    workspace_name: &str,
    process_id: i64,
    session: &mut SessionConnection,
) {
    let queued = pending_inputs
        .borrow_mut()
        .remove(&process_id)
        .unwrap_or_default();
    if queued.is_empty() {
        return;
    }

    let mut remaining = Vec::new();
    for input in queued {
        let send_result = match &input {
            DeferredSessionInput::Control { command, .. }
            | DeferredSessionInput::User { command, .. } => session.send_line(command),
        };
        if let Err(err) = send_result {
            warn!(
                process_id,
                error = %err,
                "failed to flush deferred session input"
            );
            remaining.push(input);
            continue;
        }

        match input {
            DeferredSessionInput::Control { thread_id, command } => {
                if let Ok(writer) = WorkspaceStore::open(database_path) {
                    let _ = writer.append_chat_message(
                        thread_id,
                        "system",
                        &command,
                        "control_command",
                    );
                }
            }
            DeferredSessionInput::User {
                thread_id: _,
                command,
                staged_review,
            } => {
                let log_text = if staged_review {
                    staged_review_prompt_text(&command)
                } else {
                    session_input_log_text(workspace_name, process_id, &command)
                };
                if let Ok(writer) = WorkspaceStore::open(database_path) {
                    let _ = writer.append_session_process_output(process_id, &log_text);
                }
            }
        }
    }

    if !remaining.is_empty() {
        pending_inputs.borrow_mut().insert(process_id, remaining);
    }
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

enum SessionConnection {
    Live(PtySession),
    Reattached {
        write: std::fs::File,
        output: Arc<Mutex<String>>,
        read_cursor: usize,
        pid: u32,
    },
}

impl SessionConnection {
    fn try_reattach_running(pid: u32) -> anyhow::Result<Self> {
        let path = terminal_device_path_for_pid(pid)?;
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

    fn send_line(&mut self, input: &str) -> anyhow::Result<()> {
        match self {
            Self::Live(session) => session.send_line(input),
            Self::Reattached { write, .. } => {
                write.write_all(input.as_bytes())?;
                write.flush()?;
                std::thread::sleep(Duration::from_millis(20));
                write.write_all(b"\r")?;
                write.flush()?;
                Ok(())
            }
        }
    }

    fn stop(&mut self) -> anyhow::Result<()> {
        match self {
            Self::Live(session) => session.stop(),
            Self::Reattached { .. } => Ok(()),
        }
    }

    fn resize(&mut self, rows: u16, cols: u16) -> anyhow::Result<()> {
        match self {
            Self::Live(session) => session.resize(rows, cols),
            Self::Reattached { .. } => Ok(()),
        }
    }

    fn has_exited(&mut self) -> anyhow::Result<bool> {
        match self {
            Self::Live(session) => session.has_exited(),
            Self::Reattached { pid, .. } => Ok(!terminal_process_alive(*pid)),
        }
    }

    fn visible_screen_text(&self) -> Option<String> {
        match self {
            Self::Live(session) => Some(session.visible_screen_text()),
            Self::Reattached { .. } => None,
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

fn terminal_device_path_for_pid(process_id: u32) -> anyhow::Result<PathBuf> {
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
        assert_eq!(session_kind_from_index(2), SessionKind::Shell);
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
    fn supported_chat_session_kinds_exclude_cursor() {
        assert_eq!(
            supported_chat_session_kinds(),
            &[SessionKind::Codex, SessionKind::Claude, SessionKind::Shell]
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
        let mut active_sessions = HashMap::new();
        active_sessions.insert(
            1,
            SessionConnection::Reattached {
                write: tempfile::tempfile().unwrap(),
                output: Arc::new(Mutex::new(String::new())),
                read_cursor: 0,
                pid: 1234,
            },
        );

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
            "functions.exec_command running cargo test\nUsing superpowers:brainstorming to shape the UI",
        );

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, CodexInlineEventKind::Tool);
        assert_eq!(events[0].title, "functions.exec_command");
        assert_eq!(events[0].status, CodexInlineEventStatus::Loading);
        assert_eq!(events[1].kind, CodexInlineEventKind::Skill);
        assert_eq!(events[1].title, "superpowers:brainstorming");
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
    fn ensure_success_tracks_spawned_session_for_followup_ready_probe() {
        let mut startup_states = HashMap::new();
        let mut session_threads = HashMap::new();

        let followup_session = apply_archcar_ensure_success(
            &ArchcarResponse::SessionSpawned {
                session_id: 57,
                thread_id: 11,
                workspace: "hoi-an".to_owned(),
                kind: SessionKind::Codex,
                pid: 4242,
            },
            &mut startup_states,
            &mut session_threads,
            Some(9),
        );

        assert_eq!(followup_session, Some(57));
        assert_eq!(session_threads.get(&57), Some(&11));
        assert_eq!(
            startup_states.get(&11),
            Some(&CodexStartupState::Loading {
                message: "Starting Codex...".to_owned(),
            })
        );
    }

    #[test]
    fn untracked_status_response_still_maps_back_to_status_probe() {
        let response = AsyncArchcarResponse {
            token: 7,
            request: AsyncArchcarRequestKind::GetSessionStatus { session_id: 61 },
            result: Ok(ArchcarResponse::SessionStatus {
                session_id: 61,
                status: "running".to_owned(),
                ready: true,
            }),
        };

        let action = fallback_action_from_archcar_response(&response);

        match action {
            Some(PendingArchcarAction::StatusProbe { session_id }) => {
                assert_eq!(session_id, 61);
            }
            other => panic!("expected status probe fallback, got {other:?}"),
        }
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
        assert_eq!(status_scope, (true, false));
        assert_eq!(started_scope, (true, true));
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
    fn sessionless_startup_error_targets_selected_codex_thread() {
        let records = Vec::<ProcessRecord>::new();
        let session_threads = RefCell::new(HashMap::<i64, i64>::new());

        let thread_id = codex_thread_id_for_startup_error(
            None,
            &session_threads,
            &records,
            SessionKind::Codex,
            Some(7),
        );

        assert_eq!(thread_id, Some(7));
    }

    #[test]
    fn startup_card_is_rendered_for_loading_state() {
        assert!(should_render_codex_startup_card(
            &CodexStartupState::Loading {
                message: "Starting Codex...".to_owned(),
            }
        ));
    }

    #[test]
    fn startup_card_is_not_rendered_for_ready_state() {
        assert!(!should_render_codex_startup_card(&CodexStartupState::Ready));
    }

    #[test]
    fn startup_error_state_requires_status_card() {
        assert!(should_render_codex_startup_card(
            &CodexStartupState::Error {
                message: "spawn failed".to_owned(),
            }
        ));
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
    fn codex_screen_ready_for_input_waits_for_boot_to_finish() {
        let booting = "\
╭────────────────────────────────────────────────────────╮
│ >_ OpenAI Codex (v0.142.3)                             │
╰────────────────────────────────────────────────────────╯

• Booting MCP server: codex_apps (0s • esc to interrupt)

› Implement {feature}
";
        let ready = "\
╭────────────────────────────────────────────────────────╮
│ >_ OpenAI Codex (v0.142.3)                             │
╰────────────────────────────────────────────────────────╯

› Implement {feature}
";

        assert!(!codex_screen_ready_for_input(booting));
        assert!(codex_screen_ready_for_input(ready));
    }

    #[test]
    fn composer_stays_ready_when_no_live_codex_session_exists() {
        assert!(composer_ready_for_input(false, false));
        assert!(!composer_ready_for_input(true, false));
        assert!(composer_ready_for_input(true, true));
    }
}
