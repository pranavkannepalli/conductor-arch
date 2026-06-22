use gtk::prelude::*;
use gtk::{
    Box as GBox, Label, ListBox, Orientation, PolicyType, ScrolledWindow, Separator, TextView,
};
use linux_conductor_core::import::default_conductor_app_database;
use linux_conductor_core::workspace::WorkspaceStore;
use rusqlite::Connection;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub(crate) fn build_history_page(database_path: PathBuf) -> (GBox, impl Fn() + Clone + 'static) {
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
        "Local Linux agent sessions plus imported Conductor chats when available.",
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

    let session_ids: Rc<RefCell<std::collections::HashMap<i32, String>>> =
        Rc::new(RefCell::new(std::collections::HashMap::new()));
    let refresh = {
        let list = list.clone();
        let session_ids = Rc::clone(&session_ids);
        let database_path = database_path.clone();
        move || {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            session_ids.borrow_mut().clear();
            for (idx, session) in history_recent_sessions(&database_path)
                .into_iter()
                .enumerate()
            {
                list.append(&session_summary_row(&session));
                session_ids
                    .borrow_mut()
                    .insert(i32::try_from(idx).unwrap_or(i32::MAX), session.id);
            }
        }
    };

    let session_ids_select = Rc::clone(&session_ids);
    let database_path_select = database_path.clone();
    list.connect_row_selected(move |_, row| {
        let Some(session_id) =
            row.and_then(|r| session_ids_select.borrow().get(&r.index()).cloned())
        else {
            return;
        };
        let buffer = message_view.buffer();
        buffer.set_text(&history_session_messages(
            &database_path_select,
            &session_id,
        ));
    });

    refresh();
    (root, refresh)
}

// ── DASHBOARD ─────────────────────────────────────────────────────────────

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
    let sessions = store.list_local_chat_history(path)?;
    Ok(sessions
        .into_iter()
        .map(|session| ChatSummary {
            id: format!("local:{}", session.process_id),
            source: "Linux".to_owned(),
            title: format!("{} session #{}", session.agent_type, session.process_id),
            agent_type: session.agent_type,
            status: session.status,
            repository_name: session.repository_name,
            workspace_name: session.workspace_name,
            workspace_path: session.workspace_path.to_string_lossy().to_string(),
            updated_at: session.updated_at,
            message_count: i64::try_from(session.message_count).unwrap_or(i64::MAX),
        })
        .collect())
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
        .strip_prefix("local:")
        .and_then(|value| value.parse::<i64>().ok())
    {
        return local_session_messages(database_path, id);
    }
    let imported_id = session_id.strip_prefix("imported:").unwrap_or(session_id);
    conductor_session_messages(imported_id)
}

fn local_session_messages(database_path: &Path, process_id: i64) -> String {
    let Ok(store) = WorkspaceStore::open(database_path) else {
        return "Could not open Linux Conductor history database.".to_owned();
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
