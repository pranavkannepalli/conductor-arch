use adw::prelude::*;
use adw::{Application, ApplicationWindow, HeaderBar};
use gtk::{
    Align, Box as GBox, Button, CssProvider, Label, ListBox, ListBoxRow, Orientation, PolicyType,
    ScrolledWindow, Separator, Stack, StackSwitcher, TextView, STYLE_PROVIDER_PRIORITY_APPLICATION,
};
use linux_conductor_core::paths::AppPaths;
use linux_conductor_core::workspace::WorkspaceStore;
use std::cell::RefCell;
use std::rc::Rc;

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

    let css = CssProvider::new();
    css.load_from_data(APP_CSS);
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().unwrap(),
        &css,
        STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // Shared state: selected workspace name
    let selected: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    // ── LAYOUT ───────────────────────────────────────────────────────
    let split = adw::OverlaySplitView::new();
    split.set_min_sidebar_width(220.0);
    split.set_max_sidebar_width(280.0);
    split.set_show_sidebar(true);

    let main_box = GBox::new(Orientation::Horizontal, 0);

    // Right panel built first — its refresh fn is passed into center/sidebar
    let (right_panel, refresh_right) =
        build_right_panel(&paths.database_path, &paths.logs_dir, Rc::clone(&selected));
    right_panel.set_width_request(340);
    right_panel.set_vexpand(true);

    // Center panel — workspace header + action toolbar + status grid
    let (center_panel, refresh_center) =
        build_center_panel(&paths, Rc::clone(&selected), refresh_right.clone());
    center_panel.set_hexpand(true);
    center_panel.set_vexpand(true);

    // Sidebar — triggers both center and right refresh on selection
    let sidebar = build_sidebar(
        &paths,
        Rc::clone(&selected),
        refresh_center.clone(),
        refresh_right.clone(),
    );

    split.set_sidebar(Some(&sidebar));
    main_box.append(&center_panel);
    main_box.append(&Separator::new(Orientation::Vertical));
    main_box.append(&right_panel);
    split.set_content(Some(&main_box));

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
    let rc = refresh_center.clone();
    let rr = refresh_right.clone();
    refresh_btn.connect_clicked(move |_| {
        rc();
        rr();
    });
    header.pack_start(&toggle_btn);
    header.pack_end(&refresh_btn);

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&split));

    window.set_content(Some(&toolbar_view));
    window.present();

    // Auto-refresh panels every 5 seconds
    let rc = refresh_center.clone();
    let rr = refresh_right.clone();
    glib::timeout_add_seconds_local(5, move || {
        rc();
        rr();
        glib::ControlFlow::Continue
    });
}

// ── SIDEBAR ───────────────────────────────────────────────────────────────

