use adw::{Toast, ToastOverlay};
use gtk::prelude::*;
use gtk::{
    Box as GBox, Button, CheckButton, Entry, Label, Orientation, Paned, PolicyType, ScrolledWindow,
    Stack, StackSwitcher, TextView,
};
use linux_conductor_core::workspace::{Workspace, WorkspaceStore};
use std::path::Path;

use crate::refresh::{RefreshHub, RefreshScope};
use crate::state::{AppState, WorkspaceTab};
use crate::{
    cli_binary, detail_row, history, session_surface, shell_quote, spawn_terminal_command,
    terminal, title_case_workspace,
};

pub(crate) fn build_workspace_command_center(
    app_state: &AppState,
    refresh_hub: RefreshHub,
    toast_overlay: ToastOverlay,
) -> (GBox, impl Fn() + Clone + 'static) {
    let root = GBox::new(Orientation::Vertical, 0);
    root.add_css_class("dashboard");

    let header = GBox::new(Orientation::Vertical, 8);
    header.add_css_class("dashboard-header");
    let title = Label::new(Some("Workspace"));
    title.add_css_class("dashboard-title");
    title.set_xalign(0.0);
    let subtitle = Label::new(Some("Select a workspace from the sidebar."));
    subtitle.add_css_class("card-meta");
    subtitle.set_xalign(0.0);
    subtitle.set_wrap(true);
    header.append(&title);
    header.append(&subtitle);
    root.append(&header);

    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    scroll.set_vexpand(true);
    let body = GBox::new(Orientation::Vertical, 14);
    body.add_css_class("detail-body");
    scroll.set_child(Some(&body));
    root.append(&scroll);

    let db_path = app_state.workspace_database_path();
    let state = app_state.clone();
    let refresh = move || {
        while let Some(child) = body.first_child() {
            body.remove(&child);
        }

        let Some(name) = state.selected_workspace() else {
            title.set_text("Workspace");
            subtitle.set_text("Select a workspace from the sidebar.");
            return;
        };
        let Ok(store) = WorkspaceStore::open(db_path.clone()) else {
            title.set_text("Workspace");
            subtitle.set_text("Could not open workspace database.");
            return;
        };
        let Ok(Some(line)) = store
            .list_status()
            .map(|lines| lines.into_iter().find(|line| line.workspace.name == name))
        else {
            title.set_text("Workspace");
            subtitle.set_text("Workspace not found.");
            return;
        };

        let ws = line.workspace;
        let checks = store.checks_summary(&ws.name).ok();
        title.set_text(&title_case_workspace(&ws.name));
        subtitle.set_text(&format!(
            "{} / {} / base {} / {}",
            line.repository_name,
            ws.branch,
            ws.base_ref,
            ws.path.display()
        ));

        body.append(&workspace_status_strip(&ws, checks.as_ref()));

        let top_grid = GBox::new(Orientation::Horizontal, 14);
        top_grid.append(&agents_panel(&db_path, &ws, refresh_hub.clone()));
        top_grid.append(&runtime_panel(
            &db_path,
            &ws,
            &store,
            refresh_hub.clone(),
            toast_overlay.clone(),
        ));
        body.append(&top_grid);

        body.append(&lifecycle_panel(
            &db_path,
            &ws,
            &state,
            refresh_hub.clone(),
            toast_overlay.clone(),
        ));

        body.append(&work_tabs(
            &db_path,
            &store,
            &ws,
            &state,
            refresh_hub.clone(),
        ));
    };
    refresh();
    (root, refresh)
}

