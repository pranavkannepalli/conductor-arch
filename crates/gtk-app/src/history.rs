use archductor_core::import::default_conductor_app_database;
use archductor_core::workspace::{WorkspaceStatusLine, WorkspaceStore};
use gtk::prelude::*;
use gtk::{
    Box as GBox, Label, ListBox, Orientation, PolicyType, ScrolledWindow, Separator, TextView,
};
use rusqlite::Connection;
use std::cell::RefCell;
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;
use tracing::error;

use crate::motion::{append_revealed_to_list, clear_list};
use crate::toast::ToastManager;

pub(crate) fn build_history_page(
    database_path: PathBuf,
    toast_manager: ToastManager,
) -> (GBox, impl Fn() + Clone + 'static) {
    let root = GBox::new(Orientation::Vertical, 0);
    root.add_css_class("dashboard");
    root.add_css_class("page-shell");
    let header = GBox::new(Orientation::Vertical, 8);
    header.add_css_class("dashboard-header");
    header.add_css_class("page-header");
    let title = Label::new(Some("History"));
    title.add_css_class("dashboard-title");
    title.set_xalign(0.0);
    let subtitle = Label::new(Some(
        "All remaining workspaces across active, backlog, and archived states.",
    ));
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
    list.add_css_class("shell-list");
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

    let workspace_rows: Rc<RefCell<HashMap<i32, WorkspaceHistoryEntry>>> =
        Rc::new(RefCell::new(HashMap::new()));
    let refresh_generation = Rc::new(RefCell::new(0u64));
    let refresh = {
        let list = list.clone();
        let message_view = message_view.clone();
        let workspace_rows = Rc::clone(&workspace_rows);
        let database_path = database_path.clone();
        let refresh_generation = Rc::clone(&refresh_generation);
        let toast_manager = toast_manager.clone();
        move || {
            clear_list(&list);
            workspace_rows.borrow_mut().clear();
            message_view
                .buffer()
                .set_text("Select a workspace to inspect its current state.");

            let loading = Label::new(Some("Loading workspace history..."));
            loading.add_css_class("empty-label");
            loading.set_xalign(0.0);
            loading.set_margin_start(24);
            loading.set_margin_top(24);
            append_revealed_to_list(&list, &loading);

            *refresh_generation.borrow_mut() += 1;
            let generation = *refresh_generation.borrow();
            let (tx, rx) = mpsc::channel();
            let db_path = database_path.clone();
            std::thread::spawn(move || {
                let _ = tx.send(history_recent_workspaces(&db_path));
            });

            let list = list.clone();
            let workspace_rows = Rc::clone(&workspace_rows);
            let refresh_generation = Rc::clone(&refresh_generation);
            let toast_manager = toast_manager.clone();
            // PER-190: temporary worker-result poll for DB/import history load;
            // remove when this page switches to a GLib main-context future.
            glib::timeout_add_local(Duration::from_millis(100), move || match rx.try_recv() {
                Ok(Ok(workspaces)) => {
                    if *refresh_generation.borrow() != generation {
                        return glib::ControlFlow::Break;
                    }
                    clear_list(&list);
                    workspace_rows.borrow_mut().clear();
                    for (idx, workspace) in workspaces.into_iter().enumerate() {
                        append_revealed_to_list(&list, &workspace_history_row(&workspace));
                        workspace_rows
                            .borrow_mut()
                            .insert(i32::try_from(idx).unwrap_or(i32::MAX), workspace);
                    }
                    if list.first_child().is_none() {
                        let empty = Label::new(Some("No workspace history yet."));
                        empty.add_css_class("empty-label");
                        empty.set_xalign(0.0);
                        empty.set_margin_start(24);
                        empty.set_margin_top(24);
                        append_revealed_to_list(&list, &empty);
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(err)) => {
                    if *refresh_generation.borrow() == generation {
                        clear_list(&list);
                        let message = format!("Could not load workspace history: {err:#}");
                        toast_manager.error(message.clone());
                        let empty = Label::new(Some(&message));
                        empty.add_css_class("empty-label");
                        empty.set_xalign(0.0);
                        empty.set_margin_start(24);
                        empty.set_margin_top(24);
                        empty.set_wrap(true);
                        append_revealed_to_list(&list, &empty);
                    }
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if *refresh_generation.borrow() == generation {
                        clear_list(&list);
                        let message = "Could not load workspace history.";
                        toast_manager.error(message);
                        let empty = Label::new(Some(message));
                        empty.add_css_class("empty-label");
                        empty.set_xalign(0.0);
                        empty.set_margin_start(24);
                        empty.set_margin_top(24);
                        append_revealed_to_list(&list, &empty);
                    }
                    glib::ControlFlow::Break
                }
            });
        }
    };

    let workspace_rows_select = Rc::clone(&workspace_rows);
    list.connect_row_selected(move |_, row| {
        guarded_gtk_callback((), || {
            let Some(workspace) =
                row.and_then(|r| workspace_rows_select.borrow().get(&r.index()).cloned())
            else {
                return;
            };
            message_view
                .buffer()
                .set_text(&workspace_history_detail(&workspace));
        })
    });

    refresh();
    (root, refresh)
}

// ── WORKSPACE HISTORY ─────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceHistoryEntry {
    name: String,
    repository_name: String,
    branch: String,
    base_ref: String,
    path: String,
    status: String,
    bucket: String,
    updated_at: String,
    created_at: String,
    archived_at: Option<String>,
    open_todos: usize,
    active_sessions: usize,
    run_running: bool,
    pull_request: Option<i64>,
    diff_additions: usize,
    diff_deletions: usize,
}

