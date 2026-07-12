use adw::ApplicationWindow;
use gtk::prelude::*;
use gtk::{
    Align, Box as GBox, Button, Entry, EventControllerKey, EventControllerMotion, GestureClick,
    Image, Label, ListBox, ListBoxRow, Orientation, PolicyType, Popover, Revealer,
    RevealerTransitionType, ScrolledWindow, Spinner, Stack,
};
use linux_archductor_core::archcar::protocol::ArchcarRequest;
use linux_archductor_core::repository::RepositoryStore;
use linux_archductor_core::workspace::{CreateWorkspace, SessionKind, WorkspaceStore};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use tracing::error;

use crate::archcar_async::{spawn_archcar_request, spawn_background_job};
use crate::buttons::{icon_button, menu_text_button, resolve_icon_name, text_button};
use crate::projects::show_project_creation_popover;
use crate::refresh::{RefreshHub, RefreshScope};
use crate::state::{AppPage, AppState, WorkspaceTab};
use crate::title_case_workspace;
use crate::toast::{surface_label_error, ToastManager};

pub(crate) fn build_app_sidebar(
    app_state: &AppState,
    refresh_hub: RefreshHub,
    stack: Stack,
    window: ApplicationWindow,
    split: adw::OverlaySplitView,
    debug_mode: bool,
    refresh_workspace: impl Fn() + Clone + 'static,
    refresh_view_preferences: Rc<dyn Fn()>,
    toast_manager: ToastManager,
) -> (GBox, impl Fn() + Clone + 'static) {
    let app_state = app_state.clone();
    let sidebar_box = GBox::new(Orientation::Vertical, 0);
    sidebar_box.add_css_class("sidebar");

    let chrome_row = GBox::new(Orientation::Horizontal, 4);
    chrome_row.add_css_class("sidebar-chrome");

    let chrome_left = GBox::new(Orientation::Horizontal, 4);
    chrome_left.set_hexpand(true);
    let chrome_right = GBox::new(Orientation::Horizontal, 4);

    let close_btn = sidebar_window_button("window-close-symbolic", "Close window");
    {
        let window = window.clone();
        close_btn.connect_clicked(move |_| window.close());
    }
    chrome_left.append(&close_btn);

    let minimize_btn = sidebar_window_button("window-minimize-symbolic", "Minimize window");
    {
        let window = window.clone();
        minimize_btn.connect_clicked(move |_| {
            window.minimize();
        });
    }
    chrome_left.append(&minimize_btn);

    let expand_btn =
        sidebar_window_button("window-maximize-symbolic", "Maximize or restore window");
    {
        let window = window.clone();
        expand_btn.connect_clicked(move |_| {
            if window.is_maximized() {
                window.unmaximize();
            } else {
                window.maximize();
            }
        });
    }
    chrome_left.append(&expand_btn);

    let sidebar_toggle_btn = sidebar_icon_button("sidebar-show-symbolic", "Hide sidebar");
    {
        let split = split.clone();
        sidebar_toggle_btn.connect_clicked(move |_| {
            split.set_collapsed(true);
        });
    }
    chrome_right.append(&sidebar_toggle_btn);

    let back_btn = sidebar_arrow_button("go-previous-symbolic", "Back");
    let forward_btn = sidebar_arrow_button("go-next-symbolic", "Forward");
    chrome_right.append(&back_btn);
    chrome_right.append(&forward_btn);
    chrome_row.append(&chrome_left);
    chrome_row.append(&chrome_right);
    sidebar_box.append(&chrome_row);

    let sync_nav_buttons = {
        let app_state = app_state.clone();
        let back_btn = back_btn.clone();
        let forward_btn = forward_btn.clone();
        Rc::new(move || {
            back_btn.set_sensitive(app_state.can_navigate_back());
            forward_btn.set_sensitive(app_state.can_navigate_forward());
        })
    };
    sync_nav_buttons();

    let nav_group = GBox::new(Orientation::Vertical, 0);
    nav_group.add_css_class("sidebar-nav-group");

    // Dashboard nav item
    let dashboard_nav_btn = sidebar_nav_button("go-home-symbolic", "Dashboard");
    {
        let stack_d = stack.clone();
        let state_d = app_state.clone();
        let refresh_hub = refresh_hub.clone();
        let sync_nav_buttons = sync_nav_buttons.clone();
        dashboard_nav_btn.connect_clicked(move |_| {
            state_d.navigate_to_page(AppPage::Dashboard);
            stack_d.set_visible_child_name("dashboard");
            refresh_hub.refresh(RefreshScope::Sidebar);
            sync_nav_buttons();
        });
    }
    nav_group.append(&dashboard_nav_btn);

    // History nav item
    let history_nav_btn = sidebar_nav_button("view-list-symbolic", "History");
    {
        let stack_h = stack.clone();
        let state_h = app_state.clone();
        let refresh_hub = refresh_hub.clone();
        let sync_nav_buttons = sync_nav_buttons.clone();
        history_nav_btn.connect_clicked(move |_| {
            state_h.navigate_to_page(AppPage::History);
            stack_h.set_visible_child_name("history");
            refresh_hub.refresh(RefreshScope::Sidebar);
            sync_nav_buttons();
        });
    }
    nav_group.append(&history_nav_btn);
    if debug_mode {
        let pty_inspector_nav_btn =
            sidebar_nav_button("utilities-terminal-symbolic", "PTY Inspector");
        {
            let stack_p = stack.clone();
            let state_p = app_state.clone();
            let refresh_hub = refresh_hub.clone();
            let sync_nav_buttons = sync_nav_buttons.clone();
            pty_inspector_nav_btn.connect_clicked(move |_| {
                state_p.navigate_to_page(AppPage::PtyInspector);
                stack_p.set_visible_child_name("pty-inspector");
                refresh_hub.refresh(RefreshScope::Sidebar);
                sync_nav_buttons();
            });
        }
        nav_group.append(&pty_inspector_nav_btn);
    }
    sidebar_box.append(&nav_group);

    sidebar_box.append(&gtk::Separator::new(Orientation::Horizontal));

    // Workspaces header with filter + add buttons
    let projects_header = GBox::new(Orientation::Horizontal, 8);
    projects_header.add_css_class("projects-header");
    let header_lbl = Label::new(Some("Projects"));
    header_lbl.add_css_class("sidebar-header");
    header_lbl.set_xalign(0.0);
    header_lbl.set_hexpand(true);
    projects_header.append(&header_lbl);
    let add_workspace_btn =
        sidebar_icon_button("document-new-symbolic", "Add repository or workspace");
    {
        let db_path_hdr = app_state.workspace_database_path();
        let hub_hdr = refresh_hub.clone();
        let rw_hdr = refresh_workspace.clone();
        let rvp_hdr = refresh_view_preferences.clone();
        let toast_hdr = toast_manager.clone();
        add_workspace_btn.connect_clicked(move |button| {
            show_project_creation_popover(
                button,
                db_path_hdr.clone(),
                Rc::new({
                    let hub_hdr = hub_hdr.clone();
                    let rw_hdr = rw_hdr.clone();
                    let rvp_hdr = rvp_hdr.clone();
                    move || {
                        hub_hdr.refresh(RefreshScope::Projects);
                        hub_hdr.refresh(RefreshScope::Sidebar);
                        rw_hdr();
                        rvp_hdr();
                    }
                }),
                toast_hdr.clone(),
            );
        });
    }
    projects_header.append(&add_workspace_btn);
    sidebar_box.append(&projects_header);

    // Minimal search bar
    let search_entry = Entry::new();
    search_entry.set_placeholder_text(Some("Filter workspaces..."));
    search_entry.add_css_class("sidebar-search-minimal");
    search_entry.set_margin_start(0);
    search_entry.set_margin_end(0);
    search_entry.set_margin_bottom(0);
    sidebar_box.append(&search_entry);

    let filter_btn = sidebar_icon_button("view-filter-symbolic", "Filter workspaces");
    {
        let search_entry = search_entry.clone();
        filter_btn.connect_clicked(move |_| {
            search_entry.grab_focus();
        });
    }
    projects_header.insert_child_after(&filter_btn, Some(&header_lbl));

    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    scroll.set_vexpand(true);

    let list = ListBox::new();
    list.add_css_class("workspace-list");
    list.set_selection_mode(gtk::SelectionMode::Single);
    let names: Rc<RefCell<HashMap<i32, String>>> = Rc::new(RefCell::new(HashMap::new()));
    let pending_workspace_creates: Rc<RefCell<HashSet<String>>> =
        Rc::new(RefCell::new(HashSet::new()));
    let db_path = app_state.workspace_database_path();
    let db_path_populate = db_path.clone();

    let populate = {
        let list = list.clone();
        let names = Rc::clone(&names);
        let state = app_state.clone();
        let app_state = app_state.clone();
        let search_entry = search_entry.clone();
        let refresh_hub = refresh_hub.clone();
        let refresh_workspace = refresh_workspace.clone();
        let refresh_view_preferences = refresh_view_preferences.clone();
        let stack = stack.clone();
        let sync_nav_buttons = sync_nav_buttons.clone();
        let db_path_populate = db_path_populate.clone();
        let pending_workspace_creates = Rc::clone(&pending_workspace_creates);
        let toast_populate = toast_manager.clone();
        move || {
            sync_nav_buttons();
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            names.borrow_mut().clear();
            let filter = search_entry.text().to_string().to_lowercase();
            let prev_selected = state.selected_workspace();
            let mut row_idx = 0;

            if let (Ok(repo_store), Ok(workspace_store)) = (
                RepositoryStore::open(db_path_populate.clone()),
                WorkspaceStore::open(db_path_populate.clone()),
            ) {
                let repositories = repo_store.list().unwrap_or_default();
                let statuses = workspace_store.list_status().unwrap_or_default();
                let mut grouped: HashMap<String, Vec<_>> = HashMap::new();

                for line in statuses {
                    if line.workspace.status == "archived" {
                        continue;
                    }
                    grouped
                        .entry(line.repository_name.clone())
                        .or_default()
                        .push(line);
                }

                for repo in repositories {
                    let repo_name = repo.name;
                    let repo_matches =
                        filter.is_empty() || repo_name.to_lowercase().contains(&filter);
                    let mut lines = grouped.remove(&repo_name).unwrap_or_default();
                    lines.retain(|line| {
                        let ws = &line.workspace;
                        filter.is_empty()
                            || repo_matches
                            || ws.name.to_lowercase().contains(&filter)
                            || ws.branch.to_lowercase().contains(&filter)
                    });

                    if !repo_matches && lines.is_empty() {
                        continue;
                    }

                    let create_pending = pending_workspace_creates.borrow().contains(&repo_name);
                    let header_row = section_header_row(&repo_name, lines.len(), create_pending, {
                        let db_path = db_path_populate.clone();
                        let refresh_hub = refresh_hub.clone();
                        let refresh_workspace = refresh_workspace.clone();
                        let refresh_view_preferences = refresh_view_preferences.clone();
                        let app_state = app_state.clone();
                        let stack = stack.clone();
                        let repo_name = repo_name.clone();
                        let pending_workspace_creates = Rc::clone(&pending_workspace_creates);
                        let toast_create = toast_populate.clone();
                        move |add_btn: Button| {
                            if !pending_workspace_creates
                                .borrow_mut()
                                .insert(repo_name.clone())
                            {
                                return;
                            }
                            add_btn.set_sensitive(false);
                            add_btn.set_tooltip_text(Some("Creating workspace..."));
                            let refresh_hub = refresh_hub.clone();
                            let refresh_workspace = refresh_workspace.clone();
                            let refresh_view_preferences = refresh_view_preferences.clone();
                            let app_state = app_state.clone();
                            let stack = stack.clone();
                            let pending_workspace_creates = Rc::clone(&pending_workspace_creates);
                            let repo_name_for_callback = repo_name.clone();
                            let toast_create = toast_create.clone();
                            spawn_background_job(
                                {
                                    let db_path = db_path.clone();
                                    let repo_name = repo_name.clone();
                                    move || {
                                        WorkspaceStore::open(db_path).and_then(|store| {
                                            store.create(CreateWorkspace {
                                                repository_name: repo_name,
                                                name: String::new(),
                                                branch: String::new(),
                                                base_ref: None,
                                            })
                                        })
                                    }
                                },
                                move |result| {
                                    pending_workspace_creates
                                        .borrow_mut()
                                        .remove(&repo_name_for_callback);
                                    add_btn.set_sensitive(true);
                                    add_btn.set_tooltip_text(Some("Create workspace"));
                                    match result {
                                        Ok(workspace) => {
                                            let default_tab = WorkspaceStore::open(
                                                app_state.workspace_database_path(),
                                            )
                                            .and_then(|store| {
                                                store.workspace_view_defaults(&workspace.name)
                                            })
                                            .ok()
                                            .and_then(|defaults| defaults.default_visible_tab)
                                            .and_then(|tab| WorkspaceTab::from_config(&tab));
                                            app_state.navigate_to_workspace_with_default_tab(
                                                Some(workspace.name),
                                                default_tab,
                                            );
                                            stack.set_visible_child_name("workspace");
                                            refresh_hub.refresh(RefreshScope::Projects);
                                            refresh_hub.refresh(RefreshScope::Sidebar);
                                            refresh_hub.refresh(RefreshScope::Dashboard);
                                            refresh_workspace();
                                            refresh_view_preferences();
                                        }
                                        Err(err) => toast_create
                                            .error(format!("Create workspace failed: {err:#}")),
                                    }
                                },
                            );
                        }
                    });
                    list.append(&header_row);
                    row_idx += 1;

                    if lines.is_empty() {
                        let empty_row = empty_repo_row();
                        list.append(&empty_row);
                        row_idx += 1;
                        continue;
                    }

                    for line in lines {
                        let ws = &line.workspace;
                        let row = build_workspace_row(
                            &ws.name,
                            &ws.branch,
                            &ws.status,
                            line.diff_additions,
                            line.diff_deletions,
                            &ws.updated_at,
                        );
                        if ws.status == "active" {
                            attach_workspace_row_context_menu(
                                &row,
                                ws.name.clone(),
                                app_state.clone(),
                                stack.clone(),
                                window.clone(),
                                refresh_hub.clone(),
                                refresh_workspace.clone(),
                                refresh_view_preferences.clone(),
                                toast_populate.clone(),
                            );
                            names.borrow_mut().insert(row_idx, ws.name.clone());
                        }
                        list.append(&row);
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

            if sidebar_should_restore_workspace_selection(&state.snapshot().active_page) {
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
        }
    };

    populate();
    let pop_search = populate.clone();
    search_entry.connect_changed(move |_| pop_search());

    let names_select = Rc::clone(&names);
    let state_select = app_state.clone();
    let stack_select = stack.clone();
    let refresh_select = refresh_hub.clone();
    let db_path_select = db_path.clone();
    let refresh_view_preferences_select = refresh_view_preferences.clone();

    let refresh_workspace_select = refresh_workspace.clone();
    let archcar_paths = app_state.paths.clone();
    list.connect_row_selected(move |_, row| {
        guarded_gtk_callback((), || {
            let Some(name) = row.and_then(|r| names_select.borrow().get(&r.index()).cloned())
            else {
                return;
            };
            spawn_archcar_request(
                archcar_paths.clone(),
                ArchcarRequest::EnsureWorkspaceDefaultSession {
                    workspace: name.clone(),
                    kind: SessionKind::Codex,
                    harness: None,
                },
            );
            let default_tab = WorkspaceStore::open(db_path_select.clone())
                .and_then(|store| store.workspace_view_defaults(&name))
                .ok()
                .and_then(|defaults| defaults.default_visible_tab)
                .and_then(|tab| WorkspaceTab::from_config(&tab));
            state_select.navigate_to_workspace_with_default_tab(Some(name), default_tab);
            refresh_view_preferences_select();
            refresh_workspace_select();
            refresh_select.refresh(RefreshScope::Dashboard);
            stack_select.set_visible_child_name("workspace");
        })
    });

    scroll.set_child(Some(&list));
    sidebar_box.append(&scroll);

    // Bottom bar: Add repository + Settings
    let bottom_bar = GBox::new(Orientation::Horizontal, 4);
    bottom_bar.add_css_class("sidebar-bottom-bar");
    let spacer = GBox::new(Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    bottom_bar.append(&spacer);

    let add_repo_btn = sidebar_icon_button("folder-new-symbolic", "Add repository");
    {
        let db_path_bar = app_state.workspace_database_path();
        let hub_bar = refresh_hub.clone();
        let rw_bar = refresh_workspace.clone();
        let rvp_bar = refresh_view_preferences.clone();
        let toast_bar = toast_manager.clone();
        add_repo_btn.connect_clicked(move |button| {
            show_project_creation_popover(
                button,
                db_path_bar.clone(),
                Rc::new({
                    let hub_bar = hub_bar.clone();
                    let rw_bar = rw_bar.clone();
                    let rvp_bar = rvp_bar.clone();
                    move || {
                        hub_bar.refresh(RefreshScope::Projects);
                        hub_bar.refresh(RefreshScope::Sidebar);
                        rw_bar();
                        rvp_bar();
                    }
                }),
                toast_bar.clone(),
            );
        });
    }
    bottom_bar.append(&add_repo_btn);

    let settings_btn = sidebar_icon_button("emblem-system-symbolic", "Settings");
    {
        let stack_s = stack.clone();
        let state_s = app_state.clone();
        let refresh_hub = refresh_hub.clone();
        let sync_nav_buttons = sync_nav_buttons.clone();
        settings_btn.connect_clicked(move |_| {
            state_s.navigate_to_page(AppPage::Settings);
            stack_s.set_visible_child_name("settings");
            refresh_hub.refresh(RefreshScope::Sidebar);
            sync_nav_buttons();
        });
    }
    bottom_bar.append(&settings_btn);

    sidebar_box.append(&bottom_bar);

    {
        let state_back = app_state.clone();
        let stack_back = stack.clone();
        let refresh_back = refresh_hub.clone();
        let refresh_workspace_back = refresh_workspace.clone();
        let sync_nav_buttons = sync_nav_buttons.clone();
        back_btn.connect_clicked(move |_| {
            if !state_back.navigate_back() {
                return;
            }
            match state_back.snapshot().active_page {
                AppPage::Dashboard => stack_back.set_visible_child_name("dashboard"),
                AppPage::Projects => stack_back.set_visible_child_name("projects"),
                AppPage::Workspace => {
                    stack_back.set_visible_child_name("workspace");
                    refresh_workspace_back();
                }
                AppPage::History => stack_back.set_visible_child_name("history"),
                AppPage::PtyInspector => stack_back.set_visible_child_name("pty-inspector"),
                AppPage::Settings => stack_back.set_visible_child_name("settings"),
                AppPage::Review => stack_back.set_visible_child_name("workspace"),
            }
            refresh_back.refresh(RefreshScope::Sidebar);
            sync_nav_buttons();
        });
    }

    {
        let state_forward = app_state.clone();
        let stack_forward = stack.clone();
        let refresh_forward = refresh_hub.clone();
        let refresh_workspace_forward = refresh_workspace.clone();
        let sync_nav_buttons = sync_nav_buttons.clone();
        forward_btn.connect_clicked(move |_| {
            if !state_forward.navigate_forward() {
                return;
            }
            match state_forward.snapshot().active_page {
                AppPage::Dashboard => stack_forward.set_visible_child_name("dashboard"),
                AppPage::Projects => stack_forward.set_visible_child_name("projects"),
                AppPage::Workspace => {
                    stack_forward.set_visible_child_name("workspace");
                    refresh_workspace_forward();
                }
                AppPage::History => stack_forward.set_visible_child_name("history"),
                AppPage::PtyInspector => stack_forward.set_visible_child_name("pty-inspector"),
                AppPage::Settings => stack_forward.set_visible_child_name("settings"),
                AppPage::Review => stack_forward.set_visible_child_name("workspace"),
            }
            refresh_forward.refresh(RefreshScope::Sidebar);
            sync_nav_buttons();
        });
    }

    (sidebar_box, populate)
}

fn sidebar_icon_button(icon: &str, tooltip: &str) -> Button {
    sidebar_button(icon, tooltip, "sidebar-icon-button")
}

fn sidebar_arrow_button(icon: &str, tooltip: &str) -> Button {
    sidebar_button(icon, tooltip, "sidebar-arrow-button")
}

fn sidebar_window_button(icon: &str, tooltip: &str) -> Button {
    sidebar_button(icon, tooltip, "sidebar-window-button")
}

fn sidebar_button(icon: &str, tooltip: &str, class_name: &str) -> Button {
    let button = icon_button(icon, tooltip);
    button.add_css_class(class_name);
    button.set_tooltip_text(Some(tooltip));
    button
}

fn sidebar_nav_button(icon: &str, tooltip: &str) -> Button {
    let button = text_button(tooltip);
    button.add_css_class("sidebar-nav-button");
    button.set_tooltip_text(Some(tooltip));

    let row = GBox::new(Orientation::Horizontal, 8);
    row.set_margin_start(2);
    row.set_margin_end(2);
    row.set_margin_top(2);
    row.set_margin_bottom(2);

    let image = Image::from_icon_name(resolve_icon_name(icon));
    image.add_css_class("sidebar-nav-icon");
    row.append(&image);

    let label = Label::new(Some(tooltip));
    label.add_css_class("sidebar-nav-label");
    label.set_xalign(0.0);
    label.set_hexpand(true);
    row.append(&label);

    button.set_child(Some(&row));
    button
}

fn guarded_gtk_callback<T, F>(fallback: T, callback: F) -> T
where
    F: FnOnce() -> T,
{
    match catch_unwind(AssertUnwindSafe(callback)) {
        Ok(value) => value,
        Err(_) => {
            error!("recovered panic inside sidebar GTK callback");
            fallback
        }
    }
}

fn build_workspace_row(
    name: &str,
    branch: &str,
    status: &str,
    diff_additions: usize,
    diff_deletions: usize,
    updated_at: &str,
) -> ListBoxRow {
    let row_box = GBox::new(Orientation::Horizontal, 10);
    row_box.add_css_class("project-row");
    row_box.add_css_class("workspace-row-shell");

    let branch_icon = Image::from_icon_name(resolve_icon_name("list-drag-handle-symbolic"));
    branch_icon.add_css_class("workspace-row-branch-icon");
    row_box.append(&branch_icon);

    // Text column
    let text_box = GBox::new(Orientation::Vertical, 2);
    text_box.set_hexpand(true);

    // Top row: name
    let top_row = GBox::new(Orientation::Horizontal, 6);
    top_row.set_hexpand(true);

    let name_label = Label::new(Some(&title_case_workspace(name)));
    name_label.add_css_class("workspace-name");
    name_label.set_xalign(0.0);
    name_label.set_hexpand(true);
    name_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    top_row.append(&name_label);

    text_box.append(&top_row);

    // Second row: branch · time ago
    let mut meta_parts: Vec<String> = Vec::new();
    if !branch.is_empty() {
        meta_parts.push(branch.to_string());
    }
    if status == "creating" {
        meta_parts.push("Creating workspace...".to_owned());
    } else if status == "failed" {
        meta_parts.push("Creation failed".to_owned());
    }
    let ts = relative_time(updated_at);
    if !ts.is_empty() {
        meta_parts.push(ts);
    }
    let meta_text = meta_parts.join(" · ");
    let meta_label = Label::new(Some(&meta_text));
    meta_label.add_css_class("workspace-row-timestamp");
    meta_label.add_css_class("workspace-meta");
    meta_label.set_xalign(0.0);
    meta_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text_box.append(&meta_label);

    row_box.append(&text_box);

    let trailing_box = GBox::new(Orientation::Horizontal, 0);
    trailing_box.add_css_class("workspace-row-trailing");
    trailing_box.set_halign(Align::End);
    if status == "creating" {
        let spinner = Spinner::new();
        spinner.start();
        trailing_box.append(&spinner);
    } else if status == "failed" {
        let failed = Label::new(Some("Failed"));
        failed.add_css_class("status-error");
        trailing_box.append(&failed);
    } else {
        trailing_box.append(&workspace_diff_stats(diff_additions, diff_deletions));
    }
    row_box.append(&trailing_box);

    let row = ListBoxRow::builder().child(&row_box).build();
    if status != "active" {
        row.set_selectable(false);
        row.set_activatable(false);
    }
    row
}

fn workspace_diff_stats(additions: usize, deletions: usize) -> GBox {
    let stats = GBox::new(Orientation::Horizontal, 5);
    stats.add_css_class("workspace-row-diff-stats");
    stats.set_halign(Align::End);

    let additions_label = Label::new(Some(&format!("+ {additions}")));
    additions_label.add_css_class("workspace-row-diff-added");
    additions_label.set_xalign(1.0);
    stats.append(&additions_label);

    let deletions_label = Label::new(Some(&format!("- {deletions}")));
    deletions_label.add_css_class("workspace-row-diff-removed");
    deletions_label.set_xalign(1.0);
    stats.append(&deletions_label);

    stats
}

fn attach_workspace_row_context_menu(
    row: &ListBoxRow,
    workspace_name: String,
    state: AppState,
    stack: Stack,
    window: ApplicationWindow,
    refresh_hub: RefreshHub,
    refresh_workspace: impl Fn() + Clone + 'static,
    refresh_view_preferences: Rc<dyn Fn()>,
    toast_manager: ToastManager,
) {
    let popover = Popover::new();
    popover.add_css_class("context-menu-popover");
    popover.set_position(gtk::PositionType::Bottom);
    let menu = GBox::new(Orientation::Vertical, 4);
    menu.add_css_class("chat-menu-list");

    let rename_btn = menu_text_button("Rename");
    {
        let workspace_name = workspace_name.clone();
        let state = state.clone();
        let refresh_hub = refresh_hub.clone();
        let refresh_workspace = refresh_workspace.clone();
        let refresh_view_preferences = refresh_view_preferences.clone();
        let popover_for_item = popover.downgrade();
        let window = window.downgrade();
        let toast_manager = toast_manager.clone();
        rename_btn.connect_clicked(move |_| {
            if let Some(popover) = popover_for_item.upgrade() {
                popover.popdown();
            }
            let Some(window) = window.upgrade() else {
                return;
            };
            show_workspace_text_dialog(
                &window,
                "Rename workspace",
                "Workspace name",
                &workspace_name,
                "Rename",
                toast_manager.clone(),
                Rc::new({
                    let workspace_name = workspace_name.clone();
                    let state = state.clone();
                    let refresh_hub = refresh_hub.clone();
                    let refresh_workspace = refresh_workspace.clone();
                    let refresh_view_preferences = refresh_view_preferences.clone();
                    move |new_name| {
                        if new_name.is_empty() || new_name == workspace_name {
                            return Ok(());
                        }
                        let workspace = WorkspaceStore::open(state.workspace_database_path())
                            .and_then(|store| store.rename(&workspace_name, &new_name))
                            .map_err(|err| format!("{err:#}"))?;
                        state.rename_workspace_in_navigation(&workspace_name, &workspace.name);
                        refresh_view_preferences();
                        refresh_workspace();
                        refresh_hub.refresh(RefreshScope::All);
                        Ok(())
                    }
                }),
            );
        });
    }
    menu.append(&rename_btn);

    let duplicate_btn = menu_text_button("Duplicate");
    {
        let workspace_name = workspace_name.clone();
        let refresh_hub = refresh_hub.clone();
        let refresh_workspace = refresh_workspace.clone();
        let refresh_view_preferences = refresh_view_preferences.clone();
        let state = state.clone();
        let stack = stack.clone();
        let popover_for_item = popover.downgrade();
        let window = window.downgrade();
        let toast_manager = toast_manager.clone();
        duplicate_btn.connect_clicked(move |_| {
            if let Some(popover) = popover_for_item.upgrade() {
                popover.popdown();
            }
            let Some(window) = window.upgrade() else {
                return;
            };
            show_workspace_text_dialog(
                &window,
                "Duplicate workspace",
                "New workspace name",
                "",
                "Duplicate",
                toast_manager.clone(),
                Rc::new({
                    let workspace_name = workspace_name.clone();
                    let refresh_hub = refresh_hub.clone();
                    let refresh_workspace = refresh_workspace.clone();
                    let refresh_view_preferences = refresh_view_preferences.clone();
                    let state = state.clone();
                    let stack = stack.clone();
                    let window = window.clone();
                    let toast_manager = toast_manager.clone();
                    move |new_name| {
                        if new_name.is_empty() {
                            return Ok(());
                        }
                        let refresh_hub = refresh_hub.clone();
                        let refresh_workspace = refresh_workspace.clone();
                        let refresh_view_preferences = refresh_view_preferences.clone();
                        let state = state.clone();
                        let stack = stack.clone();
                        let window = window.clone();
                        let toast_manager = toast_manager.clone();
                        spawn_background_job(
                            {
                                let db_path = state.workspace_database_path().to_path_buf();
                                let workspace_name = workspace_name.clone();
                                let new_name = new_name.clone();
                                move || {
                                    WorkspaceStore::open(db_path)
                                        .and_then(|store| {
                                            store.duplicate(&workspace_name, &new_name, None)
                                        })
                                        .map_err(|err| format!("{err:#}"))
                                }
                            },
                            move |result| match result {
                                Ok(workspace) => {
                                    state.navigate_to_workspace(Some(workspace.name));
                                    stack.set_visible_child_name("workspace");
                                    refresh_view_preferences();
                                    refresh_workspace();
                                    refresh_hub.refresh(RefreshScope::All);
                                }
                                Err(err) => {
                                    show_workspace_error_dialog(
                                        &window,
                                        "Workspace action failed",
                                        &err,
                                        &toast_manager,
                                    );
                                }
                            },
                        );
                        Ok(())
                    }
                }),
            );
        });
    }
    menu.append(&duplicate_btn);

    for (label, destructive, action) in [("Archive", false, "archive"), ("Delete", true, "delete")]
    {
        let item = menu_text_button(label);
        if destructive {
            item.add_css_class("destructive-action");
        }
        let workspace_name = workspace_name.clone();
        let refresh_hub = refresh_hub.clone();
        let refresh_workspace = refresh_workspace.clone();
        let refresh_view_preferences = refresh_view_preferences.clone();
        let state = state.clone();
        let stack = stack.downgrade();
        let row = row.downgrade();
        let popover_for_item = popover.downgrade();
        let window = window.downgrade();
        let toast_manager = toast_manager.clone();
        item.connect_clicked(move |_| {
            if let Some(popover) = popover_for_item.upgrade() {
                popover.popdown();
            }
            let (Some(window), Some(row), Some(stack)) =
                (window.upgrade(), row.upgrade(), stack.upgrade())
            else {
                return;
            };
            let title = if action == "archive" {
                "Archive workspace"
            } else {
                "Delete workspace"
            };
            let message = if action == "archive" {
                format!("Archive {workspace_name}?")
            } else {
                format!(
                    "Delete {workspace_name}? This removes the worktree, deletes the local branch, and can discard unmerged commits."
                )
            };
            show_workspace_confirm_dialog(
                &window,
                title,
                &message,
                label,
                destructive,
                toast_manager.clone(),
                Rc::new({
                    let workspace_name = workspace_name.clone();
                    let refresh_hub = refresh_hub.clone();
                    let refresh_workspace = refresh_workspace.clone();
                    let refresh_view_preferences = refresh_view_preferences.clone();
                    let state = state.clone();
                    let stack = stack.clone();
                    let row = row.clone();
                    let window = window.clone();
                    let toast_manager = toast_manager.clone();
                    move || {
                        let refresh_hub = refresh_hub.clone();
                        let refresh_workspace = refresh_workspace.clone();
                        let refresh_view_preferences = refresh_view_preferences.clone();
                        let state = state.clone();
                        let stack = stack.clone();
                        let row = row.clone();
                        let workspace_name = workspace_name.clone();
                        let window = window.clone();
                        let toast_manager = toast_manager.clone();
                        spawn_background_job(
                            {
                                let db_path = state.workspace_database_path().to_path_buf();
                                let workspace_name = workspace_name.clone();
                                move || {
                                    WorkspaceStore::open(db_path).and_then(|store| match action {
                                        "archive" => {
                                            store.archive(&workspace_name, false).map(|_| ())
                                        }
                                        "delete" => {
                                            store.delete(&workspace_name, true, true).map(|_| ())
                                        }
                                        _ => unreachable!(),
                                    })
                                }
                            },
                            move |result| match result {
                                Ok(()) => {
                                            let snapshot = state.snapshot();
                                            let was_selected_workspace =
                                                snapshot.selected_workspace.as_deref()
                                                    == Some(workspace_name.as_str())
                                                    && matches!(
                                                        snapshot.active_page,
                                                        AppPage::Workspace | AppPage::Review
                                                    );
                                            state.remove_workspace_from_navigation(
                                                &workspace_name,
                                                AppPage::Dashboard,
                                            );
                                            if was_selected_workspace {
                                                stack.set_visible_child_name("dashboard");
                                            }
                                            if let Some(list) =
                                                row.parent().and_downcast::<ListBox>()
                                            {
                                                list.remove(&row);
                                            }
                                            refresh_view_preferences();
                                            refresh_workspace();
                                            refresh_hub.refresh(RefreshScope::All);
                                }
                                Err(err) => {
                                            refresh_view_preferences();
                                            refresh_workspace();
                                            refresh_hub.refresh(RefreshScope::Sidebar);
                                            show_workspace_error_dialog(
                                                &window,
                                                "Workspace action failed",
                                                &format!("{err:#}"),
                                                &toast_manager,
                                            );
                                }
                            },
                        );
                        Ok(())
                    }
                }),
            );
        });
        menu.append(&item);
    }

    popover.set_child(Some(&menu));
    let menu_btn = icon_button("view-more-symbolic", "Workspace actions");
    menu_btn.add_css_class("workspace-row-menu-button");
    menu_btn.set_margin_start(2);
    popover.set_parent(row);
    let menu_revealer = Revealer::new();
    menu_revealer.add_css_class("workspace-row-menu-revealer");
    menu_revealer.set_transition_type(RevealerTransitionType::SlideLeft);
    menu_revealer.set_transition_duration(140);
    menu_revealer.set_reveal_child(false);
    menu_revealer.set_child(Some(&menu_btn));
    let row_hovered = Rc::new(Cell::new(false));
    let menu_open = Rc::new(Cell::new(false));
    let sync_menu_revealer = {
        let menu_revealer = menu_revealer.clone();
        let row_hovered = row_hovered.clone();
        let menu_open = menu_open.clone();
        Rc::new(move || {
            menu_revealer.set_reveal_child(row_hovered.get() || menu_open.get());
        })
    };
    {
        let popover = popover.downgrade();
        let menu_open = menu_open.clone();
        let sync_menu_revealer = sync_menu_revealer.clone();
        let row_for_menu = row.clone();
        menu_btn.connect_clicked(move |button| {
            if let Some(popover) = popover.upgrade() {
                menu_open.set(true);
                sync_menu_revealer();
                let rect = button.compute_bounds(&row_for_menu).map(|bounds| {
                    gtk::gdk::Rectangle::new(
                        bounds.x().round() as i32,
                        bounds.y().round() as i32,
                        bounds.width().ceil().max(1.0) as i32,
                        bounds.height().ceil().max(1.0) as i32,
                    )
                });
                popover.set_pointing_to(rect.as_ref());
                popover.popup();
            }
        });
    }
    {
        let menu_open = menu_open.clone();
        let sync_menu_revealer = sync_menu_revealer.clone();
        popover.connect_closed(move |_| {
            menu_open.set(false);
            sync_menu_revealer();
        });
    }
    if let Some(row_box) = row.child().and_downcast::<GBox>() {
        if let Some(trailing_box) = row_box.last_child().and_downcast::<GBox>() {
            trailing_box.append(&menu_revealer);
        }
        let hover_controller = EventControllerMotion::new();
        {
            let row_hovered = row_hovered.clone();
            let sync_menu_revealer = sync_menu_revealer.clone();
            hover_controller.connect_enter(move |_, _, _| {
                row_hovered.set(true);
                sync_menu_revealer();
            });
        }
        {
            let row_hovered = row_hovered.clone();
            let sync_menu_revealer = sync_menu_revealer.clone();
            hover_controller.connect_leave(move |_| {
                row_hovered.set(false);
                sync_menu_revealer();
            });
        }
        row_box.add_controller(hover_controller);
    }

    let gesture = GestureClick::new();
    gesture.set_button(3);
    let popover_for_click = popover.downgrade();
    let menu_open_for_click = menu_open.clone();
    let sync_revealer_for_click = sync_menu_revealer.clone();
    gesture.connect_pressed(move |_, _, x, y| {
        let Some(popover_for_click) = popover_for_click.upgrade() else {
            return;
        };
        menu_open_for_click.set(true);
        sync_revealer_for_click();
        let rect = gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1);
        popover_for_click.set_pointing_to(Some(&rect));
        popover_for_click.popup();
    });
    row.add_controller(gesture);

    row.set_focusable(true);
    let key_controller = EventControllerKey::new();
    let popover_for_key = popover.downgrade();
    let menu_open_for_key = menu_open.clone();
    let sync_revealer_for_key = sync_menu_revealer.clone();
    key_controller.connect_key_pressed(move |_, key, _, modifiers| {
        let menu_key = key == gtk::gdk::Key::Menu;
        let shift_f10 =
            key == gtk::gdk::Key::F10 && modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK);
        if !(menu_key || shift_f10) {
            return gtk::glib::Propagation::Proceed;
        }
        if let Some(popover) = popover_for_key.upgrade() {
            menu_open_for_key.set(true);
            sync_revealer_for_key();
            popover.set_pointing_to(None);
            popover.popup();
        }
        gtk::glib::Propagation::Stop
    });
    row.add_controller(key_controller);
}
fn show_workspace_text_dialog(
    window: &ApplicationWindow,
    title: &str,
    placeholder: &str,
    initial: &str,
    action_label: &str,
    toast_manager: ToastManager,
    on_submit: Rc<dyn Fn(String) -> Result<(), String>>,
) {
    let dialog = gtk::Window::builder()
        .title(title)
        .modal(true)
        .default_width(360)
        .build();
    dialog.set_transient_for(Some(window));

    let body = GBox::new(Orientation::Vertical, 10);
    body.add_css_class("modal-body");
    body.set_margin_top(14);
    body.set_margin_bottom(14);
    body.set_margin_start(14);
    body.set_margin_end(14);

    let entry = Entry::new();
    entry.set_placeholder_text(Some(placeholder));
    entry.set_text(initial);
    entry.set_hexpand(true);
    body.append(&entry);

    let error_label = Label::new(None);
    error_label.add_css_class("status-error");
    error_label.set_xalign(0.0);
    error_label.set_wrap(true);
    error_label.set_visible(false);
    body.append(&error_label);

    let buttons = GBox::new(Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel_btn = text_button("Cancel");
    let action_btn = text_button(action_label);
    action_btn.add_css_class("suggested-action");
    {
        let dialog = dialog.clone();
        cancel_btn.connect_clicked(move |_| dialog.close());
    }
    {
        let dialog = dialog.clone();
        let entry = entry.clone();
        let error_label = error_label.clone();
        let toast_manager = toast_manager.clone();
        action_btn.connect_clicked(move |_| match on_submit(entry.text().trim().to_owned()) {
            Ok(()) => dialog.close(),
            Err(message) => {
                surface_label_error(&error_label, &toast_manager, message);
                error_label.set_visible(true);
            }
        });
    }
    buttons.append(&cancel_btn);
    buttons.append(&action_btn);
    body.append(&buttons);

    dialog.set_child(Some(&body));
    dialog.present();
    entry.grab_focus();
}

fn show_workspace_confirm_dialog(
    window: &ApplicationWindow,
    title: &str,
    message: &str,
    action_label: &str,
    destructive: bool,
    toast_manager: ToastManager,
    on_confirm: Rc<dyn Fn() -> Result<(), String>>,
) {
    let dialog = gtk::Window::builder()
        .title(title)
        .modal(true)
        .default_width(360)
        .build();
    dialog.set_transient_for(Some(window));

    let body = GBox::new(Orientation::Vertical, 10);
    body.add_css_class("modal-body");
    body.set_margin_top(14);
    body.set_margin_bottom(14);
    body.set_margin_start(14);
    body.set_margin_end(14);

    let label = Label::new(Some(message));
    label.set_xalign(0.0);
    label.set_wrap(true);
    body.append(&label);

    let error_label = Label::new(None);
    error_label.add_css_class("status-error");
    error_label.set_xalign(0.0);
    error_label.set_wrap(true);
    error_label.set_visible(false);
    body.append(&error_label);

    let buttons = GBox::new(Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel_btn = text_button("Cancel");
    let action_btn = text_button(action_label);
    action_btn.add_css_class(if destructive {
        "destructive-action"
    } else {
        "suggested-action"
    });
    {
        let dialog = dialog.clone();
        cancel_btn.connect_clicked(move |_| dialog.close());
    }
    {
        let dialog = dialog.clone();
        let error_label = error_label.clone();
        let toast_manager = toast_manager.clone();
        action_btn.connect_clicked(move |_| match on_confirm() {
            Ok(()) => dialog.close(),
            Err(message) => {
                surface_label_error(&error_label, &toast_manager, message);
                error_label.set_visible(true);
            }
        });
    }
    buttons.append(&cancel_btn);
    buttons.append(&action_btn);
    body.append(&buttons);

    dialog.set_child(Some(&body));
    dialog.present();
}

fn show_workspace_error_dialog(
    window: &ApplicationWindow,
    title: &str,
    message: &str,
    toast_manager: &ToastManager,
) {
    toast_manager.error(message.to_owned());
    let dialog = gtk::Window::builder()
        .title(title)
        .modal(true)
        .default_width(380)
        .build();
    dialog.set_transient_for(Some(window));

    let body = GBox::new(Orientation::Vertical, 10);
    body.add_css_class("modal-body");
    body.set_margin_top(14);
    body.set_margin_bottom(14);
    body.set_margin_start(14);
    body.set_margin_end(14);

    let label = Label::new(Some(message));
    label.add_css_class("status-error");
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_selectable(true);
    body.append(&label);

    let buttons = GBox::new(Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let close_btn = text_button("Close");
    {
        let dialog = dialog.clone();
        close_btn.connect_clicked(move |_| dialog.close());
    }
    buttons.append(&close_btn);
    body.append(&buttons);

    dialog.set_child(Some(&body));
    dialog.present();
}

fn primary_sidebar_nav_labels(debug_mode: bool) -> Vec<&'static str> {
    let mut labels = vec!["Dashboard", "History"];
    if debug_mode {
        labels.push("PTY Inspector");
    }
    labels
}

fn sidebar_should_restore_workspace_selection(page: &AppPage) -> bool {
    matches!(page, AppPage::Workspace | AppPage::Review)
}

fn section_header_row(
    name: &str,
    _workspace_count: usize,
    create_pending: bool,
    on_add_workspace: impl Fn(Button) + 'static,
) -> ListBoxRow {
    let shell = GBox::new(Orientation::Horizontal, 6);
    shell.add_css_class("repo-section-row");

    let repo_icon = Image::from_icon_name(resolve_icon_name("folder-symbolic"));
    repo_icon.add_css_class("repo-section-icon");
    shell.append(&repo_icon);

    let repo_lbl = Label::new(Some(name));
    repo_lbl.add_css_class("repo-section-header");
    repo_lbl.set_xalign(0.0);
    repo_lbl.set_hexpand(true);
    repo_lbl.set_ellipsize(gtk::pango::EllipsizeMode::End);
    shell.append(&repo_lbl);

    let add_btn = sidebar_icon_button("list-add-symbolic", "Create workspace");
    add_btn.add_css_class("repo-header-add");
    if create_pending {
        add_btn.set_sensitive(false);
        add_btn.set_tooltip_text(Some("Creating workspace..."));
    } else {
        add_btn.set_tooltip_text(Some("Create workspace"));
    }
    add_btn.connect_clicked({
        let add_btn = add_btn.clone();
        move |_| on_add_workspace(add_btn.clone())
    });
    shell.append(&add_btn);

    let row = ListBoxRow::builder().child(&shell).build();
    row.set_selectable(false);
    row.set_activatable(false);
    row
}

fn empty_repo_row() -> ListBoxRow {
    let empty = Label::new(Some("No workspaces"));
    empty.add_css_class("workspace-meta");
    empty.add_css_class("repo-empty-label");
    empty.set_xalign(0.0);
    let row = ListBoxRow::builder().child(&empty).build();
    row.set_selectable(false);
    row.set_activatable(false);
    row
}

fn relative_time(ts: &str) -> String {
    let ts_clean = ts.replace('T', " ");
    let parts: Vec<&str> = ts_clean.splitn(2, ' ').collect();
    if parts.len() < 2 {
        return String::new();
    }

    let date_parts: Vec<i64> = parts[0].split('-').filter_map(|p| p.parse().ok()).collect();
    let time_str = parts[1].split('.').next().unwrap_or(parts[1]);
    let time_parts: Vec<i64> = time_str.split(':').filter_map(|p| p.parse().ok()).collect();

    if date_parts.len() < 3 || time_parts.len() < 2 {
        return String::new();
    }

    let year = date_parts[0];
    let month = date_parts[1];
    let day = date_parts[2];
    let hour = time_parts[0];
    let min = time_parts[1];
    let sec = if time_parts.len() > 2 {
        time_parts[2]
    } else {
        0
    };

    let y = year - 1;
    let leap_days = y / 4 - y / 100 + y / 400;
    let days_of_month: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let month_days: i64 = days_of_month[..((month - 1) as usize)].iter().sum();
    let is_leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let leap_bonus = if is_leap && month > 2 { 1 } else { 0 };
    let epoch_days = year * 365 + leap_days - 719_527 + month_days + leap_bonus + day - 1;
    let ts_secs = epoch_days * 86400 + hour * 3600 + min * 60 + sec;

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(ts_secs);

    let delta = now_secs - ts_secs;
    if delta < 0 {
        return "just now".to_string();
    }

    match delta {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{}m ago", delta / 60),
        3600..=86399 => format!("{}h ago", delta / 3600),
        86400..=604799 => format!("{}d ago", delta / 86400),
        _ => format!("{}w ago", delta / 604800),
    }
}

#[cfg(test)]
mod tests {
    use super::{primary_sidebar_nav_labels, sidebar_should_restore_workspace_selection};
    use crate::state::AppPage;

    #[test]
    fn primary_sidebar_nav_labels_gate_pty_inspector_under_history() {
        assert_eq!(
            primary_sidebar_nav_labels(false),
            vec!["Dashboard", "History"]
        );
        assert_eq!(
            primary_sidebar_nav_labels(true),
            vec!["Dashboard", "History", "PTY Inspector"]
        );
    }

    #[test]
    fn sidebar_restores_workspace_selection_only_on_workspace_pages() {
        assert!(sidebar_should_restore_workspace_selection(
            &AppPage::Workspace
        ));
        assert!(sidebar_should_restore_workspace_selection(&AppPage::Review));
        assert!(!sidebar_should_restore_workspace_selection(
            &AppPage::Dashboard
        ));
        assert!(!sidebar_should_restore_workspace_selection(
            &AppPage::History
        ));
    }
}
