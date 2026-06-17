use adw::prelude::*;
use adw::{Application, ApplicationWindow, HeaderBar};
use gtk::{
    Align, Box as GBox, Button, CssProvider, Label, ListBox, ListBoxRow, Orientation, PolicyType,
    ScrolledWindow, Separator, Stack, StackSwitcher, TextView, STYLE_PROVIDER_PRIORITY_APPLICATION,
};
use linux_conductor_core::paths::AppPaths;
use linux_conductor_core::workspace::WorkspaceStore;

const APP_ID: &str = "io.github.pranavkannepalli.linux-conductor";

fn main() {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run();
}

fn build_ui(app: &Application) {
    let paths = AppPaths::from_env();

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Linux Conductor")
        .default_width(1280)
        .default_height(800)
        .build();

    // Load custom CSS for dark dense styling
    let css = CssProvider::new();
    css.load_from_data(APP_CSS);
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().unwrap(),
        &css,
        STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // Top-level split: left sidebar | main content
    let split = adw::OverlaySplitView::new();
    split.set_min_sidebar_width(220.0);
    split.set_max_sidebar_width(280.0);
    split.set_show_sidebar(true);

    // ── LEFT SIDEBAR ──────────────────────────────────────────────────
    let sidebar = build_sidebar(&paths);
    split.set_sidebar(Some(&sidebar));

    // ── MAIN CONTENT ─────────────────────────────────────────────────
    let main_box = GBox::new(Orientation::Horizontal, 0);
    main_box.add_css_class("main-area");

    // Center workspace panel
    let center = build_center_panel(&paths);
    center.set_hexpand(true);
    center.set_vexpand(true);

    // Right panel: diff / checks / todos
    let right = build_right_panel(&paths);
    right.set_width_request(340);
    right.set_vexpand(true);

    let sep = Separator::new(Orientation::Vertical);
    main_box.append(&center);
    main_box.append(&sep);
    main_box.append(&right);

    split.set_content(Some(&main_box));

    // Header bar with toggle sidebar button
    let header = HeaderBar::new();
    header.set_show_end_title_buttons(true);
    let toggle_btn = Button::from_icon_name("sidebar-show-symbolic");
    toggle_btn.set_tooltip_text(Some("Toggle sidebar"));
    let split_clone = split.clone();
    toggle_btn.connect_clicked(move |_| {
        split_clone.set_show_sidebar(!split_clone.shows_sidebar());
    });
    header.pack_start(&toggle_btn);

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&split));

    window.set_content(Some(&toolbar_view));
    window.present();
}

// ── SIDEBAR ───────────────────────────────────────────────────────────────

fn build_sidebar(paths: &AppPaths) -> GBox {
    let sidebar_box = GBox::new(Orientation::Vertical, 0);
    sidebar_box.add_css_class("sidebar");
    sidebar_box.set_width_request(220);

    let header = Label::new(Some("Workspaces"));
    header.add_css_class("sidebar-header");
    header.set_xalign(0.0);
    header.set_margin_start(12);
    header.set_margin_top(10);
    header.set_margin_bottom(6);
    sidebar_box.append(&header);

    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    scroll.set_vexpand(true);

    let list = ListBox::new();
    list.add_css_class("workspace-list");
    list.set_selection_mode(gtk::SelectionMode::Single);

    // Populate with workspaces from DB
    if let Ok(store) = WorkspaceStore::open(paths.database_path.clone()) {
        if let Ok(workspaces) = store.list() {
            for ws in &workspaces {
                let row = build_workspace_row(
                    ws.name.as_str(),
                    ws.branch.as_str(),
                    ws.status.as_str(),
                    ws.port_base as i64,
                );
                list.append(&row);
            }
        }
    }

    if list.first_child().is_none() {
        let empty = Label::new(Some(
            "No workspaces yet.\nRun: linux-conductor workspace create",
        ));
        empty.add_css_class("empty-label");
        empty.set_wrap(true);
        empty.set_margin_start(12);
        empty.set_margin_end(12);
        empty.set_margin_top(16);
        list.append(&ListBoxRow::builder().child(&empty).build());
    }

    scroll.set_child(Some(&list));
    sidebar_box.append(&scroll);

    // Bottom "Add workspace" button
    let add_btn = Button::with_label("+ New Workspace");
    add_btn.add_css_class("add-workspace-btn");
    add_btn.set_margin_start(8);
    add_btn.set_margin_end(8);
    add_btn.set_margin_top(8);
    add_btn.set_margin_bottom(8);
    sidebar_box.append(&add_btn);

    sidebar_box
}

