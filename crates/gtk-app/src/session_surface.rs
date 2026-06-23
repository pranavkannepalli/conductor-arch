use gtk::prelude::*;
use gtk::{
    Align, Box as GBox, Button, CheckButton, ComboBoxText, Entry, EventControllerKey, Image, Label,
    Orientation, Overlay, Popover, ScrolledWindow, TextBuffer, TextView, ToggleButton, Widget,
};
use linux_conductor_core::pty::PtySession;
use linux_conductor_core::workspace::{
    ProcessRecord, ProcessStatus, SessionHarnessOptions, SessionKind, WorkspaceStore,
};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::{env, fs as stdfs};

use crate::buttons::{icon_button, style_icon_button, style_text_button, text_button};
use crate::state::AppState;
use crate::terminal::terminal_display_text;

const SESSION_SCROLLBACK_LINES: usize = 2_000;
const SESSION_TAIL_HISTORY: usize = 120;
const SESSION_POLL_INTERVAL_MS: u64 = 100;
const SESSION_RECONCILE_EVERY_TICKS: u32 = 10;

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
    _database_path: PathBuf,
    _workspace_name: &str,
    repository_name: &str,
    branch_name: &str,
    collapse_sidebar: Rc<dyn Fn()>,
    _app_state: AppState,
    _refresh: impl Fn() + Clone + 'static,
    include_header: bool,
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

    // ── Messages scroll area ──────────────────────────────────────────
    let messages = GBox::new(Orientation::Vertical, 0);
    messages.add_css_class("chat-messages");
    messages.set_vexpand(true);
    messages.set_hexpand(true);

    messages.append(&chat_user_bubble(
        "Walk me through how this workspace is set up.",
    ));

    let agent1 = Label::new(Some(
        "This workspace runs coding agents in parallel. Each agent gets its own branch, files, chat, terminal, preview, and reviewable diff.\n\n1. Send a task.\n2. Run the app (Ctrl R).\n3. Review the diff before keeping it.",
    ));
    agent1.add_css_class("chat-agent-text");
    agent1.set_wrap(true);
    agent1.set_xalign(0.0);
    agent1.set_hexpand(true);
    agent1.set_margin_bottom(28);
    messages.append(&agent1);

    messages.append(&chat_user_bubble("Add a 10-train milestone animation."));

    let agent2 = Label::new(Some(
        "Done. I updated the animation in this workspace and kept the preview running. The code is isolated from main until you review the diff.",
    ));
    agent2.add_css_class("chat-agent-text");
    agent2.set_wrap(true);
    agent2.set_xalign(0.0);
    agent2.set_hexpand(true);
    messages.append(&agent2);

    let scroll = ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_vexpand(true);
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
        &["Codex", "Claude", "Cursor", "Shell"],
        0,
    );
    let fast_btn = mode_icon_toggle_button("Fast", "weather-clear-symbolic", true);
    let thinking_btn = mode_menu_button("High", "◔", &["Low", "Medium", "High", "Extra high"], 2);
    thinking_btn.add_css_class("chat-thinking-menu");
    let goal_btn = mode_icon_toggle_button("Goal", "target-symbolic", false);
    let plan_btn = mode_icon_toggle_button("Plan", "map-symbolic", false);

    left_group.append(&interface_btn);
    left_group.append(&fast_btn);
    left_group.append(&thinking_btn);
    left_group.append(&goal_btn);
    left_group.append(&plan_btn);

    let right_group = GBox::new(Orientation::Horizontal, 8);
    right_group.set_halign(Align::End);
    right_group.set_hexpand(false);

    let context_btn = icon_text_button("◔", "chat-context-btn");
    context_btn.set_tooltip_text(Some("Context used"));
    let attach_btn = icon_button("mail-attachment-symbolic", "Attach file");
    attach_btn.add_css_class("chat-toolbar-btn");
    attach_btn.set_tooltip_text(Some("Attach file"));
    let issue_btn = icon_button("github-symbolic", "Link GitHub issue");
    issue_btn.add_css_class("chat-toolbar-btn");
    issue_btn.set_tooltip_text(Some("Link GitHub issue"));

    let send_btn = icon_button("send-symbolic", "Send message");
    send_btn.add_css_class("chat-send-btn");
    send_btn.set_tooltip_text(Some("Send message"));

    right_group.append(&context_btn);
    right_group.append(&attach_btn);
    right_group.append(&issue_btn);
    right_group.append(&send_btn);

    toolbar.append(&left_group);
    toolbar.append(&right_group);

    composer_box.append(&input_shell);
    composer_box.append(&toolbar);
    composer_wrap.append(&composer_box);
    root.append(&composer_wrap);

    let buffer = input_view.buffer();
    let buffer_for_update = buffer.clone();
    let update_composer_state = {
        let placeholder = placeholder.clone();
        let send_btn = send_btn.clone();
        let input_view = input_view.clone();
        Rc::new(move || {
            let start = buffer_for_update.start_iter();
            let end = buffer_for_update.end_iter();
            let text = buffer_for_update.text(&start, &end, true);
            let has_text = !text.as_str().trim().is_empty();
            placeholder.set_visible(!has_text);
            send_btn.set_sensitive(has_text);
            if has_text {
                send_btn.add_css_class("chat-send-btn-active");
            } else {
                send_btn.remove_css_class("chat-send-btn-active");
            }
            input_view.queue_draw();
        })
    };
    buffer.connect_changed({
        let update = update_composer_state.clone();
        move |_| update()
    });
    update_composer_state();

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

    let repo_icon = Image::from_icon_name("folder-symbolic");
    repo_icon.add_css_class("chat-repo-icon");
    breadcrumb.append(&repo_icon);

    let repo_label = Label::new(Some(repository_name));
    repo_label.add_css_class("chat-repo-label");
    repo_label.set_xalign(0.0);
    breadcrumb.append(&repo_label);

    let branch_sep = Label::new(Some(">"));
    branch_sep.add_css_class("chat-branch-separator");
    breadcrumb.append(&branch_sep);

    let branch_label = Label::new(Some(branch_name));
    branch_label.add_css_class("chat-branch-label");
    branch_label.set_xalign(0.0);
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
    bubble.set_wrap(true);
    bubble.set_xalign(0.0);
    bubble.set_hexpand(true);
    bubble.set_halign(gtk::Align::Fill);
    row.append(&bubble);
    row
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
    let icon = Image::from_icon_name(icon_name);
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
) -> Button {
    let selected_label = options.get(selected_index).copied().unwrap_or(label);
    let button = Button::new();
    button.add_css_class("chat-mode-menu");
    style_text_button(&button);
    button.set_tooltip_text(Some(selected_label));

    let shell = mode_menu_child(icon_name, selected_label);
    button.set_child(Some(&shell));
    let popover = mode_menu_popover(button.clone(), icon_name, options, selected_index);
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
        let icon = Image::from_icon_name(icon_name);
        icon.add_css_class("chat-mode-icon");
        icon.upcast()
    };
    let text = Label::new(Some(text_label));
    text.add_css_class("chat-mode-label");
    let arrow = Image::from_icon_name("pan-down-symbolic");
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
) -> Popover {
    let popover = Popover::new();
    popover.add_css_class("chat-menu-popover");
    let list = GBox::new(Orientation::Vertical, 4);
    list.add_css_class("chat-menu-list");

    for (index, option) in options.iter().enumerate() {
        let row = Button::new();
        row.add_css_class("chat-menu-item");
        let row_box = GBox::new(Orientation::Horizontal, 10);
        let icon = Image::from_icon_name("application-x-executable-symbolic");
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
        row.connect_clicked(move |_| {
            let shell = mode_menu_child(&icon_name, &option);
            button_for_row.set_child(Some(&shell));
            button_for_row.set_tooltip_text(Some(&option));
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
        .position(|choice| choice.name == "Cursor")
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
    let icon = Image::from_icon_name(choice.icon);
    icon.add_css_class("chat-editor-icon");
    let text = Label::new(Some(&choice.name));
    text.add_css_class("chat-editor-label");
    let arrow = Image::from_icon_name("pan-down-symbolic");
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
        let icon = Image::from_icon_name(choice.icon);
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
        ("Cursor", "cursor-symbolic", "cursor"),
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
            name: "Cursor".to_owned(),
            icon: "cursor-symbolic",
            command: "cursor".to_owned(),
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
    codex_personality.append(Some("careful"), "Personality: careful");
    codex_personality.append(Some("fast"), "Personality: fast");
    codex_personality.append(Some("thorough"), "Personality: thorough");
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
        ("Cursor", SessionKind::Cursor),
    ] {
        let button = text_button(label);
        match kind {
            SessionKind::Codex | SessionKind::Claude => button.add_css_class("suggested-action"),
            SessionKind::Cursor => button.add_css_class("secondary-action"),
            SessionKind::Shell => button.add_css_class("flat-action"),
        }
        let tooltip = match kind {
            SessionKind::Shell => "Open a PTY shell inside this workspace.",
            SessionKind::Codex => "Open a PTY Codex session inside this workspace.",
            SessionKind::Claude => "Open a PTY Claude session inside this workspace.",
            SessionKind::Cursor => "Open a PTY Cursor session inside this workspace.",
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
                        .set_text("No local sessions yet. Start Shell/Codex/Claude/Cursor above.");
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
    let refresh_send = refresh.clone();
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
                if let Err(err) = session.write(&(command.clone() + "\n")) {
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
            refresh_send();
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
                if let Some(bootstrap) = launch.env_value("CONDUCTOR_SESSION_BOOTSTRAP") {
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
        let mut should_sync = false;
        {
            let mut sessions = active_sessions_for_poll.borrow_mut();
            for (process_id, session) in sessions.iter_mut() {
                let output = session.read_available();
                if !output.is_empty() {
                    let _ = WorkspaceStore::open(db_for_poll.clone()).and_then(|store| {
                        store.append_session_process_output(*process_id, &output)
                    });
                    let is_selected = *selected_for_poll.borrow() == Some(*process_id);
                    if is_selected {
                        append_text(
                            &transcript_buffer_for_poll,
                            &live_session_append_text(&output),
                        );
                    }
                    last_output_for_poll
                        .borrow_mut()
                        .insert(*process_id, Instant::now());
                    if is_selected {
                        should_sync = true;
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
            if was_active {
                append_text(
                    &transcript_buffer_for_poll,
                    &live_session_append_text(&format!("[session finished] #{process_id}\n")),
                );
            }
            should_sync = true;
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
                    }
                    should_sync = true;
                }
            }
        }

        if should_sync {
            refresh_view_for_poll();
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
        "\n[user input {workspace_name}#{process_id}]\n{}\n",
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
    let mut index = 0usize;

    while index < lines.len() {
        let line = lines[index].trim_end();
        if line.trim().is_empty() {
            index += 1;
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

        if let Some(role) = session_event_role_for_marker(line) {
            push_session_event(&mut events, role, line.to_owned());
            index += 1;
            continue;
        }

        let (body, next) = collect_until_marker(&lines, index);
        push_session_event(&mut events, SessionTranscriptRole::Agent, body);
        index = next;
    }

    if events.len() > SESSION_TAIL_HISTORY {
        events.drain(0..events.len() - SESSION_TAIL_HISTORY);
    }
    events
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
    let mut index = start;
    while index < lines.len() && lines[index].trim().is_empty() {
        index += 1;
    }
    if index >= lines.len() || is_session_event_marker(lines[index].trim_end()) {
        return (String::new(), index);
    }
    collect_until_marker(lines, index)
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
        || line == "[staged review prompt]"
        || line == "[/staged review prompt]"
        || session_event_role_for_marker(line).is_some()
}

fn is_user_input_marker(line: &str) -> bool {
    line.starts_with("[user input ") && line.ends_with(']')
}

fn session_event_role_for_marker(line: &str) -> Option<SessionTranscriptRole> {
    if line.starts_with("[session ")
        || line.starts_with("[checkpoint")
        || line.starts_with("[could not ")
        || line.starts_with("[error")
    {
        return Some(SessionTranscriptRole::System);
    }
    if line.starts_with("[conductor bootstrap") || line.starts_with("[harness ") {
        return Some(SessionTranscriptRole::Harness);
    }
    if line.starts_with("[tool ") {
        return Some(SessionTranscriptRole::Tool);
    }
    if line.starts_with("[skill ") {
        return Some(SessionTranscriptRole::Skill);
    }
    None
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

fn provider_status_text(status: &linux_conductor_core::mcp::McpStatus) -> String {
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
    use linux_conductor_core::workspace::ProcessKind;
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
            kind: ProcessKind::Session,
            command: command.to_owned(),
            pid: 1234,
            log_path: PathBuf::from("/tmp/session.log"),
            status,
            started_at: "2026-06-20T12:00:00Z".to_owned(),
            exit_code: None,
            ended_at: None,
            session_harness_metadata: metadata.map(str::to_owned),
        }
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

        assert_eq!(text, "\n[user input memphis#9]\ncargo test\n");
    }

    #[test]
    fn live_session_append_renders_user_and_review_events_without_markers() {
        let user = live_session_append_text("[user input memphis#9]\ncargo test\n");
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
\n[conductor bootstrap for codex]
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
        assert!(surface.contains("Harness\n[conductor bootstrap for codex]"));
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
    fn transcript_events_are_limited_to_recent_history() {
        let transcript = (0..130)
            .map(|index| format!("[user input memphis#1]\ncommand {index}\n"))
            .collect::<String>();

        let events = parse_session_transcript_events(&transcript);

        assert_eq!(events.len(), SESSION_TAIL_HISTORY);
        assert_eq!(events.first().unwrap().body, "command 10");
        assert_eq!(events.last().unwrap().body, "command 129");
    }
}
