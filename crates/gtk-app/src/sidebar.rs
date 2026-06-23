use gtk::prelude::*;
use gtk::{
    Align, Box as GBox, Button, Entry, Image, Label, ListBox, ListBoxRow, Orientation, PolicyType,
    ScrolledWindow, Stack,
};
use linux_conductor_core::repository::RepositoryStore;
use linux_conductor_core::workspace::WorkspaceStore;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::projects::{show_create_workspace_dialog, show_repository_quick_add_dialog};
use crate::refresh::{RefreshHub, RefreshScope};
use crate::state::{AppPage, AppState, WorkspaceTab};
use crate::title_case_workspace;

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

    // Minimal search bar at top
    let search_entry = Entry::new();
    search_entry.set_placeholder_text(Some("Filter workspaces..."));
    search_entry.add_css_class("sidebar-search-minimal");
    search_entry.set_margin_start(0);
    search_entry.set_margin_end(0);
    search_entry.set_margin_bottom(0);
    sidebar_box.append(&search_entry);

    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    scroll.set_vexpand(true);

    let list = ListBox::new();
    list.add_css_class("workspace-list");
    list.set_selection_mode(gtk::SelectionMode::Single);
    let names: Rc<RefCell<HashMap<i32, String>>> = Rc::new(RefCell::new(HashMap::new()));
    let db_path = app_state.workspace_database_path();
    let db_path_populate = db_path.clone();

    let populate = {
        let list = list.clone();
        let names = Rc::clone(&names);
        let state = app_state.clone();
        let search_entry = search_entry.clone();
        let refresh_hub = refresh_hub.clone();
        let refresh_workspace = refresh_workspace.clone();
        let refresh_view_preferences = refresh_view_preferences.clone();
        let db_path_populate = db_path_populate.clone();
        move || {
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

                    let header_row = section_header_row(&repo_name, lines.len(), {
                        let db_path = db_path_populate.clone();
                        let refresh_hub = refresh_hub.clone();
                        let refresh_workspace = refresh_workspace.clone();
                        let refresh_view_preferences = refresh_view_preferences.clone();
                        let repo_name = repo_name.clone();
                        move || {
                            show_create_workspace_dialog(
                                db_path.clone(),
                                Rc::new({
                                    let refresh_hub = refresh_hub.clone();
                                    let refresh_workspace = refresh_workspace.clone();
                                    let refresh_view_preferences = refresh_view_preferences.clone();
                                    move || {
                                        refresh_hub.refresh(RefreshScope::Projects);
                                        refresh_hub.refresh(RefreshScope::Sidebar);
                                        refresh_hub.refresh(RefreshScope::Dashboard);
                                        refresh_workspace();
                                        refresh_view_preferences();
                                    }
                                }),
                                Rc::new({
                                    let refresh_hub = refresh_hub.clone();
                                    move || refresh_hub.refresh(RefreshScope::Dashboard)
                                }),
                                Rc::new(refresh_workspace.clone()),
                                Some(repo_name.clone()),
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
                            line.run_running,
                            line.active_sessions,
                            line.open_todos,
                            line.pull_request.as_ref().map(|p| p.number),
                            line.branch_push_state.as_ref().map(|s| s.ahead).unwrap_or(0),
                            &ws.updated_at,
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

    // Bottom bar nav state sync (defined before list.connect_row_selected so sync_nav_select is available)
    let dashboard_btn = Button::from_icon_name("go-home-symbolic");
    dashboard_btn.add_css_class("sidebar-bottom-icon-btn");
    dashboard_btn.set_tooltip_text(Some("Dashboard"));
    let history_btn = Button::from_icon_name("document-open-recent-symbolic");
    history_btn.add_css_class("sidebar-bottom-icon-btn");
    history_btn.set_tooltip_text(Some("History"));
    let settings_btn = Button::from_icon_name("emblem-system-symbolic");
    settings_btn.add_css_class("sidebar-bottom-icon-btn");
    settings_btn.set_tooltip_text(Some("Settings"));

    let sync_nav_state: Rc<dyn Fn()> = {
        let state = app_state.clone();
        let dashboard_btn = dashboard_btn.clone();
        let history_btn = history_btn.clone();
        Rc::new(move || {
            let active_page = state.snapshot().active_page;
            for (button, page) in [
                (&dashboard_btn, AppPage::Dashboard),
                (&history_btn, AppPage::History),
            ] {
                if active_page == page {
                    button.add_css_class("active");
                } else {
                    button.remove_css_class("active");
                }
            }
        })
    };

    let refresh_workspace_select = refresh_workspace.clone();
    let sync_nav_select = sync_nav_state.clone();
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
        refresh_workspace_select();
        refresh_select.refresh(RefreshScope::Dashboard);
        stack_select.set_visible_child_name("workspace");
        sync_nav_select();
    });

    scroll.set_child(Some(&list));
    sidebar_box.append(&scroll);

    // Bottom bar: Add repository + nav icon buttons
    let bottom_bar = GBox::new(Orientation::Horizontal, 4);
    bottom_bar.add_css_class("sidebar-bottom-bar");

    let add_repo_btn = Button::with_label("Add repository");
    add_repo_btn.add_css_class("nav-button");
    add_repo_btn.set_hexpand(true);
    {
        let db_path_bar = app_state.workspace_database_path();
        let hub_bar = refresh_hub.clone();
        let rw_bar = refresh_workspace.clone();
        let rvp_bar = refresh_view_preferences.clone();
        add_repo_btn.connect_clicked(move |_| {
            show_repository_quick_add_dialog(
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
                Some("folder"),
            );
        });
    }
    bottom_bar.append(&add_repo_btn);

    {
        let stack_d = stack.clone();
        let state_d = app_state.clone();
        let sync_d = sync_nav_state.clone();
        dashboard_btn.connect_clicked(move |_| {
            state_d.set_active_page(AppPage::Dashboard);
            stack_d.set_visible_child_name("dashboard");
            sync_d();
        });
    }
    {
        let stack_h = stack.clone();
        let state_h = app_state.clone();
        let sync_h = sync_nav_state.clone();
        history_btn.connect_clicked(move |_| {
            state_h.set_active_page(AppPage::History);
            stack_h.set_visible_child_name("history");
            sync_h();
        });
    }
    {
        let stack_s = stack.clone();
        let sync_s = sync_nav_state.clone();
        settings_btn.connect_clicked(move |_| {
            stack_s.set_visible_child_name("settings");
            sync_s();
        });
    }

    bottom_bar.append(&dashboard_btn);
    bottom_bar.append(&history_btn);
    bottom_bar.append(&settings_btn);

    sync_nav_state();
    sidebar_box.append(&bottom_bar);

    (sidebar_box, populate)
}

fn build_workspace_row(
    name: &str,
    branch: &str,
    status: &str,
    run_active: bool,
    active_sessions: usize,
    open_todos: usize,
    pr_number: Option<i64>,
    ahead: usize,
    updated_at: &str,
) -> ListBoxRow {
    let row_box = GBox::new(Orientation::Horizontal, 10);
    row_box.add_css_class("project-row");
    row_box.add_css_class("workspace-row-shell");
    let is_active = run_active || active_sessions > 0;
    if is_active {
        row_box.add_css_class("workspace-row-active");
    }

    // Status dot (colored circle)
    let dot = GBox::new(Orientation::Horizontal, 0);
    dot.add_css_class("status-dot");
    if run_active {
        dot.add_css_class("status-dot-running");
    } else if is_active {
        dot.add_css_class("status-dot-active");
    } else {
        dot.add_css_class("status-dot-idle");
    }
    dot.set_valign(Align::Start);
    row_box.append(&dot);

    // Text column
    let text_box = GBox::new(Orientation::Vertical, 2);
    text_box.set_hexpand(true);

    // Top row: name + badge
    let top_row = GBox::new(Orientation::Horizontal, 6);
    top_row.set_hexpand(true);

    let name_label = Label::new(Some(&title_case_workspace(name)));
    name_label.add_css_class("workspace-name");
    name_label.set_xalign(0.0);
    name_label.set_hexpand(true);
    name_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    top_row.append(&name_label);

    // Badge priority: PR > preview > active > ahead commits > todos
    let badge_text = if let Some(pr) = pr_number {
        Some(format!("PR #{pr}"))
    } else if run_active {
        Some("preview".to_string())
    } else if active_sessions > 0 {
        Some("active".to_string())
    } else if ahead > 0 {
        Some(format!("+{ahead}"))
    } else if open_todos > 0 {
        Some(format!("{open_todos} todo"))
    } else {
        None
    };

    if let Some(badge) = badge_text {
        let badge_label = Label::new(Some(&badge));
        badge_label.add_css_class("workspace-badge");
        if !is_active && ahead == 0 && pr_number.is_none() {
            badge_label.add_css_class("workspace-badge-muted");
        }
        badge_label.set_xalign(1.0);
        top_row.append(&badge_label);
    }

    text_box.append(&top_row);

    // Second row: branch · time ago
    let mut meta_parts: Vec<String> = Vec::new();
    if !branch.is_empty() {
        meta_parts.push(branch.to_string());
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

    let _ = status;
    row_box.append(&text_box);

    ListBoxRow::builder().child(&row_box).build()
}

fn section_header_row(
    name: &str,
    _workspace_count: usize,
    on_add_workspace: impl Fn() + 'static,
) -> ListBoxRow {
    let shell = GBox::new(Orientation::Horizontal, 6);
    shell.add_css_class("repo-section-row");

    let repo_lbl = Label::new(Some(name));
    repo_lbl.add_css_class("repo-section-header");
    repo_lbl.set_xalign(0.0);
    repo_lbl.set_hexpand(true);
    repo_lbl.set_ellipsize(gtk::pango::EllipsizeMode::End);
    shell.append(&repo_lbl);

    let add_btn = Button::from_icon_name("list-add-symbolic");
    add_btn.add_css_class("repo-header-add");
    add_btn.set_tooltip_text(Some("Create workspace"));
    add_btn.connect_clicked(move |_| on_add_workspace());
    shell.append(&add_btn);

    let chevron = Label::new(Some("▾"));
    chevron.add_css_class("repo-section-chevron");
    shell.append(&chevron);

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
    let sec = if time_parts.len() > 2 { time_parts[2] } else { 0 };

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
