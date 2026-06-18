#![allow(dead_code)]

use adw::prelude::*;
use adw::{Application, ApplicationWindow, HeaderBar};
use gtk::{
    Align, Box as GBox, Button, CssProvider, Entry, Label, ListBox, ListBoxRow, Orientation,
    PolicyType, ScrolledWindow, Separator, Stack, StackSwitcher, TextView,
    STYLE_PROVIDER_PRIORITY_APPLICATION,
};
use linux_conductor_core::import::default_conductor_app_database;
use linux_conductor_core::paths::AppPaths;
use linux_conductor_core::repository::{AddRepository, RepositoryStore};
use linux_conductor_core::workspace::{CreateWorkspace, WorkspaceStore};
use rusqlite::Connection;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::SystemTime;

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

    let selected: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(initial_workspace));

    let split = adw::OverlaySplitView::new();
    split.set_min_sidebar_width(220.0);
    split.set_max_sidebar_width(280.0);
    split.set_show_sidebar(true);

    let toast_overlay = adw::ToastOverlay::new();
    let (dashboard, refresh_dashboard) = build_dashboard_panel(&paths);
    dashboard.set_hexpand(true);
    dashboard.set_vexpand(true);

    let (workspace_detail, refresh_workspace_detail) =
        build_workspace_detail_page(&paths, Rc::clone(&selected));
    let (projects_page, refresh_projects) = build_projects_page(
        &paths,
        refresh_dashboard.clone(),
        refresh_workspace_detail.clone(),
    );
    let (history_page, refresh_history) = build_history_page();

    let main_stack = Stack::new();
    main_stack.set_hexpand(true);
    main_stack.set_vexpand(true);
    main_stack.add_named(&dashboard, Some("dashboard"));
    main_stack.add_named(&projects_page, Some("projects"));
    main_stack.add_named(&history_page, Some("history"));
    main_stack.add_named(&workspace_detail, Some("workspace"));
    main_stack.set_visible_child_name("dashboard");

    let (sidebar, refresh_sidebar) = build_app_sidebar(
        &paths,
        Rc::clone(&selected),
        main_stack.clone(),
        refresh_workspace_detail.clone(),
    );

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
    let rd = refresh_dashboard.clone();
    let rs = refresh_sidebar.clone();
    let rp = refresh_projects.clone();
    let rh = refresh_history.clone();
    let rw = refresh_workspace_detail.clone();
    refresh_btn.connect_clicked(move |_| {
        rs();
        rd();
        rp();
        rh();
        rw();
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
    let rd_kb = refresh_dashboard.clone();
    let rs_kb = refresh_sidebar.clone();
    let rp_kb = refresh_projects.clone();
    let rh_kb = refresh_history.clone();
    let rw_kb = refresh_workspace_detail.clone();
    evk.connect_key_pressed(move |_, keyval, _, modifiers| {
        if modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK) && keyval == gtk::gdk::Key::r {
            rs_kb();
            rd_kb();
            rp_kb();
            rh_kb();
            rw_kb();
            return gtk::glib::Propagation::Stop;
        }
        gtk::glib::Propagation::Proceed
    });
    window.add_controller(evk);

    // Auto-refresh panels every 5 seconds
    let rd = refresh_dashboard.clone();
    let rs = refresh_sidebar.clone();
    let rp = refresh_projects.clone();
    let rw = refresh_workspace_detail.clone();
    glib::timeout_add_seconds_local(5, move || {
        rs();
        rd();
        rp();
        rw();
        glib::ControlFlow::Continue
    });
}

// ── APP SHELL ─────────────────────────────────────────────────────────────

fn build_app_sidebar(
    paths: &AppPaths,
    selected: Rc<RefCell<Option<String>>>,
    stack: Stack,
    refresh_workspace: impl Fn() + Clone + 'static,
) -> (GBox, impl Fn() + Clone + 'static) {
    let sidebar_box = GBox::new(Orientation::Vertical, 0);
    sidebar_box.add_css_class("sidebar");
    sidebar_box.set_width_request(240);

    let dashboard_btn = Button::with_label("Dashboard");
    dashboard_btn.add_css_class("nav-button-active");
    let stack_dashboard = stack.clone();
    dashboard_btn.connect_clicked(move |_| stack_dashboard.set_visible_child_name("dashboard"));
    sidebar_box.append(&dashboard_btn);

    let history_btn = Button::with_label("History");
    history_btn.add_css_class("nav-button");
    let stack_history = stack.clone();
    history_btn.connect_clicked(move |_| stack_history.set_visible_child_name("history"));
    sidebar_box.append(&history_btn);

    let projects_btn = Button::with_label("Projects");
    projects_btn.add_css_class("nav-button");
    let stack_projects = stack.clone();
    projects_btn.connect_clicked(move |_| stack_projects.set_visible_child_name("projects"));
    sidebar_box.append(&projects_btn);

    let divider = Separator::new(Orientation::Horizontal);
    sidebar_box.append(&divider);

    let projects_header = GBox::new(Orientation::Horizontal, 8);
    projects_header.add_css_class("projects-header");
    let header = Label::new(Some("Workspaces"));
    header.add_css_class("sidebar-header");
    header.set_xalign(0.0);
    header.set_hexpand(true);
    projects_header.append(&header);
    sidebar_box.append(&projects_header);

    let search_entry = Entry::new();
    search_entry.set_placeholder_text(Some("Filter workspaces..."));
    search_entry.add_css_class("sidebar-search");
    search_entry.set_margin_start(12);
    search_entry.set_margin_end(12);
    search_entry.set_margin_bottom(8);
    sidebar_box.append(&search_entry);

    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    scroll.set_vexpand(true);

    let list = ListBox::new();
    list.add_css_class("workspace-list");
    list.set_selection_mode(gtk::SelectionMode::Single);
    let names: Rc<RefCell<std::collections::HashMap<i32, String>>> =
        Rc::new(RefCell::new(std::collections::HashMap::new()));
    let db_path = paths.database_path.clone();

    let populate = {
        let list = list.clone();
        let names = Rc::clone(&names);
        let selected = Rc::clone(&selected);
        let search_entry = search_entry.clone();
        move || {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            names.borrow_mut().clear();
            let filter = search_entry.text().to_string().to_lowercase();
            let prev_selected = selected.borrow().clone();
            let mut row_idx = 0;

            if let Ok(store) = WorkspaceStore::open(db_path.clone()) {
                if let Ok(statuses) = store.list_status() {
                    let mut current_repo = String::new();
                    for line in statuses {
                        let ws = &line.workspace;
                        if !filter.is_empty()
                            && !ws.name.to_lowercase().contains(&filter)
                            && !ws.branch.to_lowercase().contains(&filter)
                            && !line.repository_name.to_lowercase().contains(&filter)
                        {
                            continue;
                        }

                        if line.repository_name != current_repo {
                            current_repo = line.repository_name.clone();
                            let repo_lbl = Label::new(Some(&current_repo));
                            repo_lbl.add_css_class("repo-section-header");
                            repo_lbl.set_xalign(0.0);
                            repo_lbl.set_margin_start(8);
                            repo_lbl.set_margin_top(8);
                            repo_lbl.set_margin_bottom(2);
                            let header_row = ListBoxRow::builder().child(&repo_lbl).build();
                            header_row.set_selectable(false);
                            header_row.set_activatable(false);
                            list.append(&header_row);
                            row_idx += 1;
                        }

                        let row = build_workspace_row(
                            &ws.name,
                            &ws.branch,
                            &ws.status,
                            i64::from(ws.port_base),
                            line.pull_request.as_ref().map(|p| p.number),
                            line.run_running,
                            false,
                            line.active_sessions,
                            line.open_todos,
                        );
                        list.append(&row);
                        names.borrow_mut().insert(row_idx, ws.name.clone());
                        row_idx += 1;
                    }
                }
            }

            if list.first_child().is_none() {
                let empty = Label::new(Some("No workspaces."));
                empty.add_css_class("empty-label");
                empty.set_margin_start(12);
                empty.set_margin_top(16);
                list.append(&ListBoxRow::builder().child(&empty).build());
            }

            let names_ref = names.borrow();
            let target_idx = prev_selected.as_deref().and_then(|name| {
                names_ref
                    .iter()
                    .find_map(|(&idx, row_name)| (row_name == name).then_some(idx))
            });
            drop(names_ref);
            if let Some(idx) = target_idx {
                if let Some(row) = list.row_at_index(idx) {
                    list.select_row(Some(&row));
                }
            }
        }
    };

    populate();
    let pop_search = populate.clone();
    search_entry.connect_changed(move |_| pop_search());

    let names_select = Rc::clone(&names);
    let selected_select = Rc::clone(&selected);
    let stack_select = stack.clone();
    list.connect_row_selected(move |_, row| {
        let Some(name) = row.and_then(|r| names_select.borrow().get(&r.index()).cloned()) else {
            return;
        };
        *selected_select.borrow_mut() = Some(name);
        refresh_workspace();
        stack_select.set_visible_child_name("workspace");
    });

    scroll.set_child(Some(&list));
    sidebar_box.append(&scroll);
    (sidebar_box, populate)
}