fn build_workspace_row(name: &str, branch: &str, status: &str, port: i64) -> ListBoxRow {
    let row_box = GBox::new(Orientation::Vertical, 2);
    row_box.set_margin_start(12);
    row_box.set_margin_end(8);
    row_box.set_margin_top(6);
    row_box.set_margin_bottom(6);

    let name_label = Label::new(Some(name));
    name_label.add_css_class("workspace-name");
    name_label.set_xalign(0.0);

    let meta_text = format!("{branch} · :{port}");
    let meta_label = Label::new(Some(&meta_text));
    meta_label.add_css_class("workspace-meta");
    meta_label.set_xalign(0.0);

    let status_label = Label::new(Some(status));
    status_label.add_css_class("workspace-status");
    status_label.set_xalign(0.0);

    row_box.append(&name_label);
    row_box.append(&meta_label);
    row_box.append(&status_label);

    ListBoxRow::builder().child(&row_box).build()
}

// ── CENTER PANEL ──────────────────────────────────────────────────────────

fn build_center_panel(paths: &AppPaths) -> GBox {
    let center = GBox::new(Orientation::Vertical, 0);
    center.add_css_class("center-panel");

    // Workspace action toolbar
    let toolbar = build_workspace_toolbar(paths);
    center.append(&toolbar);

    let sep = Separator::new(Orientation::Horizontal);
    center.append(&sep);

    // Info area
    let info_scroll = ScrolledWindow::new();
    info_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    info_scroll.set_vexpand(true);

    let info_box = GBox::new(Orientation::Vertical, 12);
    info_box.set_margin_start(20);
    info_box.set_margin_end(20);
    info_box.set_margin_top(20);
    info_box.set_margin_bottom(20);

    // Status grid
    let status_label = build_info_section("Quick Status", build_status_grid(paths));
    info_box.append(&status_label);

    // Session controls
    let session_section = build_session_controls(paths);
    info_box.append(&session_section);

    info_scroll.set_child(Some(&info_box));
    center.append(&info_scroll);

    center
}

fn build_workspace_toolbar(_paths: &AppPaths) -> GBox {
    let bar = GBox::new(Orientation::Horizontal, 6);
    bar.add_css_class("workspace-toolbar");
    bar.set_margin_start(12);
    bar.set_margin_end(12);
    bar.set_margin_top(8);
    bar.set_margin_bottom(8);

    let run_btn = Button::with_label("▶ Run");
    run_btn.add_css_class("suggested-action");
    run_btn.set_tooltip_text(Some("Start run script (linux-conductor run <workspace>)"));
    run_btn.connect_clicked(move |_| {
        spawn_terminal_command(
            "linux-conductor run $(linux-conductor workspace list | head -1 | awk '{print $1}')",
        );
    });

    let stop_btn = Button::with_label("■ Stop");
    stop_btn.set_tooltip_text(Some("Stop run script"));
    stop_btn.connect_clicked(|_| {
        spawn_terminal_command(
            "linux-conductor stop $(linux-conductor workspace list | head -1 | awk '{print $1}')",
        );
    });

    let editor_btn = Button::with_label("⎋ Editor");
    editor_btn.set_tooltip_text(Some("Open workspace in VS Code"));
    editor_btn.connect_clicked(|_| {
        spawn_terminal_command("linux-conductor open $(linux-conductor workspace list | head -1 | awk '{print $1}') --editor code");
    });

    let pr_btn = Button::with_label("↑ Create PR");
    pr_btn.set_tooltip_text(Some("Push branch and create GitHub PR"));
    pr_btn.connect_clicked(|_| {
        spawn_terminal_command("linux-conductor pr create $(linux-conductor workspace list | head -1 | awk '{print $1}')");
    });

    let archive_btn = Button::with_label("✕ Archive");
    archive_btn.add_css_class("destructive-action");
    archive_btn.set_tooltip_text(Some("Archive workspace"));
    archive_btn.connect_clicked(|_| {
        spawn_terminal_command("linux-conductor archive $(linux-conductor workspace list | head -1 | awk '{print $1}')");
    });

    bar.append(&run_btn);
    bar.append(&stop_btn);
    bar.append(&editor_btn);
    bar.append(&pr_btn);
    bar.append(&archive_btn);

    bar
}