fn workspace_status_strip(
    ws: &Workspace,
    checks: Option<&linux_conductor_core::workspace::ChecksSummary>,
) -> GBox {
    let strip = GBox::new(Orientation::Horizontal, 10);
    strip.add_css_class("command-center-strip");
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

fn agents_panel(db_path: &Path, ws: &Workspace, refresh_hub: RefreshHub) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 10);
    panel.add_css_class("command-panel");
    panel.set_hexpand(true);
    panel.append(&section_title("Agents"));

    let actions = GBox::new(Orientation::Horizontal, 8);
    for (label, kind) in [
        ("Shell", "shell"),
        ("Codex", "codex"),
        ("Claude", "claude"),
        ("Cursor", "cursor"),
    ] {
        let button = Button::with_label(label);
        let workspace = ws.name.clone();
        button.connect_clicked(move |_| {
            spawn_terminal_command(&format!(
                "{} session open {} --kind {}",
                cli_binary().display(),
                shell_quote(&workspace),
                kind
            ));
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
        move || refresh_sessions.refresh(RefreshScope::Workspace),
    ));
    panel.append(&session_box);
    panel
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

    let actions = GBox::new(Orientation::Horizontal, 8);
    let setup_btn = Button::with_label("Setup");
    let run_btn = Button::with_label("Run");
    let stop_btn = Button::with_label("Stop");
    let spotlight_on_btn = Button::with_label("Spotlight On");
    let spotlight_sync_btn = Button::with_label("Spotlight Sync");
    let spotlight_repair_btn = Button::with_label("Repair Spotlight");
    let spotlight_off_btn = Button::with_label("Spotlight Off");
    let folder_btn = Button::with_label("Open Folder");
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
            Ok(record) => status_setup.set_text(&format!("Setup started: pid {}", record.pid)),
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
            Ok(record) => status_run.set_text(&format!("Run started: pid {}", record.pid)),
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
            Ok(record) => status_stop.set_text(&format!("Stopped pid {}", record.pid)),
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
            Ok(session) => status_spotlight_on
                .set_text(&format!("Spotlight active for {}", session.workspace_name)),
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
            Ok(session) => status_spotlight_sync
                .set_text(&format!("Spotlight synced for {}", session.workspace_name)),
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
            Ok(session) => status_spotlight_repair.set_text(&format!(
                "Spotlight root repaired for {}",
                session.workspace_name
            )),
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
            Ok(session) => status_spotlight_off
                .set_text(&format!("Spotlight stopped for {}", session.workspace_name)),
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

    actions.append(&setup_btn);
    actions.append(&run_btn);
    actions.append(&stop_btn);
    actions.append(&spotlight_on_btn);
    actions.append(&spotlight_sync_btn);
    actions.append(&spotlight_repair_btn);
    actions.append(&spotlight_off_btn);
    actions.append(&folder_btn);
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

    let row = GBox::new(Orientation::Horizontal, 8);
    let rename_entry = Entry::new();
    rename_entry.set_placeholder_text(Some("new workspace name"));
    rename_entry.set_text(&ws.name);
    let rename_btn = Button::with_label("Rename");
    let confirm = CheckButton::with_label("Confirm archive/discard");
    let archive_btn = Button::with_label("Archive");
    let restore_btn = Button::with_label("Restore");
    let discard_btn = Button::with_label("Discard");
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

    row.append(&rename_entry);
    row.append(&rename_btn);
    row.append(&confirm);
    row.append(&archive_btn);
    row.append(&restore_btn);
    row.append(&discard_btn);
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
) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 8);
    let tabs = Stack::new();
    tabs.set_vexpand(true);
    let switcher = StackSwitcher::new();
    switcher.set_stack(Some(&tabs));
    switcher.add_css_class("panel-switcher");
    panel.append(&switcher);

    tabs.add_titled(
        &changes_checks_review_tabs(store, &ws.name),
        Some("work"),
        "Changes",
    );
    tabs.add_titled(
        &chat_terminal_split(db_path, ws, refresh_hub.clone()),
        Some("chat-terminal"),
        "Chat / Terminal",
    );
    tabs.add_titled(
        &terminal::embedded_terminal_panel(
            db_path.to_path_buf(),
            &ws.name,
            &ws.path,
            true,
            refresh_hub.clone(),
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
        &text_panel(&workspace_processes_text(store, &ws.name)),
        Some("processes"),
        "Processes",
    );

    let state_tabs = state.clone();
    tabs.connect_visible_child_name_notify(move |stack| {
        match stack.visible_child_name().as_deref() {
            Some("work") => state_tabs.set_active_workspace_tab(WorkspaceTab::Changes),
            Some("todos") => state_tabs.set_active_workspace_tab(WorkspaceTab::Todos),
            Some("processes") => state_tabs.set_active_workspace_tab(WorkspaceTab::Processes),
            Some("terminal") => state_tabs.set_active_workspace_tab(WorkspaceTab::Terminal),
            Some("chat-terminal") => state_tabs.set_active_workspace_tab(WorkspaceTab::Chats),
            _ => state_tabs.set_active_workspace_tab(WorkspaceTab::Chats),
        }
    });
    panel.append(&tabs);
    panel
}

