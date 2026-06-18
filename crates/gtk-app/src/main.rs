#![allow(dead_code)]
#![allow(clippy::ptr_arg, clippy::too_many_arguments)]

mod dashboard;
mod history;
mod projects;
mod refresh;
mod session_surface;
mod sidebar;
mod state;
mod terminal;
mod workspace_command_center;

use adw::prelude::*;
use adw::{Application, ApplicationWindow, HeaderBar};
use gtk::{
    Box as GBox, Button, CssProvider, Label, Orientation, Stack,
    STYLE_PROVIDER_PRIORITY_APPLICATION,
};
use linux_conductor_core::paths::AppPaths;
use linux_conductor_core::workspace::WorkspaceStore;
use refresh::{RefreshHub, RefreshScope};
use state::{AppPage, AppState};
use std::path::PathBuf;

const APP_ID: &str = "io.github.pranavkannepalli.linux-conductor";

fn main() {
    // Parse --workspace <name> before GTK takes over argv
    let initial_workspace: Option<String> = {
        let args: Vec<String> = std::env::args().collect();
        args.windows(2)
            .find(|w| w[0] == "--workspace")
            .map(|w| w[1].clone())
    };

    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(move |app| build_ui(app, initial_workspace.clone()));
    app.run();
}

fn build_ui(app: &Application, initial_workspace: Option<String>) {
    let paths = AppPaths::from_env();
    let app_state = AppState::new(paths.clone(), initial_workspace);
    let refresh_hub = RefreshHub::default();

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Linux Conductor")
        .default_width(1280)
        .default_height(800)
        .build();

    let css = CssProvider::new();
    css.load_from_data(APP_CSS);
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().unwrap(),
        &css,
        STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let split = adw::OverlaySplitView::new();
    split.set_min_sidebar_width(220.0);
    split.set_max_sidebar_width(280.0);
    split.set_show_sidebar(true);

    let toast_overlay = adw::ToastOverlay::new();
    let (dashboard, refresh_dashboard) = dashboard::build_dashboard_panel(&app_state.paths);
    dashboard.set_hexpand(true);
    dashboard.set_vexpand(true);

    let (workspace_detail, refresh_workspace_detail) =
        workspace_command_center::build_workspace_command_center(&app_state, refresh_hub.clone());
    let (projects_page, refresh_projects) = projects::build_projects_page(
        &app_state.paths,
        refresh_dashboard.clone(),
        refresh_workspace_detail.clone(),
    );
    let (history_page, refresh_history) = history::build_history_page();

    let main_stack = Stack::new();
    main_stack.set_hexpand(true);
    main_stack.set_vexpand(true);
    main_stack.add_named(&dashboard, Some("dashboard"));
    main_stack.add_named(&projects_page, Some("projects"));
    main_stack.add_named(&history_page, Some("history"));
    main_stack.add_named(&workspace_detail, Some("workspace"));
    main_stack.set_visible_child_name(match app_state.snapshot().active_page {
        AppPage::Workspace => "workspace",
        _ => "dashboard",
    });

    let (sidebar, refresh_sidebar) = sidebar::build_app_sidebar(
        &app_state,
        refresh_hub.clone(),
        main_stack.clone(),
        refresh_workspace_detail.clone(),
    );

    refresh_hub.set_dashboard(refresh_dashboard.clone());
    refresh_hub.set_workspace(refresh_workspace_detail.clone());
    refresh_hub.set_projects(refresh_projects.clone());
    refresh_hub.set_history(refresh_history.clone());
    refresh_hub.set_sidebar(refresh_sidebar.clone());

    split.set_sidebar(Some(&sidebar));
    toast_overlay.set_child(Some(&main_stack));
    split.set_content(Some(&toast_overlay));

    // Header bar
    let header = HeaderBar::new();
    header.set_show_end_title_buttons(true);
    let toggle_btn = Button::from_icon_name("sidebar-show-symbolic");
    toggle_btn.set_tooltip_text(Some("Toggle sidebar"));
    let split_clone = split.clone();
    toggle_btn.connect_clicked(move |_| {
        split_clone.set_show_sidebar(!split_clone.shows_sidebar());
    });
    // Refresh button
    let refresh_btn = Button::from_icon_name("view-refresh-symbolic");
    refresh_btn.set_tooltip_text(Some("Refresh workspace state"));
    let hub_refresh = refresh_hub.clone();
    refresh_btn.connect_clicked(move |_| {
        hub_refresh.refresh(RefreshScope::All);
    });
    header.pack_start(&toggle_btn);
    header.pack_end(&refresh_btn);

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&split));

    window.set_content(Some(&toolbar_view));
    window.present();

    // Keyboard shortcut: Ctrl+R → refresh all panels
    let evk = gtk::EventControllerKey::new();
    let hub_kb = refresh_hub.clone();
    evk.connect_key_pressed(move |_, keyval, _, modifiers| {
        if modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK) && keyval == gtk::gdk::Key::r {
            hub_kb.refresh(RefreshScope::All);
            return gtk::glib::Propagation::Stop;
        }
        gtk::glib::Propagation::Proceed
    });
    window.add_controller(evk);

    // Auto-refresh panels every 5 seconds
    let hub_auto = refresh_hub.clone();
    glib::timeout_add_seconds_local(5, move || {
        hub_auto.refresh(RefreshScope::Sidebar);
        hub_auto.refresh(RefreshScope::Dashboard);
        hub_auto.refresh(RefreshScope::Projects);
        hub_auto.refresh(RefreshScope::Workspace);
        glib::ControlFlow::Continue
    });

    let db_path_runtime_auto = app_state.workspace_database_path();
    let hub_runtime_auto = refresh_hub.clone();
    glib::timeout_add_seconds_local(5, move || {
        if let Ok((synced, reconciled)) = WorkspaceStore::open(db_path_runtime_auto.clone())
            .and_then(|store| {
                let synced = store.spotlight_sync_active_sessions()?;
                let reconciled = store.reconcile_terminal_processes()?;
                Ok::<_, anyhow::Error>((synced, reconciled))
            })
        {
            if !synced.is_empty() || !reconciled.is_empty() {
                hub_runtime_auto.refresh(RefreshScope::All);
            }
        }
        glib::ControlFlow::Continue
    });
}