fn build_sidebar(
    paths: &AppPaths,
    selected: Rc<RefCell<Option<String>>>,
    refresh_center: impl Fn() + Clone + 'static,
    refresh_right: impl Fn() + Clone + 'static,
) -> GBox {
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

    // Store (name, index) map for row → workspace name lookup
    let names: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));

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
                names.borrow_mut().push(ws.name.clone());
            }
        }
    }

    if list.first_child().is_none() {
        let empty = Label::new(Some(
            "No workspaces yet.\n\nRun:\nlinux-conductor workspace create",
        ));
        empty.add_css_class("empty-label");
        empty.set_wrap(true);
        empty.set_margin_start(12);
        empty.set_margin_end(12);
        empty.set_margin_top(16);
        list.append(&ListBoxRow::builder().child(&empty).build());
    }

    // On selection change: update shared state and refresh panels
    let sel_clone = Rc::clone(&selected);
    let names_clone = Rc::clone(&names);
    list.connect_row_selected(move |_, row| {
        let name = row.and_then(|r| {
            let idx = r.index() as usize;
            names_clone.borrow().get(idx).cloned()
        });
        *sel_clone.borrow_mut() = name;
        refresh_center();
        refresh_right();
    });

    // Select first workspace by default
    if let Some(first) = list.row_at_index(0) {
        list.select_row(Some(&first));
    }

    scroll.set_child(Some(&list));
    sidebar_box.append(&scroll);

    // "New workspace" opens terminal with create wizard
    let add_btn = Button::with_label("+ New Workspace");
    add_btn.add_css_class("add-workspace-btn");
    add_btn.set_margin_start(8);
    add_btn.set_margin_end(8);
    add_btn.set_margin_top(8);
    add_btn.set_margin_bottom(8);
    add_btn.connect_clicked(|_| {
        spawn_terminal_command(
            r#"echo 'Available repos:'; linux-conductor repo list; echo;
read -rp 'Repo name: ' REPO
read -rp 'Workspace name: ' NAME
read -rp 'Branch name: ' BRANCH
linux-conductor workspace create "$REPO" --name "$NAME" --branch "$BRANCH""#,
        );
    });
    sidebar_box.append(&add_btn);

    // "Add repo" button for onboarding
    let repo_btn = Button::with_label("+ Add Repository");
    repo_btn.add_css_class("add-workspace-btn");
    repo_btn.set_margin_start(8);
    repo_btn.set_margin_end(8);
    repo_btn.set_margin_bottom(8);
    repo_btn.connect_clicked(|_| {
        spawn_terminal_command(
            r#"read -rp 'Repository path: ' PATH
linux-conductor repo add "$PATH"
echo; echo 'Run linux-conductor-gtk to see the updated workspace list.'"#,
        );
    });
    sidebar_box.append(&repo_btn);

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