fn build_workspace_detail_page(
    paths: &AppPaths,
    selected: Rc<RefCell<Option<String>>>,
) -> (GBox, impl Fn() + Clone + 'static) {
    let root = GBox::new(Orientation::Vertical, 0);
    root.add_css_class("dashboard");

    let header = GBox::new(Orientation::Vertical, 8);
    header.add_css_class("dashboard-header");
    let title = Label::new(Some("Workspace"));
    title.add_css_class("dashboard-title");
    title.set_xalign(0.0);
    let subtitle = Label::new(None);
    subtitle.add_css_class("card-meta");
    subtitle.set_xalign(0.0);
    header.append(&title);
    header.append(&subtitle);
    root.append(&header);

    let body = GBox::new(Orientation::Vertical, 14);
    body.add_css_class("detail-body");
    root.append(&body);

    let db_path = paths.database_path.clone();
    let refresh = move || {
        while let Some(child) = body.first_child() {
            body.remove(&child);
        }

        let Some(name) = selected.borrow().clone() else {
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
        title.set_text(&title_case_workspace(&ws.name));
        subtitle.set_text(&format!(
            "{} / {} / {}",
            line.repository_name,
            ws.branch,
            ws.path.display()
        ));

        let actions = GBox::new(Orientation::Horizontal, 8);
        let shell_btn = Button::with_label("Shell");
        let codex_btn = Button::with_label("New Codex Chat");
        let claude_btn = Button::with_label("New Claude Chat");
        let cursor_btn = Button::with_label("New Cursor Chat");
        let run_btn = Button::with_label("Run");
        let stop_btn = Button::with_label("Stop");
        let open_btn = Button::with_label("Open Folder");
        for (button, kind) in [
            (shell_btn.clone(), "shell"),
            (codex_btn.clone(), "codex"),
            (claude_btn.clone(), "claude"),
            (cursor_btn.clone(), "cursor"),
        ] {
            let workspace = ws.name.clone();
            button.connect_clicked(move |_| {
                spawn_terminal_command(&format!(
                    "{} session open {} --kind {}",
                    cli_binary().display(),
                    shell_quote(&workspace),
                    kind
                ));
            });
        }
        let run_workspace = ws.name.clone();
        let db_path_run = db_path.clone();
        run_btn.connect_clicked(move |_| {
            if let Ok(store) = WorkspaceStore::open(db_path_run.clone()) {
                let _ = store.run_workspace(&run_workspace);
            }
        });
        let stop_workspace = ws.name.clone();
        let db_path_stop = db_path.clone();
        stop_btn.connect_clicked(move |_| {
            if let Ok(store) = WorkspaceStore::open(db_path_stop.clone()) {
                let _ = store.stop_workspace(&stop_workspace);
            }
        });
        let path = ws.path.clone();
        open_btn.connect_clicked(move |_| {
            let _ = std::process::Command::new("open").arg(&path).spawn();
        });
        actions.append(&shell_btn);
        actions.append(&codex_btn);
        actions.append(&claude_btn);
        actions.append(&cursor_btn);
        actions.append(&run_btn);
        actions.append(&stop_btn);
        actions.append(&open_btn);
        body.append(&actions);

        body.append(&detail_row("Status", &ws.status));
        body.append(&detail_row("Port", &ws.port_base.to_string()));
        body.append(&detail_row("Todos", &line.open_todos.to_string()));
        body.append(&detail_row(
            "Activity",
            if line.run_running {
                "Run active"
            } else if line.active_sessions > 0 {
                "Agent active"
            } else {
                "Idle"
            },
        ));
        if let Some(pr) = line.pull_request {
            body.append(&detail_row(
                "Pull request",
                &format!("#{} {} {}", pr.number, pr.state, pr.url),
            ));
        }

        let lifecycle = GBox::new(Orientation::Horizontal, 8);
        let archive_btn = Button::with_label("Archive");
        let restore_btn = Button::with_label("Restore");
        let discard_btn = Button::with_label("Discard");
        for (button, action) in [
            (archive_btn.clone(), "archive"),
            (restore_btn.clone(), "restore"),
            (discard_btn.clone(), "discard"),
        ] {
            let workspace = ws.name.clone();
            let db_path_action = db_path.clone();
            button.connect_clicked(move |_| {
                if let Ok(store) = WorkspaceStore::open(db_path_action.clone()) {
                    let _ = match action {
                        "archive" => store.archive(&workspace, false),
                        "restore" => store.restore(&workspace),
                        "discard" => store.discard(&workspace),
                        _ => unreachable!(),
                    };
                }
            });
        }
        lifecycle.append(&archive_btn);
        lifecycle.append(&restore_btn);
        lifecycle.append(&discard_btn);
        body.append(&lifecycle);

        let tabs = Stack::new();
        tabs.set_vexpand(true);
        let switcher = StackSwitcher::new();
        switcher.set_stack(Some(&tabs));
        switcher.add_css_class("panel-switcher");
        body.append(&switcher);

        let chat_box = GBox::new(Orientation::Vertical, 8);
        for chat in conductor_sessions_for_workspace_path(&ws.path)
            .into_iter()
            .take(8)
        {
            chat_box.append(&session_summary_row(&chat));
        }
        tabs.add_titled(&chat_box, Some("chats"), "Chats");
        tabs.add_titled(
            &text_panel(&workspace_changes_text(&store, &ws.name)),
            Some("changes"),
            "Changes",
        );
        tabs.add_titled(
            &text_panel(&workspace_checks_text(&store, &ws.name)),
            Some("checks"),
            "Checks",
        );
        tabs.add_titled(
            &workspace_todos_panel(&store, &ws.name),
            Some("todos"),
            "Todos",
        );
        tabs.add_titled(
            &text_panel(&workspace_processes_text(&store, &ws.name)),
            Some("processes"),
            "Processes",
        );
        body.append(&tabs);
    };
    refresh();
    (root, refresh)
}

fn build_projects_page(
    paths: &AppPaths,
    refresh_dashboard: impl Fn() + Clone + 'static,
    refresh_workspace: impl Fn() + Clone + 'static,
) -> (GBox, impl Fn() + Clone + 'static) {
    let root = GBox::new(Orientation::Vertical, 0);
    root.add_css_class("dashboard");
    let header = GBox::new(Orientation::Vertical, 8);
    header.add_css_class("dashboard-header");
    let title = Label::new(Some("Projects"));
    title.add_css_class("dashboard-title");
    title.set_xalign(0.0);
    let subtitle = Label::new(Some("Create workspaces and inspect imported repositories."));
    subtitle.add_css_class("card-meta");
    subtitle.set_xalign(0.0);
    header.append(&title);
    header.append(&subtitle);
    root.append(&header);

    let body = GBox::new(Orientation::Vertical, 14);
    body.add_css_class("detail-body");
    root.append(&body);

    let repo_title = Label::new(Some("Add Repository"));
    repo_title.add_css_class("section-title");
    repo_title.set_xalign(0.0);
    body.append(&repo_title);

    let repo_box = GBox::new(Orientation::Horizontal, 8);
    let repo_path_entry = Entry::new();
    repo_path_entry.set_placeholder_text(Some("local path or git URL"));
    let repo_name_entry = Entry::new();
    repo_name_entry.set_placeholder_text(Some("project name"));
    let add_repo_btn = Button::with_label("Add Local");
    let clone_repo_btn = Button::with_label("Clone");
    let repo_result = Label::new(None);
    repo_result.add_css_class("card-meta");
    repo_result.set_xalign(0.0);
    repo_box.append(&repo_path_entry);
    repo_box.append(&repo_name_entry);
    repo_box.append(&add_repo_btn);
    repo_box.append(&clone_repo_btn);
    body.append(&repo_box);
    body.append(&repo_result);

    let workspace_title = Label::new(Some("New Workspace"));
    workspace_title.add_css_class("section-title");
    workspace_title.set_xalign(0.0);
    workspace_title.set_margin_top(10);
    body.append(&workspace_title);

    let create_box = GBox::new(Orientation::Horizontal, 8);
    let repo_entry = Entry::new();
    repo_entry.set_placeholder_text(Some("repository name"));
    let name_entry = Entry::new();
    name_entry.set_placeholder_text(Some("workspace name"));
    let branch_entry = Entry::new();
    branch_entry.set_placeholder_text(Some("branch name"));
    let create_btn = Button::with_label("Create Workspace");
    let result = Label::new(None);
    result.add_css_class("card-meta");
    result.set_xalign(0.0);
    create_box.append(&repo_entry);
    create_box.append(&name_entry);
    create_box.append(&branch_entry);
    create_box.append(&create_btn);
    body.append(&create_box);
    body.append(&result);

    let repo_list = GBox::new(Orientation::Vertical, 8);
    body.append(&repo_list);

    let db_path = paths.database_path.clone();
    let refresh = {
        let repo_list = repo_list.clone();
        move || {
            while let Some(child) = repo_list.first_child() {
                repo_list.remove(&child);
            }
            if let Ok(store) = RepositoryStore::open(db_path.clone()) {
                if let Ok(repos) = store.list_with_workspace_counts() {
                    for (repo, active, total) in repos {
                        repo_list.append(&detail_row(
                            &repo.name,
                            &format!(
                                "{} active / {} total / {}",
                                active,
                                total,
                                repo.root_path.display()
                            ),
                        ));
                    }
                }
            }
        }
    };

    let db_path_repo = paths.database_path.clone();
    let refresh_after_repo = refresh.clone();
    let repo_result_add = repo_result.clone();
    let repo_path_add = repo_path_entry.clone();
    let repo_name_add = repo_name_entry.clone();
    add_repo_btn.connect_clicked(move |_| {
        let path = repo_path_add.text().trim().to_owned();
        let name = repo_name_add.text().trim().to_owned();
        if path.is_empty() {
            repo_result_add.set_text("Local repository path is required.");
            return;
        }
        match RepositoryStore::open(db_path_repo.clone()).and_then(|store| {
            store.add(AddRepository {
                name: (!name.is_empty()).then_some(name),
                root_path: PathBuf::from(path),
                default_branch: None,
                remote_name: "origin".to_owned(),
                workspace_parent_path: None,
            })
        }) {
            Ok(repo) => {
                repo_result_add.set_text(&format!("Added {}", repo.name));
                refresh_after_repo();
            }
            Err(err) => repo_result_add.set_text(&format!("Add failed: {err:#}")),
        }
    });

    let db_path_clone = paths.database_path.clone();
    let refresh_after_clone = refresh.clone();
    clone_repo_btn.connect_clicked(move |_| {
        let url = repo_path_entry.text().trim().to_owned();
        let explicit_name = repo_name_entry.text().trim().to_owned();
        if url.is_empty() {
            repo_result.set_text("Git URL is required.");
            return;
        }
        let name = if explicit_name.is_empty() {
            repo_name_from_url(&url)
        } else {
            explicit_name
        };
        let clone_path = default_clone_parent().join(&name);
        if let Some(parent) = clone_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let clone_result = if clone_path.exists() {
            Ok(())
        } else {
            std::process::Command::new("git")
                .args(["clone", &url])
                .arg(&clone_path)
                .status()
                .map(|status| {
                    if status.success() {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("git clone exited with {status}"))
                    }
                })
                .unwrap_or_else(|err| Err(err.into()))
        };
        match clone_result.and_then(|_| {
            RepositoryStore::open(db_path_clone.clone()).and_then(|store| {
                store.add(AddRepository {
                    name: Some(name),
                    root_path: clone_path,
                    default_branch: None,
                    remote_name: "origin".to_owned(),
                    workspace_parent_path: None,
                })
            })
        }) {
            Ok(repo) => {
                repo_result.set_text(&format!("Cloned and added {}", repo.name));
                refresh_after_clone();
            }
            Err(err) => repo_result.set_text(&format!("Clone failed: {err:#}")),
        }
    });

    let db_path_create = paths.database_path.clone();
    let refresh_after_create = refresh.clone();
    create_btn.connect_clicked(move |_| {
        let repo = repo_entry.text().trim().to_owned();
        let name = name_entry.text().trim().to_owned();
        let branch = branch_entry.text().trim().to_owned();
        if repo.is_empty() || name.is_empty() || branch.is_empty() {
            result.set_text("Repository, workspace name, and branch are required.");
            return;
        }
        match WorkspaceStore::open(db_path_create.clone()).and_then(|store| {
            store.create(CreateWorkspace {
                repository_name: repo,
                name,
                branch,
                base_ref: None,
            })
        }) {
            Ok(workspace) => {
                result.set_text(&format!("Created {}", workspace.path.display()));
                refresh_after_create();
                refresh_dashboard();
                refresh_workspace();
            }
            Err(err) => result.set_text(&format!("Create failed: {err:#}")),
        }
    });

    refresh();
    (root, refresh)
}

