use archductor_core::paths::AppPaths;
use archductor_core::repository::RepositoryStore;
use archductor_core::workspace::WorkspaceStatusLine;
use archductor_core::workspace::WorkspaceStore;
use gtk::prelude::*;
use gtk::{Box as GBox, Button, Label, Orientation, PolicyType, ScrolledWindow};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use crate::history_data::workspace_has_open_pull_request;
use crate::motion::{append_revealed, clear_box};
use crate::tabs::{set_standard_tab_active, standard_tab, standard_tab_strip};
use crate::title_case_workspace;
use crate::workspace_command_center::workspace_pull_request_status_summary;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DashboardBucket {
    Ready,
    Running,
    Review,
    Archived,
}

fn dashboard_bucket(line: &WorkspaceStatusLine) -> DashboardBucket {
    if line.workspace.status == "archived" {
        DashboardBucket::Archived
    } else if workspace_has_open_pull_request(line) {
        DashboardBucket::Review
    } else if line.run_running || line.active_sessions > 0 {
        DashboardBucket::Running
    } else {
        DashboardBucket::Ready
    }
}

pub(crate) fn build_dashboard_panel(
    paths: &AppPaths,
    open_workspace: Rc<dyn Fn(String)>,
) -> (GBox, impl Fn() + Clone + 'static) {
    let root = GBox::new(Orientation::Vertical, 0);
    root.add_css_class("dashboard");
    root.add_css_class("page-shell");

    let header = GBox::new(Orientation::Vertical, 10);
    header.add_css_class("dashboard-header");
    header.add_css_class("page-header");

    let title = Label::new(Some("Dashboard"));
    title.add_css_class("dashboard-title");
    title.set_xalign(0.0);
    header.append(&title);

    let subtitle = Label::new(Some(
        "See what is ready, running, under review, or archived across your projects.",
    ));
    subtitle.add_css_class("dashboard-subtitle");
    subtitle.set_xalign(0.0);
    subtitle.set_wrap(true);
    header.append(&subtitle);

    let (project_tabs_scroll, project_tabs) = standard_tab_strip();
    project_tabs_scroll.add_css_class("project-tabs");
    header.append(&project_tabs_scroll);
    root.append(&header);

    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    scroll.set_vexpand(true);

    let board = GBox::new(Orientation::Horizontal, 12);
    board.add_css_class("kanban-board");
    board.add_css_class("page-board");
    scroll.set_child(Some(&board));
    root.append(&scroll);

    let db_path = paths.database_path.clone();
    let selected_project = Rc::new(RefCell::new(None::<String>));
    let refresh = move || {
        render_dashboard(
            &db_path,
            &project_tabs,
            &board,
            selected_project.clone(),
            open_workspace.clone(),
        );
    };

    refresh();
    (root, refresh)
}

fn render_dashboard(
    db_path: &Path,
    project_tabs: &GBox,
    board: &GBox,
    selected_project: Rc<RefCell<Option<String>>>,
    open_workspace: Rc<dyn Fn(String)>,
) {
    clear_box(project_tabs);
    clear_box(board);

    let Ok(store) = WorkspaceStore::open_app(db_path) else {
        append_empty_dashboard(project_tabs, board, "No workspace database yet.");
        return;
    };
    let Ok(statuses) = store.list_status() else {
        append_empty_dashboard(project_tabs, board, "Could not read workspace state.");
        return;
    };

    let Ok(repo_names) = dashboard_repository_names(db_path) else {
        append_empty_dashboard(project_tabs, board, "Could not read project state.");
        return;
    };

    let selected = dashboard_selected_project(selected_project.borrow().as_deref(), &repo_names);
    *selected_project.borrow_mut() = selected.clone();

    let all_tab = dashboard_project_tab("All projects", selected.is_none(), {
        let db_path = db_path.to_path_buf();
        let project_tabs = project_tabs.downgrade();
        let board = board.downgrade();
        let selected_project = selected_project.clone();
        let open_workspace = open_workspace.clone();
        move || {
            *selected_project.borrow_mut() = None;
            let (Some(project_tabs), Some(board)) = (project_tabs.upgrade(), board.upgrade())
            else {
                return;
            };
            render_dashboard(
                &db_path,
                &project_tabs,
                &board,
                selected_project.clone(),
                open_workspace.clone(),
            );
        }
    });
    project_tabs.append(&all_tab);
    for repo in &repo_names {
        let tab = dashboard_project_tab(repo, selected.as_deref() == Some(repo.as_str()), {
            let db_path = db_path.to_path_buf();
            let project_tabs = project_tabs.downgrade();
            let board = board.downgrade();
            let selected_project = selected_project.clone();
            let open_workspace = open_workspace.clone();
            let repo = repo.clone();
            move || {
                *selected_project.borrow_mut() = Some(repo.clone());
                let (Some(project_tabs), Some(board)) = (project_tabs.upgrade(), board.upgrade())
                else {
                    return;
                };
                render_dashboard(
                    &db_path,
                    &project_tabs,
                    &board,
                    selected_project.clone(),
                    open_workspace.clone(),
                );
            }
        });
        project_tabs.append(&tab);
    }

    let visible_statuses = statuses
        .iter()
        .filter(|line| dashboard_project_matches(&line.repository_name, selected.as_deref()))
        .collect::<Vec<_>>();

    let mut ready = Vec::new();
    let mut running = Vec::new();
    let mut review = Vec::new();
    let mut archived = Vec::new();

    for line in visible_statuses {
        match dashboard_bucket(line) {
            DashboardBucket::Ready => ready.push(line),
            DashboardBucket::Running => running.push(line),
            DashboardBucket::Review => review.push(line),
            DashboardBucket::Archived => archived.push(line),
        }
    }

    append_dashboard_column(
        board,
        "Ready",
        "No ready workspaces",
        &ready,
        &store,
        &open_workspace,
    );
    append_dashboard_column(
        board,
        "Running",
        "Nothing running",
        &running,
        &store,
        &open_workspace,
    );
    append_dashboard_column(
        board,
        "Review",
        "Nothing in review",
        &review,
        &store,
        &open_workspace,
    );
    append_dashboard_column(
        board,
        "Archived",
        "No archived workspaces",
        &archived,
        &store,
        &open_workspace,
    );
}