fn build_center_panel(
    paths: &AppPaths,
    selected: Rc<RefCell<Option<String>>>,
    refresh_right: impl Fn() + Clone + 'static,
) -> (GBox, impl Fn() + Clone + 'static) {
    let center = GBox::new(Orientation::Vertical, 0);
    center.add_css_class("center-panel");

    // Workspace title label in toolbar area
    let ws_title = Label::new(Some("No workspace selected"));
    ws_title.add_css_class("workspace-title");
    ws_title.set_xalign(0.0);
    ws_title.set_margin_start(12);
    ws_title.set_margin_top(8);

    // Action toolbar
    let toolbar = GBox::new(Orientation::Horizontal, 6);
    toolbar.add_css_class("workspace-toolbar");
    toolbar.set_margin_start(12);
    toolbar.set_margin_end(12);
    toolbar.set_margin_bottom(8);

    let run_btn = Button::with_label("▶ Run");
    run_btn.add_css_class("suggested-action");
    run_btn.set_tooltip_text(Some("Start run script"));
    let sel = Rc::clone(&selected);
    run_btn.connect_clicked(move |_| {
        if let Some(ws) = sel.borrow().clone() {
            spawn_terminal_command(&format!("linux-conductor run {ws}"));
        }
    });

    let stop_btn = Button::with_label("■ Stop");
    stop_btn.set_tooltip_text(Some("Stop run script"));
    let sel = Rc::clone(&selected);
    stop_btn.connect_clicked(move |_| {
        if let Some(ws) = sel.borrow().clone() {
            spawn_terminal_command(&format!("linux-conductor stop {ws}"));
        }
    });

    let editor_btn = Button::with_label("⎋ Editor");
    editor_btn.set_tooltip_text(Some("Open in VS Code"));
    let sel = Rc::clone(&selected);
    editor_btn.connect_clicked(move |_| {
        if let Some(ws) = sel.borrow().clone() {
            spawn_terminal_command(&format!("linux-conductor open {ws} --editor code"));
        }
    });

    let pr_btn = Button::with_label("↑ PR");
    pr_btn.set_tooltip_text(Some("Push branch and create GitHub PR"));
    let sel = Rc::clone(&selected);
    let rr = refresh_right.clone();
    pr_btn.connect_clicked(move |_| {
        if let Some(ws) = sel.borrow().clone() {
            spawn_terminal_command(&format!("linux-conductor pr create {ws}"));
            rr();
        }
    });

    let archive_btn = Button::with_label("✕ Archive");
    archive_btn.add_css_class("destructive-action");
    archive_btn.set_tooltip_text(Some("Archive workspace"));
    let sel = Rc::clone(&selected);
    let rr = refresh_right.clone();
    archive_btn.connect_clicked(move |_| {
        if let Some(ws) = sel.borrow().clone() {
            spawn_terminal_command(&format!("linux-conductor archive {ws}"));
            rr();
        }
    });

    toolbar.append(&run_btn);
    toolbar.append(&stop_btn);
    toolbar.append(&editor_btn);
    toolbar.append(&pr_btn);
    toolbar.append(&archive_btn);

    let toolbar_box = GBox::new(Orientation::Vertical, 0);
    toolbar_box.add_css_class("workspace-toolbar");
    toolbar_box.append(&ws_title);
    toolbar_box.append(&toolbar);

    center.append(&toolbar_box);
    center.append(&Separator::new(Orientation::Horizontal));

    // Info scroll area
    let info_scroll = ScrolledWindow::new();
    info_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    info_scroll.set_vexpand(true);

    let info_box = GBox::new(Orientation::Vertical, 12);
    info_box.set_margin_start(20);
    info_box.set_margin_end(20);
    info_box.set_margin_top(20);
    info_box.set_margin_bottom(20);

    // Session controls section
    let session_section = build_session_controls(Rc::clone(&selected));
    info_box.append(&session_section);

    // Status grid (all workspaces overview)
    let status_section_title = Label::new(Some("All Workspaces"));
    status_section_title.add_css_class("section-title");
    status_section_title.set_xalign(0.0);
    info_box.append(&status_section_title);

    let status_container = GBox::new(Orientation::Vertical, 4);
    status_container.add_css_class("status-container");
    info_box.append(&status_container);

    info_scroll.set_child(Some(&info_box));
    center.append(&info_scroll);

    let db_path = paths.database_path.clone();
    let ws_title_clone = ws_title.clone();
    let status_container_clone = status_container.clone();
    let sel_clone = Rc::clone(&selected);

    let refresh = move || {
        // Update workspace title
        let title_text = sel_clone
            .borrow()
            .as_deref()
            .map(|n| format!("▶ {n}"))
            .unwrap_or_else(|| "Select a workspace".to_owned());
        ws_title_clone.set_text(&title_text);

        // Refresh status grid
        while let Some(child) = status_container_clone.first_child() {
            status_container_clone.remove(&child);
        }
        populate_status_grid(&status_container_clone, &db_path);
    };

    // Initial populate
    refresh();

    (center, refresh)
}

fn populate_status_grid(container: &GBox, db_path: &std::path::PathBuf) {
    if let Ok(store) = WorkspaceStore::open(db_path.clone()) {
        if let Ok(statuses) = store.list_status() {
            if statuses.is_empty() {
                let lbl = Label::new(Some(
                    "No workspaces.\nCreate: linux-conductor workspace create <repo> --name <name> --branch <branch>",
                ));
                lbl.add_css_class("info-text");
                lbl.set_xalign(0.0);
                lbl.set_wrap(true);
                container.append(&lbl);
                return;
            }
            for line in &statuses {
                let ws = &line.workspace;
                let row = GBox::new(Orientation::Horizontal, 8);

                let name_lbl = Label::new(Some(&ws.name));
                name_lbl.add_css_class("status-name");
                name_lbl.set_xalign(0.0);
                name_lbl.set_width_chars(16);

                let pr_text = line
                    .pull_request
                    .as_ref()
                    .map(|p| format!("PR #{}", p.number))
                    .unwrap_or_else(|| "no PR".to_owned());
                let run_text = if line.run_running { "▶" } else { "■" };
                let detail = format!(
                    "{} {} · {} · {} open todo(s)",
                    run_text, ws.branch, pr_text, line.open_todos
                );
                let detail_lbl = Label::new(Some(&detail));
                detail_lbl.add_css_class("status-detail");
                detail_lbl.set_xalign(0.0);
                detail_lbl.set_hexpand(true);
                detail_lbl.set_ellipsize(gtk::pango::EllipsizeMode::End);

                row.append(&name_lbl);
                row.append(&detail_lbl);
                container.append(&row);
            }
        }
    }
}