fn changes_checks_review_tabs(store: &WorkspaceStore, name: &str) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 8);
    let tabs = Stack::new();
    tabs.set_vexpand(true);
    let switcher = StackSwitcher::new();
    switcher.set_stack(Some(&tabs));
    switcher.add_css_class("panel-switcher");
    panel.append(&switcher);
    tabs.add_titled(
        &text_panel(&workspace_changes_text(store, name)),
        Some("changes"),
        "Changes",
    );
    tabs.add_titled(
        &text_panel(&workspace_checks_text(store, name)),
        Some("checks"),
        "Checks",
    );
    tabs.add_titled(
        &text_panel(&workspace_review_text(store, name)),
        Some("review"),
        "Review",
    );
    panel.append(&tabs);
    panel
}

fn chat_terminal_split(db_path: &Path, ws: &Workspace, refresh_hub: RefreshHub) -> Paned {
    let split = Paned::new(Orientation::Horizontal);
    split.set_wide_handle(true);
    split.set_position(520);

    let chat_box = GBox::new(Orientation::Vertical, 8);
    chat_box.add_css_class("command-panel");
    chat_box.append(&section_title("Chat"));
    let db_for_sessions = db_path.to_path_buf();
    let workspace_for_sessions = ws.name.clone();
    let refresh_sessions = refresh_hub.clone();
    chat_box.append(&session_surface::agent_session_panel(
        db_for_sessions,
        &workspace_for_sessions,
        move || refresh_sessions.refresh(RefreshScope::Workspace),
    ));
    for chat in history::conductor_sessions_for_workspace_path(&ws.path)
        .into_iter()
        .take(8)
    {
        chat_box.append(&history::session_summary_row(&chat));
    }

    let terminal_box = GBox::new(Orientation::Vertical, 8);
    terminal_box.add_css_class("command-panel");
    terminal_box.append(&section_title("Terminal"));
    terminal_box.append(&terminal::embedded_terminal_panel(
        db_path.to_path_buf(),
        &ws.name,
        &ws.path,
        false,
        refresh_hub,
    ));

    split.set_start_child(Some(&chat_box));
    split.set_end_child(Some(&terminal_box));
    split
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
    out.push_str("\n\nDiff\n");
    out.push_str(
        &store
            .unified_diff(name, None)
            .unwrap_or_else(|err| format!("Could not read diff: {err:#}\n")),
    );
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
            format!(
                "Changed files: {}\nRun: {}\nSessions: {}\nPR: {}\nTodos: {} open / {} total\nReview comments: {} open\nBranch: {}\nConflicts:\n{}",
                summary.changed_files,
                summary
                    .run_status
                    .map(|status| status.as_str().to_owned())
                    .unwrap_or_else(|| "none".to_owned()),
                summary.active_sessions,
                pr,
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

fn workspace_review_text(store: &WorkspaceStore, name: &str) -> String {
    match store.list_review_comments(name) {
        Ok(comments) if comments.is_empty() => "No review comments.".to_owned(),
        Ok(comments) => comments
            .into_iter()
            .map(|comment| {
                let line = comment
                    .line_number
                    .map(|line| format!(":{line}"))
                    .unwrap_or_default();
                format!(
                    "#{} [{}] {}{} - {}",
                    comment.id, comment.status, comment.file_path, line, comment.body
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Err(err) => format!("Could not read review comments: {err:#}"),
    }
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
    let entry_row = GBox::new(Orientation::Horizontal, 8);
    let entry = Entry::new();
    entry.set_placeholder_text(Some("Add todo..."));
    entry.set_hexpand(true);
    let add_btn = Button::with_label("Add Todo");
    let db_path = linux_conductor_core::paths::AppPaths::from_env().database_path;
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
        Ok(Some(session)) => format!(
            "{} since {} patch={}",
            session.status,
            session.started_at,
            session.patch_path.display()
        ),
        Ok(None) => "Inactive".to_owned(),
        Err(err) => format!("Could not read Spotlight status: {err:#}"),
    }
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
        let detail = spotlight_dirty_root_detail(err)
            .map(|detail| format!(" Affected paths: {detail}."))
            .unwrap_or_default();
        return RuntimeActionFeedback {
            status_text: format!(
                "{action} blocked: repository root has extra edits outside the active Spotlight patch.{detail} Use Repair Spotlight to discard root-only edits and reapply the active patch, or clean/save root changes manually."
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn lifecycle_action_failure_feedback_includes_status_and_toast() {
        let feedback = lifecycle_action_failure_feedback("Rename", &anyhow::anyhow!("bad name"));

        assert_eq!(feedback.status_text, "Rename failed: bad name");
        assert_eq!(
            feedback.toast_text.as_deref(),
            Some("Rename failed: bad name")
        );
    }
}