fn build_status_grid(paths: &AppPaths) -> GBox {
    let grid = GBox::new(Orientation::Vertical, 4);

    let mut lines: Vec<(String, String)> = Vec::new();

    if let Ok(store) = WorkspaceStore::open(paths.database_path.clone()) {
        if let Ok(statuses) = store.list_status() {
            for line in statuses.iter().take(5) {
                let ws = &line.workspace;
                let pr_text = line
                    .pull_request
                    .as_ref()
                    .map(|p| format!("PR #{}", p.number))
                    .unwrap_or_else(|| "no PR".to_owned());
                let run_text = if line.run_running {
                    "running"
                } else {
                    "stopped"
                };
                lines.push((
                    ws.name.clone(),
                    format!("{} · {} · {}", ws.branch, run_text, pr_text),
                ));
            }
        }
    }

    if lines.is_empty() {
        let label = Label::new(Some("No workspaces. Create one with:\nlinux-conductor workspace create <repo> --name <name> --branch <branch>"));
        label.add_css_class("info-text");
        label.set_xalign(0.0);
        label.set_wrap(true);
        grid.append(&label);
    } else {
        for (name, detail) in lines {
            let row = GBox::new(Orientation::Horizontal, 8);
            let name_lbl = Label::new(Some(&name));
            name_lbl.add_css_class("status-name");
            name_lbl.set_xalign(0.0);
            name_lbl.set_width_chars(18);

            let detail_lbl = Label::new(Some(&detail));
            detail_lbl.add_css_class("status-detail");
            detail_lbl.set_xalign(0.0);
            detail_lbl.set_hexpand(true);
            detail_lbl.set_ellipsize(gtk::pango::EllipsizeMode::End);

            row.append(&name_lbl);
            row.append(&detail_lbl);
            grid.append(&row);
        }
    }

    grid
}

fn build_info_section(title: &str, content: GBox) -> GBox {
    let section = GBox::new(Orientation::Vertical, 6);

    let title_lbl = Label::new(Some(title));
    title_lbl.add_css_class("section-title");
    title_lbl.set_xalign(0.0);

    section.append(&title_lbl);
    section.append(&content);
    section
}

fn build_session_controls(_paths: &AppPaths) -> GBox {
    let section = GBox::new(Orientation::Vertical, 8);

    let title_lbl = Label::new(Some("Launch Session"));
    title_lbl.add_css_class("section-title");
    title_lbl.set_xalign(0.0);
    section.append(&title_lbl);

    let hint = Label::new(Some(
        "Sessions launch in your default terminal.\nSelect a workspace from the sidebar first.",
    ));
    hint.add_css_class("info-text");
    hint.set_xalign(0.0);
    hint.set_wrap(true);
    section.append(&hint);

    let btn_row = GBox::new(Orientation::Horizontal, 8);
    btn_row.set_margin_top(4);

    let shell_btn = Button::with_label("Shell");
    shell_btn.connect_clicked(|_| {
        spawn_terminal_command("linux-conductor session start $(linux-conductor workspace list | head -1 | awk '{print $1}') --kind shell");
    });

    let codex_btn = Button::with_label("Codex");
    codex_btn.connect_clicked(|_| {
        spawn_terminal_command("linux-conductor session start $(linux-conductor workspace list | head -1 | awk '{print $1}') --kind codex");
    });

    let claude_btn = Button::with_label("Claude Code");
    claude_btn.connect_clicked(|_| {
        spawn_terminal_command("linux-conductor session start $(linux-conductor workspace list | head -1 | awk '{print $1}') --kind claude");
    });

    btn_row.append(&shell_btn);
    btn_row.append(&codex_btn);
    btn_row.append(&claude_btn);
    section.append(&btn_row);

    section
}