fn build_session_controls(selected: Rc<RefCell<Option<String>>>) -> GBox {
    let section = GBox::new(Orientation::Vertical, 8);

    let title_lbl = Label::new(Some("Launch Session"));
    title_lbl.add_css_class("section-title");
    title_lbl.set_xalign(0.0);
    section.append(&title_lbl);

    let hint = Label::new(Some(
        "Sessions open in your default terminal emulator.\nSelect a workspace from the sidebar.",
    ));
    hint.add_css_class("info-text");
    hint.set_xalign(0.0);
    hint.set_wrap(true);
    section.append(&hint);

    let btn_row = GBox::new(Orientation::Horizontal, 8);
    btn_row.set_margin_top(4);

    let shell_btn = Button::with_label("Shell");
    let sel = Rc::clone(&selected);
    shell_btn.connect_clicked(move |_| {
        if let Some(ws) = sel.borrow().clone() {
            spawn_terminal_command(&format!("linux-conductor session start {ws} --kind shell"));
        }
    });

    let codex_btn = Button::with_label("Codex");
    let sel = Rc::clone(&selected);
    codex_btn.connect_clicked(move |_| {
        if let Some(ws) = sel.borrow().clone() {
            spawn_terminal_command(&format!("linux-conductor session start {ws} --kind codex"));
        }
    });

    let claude_btn = Button::with_label("Claude Code");
    let sel = Rc::clone(&selected);
    claude_btn.connect_clicked(move |_| {
        if let Some(ws) = sel.borrow().clone() {
            spawn_terminal_command(&format!("linux-conductor session start {ws} --kind claude"));
        }
    });

    let diff_btn = Button::with_label("View Diff");
    let sel = Rc::clone(&selected);
    diff_btn.connect_clicked(move |_| {
        if let Some(ws) = sel.borrow().clone() {
            spawn_terminal_command(&format!("linux-conductor diff {ws}"));
        }
    });

    let checks_btn = Button::with_label("Checks");
    let sel = Rc::clone(&selected);
    checks_btn.connect_clicked(move |_| {
        if let Some(ws) = sel.borrow().clone() {
            spawn_terminal_command(&format!("linux-conductor checks {ws}"));
        }
    });

    btn_row.append(&shell_btn);
    btn_row.append(&codex_btn);
    btn_row.append(&claude_btn);
    btn_row.append(&diff_btn);
    btn_row.append(&checks_btn);
    section.append(&btn_row);

    section
}

// ── RIGHT PANEL ───────────────────────────────────────────────────────────