fn build_history_page() -> (GBox, impl Fn() + Clone + 'static) {
    let root = GBox::new(Orientation::Vertical, 0);
    root.add_css_class("dashboard");
    let header = GBox::new(Orientation::Vertical, 8);
    header.add_css_class("dashboard-header");
    let title = Label::new(Some("History"));
    title.add_css_class("dashboard-title");
    title.set_xalign(0.0);
    let subtitle = Label::new(Some("Old Conductor chats from the macOS app database."));
    subtitle.add_css_class("card-meta");
    subtitle.set_xalign(0.0);
    header.append(&title);
    header.append(&subtitle);
    root.append(&header);

    let split = GBox::new(Orientation::Horizontal, 0);
    split.set_vexpand(true);
    let list_scroll = ScrolledWindow::new();
    list_scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    list_scroll.set_width_request(380);
    let list = ListBox::new();
    list.add_css_class("workspace-list");
    list_scroll.set_child(Some(&list));
    let message_view = TextView::new();
    message_view.set_editable(false);
    message_view.set_monospace(false);
    message_view.add_css_class("history-view");
    let message_scroll = ScrolledWindow::new();
    message_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    message_scroll.set_child(Some(&message_view));
    split.append(&list_scroll);
    split.append(&Separator::new(Orientation::Vertical));
    split.append(&message_scroll);
    root.append(&split);

    let session_ids: Rc<RefCell<std::collections::HashMap<i32, String>>> =
        Rc::new(RefCell::new(std::collections::HashMap::new()));
    let refresh = {
        let list = list.clone();
        let session_ids = Rc::clone(&session_ids);
        move || {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            session_ids.borrow_mut().clear();
            for (idx, session) in conductor_recent_sessions().into_iter().enumerate() {
                list.append(&session_summary_row(&session));
                session_ids
                    .borrow_mut()
                    .insert(i32::try_from(idx).unwrap_or(i32::MAX), session.id);
            }
        }
    };

    let session_ids_select = Rc::clone(&session_ids);
    list.connect_row_selected(move |_, row| {
        let Some(session_id) =
            row.and_then(|r| session_ids_select.borrow().get(&r.index()).cloned())
        else {
            return;
        };
        let buffer = message_view.buffer();
        buffer.set_text(&conductor_session_messages(&session_id));
    });

    refresh();
    (root, refresh)
}

// ── DASHBOARD ─────────────────────────────────────────────────────────────

