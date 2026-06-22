use adw::{Toast, ToastOverlay};
use gtk::prelude::*;
use gtk::{
    Align, Box as GBox, Button, CheckButton, ComboBoxText, Entry, Label, Orientation, Paned,
    PolicyType, ScrolledWindow, Stack, StackSwitcher, TextView, WrapMode,
};
use linux_conductor_core::workspace::{
    DiffFileSummary, PullRequest, PullRequestReviewThread, ReviewComment, Workspace, WorkspaceStore,
};
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
    root.add_css_class("page-shell");

    let header = GBox::new(Orientation::Vertical, 8);
    header.add_css_class("dashboard-header");
    header.add_css_class("page-header");
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
    body.add_css_class("page-body");
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
        top_grid.append(&agents_panel(&db_path, &ws, &state, refresh_hub.clone()));
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
            toast_overlay.clone(),
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
    for (label, kind) in [
        ("Shell", "shell"),
        ("Codex", "codex"),
        ("Claude", "claude"),
        ("Cursor", "cursor"),
    ] {
        let button = Button::with_label(label);
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
        app_state.clone(),
        move || refresh_sessions.refresh(RefreshScope::Workspace),
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
    let cancel_btn = Button::with_label("Cancel");
    let launch_btn = Button::with_label("Launch");
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
    toast_overlay: ToastOverlay,
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
        &chat_terminal_split(
            db_path,
            ws,
            state.clone(),
            refresh_hub.clone(),
            terminal_preferences.clone(),
            terminal_command_presets.clone(),
        ),
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

    let create_row = GBox::new(Orientation::Horizontal, 8);
    let message = Entry::new();
    message.set_placeholder_text(Some("Checkpoint message"));
    message.set_hexpand(true);
    let create_btn = Button::with_label("Create");
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
        let row = GBox::new(Orientation::Horizontal, 8);
        let label = Label::new(Some(&format!(
            "#{} {} - {}",
            checkpoint.id, checkpoint.created_at, checkpoint.message
        )));
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.set_hexpand(true);
        let restore_btn = Button::with_label("Restore");
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
) -> Paned {
    let split = Paned::new(Orientation::Horizontal);
    split.set_wide_handle(true);
    split.set_position(520);

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
        app_state,
        move || refresh_sessions.refresh(RefreshScope::Workspace),
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
    let link_btn = Button::with_label("Link");
    let unlink_btn = Button::with_label("Unlink");
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

fn workspace_changes_panel(
    db_path: &Path,
    store: &WorkspaceStore,
    name: &str,
    refresh_hub: RefreshHub,
    toast_overlay: ToastOverlay,
) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 8);
    panel.append(&detail_row(
        "Branch",
        &workspace_branch_state_text(store, name),
    ));
    panel.append(&detail_row(
        "Status",
        &store
            .git_status_short(name)
            .unwrap_or_else(|err| format!("Could not read status: {err:#}")),
    ));

    let commits = TextView::new();
    commits.set_editable(false);
    commits.set_monospace(true);
    commits.add_css_class("history-view");
    commits.buffer().set_text(
        &store
            .git_log_oneline(name, 12)
            .unwrap_or_else(|err| format!("Could not read log: {err:#}\n")),
    );
    let commits_scroll = ScrolledWindow::new();
    commits_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    commits_scroll.set_min_content_height(120);
    commits_scroll.set_child(Some(&commits));
    panel.append(&section_title("Recent commits"));
    panel.append(&commits_scroll);

    let summary = store.diff_file_summaries(name).unwrap_or_default();
    let selected_file = std::rc::Rc::new(std::cell::RefCell::new(None::<String>));
    let diff_view = TextView::new();
    diff_view.set_editable(false);
    diff_view.set_monospace(true);
    diff_view.set_vexpand(true);
    diff_view
        .buffer()
        .set_text(&workspace_diff_text(store, name, None));
    let diff_scroll = ScrolledWindow::new();
    diff_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    diff_scroll.set_vexpand(true);
    diff_scroll.set_child(Some(&diff_view));

    let selection_status = Label::new(Some("Showing full workspace diff."));
    selection_status.add_css_class("card-meta");
    selection_status.set_xalign(0.0);
    selection_status.set_wrap(true);
    let feedback = Label::new(Some("No file action run yet."));
    feedback.add_css_class("card-meta");
    feedback.set_xalign(0.0);
    feedback.set_wrap(true);

    let action_row = GBox::new(Orientation::Horizontal, 8);
    let show_all_btn = Button::with_label("Show All");
    let revert_btn = Button::with_label("Revert Selected");
    action_row.append(&show_all_btn);
    action_row.append(&revert_btn);
    panel.append(&selection_status);
    panel.append(&action_row);

    let comment_row = GBox::new(Orientation::Horizontal, 8);
    let comment_line = Entry::new();
    comment_line.set_placeholder_text(Some("line"));
    let comment_body = Entry::new();
    comment_body.set_placeholder_text(Some("Add comment on selected file"));
    comment_body.set_hexpand(true);
    let comment_btn = Button::with_label("Comment");
    comment_row.append(&comment_line);
    comment_row.append(&comment_body);
    comment_row.append(&comment_btn);
    panel.append(&comment_row);

    let comments_view = TextView::new();
    comments_view.set_editable(false);
    comments_view.set_monospace(true);
    comments_view.set_vexpand(false);
    comments_view
        .buffer()
        .set_text("Select a changed file to view inline comments.");
    let comments_scroll = ScrolledWindow::new();
    comments_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    comments_scroll.set_min_content_height(96);
    comments_scroll.set_child(Some(&comments_view));
    panel.append(&section_title("Inline comments"));
    panel.append(&comments_scroll);

    let list_box = GBox::new(Orientation::Vertical, 6);
    if summary.is_empty() {
        list_box.append(&detail_row("Files", "No changed files."));
    } else {
        for row in diff_tree_rows(&summary) {
            match row {
                DiffTreeRow::Directory(label) => {
                    let directory = Label::new(Some(&label));
                    directory.add_css_class("detail-label");
                    directory.set_xalign(0.0);
                    list_box.append(&directory);
                }
                DiffTreeRow::File(file) => {
                    let row_box = GBox::new(Orientation::Horizontal, 8);
                    let open_btn = Button::with_label(&diff_tree_file_label(&file));
                    open_btn.set_hexpand(true);
                    let selected_file_for_open = selected_file.clone();
                    let db_for_open = db_path.to_path_buf();
                    let workspace_for_open = name.to_owned();
                    let diff_buffer = diff_view.buffer();
                    let comments_buffer = comments_view.buffer();
                    let selection_status_for_open = selection_status.clone();
                    let path_for_open = file.path.clone();
                    open_btn.connect_clicked(move |_| {
                        *selected_file_for_open.borrow_mut() = Some(path_for_open.clone());
                        diff_buffer.set_text(&workspace_diff_text_for_path(
                            &db_for_open,
                            &workspace_for_open,
                            Some(path_for_open.as_str()),
                        ));
                        comments_buffer.set_text(&workspace_file_comments_text(
                            &db_for_open,
                            &workspace_for_open,
                            &path_for_open,
                        ));
                        selection_status_for_open
                            .set_text(&format!("Showing diff for {}.", path_for_open));
                    });
                    row_box.append(&open_btn);
                    list_box.append(&row_box);
                }
            }
        }
    }
    let list_scroll = ScrolledWindow::new();
    list_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    list_scroll.set_min_content_width(260);
    list_scroll.set_child(Some(&list_box));

    let split = Paned::new(Orientation::Horizontal);
    split.set_start_child(Some(&list_scroll));
    split.set_end_child(Some(&diff_scroll));
    split.set_position(280);
    split.set_wide_handle(true);
    panel.append(&split);
    panel.append(&feedback);

    let diff_buffer_for_all = diff_view.buffer();
    let db_for_all = db_path.to_path_buf();
    let workspace_for_all = name.to_owned();
    let selected_file_for_all = selected_file.clone();
    let comments_buffer_for_all = comments_view.buffer();
    let selection_status_for_all = selection_status.clone();
    show_all_btn.connect_clicked(move |_| {
        *selected_file_for_all.borrow_mut() = None;
        diff_buffer_for_all.set_text(&workspace_diff_text_for_path(
            &db_for_all,
            &workspace_for_all,
            None,
        ));
        comments_buffer_for_all.set_text("Select a changed file to view inline comments.");
        selection_status_for_all.set_text("Showing full workspace diff.");
    });

    let db_for_revert = db_path.to_path_buf();
    let workspace_for_revert = name.to_owned();
    let selected_file_for_revert = selected_file.clone();
    let diff_buffer_for_revert = diff_view.buffer();
    let comments_buffer_for_revert = comments_view.buffer();
    let feedback_for_revert = feedback.clone();
    let toast_for_revert = toast_overlay.clone();
    let selection_status_for_revert = selection_status.clone();
    let refresh_after_revert = refresh_hub.clone();
    revert_btn.connect_clicked(move |_| {
        let Some(path) = selected_file_for_revert.borrow().clone() else {
            apply_action_feedback(
                &feedback_for_revert,
                &toast_for_revert,
                "Select one tracked file before reverting.",
                true,
            );
            return;
        };
        match WorkspaceStore::open(db_for_revert.clone())
            .and_then(|store| store.revert_workspace_file(&workspace_for_revert, &path))
        {
            Ok(()) => {
                *selected_file_for_revert.borrow_mut() = None;
                diff_buffer_for_revert.set_text(&workspace_diff_text_for_path(
                    &db_for_revert,
                    &workspace_for_revert,
                    None,
                ));
                comments_buffer_for_revert
                    .set_text("Select a changed file to view inline comments.");
                selection_status_for_revert.set_text("Showing full workspace diff.");
                apply_action_feedback(
                    &feedback_for_revert,
                    &toast_for_revert,
                    &format!("Reverted {} back to HEAD.", path),
                    true,
                );
                refresh_after_revert.refresh(RefreshScope::Workspace);
            }
            Err(err) => apply_action_feedback(
                &feedback_for_revert,
                &toast_for_revert,
                &format!("Could not revert {}: {err:#}", path),
                true,
            ),
        }
    });

    let db_for_comment = db_path.to_path_buf();
    let workspace_for_comment = name.to_owned();
    let selected_file_for_comment = selected_file.clone();
    let comment_line_for_add = comment_line.clone();
    let comment_body_for_add = comment_body.clone();
    let comments_buffer_for_comment = comments_view.buffer();
    let feedback_for_comment = feedback.clone();
    let toast_for_comment = toast_overlay.clone();
    let refresh_after_comment = refresh_hub.clone();
    comment_btn.connect_clicked(move |_| {
        let Some(path) = selected_file_for_comment.borrow().clone() else {
            apply_action_feedback(
                &feedback_for_comment,
                &toast_for_comment,
                "Select a file diff before adding a comment.",
                true,
            );
            return;
        };
        let body = comment_body_for_add.text().trim().to_owned();
        if body.is_empty() {
            apply_action_feedback(
                &feedback_for_comment,
                &toast_for_comment,
                "Comment text is required.",
                true,
            );
            return;
        }
        let line = match parse_review_comment_line(comment_line_for_add.text().as_ref()) {
            Ok(line) => line,
            Err(err) => {
                apply_action_feedback(&feedback_for_comment, &toast_for_comment, err, true);
                return;
            }
        };
        match WorkspaceStore::open(db_for_comment.clone())
            .and_then(|store| store.add_review_comment(&workspace_for_comment, &path, line, &body))
        {
            Ok(comment) => {
                comment_line_for_add.set_text("");
                comment_body_for_add.set_text("");
                comments_buffer_for_comment.set_text(&workspace_file_comments_text(
                    &db_for_comment,
                    &workspace_for_comment,
                    &path,
                ));
                apply_action_feedback(
                    &feedback_for_comment,
                    &toast_for_comment,
                    &format!("Added review comment #{} on {}.", comment.id, path),
                    true,
                );
                refresh_after_comment.refresh(RefreshScope::Workspace);
            }
            Err(err) => apply_action_feedback(
                &feedback_for_comment,
                &toast_for_comment,
                &format!("Could not add review comment: {err:#}"),
                true,
            ),
        }
    });

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