fn build_right_panel(
    db_path: &std::path::PathBuf,
    logs_dir: &std::path::PathBuf,
    selected: Rc<RefCell<Option<String>>>,
) -> (GBox, impl Fn() + Clone + 'static) {
    let right = GBox::new(Orientation::Vertical, 0);
    right.add_css_class("right-panel");

    let stack = Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
    stack.set_vexpand(true);
    stack.set_hexpand(true);

    // Diff page
    let diff_view = TextView::new();
    diff_view.set_editable(false);
    diff_view.add_css_class("diff-view");
    diff_view.set_monospace(true);
    diff_view.set_margin_start(8);
    diff_view.set_margin_end(8);
    diff_view.set_margin_top(8);
    let diff_scroll = ScrolledWindow::new();
    diff_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    diff_scroll.set_child(Some(&diff_view));
    stack.add_titled(&diff_scroll, Some("diff"), "Diff");

    // Checks page
    let checks_view = TextView::new();
    checks_view.set_editable(false);
    checks_view.set_monospace(true);
    checks_view.add_css_class("checks-view");
    checks_view.set_margin_start(8);
    checks_view.set_margin_end(8);
    checks_view.set_margin_top(8);
    let checks_scroll = ScrolledWindow::new();
    checks_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    checks_scroll.set_child(Some(&checks_view));
    stack.add_titled(&checks_scroll, Some("checks"), "Checks");

    // Todos page
    let todos_box = GBox::new(Orientation::Vertical, 8);
    todos_box.set_margin_start(12);
    todos_box.set_margin_end(12);
    todos_box.set_margin_top(12);
    let todos_scroll = ScrolledWindow::new();
    todos_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    todos_scroll.set_child(Some(&todos_box));
    stack.add_titled(&todos_scroll, Some("todos"), "Todos");

    // Logs page
    let logs_view = TextView::new();
    logs_view.set_editable(false);
    logs_view.set_monospace(true);
    logs_view.add_css_class("diff-view");
    logs_view.set_margin_start(8);
    logs_view.set_margin_end(8);
    logs_view.set_margin_top(8);
    let logs_scroll = ScrolledWindow::new();
    logs_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    logs_scroll.set_child(Some(&logs_view));
    stack.add_titled(&logs_scroll, Some("logs"), "Logs");

    let switcher = StackSwitcher::new();
    switcher.set_stack(Some(&stack));
    switcher.add_css_class("panel-switcher");
    switcher.set_halign(Align::Center);
    switcher.set_margin_top(8);
    switcher.set_margin_bottom(8);

    right.append(&switcher);
    right.append(&Separator::new(Orientation::Horizontal));
    right.append(&stack);

    let db_path = db_path.clone();
    let logs_dir = logs_dir.clone();
    let sel = Rc::clone(&selected);
    let diff_buf = diff_view.buffer();
    let checks_buf = checks_view.buffer();
    let logs_buf = logs_view.buffer();
    let todos_box_clone = todos_box.clone();

    let refresh = move || {
        let ws_name = sel.borrow().clone();

        // Diff
        let diff_text = build_diff_text(&db_path, ws_name.as_deref());
        diff_buf.set_text(&diff_text);

        // Checks
        let checks_text = build_checks_text(&db_path, ws_name.as_deref());
        checks_buf.set_text(&checks_text);

        // Todos
        while let Some(child) = todos_box_clone.first_child() {
            todos_box_clone.remove(&child);
        }
        let title = Label::new(Some("── Open Todos ──"));
        title.set_xalign(0.0);
        todos_box_clone.append(&title);
        populate_todos_box(&todos_box_clone, &db_path, ws_name.as_deref());

        // Logs — read latest log file from workspace log directory
        let logs_text = build_logs_text(&logs_dir, ws_name.as_deref());
        logs_buf.set_text(&logs_text);
    };

    // Initial populate
    refresh();

    (right, refresh)
}

fn build_logs_text(logs_dir: &std::path::PathBuf, ws_name: Option<&str>) -> String {
    let mut text = String::from("── Latest Logs ──\n\n");

    let name = match ws_name {
        Some(n) => n,
        None => {
            text.push_str("Select a workspace to view its logs.\n");
            return text;
        }
    };

    let ws_log_dir = logs_dir.join(name);
    if !ws_log_dir.exists() {
        text.push_str("No logs yet. Start a run or session first.\n");
        return text;
    }

    // Find the most recent log file
    let mut entries: Vec<_> = match std::fs::read_dir(&ws_log_dir) {
        Ok(rd) => rd.flatten().collect(),
        Err(e) => {
            text.push_str(&format!("Error reading log directory: {e}\n"));
            return text;
        }
    };
    entries.sort_by_key(|e| {
        e.metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });

    let Some(latest) = entries.last() else {
        text.push_str("No log files found.\n");
        return text;
    };

    text.push_str(&format!("File: {}\n\n", latest.path().display()));

    match std::fs::read_to_string(latest.path()) {
        Ok(content) => {
            // Show last 200 lines to keep it manageable
            let lines: Vec<_> = content.lines().collect();
            let start = lines.len().saturating_sub(200);
            if start > 0 {
                text.push_str(&format!("… ({start} earlier lines omitted) …\n\n"));
            }
            text.push_str(&lines[start..].join("\n"));
        }
        Err(e) => text.push_str(&format!("Error reading log: {e}\n")),
    }

    text
}