fn workspace_history_row(workspace: &WorkspaceHistoryEntry) -> GBox {
    let row = GBox::new(Orientation::Vertical, 3);
    row.add_css_class("history-row");
    let title = Label::new(Some(&workspace.name));
    title.add_css_class("workspace-name");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let meta = Label::new(Some(&format!(
        "{} · {} · {}",
        workspace.repository_name, workspace.branch, workspace.bucket
    )));
    meta.add_css_class("workspace-meta");
    meta.set_xalign(0.0);
    meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let status = Label::new(Some(&format!(
        "{} · updated {} · +{} -{}",
        workspace.status, workspace.updated_at, workspace.diff_additions, workspace.diff_deletions
    )));
    status.add_css_class("card-meta");
    status.set_xalign(0.0);
    status.set_ellipsize(gtk::pango::EllipsizeMode::End);
    row.append(&title);
    row.append(&meta);
    row.append(&status);
    row
}

fn history_recent_workspaces(database_path: &Path) -> anyhow::Result<Vec<WorkspaceHistoryEntry>> {
    let store = WorkspaceStore::open(database_path)?;
    let mut workspaces = store.list_status().map(|lines| {
        lines
            .iter()
            .map(workspace_history_entry)
            .collect::<Vec<_>>()
    })?;
    workspaces.sort_by(|left, right| {
        workspace_history_sort_key(right)
            .cmp(&workspace_history_sort_key(left))
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(workspaces)
}

fn workspace_history_entry(line: &WorkspaceStatusLine) -> WorkspaceHistoryEntry {
    WorkspaceHistoryEntry {
        name: line.workspace.name.clone(),
        repository_name: line.repository_name.clone(),
        branch: line.workspace.branch.clone(),
        base_ref: line.workspace.base_ref.clone(),
        path: line.workspace.path.to_string_lossy().to_string(),
        status: line.workspace.status.clone(),
        bucket: workspace_history_bucket(line).to_owned(),
        updated_at: line.workspace.updated_at.clone(),
        created_at: line.workspace.created_at.clone(),
        archived_at: line.workspace.archived_at.clone(),
        open_todos: line.open_todos,
        active_sessions: line.active_sessions,
        run_running: line.run_running,
        pull_request: line.pull_request.as_ref().map(|pr| pr.number),
        diff_additions: line.diff_additions,
        diff_deletions: line.diff_deletions,
    }
}

fn workspace_history_bucket(line: &WorkspaceStatusLine) -> &'static str {
    if line.workspace.status == "archived" {
        "Archived"
    } else if line.run_running
        || line.active_sessions > 0
        || line
            .pull_request
            .as_ref()
            .is_some_and(|pr| pr.state.eq_ignore_ascii_case("open"))
    {
        "Active"
    } else {
        "Backlog"
    }
}

fn workspace_history_sort_key(workspace: &WorkspaceHistoryEntry) -> u8 {
    match workspace.bucket.as_str() {
        "Active" => 3,
        "Backlog" => 2,
        "Archived" => 1,
        _ => 0,
    }
}

fn workspace_history_detail(workspace: &WorkspaceHistoryEntry) -> String {
    let pr = workspace
        .pull_request
        .map(|number| format!("PR #{number}"))
        .unwrap_or_else(|| "No PR".to_owned());
    let archived = workspace.archived_at.as_deref().unwrap_or("Not archived");
    format!(
        "{name}\n\
         \n\
         State\n\
         {bucket} · {status}\n\
         \n\
         Project\n\
         {repository}\n\
         \n\
         Branch\n\
         {branch}\n\
         Base: {base_ref}\n\
         \n\
         Summary\n\
         +{additions} -{deletions}\n\
         {todos} open todos\n\
         {sessions} active sessions\n\
         Run: {run}\n\
         {pr}\n\
         \n\
         Dates\n\
         Created: {created_at}\n\
         Updated: {updated_at}\n\
         Archived: {archived}\n\
         \n\
         Path\n\
         {path}",
        name = workspace.name,
        bucket = workspace.bucket,
        status = workspace.status,
        repository = workspace.repository_name,
        branch = workspace.branch,
        base_ref = workspace.base_ref,
        additions = workspace.diff_additions,
        deletions = workspace.diff_deletions,
        todos = workspace.open_todos,
        sessions = workspace.active_sessions,
        run = if workspace.run_running {
            "running"
        } else {
            "idle"
        },
        pr = pr,
        created_at = workspace.created_at,
        updated_at = workspace.updated_at,
        archived = archived,
        path = workspace.path
    )
}

// ── CHAT HISTORY HELPERS ──────────────────────────────────────────────────

pub(crate) struct ChatSummary {
    id: String,
    source: String,
    title: String,
    agent_type: String,
    status: String,
    repository_name: String,
    workspace_name: String,
    workspace_path: String,
    updated_at: String,
    message_count: i64,
}

pub(crate) fn session_summary_row(session: &ChatSummary) -> GBox {
    let row = GBox::new(Orientation::Vertical, 3);
    row.add_css_class("history-row");
    let title = Label::new(Some(&session.title));
    title.add_css_class("workspace-name");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let meta = Label::new(Some(&format!(
        "{} · {} · {} · {} · {} messages",
        session.source,
        session.repository_name,
        session.workspace_name,
        session.agent_type,
        session.message_count
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

fn history_recent_sessions(database_path: &Path) -> Vec<ChatSummary> {
    let mut sessions = local_recent_sessions(database_path);
    sessions.extend(conductor_recent_sessions());
    sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    sessions.truncate(200);
    sessions
}

fn local_recent_sessions(database_path: &Path) -> Vec<ChatSummary> {
    query_local_sessions(database_path, None).unwrap_or_default()
}

fn conductor_recent_sessions() -> Vec<ChatSummary> {
    query_conductor_sessions(None).unwrap_or_default()
}

pub(crate) fn sessions_for_workspace_path(database_path: &Path, path: &Path) -> Vec<ChatSummary> {
    let mut sessions = query_local_sessions(database_path, Some(path)).unwrap_or_default();
    sessions.extend(query_conductor_sessions(Some(path)).unwrap_or_default());
    sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    sessions
}

fn query_local_sessions(
    database_path: &Path,
    path: Option<&Path>,
) -> anyhow::Result<Vec<ChatSummary>> {
    let store = WorkspaceStore::open(database_path)?;
    let mut sessions = store
        .list_local_chat_threads(path)?
        .into_iter()
        .map(|thread| ChatSummary {
            id: format!("local-thread:{}", thread.thread_id),
            source: "Linux".to_owned(),
            title: thread.title,
            agent_type: thread.provider,
            status: thread.status,
            repository_name: thread.repository_name,
            workspace_name: thread.workspace_name,
            workspace_path: thread.workspace_path.to_string_lossy().to_string(),
            updated_at: thread.updated_at,
            message_count: i64::try_from(thread.message_count).unwrap_or(i64::MAX),
        })
        .collect::<Vec<_>>();
    sessions.extend(
        store
            .list_local_chat_history(path)?
            .into_iter()
            .filter(|session| session.chat_thread_id.is_none())
            .map(|session| ChatSummary {
                id: format!("local:{}", session.process_id),
                source: "Linux Legacy".to_owned(),
                title: format!("{} session #{}", session.agent_type, session.process_id),
                agent_type: session.agent_type,
                status: session.status,
                repository_name: session.repository_name,
                workspace_name: session.workspace_name,
                workspace_path: session.workspace_path.to_string_lossy().to_string(),
                updated_at: session.updated_at,
                message_count: i64::try_from(session.message_count).unwrap_or(i64::MAX),
            }),
    );
    Ok(sessions)
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
        source: "Imported".to_owned(),
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

fn history_session_messages(database_path: &Path, session_id: &str) -> String {
    if let Some(id) = session_id
        .strip_prefix("local-thread:")
        .and_then(|value| value.parse::<i64>().ok())
    {
        return local_thread_messages(database_path, id);
    }
    if let Some(id) = session_id
        .strip_prefix("local:")
        .and_then(|value| value.parse::<i64>().ok())
    {
        return local_session_messages(database_path, id);
    }
    let imported_id = session_id.strip_prefix("imported:").unwrap_or(session_id);
    conductor_session_messages(imported_id)
}

fn local_thread_messages(database_path: &Path, thread_id: i64) -> String {
    let Ok(store) = WorkspaceStore::open(database_path) else {
        return "Could not open Archductor history database.".to_owned();
    };
    let Ok(messages) = store.local_chat_thread_messages(thread_id) else {
        return "Could not read local chat thread.".to_owned();
    };
    if messages.is_empty() {
        return "No messages in this chat.".to_owned();
    }
    messages
        .into_iter()
        .map(|message| {
            format!(
                "{}\n{}\n",
                local_role_label(&message.role),
                truncate_message(&message.content, 2200)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn local_session_messages(database_path: &Path, process_id: i64) -> String {
    let Ok(store) = WorkspaceStore::open(database_path) else {
        return "Could not open Archductor history database.".to_owned();
    };
    let Ok(messages) = store.local_chat_history_messages(process_id) else {
        return "Could not read local chat transcript.".to_owned();
    };
    if messages.is_empty() {
        return "No messages in this chat.".to_owned();
    }
    messages
        .into_iter()
        .map(|message| {
            format!(
                "{}\n{}\n",
                local_role_label(&message.role),
                truncate_message(&message.content, 2200)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn local_role_label(role: &str) -> &'static str {
    match role {
        "user" => "You",
        "review" => "Review Prompt",
        "system" => "System",
        _ => "Agent",
    }
}

fn conductor_session_messages(session_id: &str) -> String {
    let db_path = default_conductor_app_database();
    let Ok(conn) = Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return "Could not open Archductor chat database.".to_owned();
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

fn guarded_gtk_callback<T, F>(fallback: T, callback: F) -> T
where
    F: FnOnce() -> T,
{
    match catch_unwind(AssertUnwindSafe(callback)) {
        Ok(value) => value,
        Err(_) => {
            error!("recovered panic inside history GTK callback");
            fallback
        }
    }
}

#[cfg(test)]
mod workspace_history_tests {
    use super::{workspace_history_bucket, WorkspaceStatusLine};
    use archductor_core::workspace::{PullRequest, Workspace};
    use std::path::PathBuf;

    fn line(status: &str) -> WorkspaceStatusLine {
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
            pull_request: None,
            run_running: false,
            active_sessions: 0,
            branch_push_state: None,
            diff_additions: 0,
            diff_deletions: 0,
        }
    }

    #[test]
    fn workspace_history_bucket_covers_active_backlog_and_archived() {
        let backlog = line("active");
        assert_eq!(workspace_history_bucket(&backlog), "Backlog");

        let mut active = line("active");
        active.active_sessions = 1;
        assert_eq!(workspace_history_bucket(&active), "Active");

        let mut review = line("active");
        review.pull_request = Some(PullRequest {
            id: 1,
            workspace_id: 1,
            provider: "github".to_owned(),
            number: 42,
            url: "https://example.test/pull/42".to_owned(),
            state: "open".to_owned(),
            created_at: "1".to_owned(),
            updated_at: "2".to_owned(),
        });
        assert_eq!(workspace_history_bucket(&review), "Active");

        let mut closed_review = review;
        closed_review.pull_request.as_mut().unwrap().state = "closed".to_owned();
        assert_eq!(workspace_history_bucket(&closed_review), "Backlog");

        let archived = line("archived");
        assert_eq!(workspace_history_bucket(&archived), "Archived");
    }
}