fn workspace_checks_panel(
    db_path: &Path,
    store: &WorkspaceStore,
    name: &str,
    app_state: AppState,
    refresh_hub: RefreshHub,
    toast_overlay: ToastOverlay,
) -> GBox {
    let panel = GBox::new(Orientation::Vertical, 8);
    panel.append(&text_panel(&workspace_checks_text(store, name)));

    let pr_form = GBox::new(Orientation::Horizontal, 8);
    let title_entry = Entry::new();
    title_entry.set_placeholder_text(Some("PR title (blank = gh --fill)"));
    title_entry.set_hexpand(true);
    let body_entry = Entry::new();
    body_entry.set_placeholder_text(Some("PR body"));
    body_entry.set_hexpand(true);
    let draft = CheckButton::with_label("Draft");
    let create_btn = Button::with_label("Create PR");
    let inspect_row = GBox::new(Orientation::Horizontal, 8);
    let refresh_pr_btn = Button::with_label("Refresh PR State");
    let view_summary_btn = Button::with_label("View PR Summary");
    let stage_summary_btn = Button::with_label("Stage PR Summary");
    let view_checks_btn = Button::with_label("View Checks");
    let stage_checks_btn = Button::with_label("Stage Failing Checks");
    let view_reviews_btn = Button::with_label("View PR Comments");
    let stage_reviews_btn = Button::with_label("Stage PR Comments");
    let thread_row = GBox::new(Orientation::Horizontal, 8);
    let thread_id_entry = Entry::new();
    thread_id_entry.set_placeholder_text(Some("Review thread ID from PR summary"));
    thread_id_entry.set_hexpand(true);
    let resolve_thread_btn = Button::with_label("Resolve Thread");
    let reopen_thread_btn = Button::with_label("Reopen Thread");
    let merge_row = GBox::new(Orientation::Horizontal, 8);
    let merge_method = ComboBoxText::new();
    merge_method.append(Some("squash"), "Squash");
    merge_method.append(Some("merge"), "Merge");
    merge_method.append(Some("rebase"), "Rebase");
    merge_method.set_active_id(Some("squash"));
    let merge_btn = Button::with_label("Merge PR");
    let archive_after_merge_btn = Button::with_label("Archive Workspace");
    let feedback = Label::new(None);
    feedback.add_css_class("card-meta");
    feedback.set_xalign(0.0);
    feedback.set_wrap(true);
    let checks_output = Label::new(None);
    checks_output.add_css_class("checks-view");
    checks_output.set_xalign(0.0);
    checks_output.set_wrap(true);
    checks_output.set_selectable(true);

    let db_for_create = db_path.to_path_buf();
    let workspace_for_create = name.to_owned();
    let refresh_after_create = refresh_hub.clone();
    let title_for_create = title_entry.clone();
    let body_for_create = body_entry.clone();
    let draft_for_create = draft.clone();
    let feedback_for_create = feedback.clone();
    let toast_for_create = toast_overlay.clone();
    create_btn.connect_clicked(move |_| {
        let title = optional_entry_text(&title_for_create);
        let body = optional_entry_text(&body_for_create);
        let result = WorkspaceStore::open(db_for_create.clone()).and_then(|store| {
            store.create_pull_request(
                &workspace_for_create,
                title.as_deref(),
                body.as_deref(),
                draft_for_create.is_active(),
            )
        });
        apply_action_feedback(
            &feedback_for_create,
            &toast_for_create,
            &pull_request_create_feedback(result),
            true,
        );
        refresh_after_create.refresh(RefreshScope::All);
    });

    let db_for_refresh = db_path.to_path_buf();
    let workspace_for_refresh = name.to_owned();
    let refresh_after_pr_refresh = refresh_hub.clone();
    let feedback_for_refresh = feedback.clone();
    let toast_for_refresh = toast_overlay.clone();
    refresh_pr_btn.connect_clicked(move |_| {
        let result = WorkspaceStore::open(db_for_refresh.clone())
            .and_then(|store| store.refresh_pull_request_state(&workspace_for_refresh));
        apply_action_feedback(
            &feedback_for_refresh,
            &toast_for_refresh,
            &pull_request_refresh_feedback(result),
            true,
        );
        refresh_after_pr_refresh.refresh(RefreshScope::All);
    });

    let db_for_summary = db_path.to_path_buf();
    let workspace_for_summary = name.to_owned();
    let checks_output_for_summary = checks_output.clone();
    let feedback_for_summary = feedback.clone();
    let toast_for_summary = toast_overlay.clone();
    view_summary_btn.connect_clicked(move |_| {
        let result = WorkspaceStore::open(db_for_summary.clone())
            .and_then(|store| store.pull_request_readiness_text(&workspace_for_summary));
        let text = pull_request_readiness_feedback(result);
        apply_action_feedback(&feedback_for_summary, &toast_for_summary, &text, true);
        checks_output_for_summary.set_text(&text);
    });

    let db_for_stage_summary = db_path.to_path_buf();
    let workspace_for_stage_summary = name.to_owned();
    let feedback_for_stage_summary = feedback.clone();
    let toast_for_stage_summary = toast_overlay.clone();
    let app_state_for_stage_summary = app_state.clone();
    stage_summary_btn.connect_clicked(move |_| {
        match WorkspaceStore::open(db_for_stage_summary.clone()).and_then(|store| {
            store.pull_request_readiness_agent_prompt(&workspace_for_stage_summary)
        }) {
            Ok(prompt) => {
                app_state_for_stage_summary.set_staged_review_prompt(Some(prompt));
                apply_action_feedback(
                    &feedback_for_stage_summary,
                    &toast_for_stage_summary,
                    "Staged PR readiness summary for the selected agent session.",
                    true,
                );
            }
            Err(err) => apply_action_feedback(
                &feedback_for_stage_summary,
                &toast_for_stage_summary,
                &format!("Could not stage PR readiness summary: {err:#}"),
                true,
            ),
        }
    });

    let db_for_checks = db_path.to_path_buf();
    let workspace_for_checks = name.to_owned();
    let checks_output_for_checks = checks_output.clone();
    let feedback_for_checks = feedback.clone();
    let toast_for_checks = toast_overlay.clone();
    view_checks_btn.connect_clicked(move |_| {
        let result = WorkspaceStore::open(db_for_checks.clone())
            .and_then(|store| store.pull_request_checks(&workspace_for_checks));
        let text = pull_request_checks_feedback(result);
        apply_action_feedback(&feedback_for_checks, &toast_for_checks, &text, true);
        checks_output_for_checks.set_text(&text);
    });

    let db_for_stage_checks = db_path.to_path_buf();
    let workspace_for_stage_checks = name.to_owned();
    let feedback_for_stage_checks = feedback.clone();
    let toast_for_stage_checks = toast_overlay.clone();
    let app_state_for_stage_checks = app_state.clone();
    stage_checks_btn.connect_clicked(move |_| {
        match WorkspaceStore::open(db_for_stage_checks.clone()).and_then(|store| {
            let prompt = store.pull_request_checks_agent_prompt(&workspace_for_stage_checks)?;
            if prompt.contains("No failing PR checks.") {
                anyhow::bail!("No failing PR checks to stage");
            }
            Ok(prompt)
        }) {
            Ok(prompt) => {
                app_state_for_stage_checks.set_staged_review_prompt(Some(prompt));
                apply_action_feedback(
                    &feedback_for_stage_checks,
                    &toast_for_stage_checks,
                    "Staged failing PR checks for the selected agent session.",
                    true,
                );
            }
            Err(err) => apply_action_feedback(
                &feedback_for_stage_checks,
                &toast_for_stage_checks,
                &format!("Could not stage failing checks: {err:#}"),
                true,
            ),
        }
    });

    let db_for_reviews = db_path.to_path_buf();
    let workspace_for_reviews = name.to_owned();
    let checks_output_for_reviews = checks_output.clone();
    let feedback_for_reviews = feedback.clone();
    let toast_for_reviews = toast_overlay.clone();
    view_reviews_btn.connect_clicked(move |_| {
        let result = WorkspaceStore::open(db_for_reviews.clone())
            .and_then(|store| store.pull_request_review_state(&workspace_for_reviews));
        let text = pull_request_review_feedback(result);
        apply_action_feedback(&feedback_for_reviews, &toast_for_reviews, &text, true);
        checks_output_for_reviews.set_text(&text);
    });

    let db_for_stage_reviews = db_path.to_path_buf();
    let workspace_for_stage_reviews = name.to_owned();
    let feedback_for_stage_reviews = feedback.clone();
    let toast_for_stage_reviews = toast_overlay.clone();
    let app_state_for_stage_reviews = app_state.clone();
    stage_reviews_btn.connect_clicked(move |_| {
        match WorkspaceStore::open(db_for_stage_reviews.clone()).and_then(|store| {
            let prompt = store.pull_request_review_agent_prompt(&workspace_for_stage_reviews)?;
            if prompt.contains("No GitHub PR review/comment output.") {
                anyhow::bail!("No GitHub PR comments or reviews to stage");
            }
            Ok(prompt)
        }) {
            Ok(prompt) => {
                app_state_for_stage_reviews.set_staged_review_prompt(Some(prompt));
                apply_action_feedback(
                    &feedback_for_stage_reviews,
                    &toast_for_stage_reviews,
                    "Staged GitHub PR comments/reviews for the selected agent session.",
                    true,
                );
            }
            Err(err) => apply_action_feedback(
                &feedback_for_stage_reviews,
                &toast_for_stage_reviews,
                &format!("Could not stage PR comments/reviews: {err:#}"),
                true,
            ),
        }
    });

    let db_for_resolve_thread = db_path.to_path_buf();
    let workspace_for_resolve_thread = name.to_owned();
    let thread_id_for_resolve = thread_id_entry.clone();
    let feedback_for_resolve_thread = feedback.clone();
    let toast_for_resolve_thread = toast_overlay.clone();
    resolve_thread_btn.connect_clicked(move |_| {
        let thread_id = thread_id_for_resolve.text().trim().to_owned();
        let result = if thread_id.is_empty() {
            Err(anyhow::anyhow!("review thread id is required"))
        } else {
            WorkspaceStore::open(db_for_resolve_thread.clone()).and_then(|store| {
                store.set_pull_request_review_thread_resolution(
                    &workspace_for_resolve_thread,
                    &thread_id,
                    true,
                )
            })
        };
        apply_action_feedback(
            &feedback_for_resolve_thread,
            &toast_for_resolve_thread,
            &pull_request_review_thread_action_feedback("Resolve", result),
            true,
        );
    });

    let db_for_reopen_thread = db_path.to_path_buf();
    let workspace_for_reopen_thread = name.to_owned();
    let thread_id_for_reopen = thread_id_entry.clone();
    let feedback_for_reopen_thread = feedback.clone();
    let toast_for_reopen_thread = toast_overlay.clone();
    reopen_thread_btn.connect_clicked(move |_| {
        let thread_id = thread_id_for_reopen.text().trim().to_owned();
        let result = if thread_id.is_empty() {
            Err(anyhow::anyhow!("review thread id is required"))
        } else {
            WorkspaceStore::open(db_for_reopen_thread.clone()).and_then(|store| {
                store.set_pull_request_review_thread_resolution(
                    &workspace_for_reopen_thread,
                    &thread_id,
                    false,
                )
            })
        };
        apply_action_feedback(
            &feedback_for_reopen_thread,
            &toast_for_reopen_thread,
            &pull_request_review_thread_action_feedback("Reopen", result),
            true,
        );
    });

    let db_for_merge = db_path.to_path_buf();
    let workspace_for_merge = name.to_owned();
    let refresh_after_merge = refresh_hub.clone();
    let merge_method_for_merge = merge_method.clone();
    let feedback_for_merge = feedback.clone();
    let toast_for_merge = toast_overlay.clone();
    merge_btn.connect_clicked(move |_| {
        let method = merge_method_for_merge
            .active_id()
            .map(|method| method.to_string())
            .unwrap_or_else(|| "squash".to_owned());
        let result = WorkspaceStore::open(db_for_merge.clone()).and_then(|store| {
            store.merge_and_maybe_archive_pull_request(&workspace_for_merge, Some(&method))
        });
        apply_action_feedback(
            &feedback_for_merge,
            &toast_for_merge,
            &pull_request_merge_and_archive_feedback(result),
            true,
        );
        refresh_after_merge.refresh(RefreshScope::All);
    });

    let db_for_archive = db_path.to_path_buf();
    let workspace_for_archive = name.to_owned();
    let refresh_after_archive = refresh_hub.clone();
    let feedback_for_archive = feedback.clone();
    let toast_for_archive = toast_overlay.clone();
    archive_after_merge_btn.connect_clicked(move |_| {
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

    pr_form.append(&title_entry);
    pr_form.append(&body_entry);
    pr_form.append(&draft);
    pr_form.append(&create_btn);
    panel.append(&pr_form);
    inspect_row.append(&refresh_pr_btn);
    inspect_row.append(&view_summary_btn);
    inspect_row.append(&stage_summary_btn);
    inspect_row.append(&view_checks_btn);
    inspect_row.append(&stage_checks_btn);
    inspect_row.append(&view_reviews_btn);
    inspect_row.append(&stage_reviews_btn);
    panel.append(&inspect_row);
    thread_row.append(&thread_id_entry);
    thread_row.append(&resolve_thread_btn);
    thread_row.append(&reopen_thread_btn);
    panel.append(&thread_row);
    merge_row.append(&merge_method);
    merge_row.append(&merge_btn);
    merge_row.append(&archive_after_merge_btn);
    panel.append(&merge_row);
    panel.append(&feedback);
    panel.append(&checks_output);
    panel.append(&workspace_conflict_resolution_panel(
        db_path,
        store,
        name,
        app_state,
        refresh_hub,
    ));
    panel
}

fn optional_entry_text(entry: &Entry) -> Option<String> {
    let value = entry.text().trim().to_owned();
    (!value.is_empty()).then_some(value)
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

        let open_workspace_btn = Button::with_label("Open workspace");
        let app_state_for_open = app_state.clone();
        let refresh_for_open = refresh_hub.clone();
        let conflict_workspace_for_open = conflict_workspace.clone();
        open_workspace_btn.connect_clicked(move |_| {
            app_state_for_open.set_selected_workspace(Some(conflict_workspace_for_open.clone()));
            refresh_for_open.refresh(RefreshScope::All);
        });

        let action_row = GBox::new(Orientation::Horizontal, 8);
        action_row.append(&title);
        action_row.append(&open_workspace_btn);
        let diff_all_btn = Button::with_label("View all diffs");
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
        let copy_all_btn = Button::with_label("Copy all from sibling");
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
            let file_row = GBox::new(Orientation::Horizontal, 8);
            let file_label = Label::new(Some(&file));
            file_label.set_xalign(0.0);
            file_label.set_wrap(true);
            file_label.set_hexpand(true);

            let diff_btn = Button::with_label("View diff");
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

            let copy_btn = Button::with_label("Copy from sibling");
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
    result: anyhow::Result<linux_conductor_core::workspace::MergePullRequestResult>,
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
    let form = GBox::new(Orientation::Horizontal, 8);
    let file_entry = Entry::new();
    file_entry.set_placeholder_text(Some("file path"));
    file_entry.set_hexpand(true);
    let line_entry = Entry::new();
    line_entry.set_placeholder_text(Some("line"));
    let body_entry = Entry::new();
    body_entry.set_placeholder_text(Some("comment"));
    body_entry.set_hexpand(true);
    let add_btn = Button::with_label("Add Comment");
    let feedback = Label::new(None);
    feedback.add_css_class("card-meta");
    feedback.set_xalign(0.0);
    feedback.set_wrap(true);
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

    let stage_btn = Button::with_label("Stage Open Comments");
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
        Ok(comments) if comments.is_empty() => panel.append(&detail_row("Review", "No comments")),
        Ok(comments) => {
            for comment in comments {
                let row = GBox::new(Orientation::Horizontal, 8);
                let summary = Label::new(Some(&review_comment_row_summary(&comment)));
                summary.set_xalign(0.0);
                summary.set_wrap(true);
                summary.set_hexpand(true);
                row.append(&summary);
                if review_comment_can_resolve(&comment) {
                    let button = Button::with_label("Resolve");
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
            "Review",
            &format!("Could not read review comments: {err:#}"),
        )),
    }
    panel
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
            linux_conductor_core::workspace::DiffFileSummary {
                path: "README.md".to_owned(),
                additions: Some(2),
                deletions: Some(1),
            },
            linux_conductor_core::workspace::DiffFileSummary {
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
        let comment = linux_conductor_core::workspace::ReviewComment {
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
            linux_conductor_core::workspace::ReviewComment {
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
            linux_conductor_core::workspace::ReviewComment {
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
            linux_conductor_core::workspace::DiffFileSummary {
                path: "src/lib.rs".to_owned(),
                additions: Some(1),
                deletions: Some(0),
            },
            linux_conductor_core::workspace::DiffFileSummary {
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
            linux_conductor_core::workspace::MergePullRequestResult {
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
            linux_conductor_core::workspace::MergePullRequestResult {
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
        let success =
            pull_request_refresh_feedback(Ok(Some(linux_conductor_core::workspace::PullRequest {
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
    fn pull_request_review_thread_action_feedback_reports_state_and_id() {
        let success = pull_request_review_thread_action_feedback(
            "Resolve",
            Ok(linux_conductor_core::workspace::PullRequestReviewThread {
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