pub(crate) fn title_case_workspace(name: &str) -> String {
    name.split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn detail_row(label: &str, value: &str) -> GBox {
    let row = GBox::new(Orientation::Horizontal, 12);
    row.add_css_class("detail-row");
    let label_widget = Label::new(Some(label));
    label_widget.add_css_class("detail-label");
    label_widget.set_xalign(0.0);
    label_widget.set_width_chars(18);
    let value_widget = Label::new(Some(value));
    value_widget.add_css_class("detail-value");
    value_widget.set_xalign(0.0);
    value_widget.set_wrap(true);
    value_widget.set_hexpand(true);
    row.append(&label_widget);
    row.append(&value_widget);
    row
}

pub(crate) fn cli_binary() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join("linux-conductor")))
        .filter(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from("linux-conductor"))
}

pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(crate) fn default_clone_parent() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("conductor")
        .join("repos")
}

pub(crate) fn repo_name_from_url(url: &str) -> String {
    url.trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("repository")
        .trim_end_matches(".git")
        .to_owned()
}

pub(crate) fn spawn_terminal_command(cmd: &str) {
    let full_cmd = format!("{cmd}; echo; echo '--- Press Enter to close ---'; read");

    #[cfg(target_os = "macos")]
    {
        let escaped = full_cmd.replace('\\', "\\\\").replace('"', "\\\"");
        if std::process::Command::new("osascript")
            .arg("-e")
            .arg(format!(
                "tell application \"Terminal\" to do script \"{}\"",
                escaped
            ))
            .arg("-e")
            .arg("tell application \"Terminal\" to activate")
            .spawn()
            .is_ok()
        {
            return;
        }
    }

    // Respect $TERMINAL env var if set
    if let Ok(term) = std::env::var("TERMINAL") {
        if std::process::Command::new(&term)
            .args(["-e", "bash", "-c", &full_cmd])
            .spawn()
            .is_ok()
        {
            return;
        }
    }

    let terminals: &[(&str, &[&str])] = &[
        ("gnome-terminal", &["--", "bash", "-c"]),
        ("xterm", &["-e", "bash", "-c"]),
        ("konsole", &["-e", "bash", "-c"]),
        ("xfce4-terminal", &["-e", "bash", "-c"]),
        ("tilix", &["-e", "bash", "-c"]),
        ("terminator", &["-e", "bash", "-c"]),
        ("alacritty", &["-e", "bash", "-c"]),
        ("kitty", &["bash", "-c"]),
        ("foot", &["bash", "-c"]),
        ("wezterm", &["start", "--", "bash", "-c"]),
        ("xterm", &["-e", "bash", "-c"]),
    ];

    for (term, prefix_args) in terminals {
        let mut command = std::process::Command::new(term);
        for arg in *prefix_args {
            command.arg(arg);
        }
        command.arg(&full_cmd);
        if command.spawn().is_ok() {
            return;
        }
    }

    eprintln!("No terminal emulator found. Run manually:\n  {cmd}");
}