fn dashboard_project_tab(label: &str, active: bool, on_click: impl Fn() + 'static) -> Button {
    let tab = standard_tab(label);
    set_standard_tab_active(&tab, active);
    tab.connect_clicked(move |_| on_click());
    tab
}

fn dashboard_project_matches(repository_name: &str, selected_project: Option<&str>) -> bool {
    selected_project.is_none_or(|project| repository_name == project)
}

fn dashboard_repository_names(db_path: &Path) -> anyhow::Result<Vec<String>> {
    let mut repository_names = RepositoryStore::open(db_path)?
        .list()?
        .into_iter()
        .map(|repository| repository.name)
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    repository_names.sort();
    repository_names.dedup();
    Ok(repository_names)
}

fn dashboard_selected_project(
    selected_project: Option<&str>,
    repository_names: &[String],
) -> Option<String> {
    selected_project
        .filter(|selected| repository_names.iter().any(|repo| repo == selected))
        .map(str::to_owned)
}

fn open_dashboard_workspace(line: &WorkspaceStatusLine, open_workspace: &Rc<dyn Fn(String)>) {
    open_workspace(line.workspace.name.clone());
}

fn append_empty_dashboard(project_tabs: &GBox, board: &GBox, message: &str) {
    let all_tab = standard_tab("All projects");
    set_standard_tab_active(&all_tab, true);
    project_tabs.append(&all_tab);

    let empty = Label::new(Some(message));
    empty.add_css_class("empty-label");
    empty.set_xalign(0.0);
    empty.set_margin_start(24);
    empty.set_margin_top(24);
    append_revealed(board, &empty);
}

fn append_dashboard_column(
    board: &GBox,
    title: &str,
    empty_message: &str,
    lines: &[&WorkspaceStatusLine],
    store: &WorkspaceStore,
    open_workspace: &Rc<dyn Fn(String)>,
) {
    let column = GBox::new(Orientation::Vertical, 8);
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
        let empty = Label::new(Some(empty_message));
        empty.add_css_class("column-empty");
        empty.set_xalign(0.0);
        append_revealed(&column, &empty);
    } else {
        for line in lines {
            append_revealed(
                &column,
                &build_dashboard_card(line, store, open_workspace.clone()),
            );
        }
    }

    append_revealed(board, &column);
}
fn dashboard_pr_meta(repository_name: &str, pr_number: i64, pr_attention: Option<&str>) -> String {
    match pr_attention {
        Some(attention) if !attention.is_empty() => {
            format!("{repository_name} · PR #{pr_number} · {attention}")
        }
        _ => format!("{repository_name} · PR #{pr_number}"),
    }
}

fn build_dashboard_card(
    line: &WorkspaceStatusLine,
    store: &WorkspaceStore,
    open_workspace: Rc<dyn Fn(String)>,
) -> Button {
    let ws = &line.workspace;
    let button = Button::new();
    button.add_css_class("flat");
    button.add_css_class("workspace-card-action");
    button.set_focusable(true);
    button.set_tooltip_text(Some(&format!("Open workspace {}", ws.name)));

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
        None => line.repository_name.clone(),
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

    button.set_child(Some(&card));
    let line = line.clone();
    button.connect_clicked(move |_| {
        open_dashboard_workspace(&line, &open_workspace);
    });
    button
}