fn build_diff_text(db_path: &std::path::PathBuf, ws_name: Option<&str>) -> String {
    let mut text = String::from("── Changed Files ──\n\n");
    if let Ok(store) = WorkspaceStore::open(db_path.clone()) {
        if let Some(name) = ws_name {
            match store.changed_files(name) {
                Ok(files) if files.is_empty() => text.push_str("(no changes)\n"),
                Ok(files) => {
                    for f in files {
                        text.push_str(&format!("  {f}\n"));
                    }
                    text.push('\n');
                    // Also show unified diff (first 60 lines to keep it readable)
                    if let Ok(diff) = store.unified_diff(name, None) {
                        text.push_str("── Unified Diff ──\n\n");
                        let lines: Vec<_> = diff.lines().take(120).collect();
                        text.push_str(&lines.join("\n"));
                        if diff.lines().count() > 120 {
                            text.push_str("\n\n… (truncated, run: linux-conductor diff ");
                            text.push_str(name);
                            text.push(')');
                        }
                    }
                }
                Err(e) => text.push_str(&format!("Error: {e}\n")),
            }
        } else {
            // Show changed files for all active workspaces
            if let Ok(workspaces) = store.list() {
                for ws in workspaces.iter().filter(|w| w.status == "active").take(5) {
                    text.push_str(&format!("▶ {}\n", ws.name));
                    if let Ok(files) = store.changed_files(&ws.name) {
                        if files.is_empty() {
                            text.push_str("  (no changes)\n");
                        }
                        for f in files {
                            text.push_str(&format!("  {f}\n"));
                        }
                    }
                    text.push('\n');
                }
            }
            if text == "── Changed Files ──\n\n" {
                text.push_str(
                    "No active workspaces.\n\nCreate one:\n  linux-conductor workspace create",
                );
            }
        }
    }
    text
}

fn build_checks_text(db_path: &std::path::PathBuf, ws_name: Option<&str>) -> String {
    let mut text = String::from("── Checks Summary ──\n\n");
    if let Ok(store) = WorkspaceStore::open(db_path.clone()) {
        if let Some(name) = ws_name {
            match store.checks_summary(name) {
                Ok(summary) => {
                    let ws = &summary.workspace;
                    text.push_str(&format!("Workspace: {} ({})\n", ws.name, ws.status));
                    text.push_str(&format!("Branch:    {}\n", ws.branch));
                    if let Some(state) = &summary.branch_push_state {
                        if state.has_upstream {
                            text.push_str(&format!(
                                "Push:      ↑{} ↓{}\n",
                                state.ahead, state.behind
                            ));
                        } else {
                            text.push_str("Push:      no upstream yet\n");
                        }
                    }
                    text.push_str(&format!("Changed:   {} file(s)\n", summary.changed_files));
                    text.push_str(&format!(
                        "Run:       {}\n",
                        summary
                            .run_status
                            .map(|s| s.as_str())
                            .unwrap_or("not started")
                    ));
                    text.push_str(&format!("Sessions:  {} active\n", summary.active_sessions));
                    match &summary.pull_request {
                        Some(pr) => text.push_str(&format!(
                            "PR:        #{} {} ({})\n",
                            pr.number, pr.url, pr.state
                        )),
                        None => text.push_str("PR:        none\n"),
                    }
                    text.push_str(&format!(
                        "Todos:     {} open / {} total\n",
                        summary.open_todos, summary.total_todos
                    ));
                    text.push_str(&format!(
                        "Review:    {} open comment(s)\n",
                        summary.open_review_comments
                    ));
                    if !summary.conflicting_workspaces.is_empty() {
                        text.push_str("\nFile conflicts:\n");
                        for (other, files) in &summary.conflicting_workspaces {
                            text.push_str(&format!("  {other}: {}\n", files.join(", ")));
                        }
                    }
                }
                Err(e) => text.push_str(&format!("Error: {e}\n")),
            }
        } else {
            if let Ok(statuses) = store.list_status() {
                for line in &statuses {
                    let ws = &line.workspace;
                    text.push_str(&format!("▶ {} ({})\n", ws.name, ws.status));
                    text.push_str(&format!("  Branch: {}\n", ws.branch));
                    if let Some(pr) = &line.pull_request {
                        text.push_str(&format!("  PR: #{} ({})\n", pr.number, pr.state));
                    } else {
                        text.push_str("  PR: none\n");
                    }
                    let run = if line.run_running {
                        "▶ running"
                    } else {
                        "■ stopped"
                    };
                    text.push_str(&format!("  Run: {run}\n"));
                    text.push_str(&format!("  Sessions: {} active\n", line.active_sessions));
                    text.push_str(&format!("  Todos: {} open\n\n", line.open_todos));
                }
            }
            if text == "── Checks Summary ──\n\n" {
                text.push_str("No workspaces found.\n");
            }
        }
    }
    text
}

