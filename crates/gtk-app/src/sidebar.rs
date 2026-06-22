use gtk::prelude::*;
use gtk::{
    Box as GBox, Button, Entry, Label, ListBox, ListBoxRow, Orientation, PolicyType,
    ScrolledWindow, Separator, Stack,
};
use linux_conductor_core::workspace::WorkspaceStore;
use std::cell::RefCell;
use std::rc::Rc;

use crate::refresh::{RefreshHub, RefreshScope};
use crate::state::{AppPage, AppState, WorkspaceTab};

pub(crate) fn build_app_sidebar(
    app_state: &AppState,
    refresh_hub: RefreshHub,
    stack: Stack,
    refresh_workspace: impl Fn() + Clone + 'static,
    refresh_view_preferences: Rc<dyn Fn()>,
) -> (GBox, impl Fn() + Clone + 'static) {
    let sidebar_box = GBox::new(Orientation::Vertical, 0);
    sidebar_box.add_css_class("sidebar");
    sidebar_box.set_width_request(240);

    let nav_group = GBox::new(Orientation::Vertical, 4);
    nav_group.add_css_class("sidebar-nav-group");

    let dashboard_btn = Button::with_label("Dashboard");
    dashboard_btn.add_css_class("nav-button-active");
    let stack_dashboard = stack.clone();
    let state_dashboard = app_state.clone();
    dashboard_btn.connect_clicked(move |_| {
        state_dashboard.set_active_page(AppPage::Dashboard);
        stack_dashboard.set_visible_child_name("dashboard");
    });
    nav_group.append(&dashboard_btn);

    let history_btn = Button::with_label("History");
    history_btn.add_css_class("nav-button");
    let stack_history = stack.clone();
    let state_history = app_state.clone();
    history_btn.connect_clicked(move |_| {
        state_history.set_active_page(AppPage::History);
        stack_history.set_visible_child_name("history");
    });
    nav_group.append(&history_btn);

    let projects_btn = Button::with_label("Projects");
    projects_btn.add_css_class("nav-button");
    let stack_projects = stack.clone();
    let state_projects = app_state.clone();
    projects_btn.connect_clicked(move |_| {
        state_projects.set_active_page(AppPage::Projects);
        stack_projects.set_visible_child_name("projects");
    });
    nav_group.append(&projects_btn);
    sidebar_box.append(&nav_group);

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
    let db_path = app_state.workspace_database_path();
    let db_path_populate = db_path.clone();

    let populate = {
        let list = list.clone();
        let names = Rc::clone(&names);
        let state = app_state.clone();
        let search_entry = search_entry.clone();
        let db_path_populate = db_path_populate.clone();
        move || {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            names.borrow_mut().clear();
            let filter = search_entry.text().to_string().to_lowercase();
            let prev_selected = state.selected_workspace();
            let mut row_idx = 0;

            if let Ok(store) = WorkspaceStore::open(db_path_populate.clone()) {
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
    let state_select = app_state.clone();
    let stack_select = stack.clone();
    let refresh_select = refresh_hub.clone();
    let db_path_select = db_path.clone();
    let refresh_view_preferences_select = refresh_view_preferences.clone();
    list.connect_row_selected(move |_, row| {
        let Some(name) = row.and_then(|r| names_select.borrow().get(&r.index()).cloned()) else {
            return;
        };
        let default_tab = WorkspaceStore::open(db_path_select.clone())
            .and_then(|store| store.workspace_view_defaults(&name))
            .ok()
            .and_then(|defaults| defaults.default_visible_tab)
            .and_then(|tab| WorkspaceTab::from_config(&tab));
        state_select.set_selected_workspace_with_default_tab(Some(name), default_tab);
        refresh_view_preferences_select();
        refresh_workspace();
        refresh_select.refresh(RefreshScope::Dashboard);
        stack_select.set_visible_child_name("workspace");
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
    row_box.add_css_class("workspace-row-shell");

    let icon = Label::new(Some(if run_active { "◐" } else { "◦" }));
    icon.add_css_class(if run_active {
        "project-icon-hot"
    } else {
        "project-icon"
    });

    let text_box = GBox::new(Orientation::Vertical, 2);
    text_box.add_css_class("workspace-row-text");
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