#[cfg(test)]
mod tests {
    use super::{
        dashboard_bucket, dashboard_pr_meta, dashboard_project_matches, dashboard_repository_names,
        dashboard_selected_project, open_dashboard_workspace, DashboardBucket,
    };
    use archductor_core::repository::{AddRepository, RepositoryStore};
    use archductor_core::workspace::{PullRequest, Workspace, WorkspaceStatusLine};
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use std::{cell::RefCell, rc::Rc};

    fn line(
        status: &str,
        run_running: bool,
        active_sessions: usize,
        pull_request: Option<(i64, &str)>,
    ) -> WorkspaceStatusLine {
        WorkspaceStatusLine {
            workspace: Workspace {
                id: 1,
                repository_id: 1,
                name: "berlin".to_owned(),
                path: PathBuf::from("/tmp/berlin"),
                branch: "lc/berlin".to_owned(),
                base_ref: "main".to_owned(),
                port_base: 3000,
                status: status.to_owned(),
                archived_at: None,
                created_at: "1".to_owned(),
                updated_at: "2".to_owned(),
            },
            repository_name: "demo".to_owned(),
            open_todos: 0,
            pull_request: pull_request.map(|(number, state)| PullRequest {
                id: 1,
                workspace_id: 1,
                provider: "github".to_owned(),
                number,
                url: format!("https://example.test/pull/{number}"),
                state: state.to_owned(),
                created_at: "1".to_owned(),
                updated_at: "2".to_owned(),
            }),
            run_running,
            active_sessions,
            branch_push_state: None,
            diff_additions: 0,
            diff_deletions: 0,
        }
    }

    #[test]
    fn dashboard_buckets_describe_workspace_state() {
        assert_eq!(
            dashboard_bucket(&line("active", false, 0, None)),
            DashboardBucket::Ready
        );
        assert_eq!(
            dashboard_bucket(&line("active", true, 0, None)),
            DashboardBucket::Running
        );
        assert_eq!(
            dashboard_bucket(&line("active", false, 1, None)),
            DashboardBucket::Running
        );
        assert_eq!(
            dashboard_bucket(&line("active", false, 0, Some((42, "open")))),
            DashboardBucket::Review
        );
        assert_eq!(
            dashboard_bucket(&line("archived", false, 0, None)),
            DashboardBucket::Archived
        );
    }

    #[test]
    fn dashboard_review_bucket_requires_an_open_pull_request() {
        assert_eq!(
            dashboard_bucket(&line("active", false, 0, Some((42, "open")))),
            DashboardBucket::Review
        );
        assert_eq!(
            dashboard_bucket(&line("active", false, 0, Some((42, "closed")))),
            DashboardBucket::Ready
        );
        assert_eq!(
            dashboard_bucket(&line("active", true, 0, Some((42, "merged")))),
            DashboardBucket::Running
        );
    }

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

    #[test]
    fn dashboard_project_filter_matches_selected_repository() {
        assert!(dashboard_project_matches("demo", None));
        assert!(dashboard_project_matches("demo", Some("demo")));
        assert!(!dashboard_project_matches("demo", Some("other")));
    }

    #[test]
    fn dashboard_preserves_any_selected_project_that_still_exists() {
        let repositories = ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"]
            .map(str::to_owned)
            .to_vec();

        assert_eq!(
            dashboard_selected_project(Some("foxtrot"), &repositories),
            Some("foxtrot".to_owned())
        );
        assert_eq!(
            dashboard_selected_project(Some("missing"), &repositories),
            None
        );
    }

    #[test]
    fn dashboard_includes_and_preserves_a_project_without_workspaces() {
        let temp = tempfile::tempdir().unwrap();
        let repository_path = temp.path().join("empty-project");
        fs::create_dir(&repository_path).unwrap();
        assert!(Command::new("git")
            .args(["init", "--initial-branch", "main"])
            .arg(&repository_path)
            .status()
            .unwrap()
            .success());
        let database_path = temp.path().join("state.db");
        RepositoryStore::open(&database_path)
            .unwrap()
            .add(AddRepository {
                name: Some("empty-project".to_owned()),
                root_path: repository_path,
                default_branch: Some("main".to_owned()),
                remote_name: "origin".to_owned(),
                workspace_parent_path: Some(temp.path().join("workspaces/empty-project")),
            })
            .unwrap();

        let repository_names = dashboard_repository_names(&database_path).unwrap();

        assert_eq!(repository_names, vec!["empty-project"]);
        assert_eq!(
            dashboard_selected_project(Some("empty-project"), &repository_names),
            Some("empty-project".to_owned())
        );
    }

    #[test]
    fn dashboard_card_action_opens_its_workspace() {
        let opened = Rc::new(RefCell::new(None));
        let opened_for_callback = opened.clone();
        let open_workspace: Rc<dyn Fn(String)> = Rc::new(move |name| {
            *opened_for_callback.borrow_mut() = Some(name);
        });

        open_dashboard_workspace(&line("active", false, 0, None), &open_workspace);

        assert_eq!(opened.borrow().as_deref(), Some("berlin"));
    }
}