fn build_dashboard_panel(paths: &AppPaths) -> (GBox, impl Fn() + Clone + 'static) {
    let root = GBox::new(Orientation::Vertical, 0);
    root.add_css_class("dashboard");

    let header = GBox::new(Orientation::Vertical, 14);
    header.add_css_class("dashboard-header");

    let title = Label::new(Some("Dashboard"));
    title.add_css_class("dashboard-title");
    title.set_xalign(0.0);
    header.append(&title);

    let project_tabs = GBox::new(Orientation::Horizontal, 18);
    project_tabs.add_css_class("project-tabs");
    header.append(&project_tabs);
    root.append(&header);

    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    scroll.set_vexpand(true);

    let board = GBox::new(Orientation::Horizontal, 22);
    board.add_css_class("kanban-board");
    scroll.set_child(Some(&board));
    root.append(&scroll);

    let db_path = paths.database_path.clone();
    let refresh = move || {
        while let Some(child) = project_tabs.first_child() {
            project_tabs.remove(&child);
        }
        while let Some(child) = board.first_child() {
            board.remove(&child);
        }

        let Ok(store) = WorkspaceStore::open(db_path.clone()) else {
            append_empty_dashboard(&project_tabs, &board, "No workspace database yet.");
            return;
        };
        let Ok(statuses) = store.list_status() else {
            append_empty_dashboard(&project_tabs, &board, "Could not read workspace state.");
            return;
        };

        let mut repo_names = statuses
            .iter()
            .map(|line| line.repository_name.clone())
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>();
        repo_names.sort();
        repo_names.dedup();

        let all_tab = Label::new(Some("All projects"));
        all_tab.add_css_class("project-tab-active");
        project_tabs.append(&all_tab);
        for repo in repo_names.iter().take(5) {
            let tab = Label::new(Some(repo));
            tab.add_css_class("project-tab");
            project_tabs.append(&tab);
        }

        let mut backlog = Vec::new();
        let mut in_progress = Vec::new();
        let mut in_review = Vec::new();
        let mut done = Vec::new();

        for line in &statuses {
            if line.workspace.status == "archived" {
                done.push(line);
            } else if line.pull_request.is_some() {
                in_review.push(line);
            } else if line.run_running || line.active_sessions > 0 {
                in_progress.push(line);
            } else {
                backlog.push(line);
            }
        }

        append_dashboard_column(&board, "Backlog", &backlog, &store);
        append_dashboard_column(&board, "In progress", &in_progress, &store);
        append_dashboard_column(&board, "In review", &in_review, &store);
        append_dashboard_column(&board, "Done", &done, &store);
    };

    refresh();
    (root, refresh)
}

fn append_empty_dashboard(project_tabs: &GBox, board: &GBox, message: &str) {
    let all_tab = Label::new(Some("All projects"));
    all_tab.add_css_class("project-tab-active");
    project_tabs.append(&all_tab);

    let empty = Label::new(Some(message));
    empty.add_css_class("empty-label");
    empty.set_xalign(0.0);
    empty.set_margin_start(24);
    empty.set_margin_top(24);
    board.append(&empty);
}

fn append_dashboard_column(
    board: &GBox,
    title: &str,
    lines: &[&linux_conductor_core::workspace::WorkspaceStatusLine],
    store: &WorkspaceStore,
) {
    let column = GBox::new(Orientation::Vertical, 12);
    column.add_css_class("kanban-column");
    column.set_hexpand(true);

    let header = GBox::new(Orientation::Horizontal, 8);
    let title_label = Label::new(Some(title));
    title_label.add_css_class("column-title");
    title_label.set_xalign(0.0);
    title_label.set_hexpand(true);
    let count = Label::new(Some(&lines.len().to_string()));
    count.add_css_class("column-count");
    header.append(&title_label);
    header.append(&count);
    column.append(&header);

    if lines.is_empty() {
        let empty = Label::new(Some("No workspaces"));
        empty.add_css_class("column-empty");
        empty.set_xalign(0.0);
        column.append(&empty);
    } else {
        for line in lines.iter().take(12) {
            column.append(&build_dashboard_card(line, store));
        }
    }

    board.append(&column);
}

fn build_dashboard_card(
    line: &linux_conductor_core::workspace::WorkspaceStatusLine,
    store: &WorkspaceStore,
) -> GBox {
    let ws = &line.workspace;
    let card = GBox::new(Orientation::Vertical, 10);
    card.add_css_class("workspace-card");

    let top = GBox::new(Orientation::Horizontal, 8);
    let branch = Label::new(Some(&ws.branch));
    branch.add_css_class("card-branch");
    branch.set_xalign(0.0);
    branch.set_hexpand(true);
    let diff = store.changed_files(&ws.name).map(|f| f.len()).unwrap_or(0);
    let diff_text = if diff > 0 {
        format!("+{diff}")
    } else {
        "clean".to_owned()
    };
    let diff_label = Label::new(Some(&diff_text));
    diff_label.add_css_class(if diff > 0 {
        "card-diff-hot"
    } else {
        "card-diff"
    });
    top.append(&branch);
    top.append(&diff_label);
    card.append(&top);

    let name = Label::new(Some(&title_case_workspace(&ws.name)));
    name.add_css_class("card-title");
    name.set_xalign(0.0);
    name.set_wrap(true);
    card.append(&name);

    let meta = match &line.pull_request {
        Some(pr) => format!(
            "{} · PR #{} · {}",
            line.repository_name, pr.number, pr.state
        ),
        None => format!("{} · port {}", line.repository_name, ws.port_base),
    };
    let meta_label = Label::new(Some(&meta));
    meta_label.add_css_class("card-meta");
    meta_label.set_xalign(0.0);
    meta_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    card.append(&meta_label);

    let foot = GBox::new(Orientation::Horizontal, 8);
    let activity = if line.run_running {
        "Running"
    } else if line.active_sessions > 0 {
        "Agent active"
    } else if ws.status == "archived" {
        "Archived"
    } else {
        "Ready"
    };
    let activity_label = Label::new(Some(activity));
    activity_label.add_css_class("card-activity");
    activity_label.set_xalign(0.0);
    activity_label.set_hexpand(true);
    let todo_label = Label::new(Some(&format!("{} todos", line.open_todos)));
    todo_label.add_css_class("card-meta");
    foot.append(&activity_label);
    foot.append(&todo_label);
    card.append(&foot);

    card
}

fn title_case_workspace(name: &str) -> String {
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

#[derive(Debug, Clone)]
struct ChatSummary {
    id: String,
    title: String,
    agent_type: String,
    status: String,
    repository_name: String,
    workspace_name: String,
    workspace_path: String,
    updated_at: String,
    message_count: i64,
}

fn detail_row(label: &str, value: &str) -> GBox {
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

fn session_summary_row(session: &ChatSummary) -> GBox {
    let row = GBox::new(Orientation::Vertical, 3);
    row.add_css_class("history-row");
    let title = Label::new(Some(&session.title));
    title.add_css_class("workspace-name");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let meta = Label::new(Some(&format!(
        "{} · {} · {} · {} messages",
        session.repository_name, session.workspace_name, session.agent_type, session.message_count
    )));
    meta.add_css_class("workspace-meta");
    meta.set_xalign(0.0);
    meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let status = Label::new(Some(&format!(
        "{} · {}",
        session.status, session.updated_at
    )));
    status.add_css_class("card-meta");
    status.set_xalign(0.0);
    status.set_ellipsize(gtk::pango::EllipsizeMode::End);
    row.append(&title);
    row.append(&meta);
    row.append(&status);
    row
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
    let add_btn = Button::with_label("Add Todo");
    let db_path = AppPaths::from_env().database_path;
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
    out.push_str("Runs\n");
    match store.list_runs(name) {
        Ok(records) if records.is_empty() => out.push_str("No runs recorded.\n"),
        Ok(records) => {
            for record in records {
                out.push_str(&format!(
                    "#{} {} pid={} started={} log={}\n",
                    record.id,
                    record.status.as_str(),
                    record.pid,
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
                    "#{} {} {} pid={} started={} log={}\n",
                    record.id,
                    record.command,
                    record.status.as_str(),
                    record.pid,
                    record.started_at,
                    record.log_path.display()
                ));
            }
        }
        Err(err) => out.push_str(&format!("Could not read sessions: {err:#}\n")),
    }
    out
}

fn conductor_recent_sessions() -> Vec<ChatSummary> {
    query_conductor_sessions(None).unwrap_or_default()
}

fn conductor_sessions_for_workspace_path(path: &std::path::Path) -> Vec<ChatSummary> {
    query_conductor_sessions(Some(path)).unwrap_or_default()
}

fn query_conductor_sessions(path: Option<&std::path::Path>) -> rusqlite::Result<Vec<ChatSummary>> {
    let db_path = default_conductor_app_database();
    let conn = Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut sql = String::from(
        "SELECT s.id,
                COALESCE(s.title, 'Untitled'),
                COALESCE(s.agent_type, ''),
                COALESCE(s.status, ''),
                COALESCE(r.name, ''),
                COALESCE(w.directory_name, ''),
                COALESCE(w.workspace_path, ''),
                COALESCE(s.updated_at, s.created_at, ''),
                COUNT(m.id)
         FROM sessions s
         LEFT JOIN workspaces w ON w.id = s.workspace_id
         LEFT JOIN repos r ON r.id = w.repository_id
         LEFT JOIN session_messages m ON m.session_id = s.id",
    );
    if path.is_some() {
        sql.push_str(" WHERE w.workspace_path = ?1");
    }
    sql.push_str(" GROUP BY s.id ORDER BY COALESCE(s.updated_at, s.created_at) DESC LIMIT 200");

    let mut stmt = conn.prepare(&sql)?;
    let rows = if let Some(path) = path {
        stmt.query_map([path.to_string_lossy().to_string()], row_to_chat_summary)?
            .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        stmt.query_map([], row_to_chat_summary)?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    Ok(rows)
}

fn row_to_chat_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatSummary> {
    Ok(ChatSummary {
        id: row.get(0)?,
        title: row.get(1)?,
        agent_type: row.get(2)?,
        status: row.get(3)?,
        repository_name: row.get(4)?,
        workspace_name: row.get(5)?,
        workspace_path: row.get(6)?,
        updated_at: row.get(7)?,
        message_count: row.get(8)?,
    })
}

fn conductor_session_messages(session_id: &str) -> String {
    let db_path = default_conductor_app_database();
    let Ok(conn) = Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return "Could not open Conductor chat database.".to_owned();
    };
    let Ok(mut stmt) = conn.prepare(
        "SELECT COALESCE(role, ''), COALESCE(content, full_message, ''), COALESCE(created_at, '')
         FROM session_messages
         WHERE session_id = ?1
         ORDER BY COALESCE(sent_at, created_at), queue_order
         LIMIT 160",
    ) else {
        return "Could not read chat messages.".to_owned();
    };
    let Ok(rows) = stmt.query_map([session_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    }) else {
        return "Could not load chat messages.".to_owned();
    };

    let mut text = String::new();
    for row in rows.flatten() {
        let (role, content, created_at) = row;
        text.push_str(&format!(
            "{} · {}\n{}\n\n",
            role,
            created_at,
            truncate_message(&content, 2200)
        ));
    }
    if text.is_empty() {
        "No messages in this chat.".to_owned()
    } else {
        text
    }
}

fn truncate_message(content: &str, max_chars: usize) -> String {
    let mut truncated = content.chars().take(max_chars).collect::<String>();
    if content.chars().count() > max_chars {
        truncated.push_str("\n...");
    }
    truncated
}

fn cli_binary() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join("linux-conductor")))
        .filter(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from("linux-conductor"))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn default_clone_parent() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("conductor")
        .join("repos")
}

