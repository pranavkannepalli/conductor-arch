use adw::prelude::*;
use adw::{Application, ApplicationWindow, HeaderBar};
use gtk::{
    Align, Box as GBox, Button, CssProvider, Entry, Label, ListBox, ListBoxRow, Orientation,
    PolicyType, ScrolledWindow, Separator, Stack, StackSwitcher, TextView,
    STYLE_PROVIDER_PRIORITY_APPLICATION,
};
use linux_conductor_core::paths::AppPaths;
use linux_conductor_core::workspace::WorkspaceStore;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::SystemTime;

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
    let (sidebar, refresh_sidebar) = build_sidebar(
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
    let rs = refresh_sidebar.clone();
    refresh_btn.connect_clicked(move |_| {
        rs();
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
    let rs = refresh_sidebar.clone();
    glib::timeout_add_seconds_local(5, move || {
        rs();
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
) -> (GBox, impl Fn() + Clone + 'static) {
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

    // row index → workspace name (header rows not included)
    let names: Rc<RefCell<std::collections::HashMap<i32, String>>> =
        Rc::new(RefCell::new(std::collections::HashMap::new()));

    let db_path = paths.database_path.clone();

    // Populate list from DB
    let populate = {
        let list = list.clone();
        let names = Rc::clone(&names);
        let db_path = db_path.clone();
        let selected = Rc::clone(&selected);
        move || {
            // Clear existing rows
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            names.borrow_mut().clear();
            let mut row_idx: i32 = 0;

            let prev_selected = selected.borrow().clone();

            if let Ok(store) = WorkspaceStore::open(db_path.clone()) {
                if let Ok(statuses) = store.list_status() {
                    let mut current_repo = String::new();
                    for line in &statuses {
                        let ws = &line.workspace;
                        // Repo section header when repo changes
                        if line.repository_name != current_repo {
                            current_repo = line.repository_name.clone();
                            let repo_lbl = Label::new(Some(&current_repo));
                            repo_lbl.add_css_class("repo-section-header");
                            repo_lbl.set_xalign(0.0);
                            repo_lbl.set_margin_start(8);
                            repo_lbl.set_margin_top(6);
                            repo_lbl.set_margin_bottom(2);
                            // Non-selectable header row
                            let header_row = ListBoxRow::builder().child(&repo_lbl).build();
                            header_row.set_selectable(false);
                            header_row.set_activatable(false);
                            list.append(&header_row);
                            row_idx += 1;
                        }
                        let pr_num = line.pull_request.as_ref().map(|p| p.number);
                        let run_active = line.run_running;
                        let has_conflicts = store
                            .find_conflicting_workspaces(&ws.name)
                            .map(|v| !v.is_empty())
                            .unwrap_or(false);
                        let row = build_workspace_row(
                            ws.name.as_str(),
                            ws.branch.as_str(),
                            ws.status.as_str(),
                            ws.port_base as i64,
                            pr_num,
                            run_active,
                            has_conflicts,
                        );
                        list.append(&row);
                        names.borrow_mut().insert(row_idx, ws.name.clone());
                        row_idx += 1;
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

            // Re-select previously selected workspace if still present
            let names_ref = names.borrow();
            let target_idx: Option<i32> = prev_selected.as_deref().and_then(|n| {
                names_ref
                    .iter()
                    .find_map(|(&idx, name)| if name == n { Some(idx) } else { None })
            });
            drop(names_ref);

            if let Some(idx) = target_idx {
                if let Some(row) = list.row_at_index(idx) {
                    list.select_row(Some(&row));
                }
            } else if list.selected_row().is_none() {
                // Select first selectable row (skip header rows)
                let mut i = 0;
                while let Some(row) = list.row_at_index(i) {
                    if row.is_selectable() {
                        list.select_row(Some(&row));
                        break;
                    }
                    i += 1;
                }
            }
        }
    };

    populate();

    // On selection change: update shared state and refresh panels
    let sel_clone = Rc::clone(&selected);
    let names_clone = Rc::clone(&names);
    list.connect_row_selected(move |_, row| {
        let name = row.and_then(|r| names_clone.borrow().get(&r.index()).cloned());
        *sel_clone.borrow_mut() = name;
        refresh_center();
        refresh_right();
    });

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

    (sidebar_box, populate)
}

fn build_workspace_row(
    name: &str,
    branch: &str,
    status: &str,
    port: i64,
    pr_number: Option<i64>,
    run_active: bool,
    has_conflicts: bool,
) -> ListBoxRow {
    let row_box = GBox::new(Orientation::Vertical, 2);
    row_box.set_margin_start(12);
    row_box.set_margin_end(8);
    row_box.set_margin_top(6);
    row_box.set_margin_bottom(6);

    // Name row with run indicator
    let name_row = GBox::new(Orientation::Horizontal, 4);
    let run_dot = Label::new(Some(if run_active { "▶" } else { "■" }));
    run_dot.add_css_class(if run_active {
        "run-dot-active"
    } else {
        "run-dot"
    });
    let name_label = Label::new(Some(name));
    name_label.add_css_class("workspace-name");
    name_label.set_xalign(0.0);
    name_label.set_hexpand(true);
    name_row.append(&run_dot);
    name_row.append(&name_label);
    if has_conflicts {
        let conflict_badge = Label::new(Some("⚠"));
        conflict_badge.add_css_class("conflict-badge");
        name_row.append(&conflict_badge);
    }
    if let Some(pr) = pr_number {
        let pr_badge = Label::new(Some(&format!("PR#{pr}")));
        pr_badge.add_css_class("pr-badge");
        name_row.append(&pr_badge);
    }

    let meta_text = format!("{branch} · :{port}");
    let meta_label = Label::new(Some(&meta_text));
    meta_label.add_css_class("workspace-meta");
    meta_label.set_xalign(0.0);

    let status_label = Label::new(Some(status));
    status_label.add_css_class("workspace-status");
    status_label.set_xalign(0.0);

    row_box.append(&name_row);
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

    let merge_btn = Button::with_label("⇓ Merge");
    merge_btn.set_tooltip_text(Some("Merge GitHub PR (squash)"));
    let sel = Rc::clone(&selected);
    let rr = refresh_right.clone();
    merge_btn.connect_clicked(move |_| {
        if let Some(ws) = sel.borrow().clone() {
            spawn_terminal_command(&format!("linux-conductor pr merge {ws} --method squash"));
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

    let discard_btn = Button::with_label("⊗ Discard");
    discard_btn.add_css_class("destructive-action");
    discard_btn.set_tooltip_text(Some("Discard workspace and remove worktree"));
    let sel = Rc::clone(&selected);
    discard_btn.connect_clicked(move |_| {
        if let Some(ws) = sel.borrow().clone() {
            spawn_terminal_command(&format!("linux-conductor discard {ws}"));
        }
    });

    let rename_btn = Button::with_label("✎ Rename");
    rename_btn.set_tooltip_text(Some("Rename workspace"));
    let sel = Rc::clone(&selected);
    rename_btn.connect_clicked(move |_| {
        if let Some(ws) = sel.borrow().clone() {
            spawn_terminal_command(&format!(
                "read -rp 'New name for \"{ws}\": ' NEW_NAME\n\
                 linux-conductor workspace rename {ws} \"$NEW_NAME\""
            ));
        }
    });

    toolbar.append(&run_btn);
    toolbar.append(&stop_btn);
    toolbar.append(&editor_btn);
    toolbar.append(&pr_btn);
    toolbar.append(&merge_btn);
    toolbar.append(&rename_btn);
    toolbar.append(&archive_btn);
    toolbar.append(&discard_btn);

    let ws_path_label = Label::new(None);
    ws_path_label.add_css_class("workspace-path-label");
    ws_path_label.set_xalign(0.0);
    ws_path_label.set_margin_start(12);
    ws_path_label.set_ellipsize(gtk::pango::EllipsizeMode::Start);

    let toolbar_box = GBox::new(Orientation::Vertical, 0);
    toolbar_box.add_css_class("workspace-toolbar");
    toolbar_box.append(&ws_title);
    toolbar_box.append(&ws_path_label);
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

    // Context brief section — shows .context/brief.md for selected workspace
    let brief_title = Label::new(Some("Task Brief"));
    brief_title.add_css_class("section-title");
    brief_title.set_xalign(0.0);
    info_box.append(&brief_title);

    let brief_label = Label::new(Some("Select a workspace to view its task brief."));
    brief_label.add_css_class("info-text");
    brief_label.set_xalign(0.0);
    brief_label.set_wrap(true);
    info_box.append(&brief_label);

    // MCP status section
    let mcp_section_title = Label::new(Some("MCP Servers"));
    mcp_section_title.add_css_class("section-title");
    mcp_section_title.set_xalign(0.0);
    info_box.append(&mcp_section_title);

    let mcp_container = GBox::new(Orientation::Vertical, 4);
    mcp_container.add_css_class("status-container");
    info_box.append(&mcp_container);

    let status_section_title = Label::new(Some("All Workspaces"));
    status_section_title.add_css_class("section-title");
    status_section_title.set_xalign(0.0);
    info_box.append(&status_section_title);

    let status_container = GBox::new(Orientation::Vertical, 4);
    status_container.add_css_class("status-container");
    info_box.append(&status_container);

    info_scroll.set_child(Some(&info_box));
    center.append(&info_scroll);

    // Agent prompt composer bar (bottom of center panel)
    center.append(&Separator::new(Orientation::Horizontal));
    let composer_box = GBox::new(Orientation::Horizontal, 8);
    composer_box.add_css_class("composer-bar");
    composer_box.set_margin_start(12);
    composer_box.set_margin_end(12);
    composer_box.set_margin_top(8);
    composer_box.set_margin_bottom(8);

    let prompt_entry = Entry::new();
    prompt_entry.set_placeholder_text(Some("Prompt to agent (saved to .context/agent-notes.md)…"));
    prompt_entry.set_hexpand(true);

    let send_btn = Button::with_label("Send");
    send_btn.add_css_class("suggested-action");

    let sel_c = Rc::clone(&selected);
    let db_path_c = paths.database_path.clone();
    let entry_c = prompt_entry.clone();
    let do_send = move || {
        let text = entry_c.text().to_string();
        if text.trim().is_empty() {
            return;
        }
        let ws_name = match sel_c.borrow().clone() {
            Some(n) => n,
            None => return,
        };
        if let Ok(store) = WorkspaceStore::open(db_path_c.clone()) {
            if let Ok(ws_path) = store.workspace_path(&ws_name) {
                let notes_path = ws_path.join(".context").join("agent-notes.md");
                if let Some(parent) = notes_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&notes_path)
                {
                    let ts = SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let _ = writeln!(f, "\n---\n*t={ts}*\n\n{text}\n");
                }
                entry_c.set_text("");
            }
        }
    };

    let do_send_rc = std::rc::Rc::new(do_send);
    let ds1 = do_send_rc.clone();
    send_btn.connect_clicked(move |_| ds1());
    let ds2 = do_send_rc.clone();
    prompt_entry.connect_activate(move |_| ds2());

    composer_box.append(&prompt_entry);
    composer_box.append(&send_btn);
    center.append(&composer_box);

    let db_path = paths.database_path.clone();
    let ws_title_clone = ws_title.clone();
    let ws_path_label_clone = ws_path_label.clone();
    let status_container_clone = status_container.clone();
    let mcp_container_clone = mcp_container.clone();
    let brief_label_clone = brief_label.clone();
    let sel_clone = Rc::clone(&selected);

    let refresh = move || {
        // Update workspace title and path subtitle
        let ws_name = sel_clone.borrow().clone();
        let title_text = ws_name
            .as_deref()
            .map(|n| format!("▶ {n}"))
            .unwrap_or_else(|| "Select a workspace".to_owned());
        ws_title_clone.set_text(&title_text);

        let path_text = ws_name
            .as_deref()
            .and_then(|n| {
                WorkspaceStore::open(db_path.clone())
                    .ok()
                    .and_then(|store| store.workspace_path(n).ok())
                    .map(|p| p.display().to_string())
            })
            .unwrap_or_default();
        ws_path_label_clone.set_text(&path_text);

        // Update task brief
        let brief_text = ws_name
            .as_deref()
            .and_then(|n| {
                WorkspaceStore::open(db_path.clone())
                    .ok()
                    .and_then(|store| store.read_context_brief(n).ok().flatten())
            })
            .unwrap_or_else(|| "Select a workspace to view its task brief.".to_owned());
        brief_label_clone.set_text(&brief_text);

        // Refresh MCP status for selected workspace
        while let Some(child) = mcp_container_clone.first_child() {
            mcp_container_clone.remove(&child);
        }
        populate_mcp_section(&mcp_container_clone, &db_path, ws_name.as_deref());

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

fn populate_mcp_section(container: &GBox, db_path: &std::path::PathBuf, ws_name: Option<&str>) {
    use linux_conductor_core::mcp::workspace_mcp_status;

    let Some(name) = ws_name else {
        let lbl = Label::new(Some("Select a workspace to see MCP servers."));
        lbl.add_css_class("info-text");
        lbl.set_xalign(0.0);
        container.append(&lbl);
        return;
    };

    if let Ok(store) = WorkspaceStore::open(db_path.clone()) {
        if let Ok(ws_path) = store.workspace_path(name) {
            let status = workspace_mcp_status(&ws_path);
            let all: Vec<_> = status
                .claude_user
                .iter()
                .chain(&status.claude_project)
                .chain(&status.codex_user)
                .chain(&status.codex_project)
                .chain(&status.cursor_user)
                .chain(&status.cursor_project)
                .collect();
            if all.is_empty() {
                let lbl = Label::new(Some("No MCP servers configured."));
                lbl.add_css_class("info-text");
                lbl.set_xalign(0.0);
                container.append(&lbl);
            } else {
                for srv in &all {
                    let row = GBox::new(Orientation::Horizontal, 8);
                    let name_lbl = Label::new(Some(&srv.name));
                    name_lbl.add_css_class("status-name");
                    name_lbl.set_xalign(0.0);
                    name_lbl.set_hexpand(true);
                    let src_lbl = Label::new(Some(&srv.source));
                    src_lbl.add_css_class("workspace-meta");
                    row.append(&name_lbl);
                    row.append(&src_lbl);
                    container.append(&row);
                }
            }
        }
    }
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
    let checks_outer = GBox::new(Orientation::Vertical, 0);
    let checks_btn_bar = GBox::new(Orientation::Horizontal, 8);
    checks_btn_bar.set_margin_start(12);
    checks_btn_bar.set_margin_end(12);
    checks_btn_bar.set_margin_top(6);
    checks_btn_bar.set_margin_bottom(6);
    let refresh_pr_btn = Button::with_label("↻ Live PR Checks");
    refresh_pr_btn.add_css_class("pill-button");
    let sync_pr_btn = Button::with_label("⇄ Sync PR State");
    sync_pr_btn.add_css_class("pill-button");
    checks_btn_bar.append(&refresh_pr_btn);
    checks_btn_bar.append(&sync_pr_btn);
    checks_outer.append(&checks_btn_bar);
    checks_outer.append(&Separator::new(Orientation::Horizontal));
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
    checks_scroll.set_vexpand(true);
    checks_outer.append(&checks_scroll);
    stack.add_titled(&checks_outer, Some("checks"), "Checks");

    // Todos page
    let todos_outer = GBox::new(Orientation::Vertical, 0);
    let todos_box = GBox::new(Orientation::Vertical, 8);
    todos_box.set_margin_start(12);
    todos_box.set_margin_end(12);
    todos_box.set_margin_top(12);
    let todos_scroll = ScrolledWindow::new();
    todos_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    todos_scroll.set_child(Some(&todos_box));
    todos_scroll.set_vexpand(true);
    todos_outer.append(&todos_scroll);
    // Todos button bar (Sync from context)
    let todos_btn_bar = GBox::new(Orientation::Horizontal, 8);
    todos_btn_bar.set_margin_start(12);
    todos_btn_bar.set_margin_end(12);
    todos_btn_bar.set_margin_top(6);
    todos_btn_bar.set_margin_bottom(2);
    let sync_todos_btn = Button::with_label("⇄ Sync from .context/");
    sync_todos_btn.add_css_class("pill-button");
    sync_todos_btn.set_tooltip_text(Some("Import todos from .context/ files"));
    todos_btn_bar.append(&sync_todos_btn);
    todos_outer.append(&todos_btn_bar);
    // Add-todo entry row at bottom of todos tab
    todos_outer.append(&Separator::new(Orientation::Horizontal));
    let todo_add_row = GBox::new(Orientation::Horizontal, 8);
    todo_add_row.set_margin_start(12);
    todo_add_row.set_margin_end(12);
    todo_add_row.set_margin_top(6);
    todo_add_row.set_margin_bottom(6);
    let todo_entry = Entry::new();
    todo_entry.set_placeholder_text(Some("New todo…"));
    todo_entry.set_hexpand(true);
    let todo_add_btn = Button::with_label("Add");
    todo_add_row.append(&todo_entry);
    todo_add_row.append(&todo_add_btn);
    todos_outer.append(&todo_add_row);
    stack.add_titled(&todos_outer, Some("todos"), "Todos");

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

    // Review comments page
    let review_box = GBox::new(Orientation::Vertical, 8);
    review_box.set_margin_start(12);
    review_box.set_margin_end(12);
    review_box.set_margin_top(12);
    let review_scroll = ScrolledWindow::new();
    review_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    review_scroll.set_child(Some(&review_box));
    review_scroll.set_vexpand(true);
    stack.add_titled(&review_scroll, Some("review"), "Review");

    // Checkpoints page
    let checkpoints_outer = GBox::new(Orientation::Vertical, 0);
    let checkpoints_box = GBox::new(Orientation::Vertical, 8);
    checkpoints_box.set_margin_start(12);
    checkpoints_box.set_margin_end(12);
    checkpoints_box.set_margin_top(12);
    let checkpoints_scroll = ScrolledWindow::new();
    checkpoints_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    checkpoints_scroll.set_child(Some(&checkpoints_box));
    checkpoints_scroll.set_vexpand(true);
    checkpoints_outer.append(&checkpoints_scroll);
    checkpoints_outer.append(&Separator::new(Orientation::Horizontal));
    // "Create checkpoint" entry row
    let cp_add_row = GBox::new(Orientation::Horizontal, 8);
    cp_add_row.set_margin_start(12);
    cp_add_row.set_margin_end(12);
    cp_add_row.set_margin_top(6);
    cp_add_row.set_margin_bottom(6);
    let cp_entry = Entry::new();
    cp_entry.set_placeholder_text(Some("Checkpoint message…"));
    cp_entry.set_hexpand(true);
    let cp_save_btn = Button::with_label("Save");
    cp_add_row.append(&cp_entry);
    cp_add_row.append(&cp_save_btn);
    checkpoints_outer.append(&cp_add_row);
    stack.add_titled(&checkpoints_outer, Some("checkpoints"), "Checkpoints");

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
    let review_box_clone = review_box.clone();
    let checkpoints_box_clone = checkpoints_box.clone();
    let db_path2 = db_path.clone();
    let db_path3 = db_path.clone();
    let db_path4 = db_path.clone();
    let db_path5 = db_path.clone();
    let db_path6 = db_path.clone();

    // Wire up "↻ Live PR Checks" — calls gh pr checks and appends live output
    {
        let sel = Rc::clone(&selected);
        let buf = checks_view.buffer();
        let db = db_path4.clone();
        refresh_pr_btn.connect_clicked(move |_| {
            if let Some(ws_name) = sel.borrow().clone() {
                if let Ok(store) = WorkspaceStore::open(db.clone()) {
                    let live = match store.pull_request_checks(&ws_name) {
                        Ok(s) => s,
                        Err(e) => format!("gh pr checks failed: {e}\n"),
                    };
                    let current = buf.text(&buf.start_iter(), &buf.end_iter(), false);
                    buf.set_text(&format!("{current}\n── Live gh pr checks ──\n\n{live}"));
                }
            }
        });
    }

    // Wire up "⇄ Sync PR State" — refreshes PR metadata from GitHub API
    {
        let sel = Rc::clone(&selected);
        let buf = checks_view.buffer();
        let db = db_path5.clone();
        sync_pr_btn.connect_clicked(move |_| {
            if let Some(ws_name) = sel.borrow().clone() {
                if let Ok(store) = WorkspaceStore::open(db.clone()) {
                    match store.refresh_pull_request_state(&ws_name) {
                        Ok(Some(pr)) => {
                            let current = buf.text(&buf.start_iter(), &buf.end_iter(), false);
                            buf.set_text(&format!(
                                "{current}\n── PR State Refreshed ──\n\nPR #{} ({})\n{}",
                                pr.number, pr.state, pr.url
                            ));
                        }
                        Ok(None) => {
                            let current = buf.text(&buf.start_iter(), &buf.end_iter(), false);
                            buf.set_text(&format!("{current}\n── No PR found for branch ──\n"));
                        }
                        Err(e) => {
                            let current = buf.text(&buf.start_iter(), &buf.end_iter(), false);
                            buf.set_text(&format!("{current}\n── Sync error: {e} ──\n"));
                        }
                    }
                }
            }
        });
    }

    // Wire up "⇄ Sync from .context/" todos button
    {
        let sel = Rc::clone(&selected);
        let todos_box_c = todos_box.clone();
        let db = db_path6.clone();
        sync_todos_btn.connect_clicked(move |_| {
            if let Some(ws_name) = sel.borrow().clone() {
                if let Ok(store) = WorkspaceStore::open(db.clone()) {
                    let _ = store.sync_todos_from_context(&ws_name);
                    // Refresh todos box inline
                    while let Some(child) = todos_box_c.first_child() {
                        todos_box_c.remove(&child);
                    }
                    let title = Label::new(Some("── Open Todos ──"));
                    title.set_xalign(0.0);
                    todos_box_c.append(&title);
                    populate_todos_box(&todos_box_c, &db, Some(&ws_name));
                }
            }
        });
    }

    let refresh: Rc<dyn Fn()> = Rc::new(move || {
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

        // Review comments
        while let Some(child) = review_box_clone.first_child() {
            review_box_clone.remove(&child);
        }
        populate_review_box(&review_box_clone, &db_path, ws_name.as_deref());

        // Checkpoints
        while let Some(child) = checkpoints_box_clone.first_child() {
            checkpoints_box_clone.remove(&child);
        }
        populate_checkpoints_box(&checkpoints_box_clone, &db_path, ws_name.as_deref());

        // Logs
        let logs_text = build_logs_text(&logs_dir, ws_name.as_deref());
        logs_buf.set_text(&logs_text);
    });

    // Wire up "Add Todo" entry — adds todo then triggers a panel refresh
    {
        let sel = Rc::clone(&selected);
        let entry = todo_entry.clone();
        let rf = Rc::clone(&refresh);
        let db = db_path2.clone();
        let do_add = Rc::new(move || {
            let text = entry.text().to_string();
            if text.trim().is_empty() {
                return;
            }
            if let Some(ws_name) = sel.borrow().clone() {
                if let Ok(store) = WorkspaceStore::open(db.clone()) {
                    let _ = store.add_todo(&ws_name, &text);
                    entry.set_text("");
                    rf();
                }
            }
        });
        let d1 = do_add.clone();
        let d2 = do_add.clone();
        todo_add_btn.connect_clicked(move |_| d1());
        todo_entry.connect_activate(move |_| d2());
    }

    // Wire up "Save checkpoint" entry
    {
        let sel = Rc::clone(&selected);
        let entry = cp_entry.clone();
        let rf = Rc::clone(&refresh);
        let db = db_path3.clone();
        let do_save = Rc::new(move || {
            let msg = entry.text().to_string();
            if msg.trim().is_empty() {
                return;
            }
            if let Some(ws_name) = sel.borrow().clone() {
                if let Ok(store) = WorkspaceStore::open(db.clone()) {
                    let _ = store.checkpoint_create(&ws_name, &msg, None);
                    entry.set_text("");
                    rf();
                }
            }
        });
        let s1 = do_save.clone();
        let s2 = do_save.clone();
        cp_save_btn.connect_clicked(move |_| s1());
        cp_entry.connect_activate(move |_| s2());
    }

    // Initial populate
    refresh();

    let rf = Rc::clone(&refresh);
    (right, move || rf())
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
    let mut text = String::from("── Status ──\n\n");
    if let Ok(store) = WorkspaceStore::open(db_path.clone()) {
        if let Some(name) = ws_name {
            // git status --short (staged/unstaged/untracked indicators)
            match store.git_status_short(name) {
                Ok(s) if s.trim().is_empty() => text.push_str("(working tree clean)\n"),
                Ok(s) => text.push_str(&s),
                Err(_) => text.push_str("(git status unavailable)\n"),
            }
            text.push_str("\n── Unified Diff ──\n\n");
            match store.unified_diff(name, None) {
                Ok(diff) if diff.trim().is_empty() => text.push_str("(no unstaged changes)\n"),
                Ok(diff) => {
                    let lines: Vec<_> = diff.lines().take(150).collect();
                    text.push_str(&lines.join("\n"));
                    if diff.lines().count() > 150 {
                        text.push_str("\n\n… (truncated — run: linux-conductor diff ");
                        text.push_str(name);
                        text.push(')');
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
                        let row = GBox::new(Orientation::Horizontal, 8);
                        let lbl = Label::new(Some(&format!("☐ #{} {}", todo.id, todo.text)));
                        lbl.set_xalign(0.0);
                        lbl.set_wrap(true);
                        lbl.set_hexpand(true);
                        let done_btn = Button::with_label("✓");
                        done_btn.add_css_class("flat");
                        done_btn.set_tooltip_text(Some("Mark done"));
                        let todo_id = todo.id;
                        let db = db_path.clone();
                        let row_clone = row.clone();
                        done_btn.connect_clicked(move |_| {
                            if let Ok(store) = WorkspaceStore::open(db.clone()) {
                                let _ = store.complete_todo(todo_id);
                            }
                            if let Some(parent) = row_clone.parent() {
                                if let Ok(p) = parent.downcast::<GBox>() {
                                    p.remove(&row_clone);
                                }
                            }
                        });
                        row.append(&lbl);
                        row.append(&done_btn);
                        container.append(&row);
                    }
                }
            }
        }
        if !any {
            let empty = Label::new(Some("No open todos. Use the entry below to add one."));
            empty.add_css_class("info-text");
            empty.set_xalign(0.0);
            empty.set_wrap(true);
            container.append(&empty);
        }
    }
}

fn populate_checkpoints_box(container: &GBox, db_path: &std::path::PathBuf, ws_name: Option<&str>) {
    let title = Label::new(Some("── Checkpoints ──"));
    title.set_xalign(0.0);
    container.append(&title);

    let Some(name) = ws_name else {
        let lbl = Label::new(Some("Select a workspace to view its checkpoints."));
        lbl.add_css_class("info-text");
        lbl.set_xalign(0.0);
        container.append(&lbl);
        return;
    };

    if let Ok(store) = WorkspaceStore::open(db_path.clone()) {
        match store.checkpoint_list(name) {
            Ok(checkpoints) if checkpoints.is_empty() => {
                let lbl = Label::new(Some(
                    "No checkpoints yet.\nType a message below and click Save.",
                ));
                lbl.add_css_class("info-text");
                lbl.set_xalign(0.0);
                lbl.set_wrap(true);
                container.append(&lbl);
            }
            Ok(checkpoints) => {
                for cp in checkpoints.iter().rev().take(20) {
                    let row = GBox::new(Orientation::Vertical, 4);
                    let ts = Label::new(Some(&cp.created_at));
                    ts.add_css_class("workspace-meta");
                    ts.set_xalign(0.0);
                    let msg_lbl = Label::new(Some(&cp.message));
                    msg_lbl.set_xalign(0.0);
                    msg_lbl.set_wrap(true);
                    let btn_row = GBox::new(Orientation::Horizontal, 4);
                    let restore_btn = Button::with_label("↩ Restore");
                    restore_btn.add_css_class("flat");
                    restore_btn.set_tooltip_text(Some(
                        "Hard-reset workspace to this checkpoint (destructive)",
                    ));
                    let cp_id = cp.id;
                    let ws = name.to_owned();
                    let db = db_path.clone();
                    restore_btn.connect_clicked(move |_| {
                        spawn_terminal_command(&format!(
                            "linux-conductor checkpoint restore {ws} {cp_id}"
                        ));
                        let _ = WorkspaceStore::open(db.clone());
                    });
                    btn_row.append(&restore_btn);
                    row.append(&ts);
                    row.append(&msg_lbl);
                    row.append(&btn_row);
                    container.append(&row);
                    container.append(&Separator::new(Orientation::Horizontal));
                }
            }
            Err(e) => {
                let lbl = Label::new(Some(&format!("Error: {e}")));
                lbl.set_xalign(0.0);
                container.append(&lbl);
            }
        }
    }
}

fn populate_review_box(container: &GBox, db_path: &std::path::PathBuf, ws_name: Option<&str>) {
    let title = Label::new(Some("── Review Comments ──"));
    title.set_xalign(0.0);
    container.append(&title);

    let Some(name) = ws_name else {
        let lbl = Label::new(Some("Select a workspace to view review comments."));
        lbl.add_css_class("info-text");
        lbl.set_xalign(0.0);
        container.append(&lbl);
        return;
    };

    if let Ok(store) = WorkspaceStore::open(db_path.clone()) {
        if let Ok(comments) = store.list_review_comments(name) {
            let open: Vec<_> = comments.iter().filter(|c| c.status == "open").collect();
            if open.is_empty() {
                let lbl = Label::new(Some(
                    "No open review comments.\n\nAdd one:\n  linux-conductor review add <ws> <file> <body>",
                ));
                lbl.add_css_class("info-text");
                lbl.set_xalign(0.0);
                lbl.set_wrap(true);
                container.append(&lbl);
            } else {
                for comment in open {
                    let row = GBox::new(Orientation::Vertical, 4);
                    let file_text = match comment.line_number {
                        Some(ln) => format!("{}:{}", comment.file_path, ln),
                        None => comment.file_path.clone(),
                    };
                    let file_lbl = Label::new(Some(&file_text));
                    file_lbl.add_css_class("workspace-meta");
                    file_lbl.set_xalign(0.0);
                    let body_lbl = Label::new(Some(&comment.body));
                    body_lbl.set_xalign(0.0);
                    body_lbl.set_wrap(true);

                    let btn_row = GBox::new(Orientation::Horizontal, 6);

                    // Send to agent — appends comment to .context/agent-notes.md
                    let send_btn = Button::with_label("→ Agent");
                    send_btn.add_css_class("flat");
                    send_btn.set_tooltip_text(Some("Send this comment to agent-notes.md"));
                    let body_clone = comment.body.clone();
                    let file_clone = file_text.clone();
                    let ws_owned = name.to_owned();
                    let db2 = db_path.clone();
                    send_btn.connect_clicked(move |_| {
                        if let Ok(store) = WorkspaceStore::open(db2.clone()) {
                            if let Ok(ws_path) = store.workspace_path(&ws_owned) {
                                let notes = ws_path.join(".context").join("agent-notes.md");
                                if let Some(p) = notes.parent() {
                                    let _ = std::fs::create_dir_all(p);
                                }
                                use std::io::Write;
                                if let Ok(mut f) = std::fs::OpenOptions::new()
                                    .create(true)
                                    .append(true)
                                    .open(&notes)
                                {
                                    let _ = writeln!(
                                        f,
                                        "\n---\n**Review comment** `{file_clone}`\n\n{body_clone}\n"
                                    );
                                }
                            }
                        }
                    });

                    let resolve_btn = Button::with_label("Resolve");
                    resolve_btn.add_css_class("flat");
                    let comment_id = comment.id;
                    let db = db_path.clone();
                    let row_clone = row.clone();
                    resolve_btn.connect_clicked(move |_| {
                        if let Ok(store) = WorkspaceStore::open(db.clone()) {
                            let _ = store.resolve_review_comment(comment_id);
                        }
                        if let Some(parent) = row_clone.parent() {
                            if let Ok(p) = parent.downcast::<GBox>() {
                                p.remove(&row_clone);
                            }
                        }
                    });
                    btn_row.append(&send_btn);
                    btn_row.append(&resolve_btn);
                    row.append(&file_lbl);
                    row.append(&body_lbl);
                    row.append(&btn_row);
                    container.append(&row);
                    container.append(&Separator::new(Orientation::Horizontal));
                }
            }
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

.workspace-path-label {
    color: #6c7086;
    font-size: 11px;
    font-family: monospace;
    margin-bottom: 2px;
}
"#;