fn populate_todos_box(container: &GBox, db_path: &std::path::PathBuf, ws_name: Option<&str>) {
    if let Ok(store) = WorkspaceStore::open(db_path.clone()) {
        let workspaces = if let Some(name) = ws_name {
            store
                .list()
                .ok()
                .and_then(|ws| ws.into_iter().find(|w| w.name == name))
                .map(|w| vec![w])
                .unwrap_or_default()
        } else {
            store.list().unwrap_or_default()
        };

        let mut any = false;
        for ws in &workspaces {
            if let Ok(todos) = store.list_todos(&ws.name) {
                let open: Vec<_> = todos.iter().filter(|t| t.status == "open").collect();
                if !open.is_empty() {
                    any = true;
                    let ws_lbl = Label::new(Some(&format!("▶ {}", ws.name)));
                    ws_lbl.set_xalign(0.0);
                    ws_lbl.add_css_class("section-title");
                    container.append(&ws_lbl);
                    for todo in &open {
                        let row = Label::new(Some(&format!("  ☐ #{} {}", todo.id, todo.text)));
                        row.set_xalign(0.0);
                        row.set_wrap(true);
                        container.append(&row);
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
            container.append(&empty);
        }
    }
}

// ── HELPERS ───────────────────────────────────────────────────────────────

fn spawn_terminal_command(cmd: &str) {
    let terminals: &[(&str, &[&str])] = &[
        ("gnome-terminal", &["--", "bash", "-c"]),
        ("xterm", &["-e", "bash", "-c"]),
        ("konsole", &["-e", "bash", "-c"]),
        ("xfce4-terminal", &["-e", "bash", "-c"]),
        ("alacritty", &["-e", "bash", "-c"]),
        ("kitty", &["bash", "-c"]),
        ("foot", &["bash", "-c"]),
        ("wezterm", &["start", "--", "bash", "-c"]),
    ];

    let full_cmd = format!("{cmd}; echo; echo '--- Press Enter to close ---'; read");

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
    background-color: #181825;
    color: #cdd6f4;
}

.sidebar {
    background-color: #1e1e2e;
    border-right: 1px solid #313244;
}

.sidebar-header {
    font-size: 10px;
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
    padding: 2px 0;
}

.workspace-list row:selected {
    background-color: #313244;
}

.workspace-list row:hover {
    background-color: #2a2a3e;
}

.workspace-name {
    font-size: 13px;
    font-weight: 600;
    color: #cdd6f4;
}

.workspace-meta {
    font-size: 11px;
    color: #585b70;
    font-family: monospace;
}

.workspace-status {
    font-size: 10px;
    color: #a6e3a1;
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
"#;