fn repo_name_from_url(url: &str) -> String {
    url.trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("repository")
        .trim_end_matches(".git")
        .to_owned()
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

    let dashboard_nav = Label::new(Some("Dashboard"));
    dashboard_nav.add_css_class("nav-row-active");
    dashboard_nav.set_xalign(0.0);
    sidebar_box.append(&dashboard_nav);

    let history_nav = Label::new(Some("History"));
    history_nav.add_css_class("nav-row");
    history_nav.set_xalign(0.0);
    sidebar_box.append(&history_nav);

    let divider = Separator::new(Orientation::Horizontal);
    sidebar_box.append(&divider);

    let projects_header = GBox::new(Orientation::Horizontal, 8);
    projects_header.add_css_class("projects-header");
    let header = Label::new(Some("Projects"));
    header.add_css_class("sidebar-header");
    header.set_xalign(0.0);
    header.set_hexpand(true);
    projects_header.append(&header);
    sidebar_box.append(&projects_header);

    let search_entry = Entry::new();
    search_entry.set_placeholder_text(Some("Filter projects…"));
    search_entry.add_css_class("sidebar-search");
    search_entry.set_margin_start(12);
    search_entry.set_margin_end(12);
    search_entry.set_margin_bottom(8);
    sidebar_box.append(&search_entry);

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
        let search_entry_c = search_entry.clone();
        move || {
            let filter = search_entry_c.text().to_string().to_lowercase();
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
                        // Apply search filter
                        if !filter.is_empty()
                            && !ws.name.to_lowercase().contains(&filter)
                            && !ws.branch.to_lowercase().contains(&filter)
                        {
                            continue;
                        }
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
                            line.active_sessions,
                            line.open_todos,
                        );
                        list.append(&row);
                        names.borrow_mut().insert(row_idx, ws.name.clone());
                        row_idx += 1;
                    }
                }
            }

            if list.first_child().is_none() {
                let empty = Label::new(Some("No workspaces yet."));
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

    // Re-populate on search input
    let pop_search = populate.clone();
    search_entry.connect_changed(move |_| pop_search());

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
    active_sessions: usize,
    open_todos: usize,
) -> ListBoxRow {
    let row_box = GBox::new(Orientation::Horizontal, 10);
    row_box.add_css_class("project-row");

    let icon = Label::new(Some(if run_active { "◐" } else { "◦" }));
    icon.add_css_class(if run_active {
        "project-icon-hot"
    } else {
        "project-icon"
    });

    let text_box = GBox::new(Orientation::Vertical, 2);
    text_box.set_hexpand(true);
    let name_label = Label::new(Some(name));
    name_label.add_css_class("workspace-name");
    name_label.set_xalign(0.0);
    name_label.set_hexpand(true);

    let mut meta_parts = vec![branch.to_owned()];
    if let Some(pr) = pr_number {
        meta_parts.push(format!("PR #{pr}"));
    } else if active_sessions > 0 {
        meta_parts.push(format!("{active_sessions} session"));
    } else if open_todos > 0 {
        meta_parts.push(format!("{open_todos} todos"));
    } else if has_conflicts {
        meta_parts.push("conflict".to_owned());
    } else {
        meta_parts.push(format!(":{port}"));
    }
    if status == "archived" {
        meta_parts.push("archived".to_owned());
    }
    let meta_text = meta_parts.join(" · ");
    let meta_label = Label::new(Some(&meta_text));
    meta_label.add_css_class("workspace-meta");
    meta_label.set_xalign(0.0);
    meta_label.set_ellipsize(gtk::pango::EllipsizeMode::End);

    text_box.append(&name_label);
    text_box.append(&meta_label);
    row_box.append(&icon);
    row_box.append(&text_box);

    ListBoxRow::builder().child(&row_box).build()
}

// ── CENTER PANEL ──────────────────────────────────────────────────────────