// ── RIGHT PANEL ───────────────────────────────────────────────────────────

fn build_right_panel(paths: &AppPaths) -> GBox {
    let right = GBox::new(Orientation::Vertical, 0);
    right.add_css_class("right-panel");

    let stack = Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
    stack.set_vexpand(true);
    stack.set_hexpand(true);

    let diff_page = build_diff_page(paths);
    stack.add_titled(&diff_page, Some("diff"), "Diff");

    let checks_page = build_checks_page(paths);
    stack.add_titled(&checks_page, Some("checks"), "Checks");

    let todos_page = build_todos_page(paths);
    stack.add_titled(&todos_page, Some("todos"), "Todos");

    let switcher = StackSwitcher::new();
    switcher.set_stack(Some(&stack));
    switcher.add_css_class("panel-switcher");
    switcher.set_halign(Align::Center);
    switcher.set_margin_top(8);
    switcher.set_margin_bottom(8);

    right.append(&switcher);
    let sep = Separator::new(Orientation::Horizontal);
    right.append(&sep);
    right.append(&stack);

    right
}

fn build_diff_page(paths: &AppPaths) -> ScrolledWindow {
    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);

    let text_view = TextView::new();
    text_view.set_editable(false);
    text_view.add_css_class("diff-view");
    text_view.set_monospace(true);
    text_view.set_margin_start(8);
    text_view.set_margin_end(8);
    text_view.set_margin_top(8);

    let mut diff_text = String::from("── Changed Files ──\n\n");

    if let Ok(store) = WorkspaceStore::open(paths.database_path.clone()) {
        if let Ok(workspaces) = store.list() {
            for ws in workspaces.iter().filter(|w| w.status == "active").take(3) {
                diff_text.push_str(&format!("▶ {}\n", ws.name));
                if let Ok(files) = store.changed_files(&ws.name) {
                    if files.is_empty() {
                        diff_text.push_str("  (no changes)\n");
                    }
                    for f in files {
                        diff_text.push_str(&format!("  {f}\n"));
                    }
                }
                diff_text.push('\n');
            }
        }
    }

    if diff_text == "── Changed Files ──\n\n" {
        diff_text.push_str("No active workspaces.\n\nCreate one:\n  linux-conductor workspace create <repo> --name <n> --branch <b>\n");
    }

    text_view.buffer().set_text(&diff_text);
    scroll.set_child(Some(&text_view));
    scroll
}

fn build_checks_page(paths: &AppPaths) -> ScrolledWindow {
    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);

    let text_view = TextView::new();
    text_view.set_editable(false);
    text_view.set_monospace(true);
    text_view.add_css_class("checks-view");
    text_view.set_margin_start(8);
    text_view.set_margin_end(8);
    text_view.set_margin_top(8);

    let mut text = String::from("── Checks Summary ──\n\n");

    if let Ok(store) = WorkspaceStore::open(paths.database_path.clone()) {
        if let Ok(statuses) = store.list_status() {
            for line in &statuses {
                let ws = &line.workspace;
                text.push_str(&format!("▶ {} ({})\n", ws.name, ws.status));
                text.push_str(&format!("  Branch: {}\n", ws.branch));
                if let Some(pr) = &line.pull_request {
                    text.push_str(&format!("  PR: #{} {}\n", pr.number, pr.state));
                } else {
                    text.push_str("  PR: none\n");
                }
                let run = if line.run_running {
                    "running"
                } else {
                    "stopped"
                };
                text.push_str(&format!("  Run: {run}\n"));
                text.push_str(&format!("  Sessions: {} active\n", line.active_sessions));
                text.push_str(&format!("  Todos: {} open\n", line.open_todos));
                text.push('\n');
            }
        }
    }

    if text == "── Checks Summary ──\n\n" {
        text.push_str("No workspaces found.\n");
    }

    text_view.buffer().set_text(&text);
    scroll.set_child(Some(&text_view));
    scroll
}