// ── STYLES ────────────────────────────────────────────────────────────────

const APP_CSS: &str = r#"
window {
    background-color: #141111;
    color: #f5f0ed;
}

.sidebar {
    background-color: #171313;
    border-right: 1px solid #302b2b;
    padding-top: 12px;
}

.sidebar-header {
    font-size: 10px;
    font-weight: bold;
    color: #8b8582;
    text-transform: uppercase;
    letter-spacing: 1px;
}

.nav-row, .nav-row-active, .nav-button, .nav-button-active {
    font-size: 15px;
    padding: 10px 14px;
    margin: 2px 10px;
    border-radius: 8px;
    background: transparent;
    border: none;
    box-shadow: none;
    text-shadow: none;
}

.nav-row, .nav-button {
    color: #b8b0ac;
}

.nav-row-active, .nav-button-active, .nav-button:hover {
    color: #f5f0ed;
    background-color: #292523;
}

.projects-header {
    padding: 16px 14px 8px 14px;
}

.workspace-list {
    background-color: transparent;
}

.workspace-list row {
    border-radius: 8px;
    margin: 2px 10px;
    padding: 0;
}

.workspace-list row:selected {
    background-color: #292523;
}

.workspace-list row:hover {
    background-color: #211d1b;
}

.workspace-name {
    font-size: 14px;
    font-weight: 600;
    color: #f5f0ed;
}

.workspace-meta {
    font-size: 11px;
    color: #8b8582;
    font-family: monospace;
}

.workspace-status {
    font-size: 10px;
    color: #a6e3a1;
}

.workspace-status-archived {
    font-size: 10px;
    color: #45475a;
    font-style: italic;
}

.archive-banner {
    background-color: #2a2830;
    color: #f9e2af;
    font-size: 12px;
    border-left: 3px solid #f9e2af;
    padding: 6px 10px;
    border-radius: 4px;
}

.quick-stats {
    padding: 4px 0;
}

.stat-running {
    color: #a6e3a1;
    font-size: 12px;
    font-weight: bold;
}

.stat-stopped {
    color: #6c7086;
    font-size: 12px;
}

.stat-dim {
    color: #6c7086;
    font-size: 12px;
}

.workspace-title {
    font-size: 14px;
    font-weight: bold;
    color: #89b4fa;
}

.add-workspace-btn {
    background-color: transparent;
    color: #89b4fa;
    border: 1px solid #313244;
    border-radius: 6px;
    font-size: 12px;
}

.add-workspace-btn:hover {
    background-color: #313244;
}

.center-panel {
    background-color: #181825;
}

.dashboard {
    background-color: #141111;
}

.dashboard-header {
    padding: 20px 26px 0 26px;
    border-bottom: 1px solid #302b2b;
}

.dashboard-title {
    font-size: 17px;
    font-weight: 700;
    color: #f5f0ed;
}

.project-tabs {
    padding-bottom: 10px;
}

.project-tab, .project-tab-active {
    font-size: 13px;
    font-weight: 600;
}

.project-tab {
    color: #8b8582;
}

.project-tab-active {
    color: #f5f0ed;
    border-bottom: 2px solid #d7bfb4;
    padding-bottom: 10px;
}

.kanban-board {
    padding: 28px 28px;
}

.kanban-column {
    min-width: 235px;
}

.column-icon {
    color: #f5d90a;
    font-size: 14px;
}

.column-title {
    color: #f5f0ed;
    font-size: 15px;
    font-weight: 700;
}

.column-count {
    color: #8b8582;
    font-size: 13px;
}

.column-empty {
    color: #6f6966;
    font-size: 12px;
    padding: 18px 0;
}

.workspace-card {
    background-color: #292523;
    border: 1px solid #49413d;
    border-radius: 8px;
    padding: 12px;
    min-height: 116px;
}

.card-branch {
    color: #9d9691;
    font-size: 12px;
}

.card-diff {
    color: #8b8582;
    font-size: 12px;
}

.card-diff-hot {
    color: #4ade80;
    font-size: 12px;
    font-weight: 700;
}

.card-title {
    color: #f5f0ed;
    font-size: 15px;
    font-weight: 700;
}

.card-meta, .card-activity {
    color: #9d9691;
    font-size: 12px;
}

.card-activity {
    color: #d7bfb4;
}

.project-row {
    padding: 8px 10px;
}

.project-icon, .project-icon-hot {
    font-size: 15px;
}

.project-icon {
    color: #6f6966;
}

.project-icon-hot {
    color: #f5d90a;
}

