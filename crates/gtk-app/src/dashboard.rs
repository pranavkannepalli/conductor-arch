use gtk::prelude::*;
use gtk::{Box as GBox, Label, Orientation, PolicyType, ScrolledWindow};
use linux_archductor_core::paths::AppPaths;
use linux_archductor_core::workspace::WorkspaceStatusLine;
use linux_archductor_core::workspace::WorkspaceStore;

use crate::title_case_workspace;
use crate::workspace_command_center::workspace_pull_request_status_summary;

pub(crate) fn build_dashboard_panel(paths: &AppPaths) -> (GBox, impl Fn() + Clone + 'static) {
    let root = GBox::new(Orientation::Vertical, 0);
    root.add_css_class("dashboard");
    root.add_css_class("page-shell");

    let header = GBox::new(Orientation::Vertical, 14);
    header.add_css_class("dashboard-header");
    header.add_css_class("page-header");

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
    board.add_css_class("page-board");
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
    lines: &[&WorkspaceStatusLine],
    store: &WorkspaceStore,
) {
    let column = GBox::new(Orientation::Vertical, 12);
    column.add_css_class("kanban-column");
    column.set_hexpand(true);

    let header = GBox::new(Orientation::Horizontal, 8);
    header.add_css_class("kanban-column-header");
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
fn dashboard_pr_meta(repository_name: &str, pr_number: i64, pr_attention: Option<&str>) -> String {
    match pr_attention {
        Some(attention) if !attention.is_empty() => {
            format!("{repository_name} · PR #{pr_number} · {attention}")
        }
        _ => format!("{repository_name} · PR #{pr_number}"),
    }
}

fn build_dashboard_card(line: &WorkspaceStatusLine, store: &WorkspaceStore) -> GBox {
    let ws = &line.workspace;
    let card = GBox::new(Orientation::Vertical, 10);
    card.add_css_class("workspace-card");
    card.add_css_class("shell-card");

    let top = GBox::new(Orientation::Horizontal, 8);
    top.add_css_class("dashboard-card-top");
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

    let pr_attention = line
        .pull_request
        .as_ref()
        .map(|pr| workspace_pull_request_status_summary(store, &ws.name, pr));
    let meta = match &line.pull_request {
        Some(pr) => dashboard_pr_meta(
            &line.repository_name,
            pr.number,
            pr_attention
                .as_ref()
                .and_then(|state| state.attention_label()),
        ),
        None => format!("{} · port {}", line.repository_name, ws.port_base),
    };
    let meta_label = Label::new(Some(&meta));
    meta_label.add_css_class("card-meta");
    if let Some(state) = pr_attention.as_ref() {
        if let Some(css_class) = state.attention_css_class() {
            meta_label.add_css_class(css_class);
        }
    }
    meta_label.set_xalign(0.0);
    meta_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    card.append(&meta_label);

    let foot = GBox::new(Orientation::Horizontal, 8);
    foot.add_css_class("dashboard-card-footer");
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

#[cfg(test)]
mod tests {
    use super::dashboard_pr_meta;

    #[test]
    fn dashboard_meta_includes_pr_attention_state() {
        let meta = dashboard_pr_meta("demo", 42, Some("checks failed"));
        assert_eq!(meta, "demo · PR #42 · checks failed");
    }

    #[test]
    fn dashboard_meta_falls_back_to_pr_state_when_attention_missing() {
        let meta = dashboard_pr_meta("demo", 42, None);
        assert_eq!(meta, "demo · PR #42");
    }
}