fn build_todos_page(paths: &AppPaths) -> ScrolledWindow {
    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);

    let vbox = GBox::new(Orientation::Vertical, 8);
    vbox.set_margin_start(12);
    vbox.set_margin_end(12);
    vbox.set_margin_top(12);

    let title = Label::new(Some("── Open Todos ──"));
    title.set_xalign(0.0);
    vbox.append(&title);

    if let Ok(store) = WorkspaceStore::open(paths.database_path.clone()) {
        if let Ok(workspaces) = store.list() {
            let mut any = false;
            for ws in &workspaces {
                if let Ok(todos) = store.list_todos(&ws.name) {
                    let open: Vec<_> = todos.iter().filter(|t| t.status == "open").collect();
                    if !open.is_empty() {
                        any = true;
                        let ws_lbl = Label::new(Some(&format!("▶ {}", ws.name)));
                        ws_lbl.set_xalign(0.0);
                        ws_lbl.add_css_class("section-title");
                        vbox.append(&ws_lbl);
                        for todo in &open {
                            let row = Label::new(Some(&format!("  ☐ {}", todo.text)));
                            row.set_xalign(0.0);
                            row.set_wrap(true);
                            vbox.append(&row);
                        }
                    }
                }
            }
            if !any {
                let empty = Label::new(Some(
                    "No open todos.\n\nAdd one:\n  linux-conductor todo add <workspace> <text>",
                ));
                empty.add_css_class("info-text");
                empty.set_xalign(0.0);
                empty.set_wrap(true);
                vbox.append(&empty);
            }
        }
    }

    scroll.set_child(Some(&vbox));
    scroll
}

// ── HELPERS ───────────────────────────────────────────────────────────────

fn spawn_terminal_command(cmd: &str) {
    // Try common terminals in order
    let terminals = [
        ("gnome-terminal", vec!["--", "bash", "-c"]),
        ("xterm", vec!["-e", "bash", "-c"]),
        ("konsole", vec!["-e", "bash", "-c"]),
        ("xfce4-terminal", vec!["-e"]),
        ("alacritty", vec!["-e", "bash", "-c"]),
        ("kitty", vec!["bash", "-c"]),
    ];

    let full_cmd = format!("{cmd}; echo; echo 'Press Enter to close...'; read");

    for (term, prefix_args) in &terminals {
        let mut command = std::process::Command::new(term);
        for arg in prefix_args {
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
.sidebar {
    background-color: #1e1e2e;
    border-right: 1px solid #313244;
}

.sidebar-header {
    font-size: 11px;
    font-weight: bold;
    color: #6c7086;
    text-transform: uppercase;
    letter-spacing: 1px;
}

.workspace-list {
    background-color: transparent;
}

.workspace-list row {
    border-radius: 6px;
    margin: 2px 6px;
}

.workspace-list row:selected {
    background-color: #313244;
}

.workspace-name {
    font-size: 13px;
    font-weight: 600;
    color: #cdd6f4;
}

.workspace-meta {
    font-size: 11px;
    color: #6c7086;
    font-family: monospace;
}

.workspace-status {
    font-size: 10px;
    color: #a6e3a1;
}

.add-workspace-btn {
    background-color: #313244;
    color: #89b4fa;
    border: 1px solid #45475a;
    border-radius: 6px;
    font-size: 12px;
}

.center-panel {
    background-color: #181825;
}

.workspace-toolbar {
    background-color: #1e1e2e;
    border-bottom: 1px solid #313244;
}

.right-panel {
    background-color: #181825;
}

.panel-switcher {
    background-color: transparent;
}

.section-title {
    font-size: 12px;
    font-weight: bold;
    color: #89b4fa;
    margin-top: 8px;
}

.info-text {
    font-size: 12px;
    color: #6c7086;
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
    font-family: "JetBrains Mono", "Fira Code", monospace;
}

.empty-label {
    font-size: 12px;
    color: #6c7086;
}
"#;