.detail-body {
    padding: 24px 28px;
}

.command-center-strip {
    padding: 0;
}

.command-panel, .metric-card {
    background-color: #211d1b;
    border: 1px solid #3a3330;
    border-radius: 8px;
    padding: 12px;
}

.metric-value {
    color: #f5f0ed;
    font-size: 16px;
    font-weight: 700;
}

.detail-row {
    background-color: #211d1b;
    border: 1px solid #3a3330;
    border-radius: 8px;
    padding: 10px 12px;
}

.detail-label {
    color: #8b8582;
    font-size: 12px;
    font-weight: 700;
}

.detail-value {
    color: #f5f0ed;
    font-size: 13px;
}

.history-row {
    padding: 10px 12px;
}

.history-view {
    background-color: #141111;
    color: #f5f0ed;
    font-size: 13px;
    padding: 18px;
}

.workspace-toolbar {
    background-color: #1e1e2e;
    padding: 8px 12px;
    border-bottom: 1px solid #313244;
}

.right-panel {
    background-color: #181825;
}

.panel-switcher {
    background-color: transparent;
}

.panel-switcher button {
    background-color: transparent;
    color: #6c7086;
    border: none;
    border-radius: 6px;
    padding: 4px 10px;
    font-size: 12px;
    font-weight: 500;
}

.panel-switcher button:hover {
    background-color: #313244;
    color: #cdd6f4;
}

.panel-switcher button:checked {
    background-color: #313244;
    color: #89b4fa;
    font-weight: 600;
}

.section-title {
    font-size: 11px;
    font-weight: bold;
    color: #89b4fa;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin-top: 8px;
}

.info-text {
    font-size: 12px;
    color: #585b70;
}

.status-name {
    font-size: 13px;
    font-weight: 600;
    color: #cdd6f4;
    font-family: monospace;
}

.status-detail {
    font-size: 12px;
    color: #a6adc8;
}

.diff-view, .checks-view {
    background-color: #1e1e2e;
    color: #cdd6f4;
    font-size: 12px;
    font-family: "JetBrains Mono", "Fira Code", "Cascadia Code", monospace;
}

.status-container {
    background-color: #1e1e2e;
    border-radius: 6px;
    padding: 8px;
}

.empty-label {
    font-size: 12px;
    color: #585b70;
}

headerbar {
    background-color: #1e1e2e;
    border-bottom: 1px solid #313244;
}

separator {
    background-color: #313244;
    min-width: 1px;
    min-height: 1px;
}

.repo-section-header {
    font-size: 9px;
    font-weight: bold;
    color: #6c7086;
    text-transform: uppercase;
    letter-spacing: 1px;
}

.run-dot-active {
    font-size: 9px;
    color: #a6e3a1;
}

.run-dot {
    font-size: 9px;
    color: #45475a;
}

.pr-badge {
    font-size: 9px;
    font-weight: bold;
    color: #89b4fa;
    background-color: #1e1e2e;
    border: 1px solid #45475a;
    border-radius: 4px;
    padding: 0 4px;
}

.composer-bar {
    background-color: #1e1e2e;
    border-top: 1px solid #313244;
}

.composer-bar entry {
    background-color: #181825;
    color: #cdd6f4;
    border: 1px solid #313244;
    border-radius: 6px;
    font-size: 13px;
}

.composer-bar entry:focus {
    border-color: #89b4fa;
}

.pill-button {
    background-color: #313244;
    color: #cdd6f4;
    border: 1px solid #45475a;
    border-radius: 6px;
    padding: 2px 10px;
    font-size: 12px;
}

.pill-button:hover {
    background-color: #45475a;
}

.conflict-badge {
    color: #f9e2af;
    font-size: 11px;
    font-weight: bold;
}

.session-badge {
    font-size: 9px;
    font-weight: bold;
    color: #a6e3a1;
    background-color: #1e1e2e;
    border: 1px solid #45475a;
    border-radius: 4px;
    padding: 0 3px;
}

.todo-count-badge {
    font-size: 9px;
    font-weight: bold;
    color: #f9e2af;
    background-color: #1e1e2e;
    border: 1px solid #45475a;
    border-radius: 4px;
    padding: 0 3px;
}

.workspace-path-label {
    color: #6c7086;
    font-size: 11px;
    font-family: monospace;
    margin-bottom: 2px;
}

.sidebar-search {
    background-color: #181825;
    color: #cdd6f4;
    border: 1px solid #313244;
    border-radius: 6px;
    font-size: 12px;
}

.sidebar-search:focus {
    border-color: #89b4fa;
}
"#;