fn build_center_panel(
    paths: &AppPaths,
    selected: Rc<RefCell<Option<String>>>,
    refresh_right: impl Fn() + Clone + 'static,
    toasts: adw::ToastOverlay,
    window: ApplicationWindow,
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
    editor_btn.set_tooltip_text(Some("Open in editor (cursor, code, or vim)"));
    let sel = Rc::clone(&selected);
    editor_btn.connect_clicked(move |_| {
        if let Some(ws) = sel.borrow().clone() {
            let editor = ["cursor", "code", "codium", "vim"]
                .iter()
                .find(|e| {
                    std::process::Command::new("which")
                        .arg(e)
                        .output()
                        .map(|o| o.status.success())
                        .unwrap_or(false)
                })
                .copied()
                .unwrap_or("code");
            spawn_terminal_command(&format!("linux-conductor open {ws} --editor {editor}"));
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
    let win_a = window.clone();
    archive_btn.connect_clicked(move |_| {
        if let Some(ws) = sel.borrow().clone() {
            let dialog = adw::MessageDialog::new(
                Some(&win_a),
                Some("Archive Workspace?"),
                Some(&format!("Archive \"{ws}\"? This stops any running processes and marks the workspace archived. The worktree directory is preserved.")),
            );
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("archive", "Archive");
            dialog.set_response_appearance("archive", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");
            let rr2 = rr.clone();
            dialog.connect_response(None, move |_, response| {
                if response == "archive" {
                    spawn_terminal_command(&format!("linux-conductor archive {ws}"));
                    rr2();
                }
            });
            dialog.present();
        }
    });

    let discard_btn = Button::with_label("⊗ Discard");
    discard_btn.add_css_class("destructive-action");
    discard_btn.set_tooltip_text(Some("Discard workspace and remove worktree"));
    let sel = Rc::clone(&selected);
    let win_d = window.clone();
    discard_btn.connect_clicked(move |_| {
        if let Some(ws) = sel.borrow().clone() {
            let dialog = adw::MessageDialog::new(
                Some(&win_d),
                Some("Discard Workspace?"),
                Some(&format!("Discard \"{ws}\"? This permanently removes the worktree directory and all uncommitted changes. This cannot be undone.")),
            );
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("discard", "Discard");
            dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");
            dialog.connect_response(None, move |_, response| {
                if response == "discard" {
                    spawn_terminal_command(&format!("linux-conductor discard {ws}"));
                }
            });
            dialog.present();
        }
    });

    let restore_btn = Button::with_label("↺ Restore");
    restore_btn.set_tooltip_text(Some("Restore archived workspace"));
    restore_btn.set_visible(false);
    let sel = Rc::clone(&selected);
    let rr = refresh_right.clone();
    restore_btn.connect_clicked(move |_| {
        if let Some(ws) = sel.borrow().clone() {
            spawn_terminal_command(&format!("linux-conductor workspace restore {ws}"));
            rr();
        }
    });

    let rename_btn = Button::with_label("✎ Rename");
    rename_btn.set_tooltip_text(Some("Rename workspace"));
    let sel = Rc::clone(&selected);
    let win_r = window.clone();
    rename_btn.connect_clicked(move |_| {
        if let Some(ws) = sel.borrow().clone() {
            let name_entry = Entry::new();
            name_entry.set_placeholder_text(Some("new-workspace-name"));
            name_entry.set_text(&ws);
            name_entry.set_width_chars(28);
            let dialog = adw::MessageDialog::new(
                Some(&win_r),
                Some("Rename Workspace"),
                Some(&format!("Enter a new name for \"{ws}\".")),
            );
            dialog.set_extra_child(Some(&name_entry));
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("rename", "Rename");
            dialog.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
            dialog.set_default_response(Some("rename"));
            dialog.set_close_response("cancel");
            let entry_c = name_entry.clone();
            dialog.connect_response(None, move |_, response| {
                if response == "rename" {
                    let new_name = entry_c.text().to_string();
                    if !new_name.trim().is_empty() && new_name != ws {
                        spawn_terminal_command(&format!(
                            "linux-conductor workspace rename {ws} {new_name}"
                        ));
                    }
                }
            });
            dialog.present();
            name_entry.select_region(0, -1);
        }
    });

    let copy_path_btn = Button::with_label("⎘ Path");
    copy_path_btn.set_tooltip_text(Some("Copy workspace path to clipboard"));
    let sel = Rc::clone(&selected);
    let db_cp = paths.database_path.clone();
    let toasts_cp = toasts.clone();
    copy_path_btn.connect_clicked(move |_| {
        if let Some(ws) = sel.borrow().clone() {
            if let Ok(store) = WorkspaceStore::open(db_cp.clone()) {
                if let Ok(path) = store.workspace_path(&ws) {
                    if let Some(display) = gtk::gdk::Display::default() {
                        let path_str = path.display().to_string();
                        display.clipboard().set_text(&path_str);
                        toasts_cp.add_toast(adw::Toast::new("Path copied to clipboard"));
                    }
                }
            }
        }
    });

    // Spacer between workflow and destructive buttons
    let spacer = Label::new(None);
    spacer.set_hexpand(true);

    toolbar.append(&run_btn);
    toolbar.append(&stop_btn);
    toolbar.append(&editor_btn);
    toolbar.append(&copy_path_btn);
    toolbar.append(&Separator::new(Orientation::Vertical));
    toolbar.append(&pr_btn);
    toolbar.append(&merge_btn);
    toolbar.append(&Separator::new(Orientation::Vertical));
    toolbar.append(&rename_btn);
    toolbar.append(&spacer);
    toolbar.append(&restore_btn);
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

    // Archived workspace banner (hidden by default)
    let archive_banner = Label::new(Some(
        "⚠ This workspace is archived. Click ↺ Restore to reactivate it.",
    ));
    archive_banner.add_css_class("archive-banner");
    archive_banner.set_xalign(0.0);
    archive_banner.set_margin_start(16);
    archive_banner.set_margin_end(16);
    archive_banner.set_margin_top(6);
    archive_banner.set_margin_bottom(6);
    archive_banner.set_visible(false);
    center.append(&archive_banner);

    // Info scroll area
    let info_scroll = ScrolledWindow::new();
    info_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    info_scroll.set_vexpand(true);

    let info_box = GBox::new(Orientation::Vertical, 12);
    info_box.set_margin_start(20);
    info_box.set_margin_end(20);
    info_box.set_margin_top(20);
    info_box.set_margin_bottom(20);

    // Quick stats strip — run · sessions · PR · todos
    let stats_box = GBox::new(Orientation::Horizontal, 16);
    stats_box.add_css_class("quick-stats");
    stats_box.set_margin_bottom(4);

    let run_stat = Label::new(Some("■ Stopped"));
    run_stat.add_css_class("stat-stopped");
    run_stat.set_xalign(0.0);

    let sess_stat = Label::new(Some("0 sessions"));
    sess_stat.add_css_class("stat-dim");

    let pr_stat = Label::new(Some("no PR"));
    pr_stat.add_css_class("stat-dim");

    let todo_stat = Label::new(Some("0 todos"));
    todo_stat.add_css_class("stat-dim");

    stats_box.append(&run_stat);
    stats_box.append(&Separator::new(Orientation::Vertical));
    stats_box.append(&sess_stat);
    stats_box.append(&Separator::new(Orientation::Vertical));
    stats_box.append(&pr_stat);
    stats_box.append(&Separator::new(Orientation::Vertical));
    stats_box.append(&todo_stat);

    info_box.append(&stats_box);
    info_box.append(&Separator::new(Orientation::Horizontal));

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

    let status_section_title = Label::new(Some("Repository Overview"));
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
    let toasts_send = toasts.clone();
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
                toasts_send.add_toast(adw::Toast::new("Prompt saved to agent-notes.md"));
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
    let restore_btn_clone = restore_btn.clone();
    let archive_btn_clone = archive_btn.clone();
    let archive_banner_clone = archive_banner.clone();
    let run_stat_clone = run_stat.clone();
    let sess_stat_clone = sess_stat.clone();
    let pr_stat_clone = pr_stat.clone();
    let todo_stat_clone = todo_stat.clone();
    let db_path2 = paths.database_path.clone();
    let db_path3 = paths.database_path.clone();

    let refresh = move || {
        // Update workspace title and path subtitle
        let ws_name = sel_clone.borrow().clone();
        let title_text = ws_name
            .as_deref()
            .map(|n| format!("▶ {n}"))
            .unwrap_or_else(|| "Select a workspace".to_owned());
        ws_title_clone.set_text(&title_text);

        // Show Restore for archived workspaces, Archive for active
        let is_archived = ws_name
            .as_deref()
            .and_then(|n| {
                WorkspaceStore::open(db_path2.clone())
                    .ok()
                    .and_then(|store| store.list_status().ok())
                    .and_then(|lines| lines.into_iter().find(|l| l.workspace.name == n))
                    .map(|l| l.workspace.status == "archived")
            })
            .unwrap_or(false);
        restore_btn_clone.set_visible(is_archived);
        archive_btn_clone.set_visible(!is_archived);
        archive_banner_clone.set_visible(is_archived);

        // Update quick stats strip
        if let Some(n) = ws_name.as_deref() {
            if let Some(line) = WorkspaceStore::open(db_path3.clone())
                .ok()
                .and_then(|store| store.list_status().ok())
                .and_then(|lines| lines.into_iter().find(|l| l.workspace.name == n))
            {
                if line.run_running {
                    run_stat_clone.set_text("▶ Running");
                    run_stat_clone.set_css_classes(&["stat-running"]);
                } else {
                    run_stat_clone.set_text("■ Stopped");
                    run_stat_clone.set_css_classes(&["stat-stopped"]);
                }
                sess_stat_clone.set_text(&format!("{} session(s)", line.active_sessions));
                pr_stat_clone.set_text(
                    &line
                        .pull_request
                        .as_ref()
                        .map(|p| format!("PR #{} ({})", p.number, p.state))
                        .unwrap_or_else(|| "no PR".to_owned()),
                );
                todo_stat_clone.set_text(&format!("{} todo(s)", line.open_todos));
            }
        } else {
            run_stat_clone.set_text("■ Stopped");
            run_stat_clone.set_css_classes(&["stat-stopped"]);
            sess_stat_clone.set_text("0 sessions");
            pr_stat_clone.set_text("no PR");
            todo_stat_clone.set_text("0 todos");
        }

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

        // Refresh status grid (siblings in same repo)
        while let Some(child) = status_container_clone.first_child() {
            status_container_clone.remove(&child);
        }
        populate_status_grid(&status_container_clone, &db_path, ws_name.as_deref());
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

fn populate_status_grid(
    container: &GBox,
    db_path: &std::path::PathBuf,
    selected_name: Option<&str>,
) {
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
            // Find the repo name of the selected workspace to filter siblings
            let selected_repo = selected_name.and_then(|sel| {
                statuses
                    .iter()
                    .find(|l| l.workspace.name == sel)
                    .map(|l| l.repository_name.clone())
            });
            let visible: Vec<_> = statuses
                .iter()
                .filter(|l| match &selected_repo {
                    Some(repo) => &l.repository_name == repo,
                    None => true,
                })
                .collect();
            if visible.is_empty() {
                let lbl = Label::new(Some("No other workspaces in this repository."));
                lbl.add_css_class("info-text");
                lbl.set_xalign(0.0);
                container.append(&lbl);
                return;
            }
            for line in &visible {
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
        "Opens in your default terminal emulator with workspace environment variables set.",
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
            spawn_terminal_command(&format!("linux-conductor session open {ws} --kind shell"));
        }
    });

    let codex_btn = Button::with_label("Codex");
    let sel = Rc::clone(&selected);
    codex_btn.connect_clicked(move |_| {
        if let Some(ws) = sel.borrow().clone() {
            spawn_terminal_command(&format!("linux-conductor session open {ws} --kind codex"));
        }
    });

    let claude_btn = Button::with_label("Claude Code");
    let sel = Rc::clone(&selected);
    claude_btn.connect_clicked(move |_| {
        if let Some(ws) = sel.borrow().clone() {
            spawn_terminal_command(&format!("linux-conductor session open {ws} --kind claude"));
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
    let pr_view_btn = Button::with_label("👁 PR View");
    pr_view_btn.add_css_class("pill-button");
    checks_btn_bar.append(&refresh_pr_btn);
    checks_btn_bar.append(&sync_pr_btn);
    checks_btn_bar.append(&pr_view_btn);
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

    // Sessions page
    let sessions_box = GBox::new(Orientation::Vertical, 8);
    sessions_box.set_margin_start(12);
    sessions_box.set_margin_end(12);
    sessions_box.set_margin_top(12);
    let sessions_scroll = ScrolledWindow::new();
    sessions_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    sessions_scroll.set_child(Some(&sessions_box));
    sessions_scroll.set_vexpand(true);
    stack.add_titled(&sessions_scroll, Some("sessions"), "Sessions");

    // Review comments page
    let review_box = GBox::new(Orientation::Vertical, 8);
    review_box.set_margin_start(12);
    review_box.set_margin_end(12);
    review_box.set_margin_top(12);
    let review_scroll = ScrolledWindow::new();
    review_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    review_scroll.set_child(Some(&review_box));
    review_scroll.set_vexpand(true);
    // Review outer with add-comment form at bottom
    let review_outer = GBox::new(Orientation::Vertical, 0);
    review_outer.append(&review_scroll);
    review_outer.append(&Separator::new(Orientation::Horizontal));
    let review_add_file = Entry::new();
    review_add_file.set_placeholder_text(Some("file path…"));
    review_add_file.set_hexpand(true);
    let review_add_body = Entry::new();
    review_add_body.set_placeholder_text(Some("Comment body…"));
    review_add_body.set_hexpand(true);
    let review_add_btn = Button::with_label("Add Comment");
    review_add_btn.add_css_class("suggested-action");
    let review_add_row = GBox::new(Orientation::Vertical, 4);
    review_add_row.set_margin_start(12);
    review_add_row.set_margin_end(12);
    review_add_row.set_margin_top(6);
    review_add_row.set_margin_bottom(6);
    let review_file_row = GBox::new(Orientation::Horizontal, 6);
    review_file_row.append(&review_add_file);
    let review_body_row = GBox::new(Orientation::Horizontal, 6);
    review_body_row.append(&review_add_body);
    review_body_row.append(&review_add_btn);
    review_add_row.append(&review_file_row);
    review_add_row.append(&review_body_row);
    review_outer.append(&review_add_row);
    stack.add_titled(&review_outer, Some("review"), "Review");

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
    let sessions_box_clone = sessions_box.clone();
    let review_box_clone = review_box.clone();
    let checkpoints_box_clone = checkpoints_box.clone();
    let stack_clone = stack.clone();
    let todos_outer_clone = todos_outer.clone();
    let review_outer_clone = review_outer.clone();
    let sessions_scroll_clone = sessions_scroll.clone();
    let db_path2 = db_path.clone();
    let db_path3 = db_path.clone();
    let db_path4 = db_path.clone();
    let db_path5 = db_path.clone();
    let db_path6 = db_path.clone();
    let db_path7 = db_path.clone();
    let db_path8 = db_path.clone();

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

    // Wire up "👁 PR View" — calls store.pull_request (gh pr view) and appends output
    {
        let sel = Rc::clone(&selected);
        let buf = checks_view.buffer();
        let db = db_path7.clone();
        pr_view_btn.connect_clicked(move |_| {
            if let Some(ws_name) = sel.borrow().clone() {
                if let Ok(store) = WorkspaceStore::open(db.clone()) {
                    let output = match store.pull_request(&ws_name) {
                        Ok(Some(pr)) => format!(
                            "PR #{} ({})\n{}\nCreated: {}\nUpdated: {}",
                            pr.number, pr.state, pr.url, pr.created_at, pr.updated_at
                        ),
                        Ok(None) => "No PR found for this workspace branch.".to_owned(),
                        Err(e) => format!("Error: {e}"),
                    };
                    let current = buf.text(&buf.start_iter(), &buf.end_iter(), false);
                    buf.set_text(&format!("{current}\n── PR View ──\n\n{output}\n"));
                }
            }
        });
    }

    // Wire up "Add Comment" in Review tab
    {
        let sel = Rc::clone(&selected);
        let file_entry = review_add_file.clone();
        let body_entry = review_add_body.clone();
        let review_box_ref = review_box.clone();
        let db = db_path8.clone();
        let do_add = Rc::new(move || {
            let file = file_entry.text().to_string();
            let body = body_entry.text().to_string();
            if file.trim().is_empty() || body.trim().is_empty() {
                return;
            }
            if let Some(ws_name) = sel.borrow().clone() {
                if let Ok(store) = WorkspaceStore::open(db.clone()) {
                    if store
                        .add_review_comment(&ws_name, &file, None, &body)
                        .is_ok()
                    {
                        file_entry.set_text("");
                        body_entry.set_text("");
                        while let Some(child) = review_box_ref.first_child() {
                            review_box_ref.remove(&child);
                        }
                        populate_review_box(&review_box_ref, &db, Some(&ws_name));
                    }
                }
            }
        });
        let d1 = do_add.clone();
        let d2 = do_add.clone();
        review_add_btn.connect_clicked(move |_| d1());
        review_add_body.connect_activate(move |_| d2());
    }

    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        let ws_name = sel.borrow().clone();

        // Diff
        let diff_text = build_diff_text(&db_path, ws_name.as_deref());
        apply_colored_diff(&diff_buf, &diff_text);

        // Checks (with color tags)
        let checks_text = build_checks_text(&db_path, ws_name.as_deref());
        apply_colored_checks(&checks_buf, &checks_text);

        // Todos (with live count in tab label)
        while let Some(child) = todos_box_clone.first_child() {
            todos_box_clone.remove(&child);
        }
        let title = Label::new(Some("── Open Todos ──"));
        title.set_xalign(0.0);
        todos_box_clone.append(&title);
        populate_todos_box(&todos_box_clone, &db_path, ws_name.as_deref());
        // Count open todos for tab label
        let open_todo_count: usize = ws_name
            .as_deref()
            .and_then(|n| {
                WorkspaceStore::open(db_path.clone())
                    .ok()
                    .and_then(|store| store.list_todos(n).ok())
                    .map(|todos| todos.iter().filter(|t| t.status == "open").count())
            })
            .unwrap_or(0);
        let todos_page = stack_clone.page(&todos_outer_clone);
        if open_todo_count > 0 {
            todos_page.set_title(&format!("Todos ({open_todo_count})"));
        } else {
            todos_page.set_title("Todos");
        }

        // Sessions (with count in tab label)
        while let Some(child) = sessions_box_clone.first_child() {
            sessions_box_clone.remove(&child);
        }
        populate_sessions_box(&sessions_box_clone, &db_path, ws_name.as_deref());
        let active_sess_count: usize = ws_name
            .as_deref()
            .and_then(|n| {
                WorkspaceStore::open(db_path.clone())
                    .ok()
                    .and_then(|store| store.list_status().ok())
                    .and_then(|lines| lines.into_iter().find(|l| l.workspace.name == n))
                    .map(|l| l.active_sessions)
            })
            .unwrap_or(0);
        let sessions_page = stack_clone.page(&sessions_scroll_clone);
        if active_sess_count > 0 {
            sessions_page.set_title(&format!("Sessions ({active_sess_count})"));
        } else {
            sessions_page.set_title("Sessions");
        }

        // Review comments (with open count in tab label)
        while let Some(child) = review_box_clone.first_child() {
            review_box_clone.remove(&child);
        }
        populate_review_box(&review_box_clone, &db_path, ws_name.as_deref());
        let open_review_count: usize = ws_name
            .as_deref()
            .and_then(|n| {
                WorkspaceStore::open(db_path.clone())
                    .ok()
                    .and_then(|store| store.list_review_comments(n).ok())
                    .map(|comments| comments.iter().filter(|c| c.status == "open").count())
            })
            .unwrap_or(0);
        let review_page = stack_clone.page(&review_outer_clone);
        if open_review_count > 0 {
            review_page.set_title(&format!("Review ({open_review_count})"));
        } else {
            review_page.set_title("Review");
        }

        // Checkpoints
        while let Some(child) = checkpoints_box_clone.first_child() {
            checkpoints_box_clone.remove(&child);
        }
        populate_checkpoints_box(&checkpoints_box_clone, &db_path, ws_name.as_deref());

        // Logs — set text and scroll to bottom
        let logs_text = build_logs_text(&logs_dir, ws_name.as_deref());
        logs_buf.set_text(&logs_text);
        let end = logs_buf.end_iter();
        logs_buf.place_cursor(&end);
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
            // git diff --stat summary (files changed, insertions, deletions)
            if let Ok(ws_path) = store.workspace_path(name) {
                let stat_out = std::process::Command::new("git")
                    .args(["diff", "--stat"])
                    .current_dir(&ws_path)
                    .output()
                    .ok()
                    .and_then(|o| String::from_utf8(o.stdout).ok())
                    .unwrap_or_default();
                let stat = stat_out.trim();
                if !stat.is_empty() {
                    text.push_str(&format!("\n{stat}\n"));
                }
            }
            // Recent commits on the branch
            if let Ok(log) = store.git_log_oneline(name, 10) {
                let log = log.trim();
                if !log.is_empty() {
                    text.push_str("\n── Recent Commits ──\n\n");
                    text.push_str(log);
                    text.push('\n');
                }
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

fn apply_colored_diff(buf: &gtk::TextBuffer, text: &str) {
    buf.set_text(text);

    let ensure_tag = |name: &str, fg: &str, weight: Option<i32>| -> gtk::TextTag {
        let table = buf.tag_table();
        if let Some(t) = table.lookup(name) {
            return t;
        }
        let tag = gtk::TextTag::new(Some(name));
        tag.set_foreground(Some(fg));
        if let Some(w) = weight {
            tag.set_weight(w);
        }
        table.add(&tag);
        tag
    };

    let add_tag = ensure_tag("diff-add", "#a6e3a1", None);
    let del_tag = ensure_tag("diff-del", "#f38ba8", None);
    let hunk_tag = ensure_tag("diff-hunk", "#89b4fa", Some(700));
    let header_tag = ensure_tag("diff-header", "#cba6f7", Some(700));
    let section_tag = ensure_tag("diff-section", "#f9e2af", Some(700));
    let meta_tag = ensure_tag("diff-meta", "#6c7086", None);

    for (line_num, line) in text.lines().enumerate() {
        let tag = if line.starts_with('+') && !line.starts_with("+++") {
            Some(&add_tag)
        } else if line.starts_with('-') && !line.starts_with("---") {
            Some(&del_tag)
        } else if line.starts_with("@@") {
            Some(&hunk_tag)
        } else if line.starts_with("diff --git")
            || line.starts_with("+++")
            || line.starts_with("---")
        {
            Some(&header_tag)
        } else if line.starts_with("──") {
            Some(&section_tag)
        } else if line.starts_with("M ") || line.starts_with(" M") || line.starts_with("??") {
            Some(&meta_tag)
        } else {
            None
        };

        if let Some(t) = tag {
            let line_n = line_num as i32;
            if let Some(start) = buf.iter_at_line(line_n) {
                if let Some(end) = buf.iter_at_line(line_n + 1) {
                    buf.apply_tag(t, &start, &end);
                } else {
                    let end = buf.end_iter();
                    buf.apply_tag(t, &start, &end);
                }
            }
        }
    }
}

fn apply_colored_checks(buf: &gtk::TextBuffer, text: &str) {
    buf.set_text(text);

    let ensure_tag = |name: &str, fg: &str| -> gtk::TextTag {
        let table = buf.tag_table();
        if let Some(t) = table.lookup(name) {
            return t;
        }
        let tag = gtk::TextTag::new(Some(name));
        tag.set_foreground(Some(fg));
        table.add(&tag);
        tag
    };

    let green = ensure_tag("checks-green", "#a6e3a1");
    let red = ensure_tag("checks-red", "#f38ba8");
    let blue = ensure_tag("checks-blue", "#89b4fa");
    let yellow = ensure_tag("checks-yellow", "#f9e2af");
    let dim = ensure_tag("checks-dim", "#6c7086");

    for (line_num, line) in text.lines().enumerate() {
        let tag = if line.contains("▶ running") || line.contains("Run:      ▶") {
            Some(&green)
        } else if line.contains("Error:") || line.contains("conflict") || line.contains("■ stopped")
        {
            Some(&red)
        } else if line.starts_with("PR:")
            || line.starts_with("Branch:")
            || line.starts_with("Workspace:")
        {
            Some(&blue)
        } else if line.contains("ahead") || line.contains("behind") || line.contains("no PR") {
            Some(&yellow)
        } else if line.starts_with("Push:") || line.starts_with("Changed:") {
            Some(&dim)
        } else {
            None
        };

        if let Some(t) = tag {
            let line_n = line_num as i32;
            if let Some(start) = buf.iter_at_line(line_n) {
                if let Some(end) = buf.iter_at_line(line_n + 1) {
                    buf.apply_tag(t, &start, &end);
                } else {
                    let end = buf.end_iter();
                    buf.apply_tag(t, &start, &end);
                }
            }
        }
    }
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

fn populate_sessions_box(container: &GBox, db_path: &std::path::PathBuf, ws_name: Option<&str>) {
    use linux_conductor_core::workspace::ProcessStatus;

    let title = Label::new(Some("── Sessions & Runs ──"));
    title.set_xalign(0.0);
    container.append(&title);

    let Some(name) = ws_name else {
        let lbl = Label::new(Some("Select a workspace to view its sessions."));
        lbl.add_css_class("info-text");
        lbl.set_xalign(0.0);
        container.append(&lbl);
        return;
    };

    if let Ok(store) = WorkspaceStore::open(db_path.clone()) {
        let sessions = store.list_sessions(name).unwrap_or_default();
        let runs = store.list_runs(name).unwrap_or_default();

        if sessions.is_empty() && runs.is_empty() {
            let lbl = Label::new(Some(
                "No sessions or runs yet.\nUse the session launchers above.",
            ));
            lbl.add_css_class("info-text");
            lbl.set_xalign(0.0);
            lbl.set_wrap(true);
            container.append(&lbl);
            return;
        }

        for rec in sessions.iter().chain(runs.iter()).take(15) {
            let row = GBox::new(Orientation::Horizontal, 8);
            let is_running = rec.status == ProcessStatus::Running;

            let kind_dot = Label::new(Some(if is_running { "▶" } else { "■" }));
            kind_dot.add_css_class(if is_running {
                "run-dot-active"
            } else {
                "run-dot"
            });

            use linux_conductor_core::workspace::ProcessKind;
            let kind_name = match rec.kind {
                ProcessKind::Session => "session",
                ProcessKind::Run => "run",
            };
            let kind_text = format!("{kind_name} pid:{}", rec.pid);
            let kind_lbl = Label::new(Some(&kind_text));
            kind_lbl.add_css_class("workspace-name");
            kind_lbl.set_xalign(0.0);
            kind_lbl.set_hexpand(true);

            let started_lbl = Label::new(Some(&rec.started_at));
            started_lbl.add_css_class("workspace-meta");

            row.append(&kind_dot);
            row.append(&kind_lbl);
            row.append(&started_lbl);

            let logs_btn = Button::with_label("📋 Logs");
            logs_btn.add_css_class("flat");
            let ws_log = name.to_owned();
            let log_kind = kind_name;
            logs_btn.connect_clicked(move |_| {
                let flag = if log_kind == "run" {
                    "--run"
                } else {
                    "--session"
                };
                spawn_terminal_command(&format!("linux-conductor logs {ws_log} {flag}"));
            });
            row.append(&logs_btn);

            if is_running {
                let stop_btn = Button::with_label("■ Stop");
                stop_btn.add_css_class("flat");
                let ws = name.to_owned();
                stop_btn.connect_clicked(move |_| {
                    spawn_terminal_command(&format!("linux-conductor stop {ws}"));
                });
                row.append(&stop_btn);
            }

            container.append(&row);
        }
    }
}

// ── HELPERS ───────────────────────────────────────────────────────────────

fn spawn_terminal_command(cmd: &str) {
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
