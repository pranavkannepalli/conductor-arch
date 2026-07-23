use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

use anyhow::Result;

use archductor_core::provider_events::{
    ProviderEventKind, ProviderEventPhase, ProviderEventRecord, ProviderEventStore,
};
use archductor_core::provider_projection::{
    provider_projection_from_records, provider_projection_item_is_relevant_chat_event,
    ProviderProjectionStatus,
};
use archductor_core::workspace::{ChatThreadRecord, WorkspaceStore};

use crate::refresh::RefreshEvent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundThreadSnapshot {
    pub workspace: String,
    pub thread_id: i64,
    pub title: String,
    pub provider: String,
    pub status: String,
    pub latest_message_id: Option<i64>,
    pub latest_provider_sequence: Option<i64>,
    pub running_session_id: Option<i64>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackgroundSyncSnapshot {
    pub running_threads: Vec<BackgroundThreadSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceChatNavItem {
    pub thread_id: i64,
    pub title: String,
    pub provider: String,
    pub status: String,
    pub running: bool,
    pub unread: bool,
    pub updated_at: String,
}

pub fn load_background_sync_snapshot(db_path: &Path) -> Result<BackgroundSyncSnapshot> {
    let store = WorkspaceStore::open_app(db_path)?;
    let running_threads = store
        .list_running_chat_thread_summaries()?
        .into_iter()
        .map(|summary| BackgroundThreadSnapshot {
            workspace: summary.workspace,
            thread_id: summary.thread_id,
            title: summary.title,
            provider: summary.provider,
            status: summary.status,
            latest_message_id: summary.latest_message_id,
            latest_provider_sequence: summary.latest_provider_sequence,
            running_session_id: summary.running_session_id,
            updated_at: summary.updated_at,
        })
        .collect();
    Ok(BackgroundSyncSnapshot { running_threads })
}

pub(crate) fn load_workspace_chat_nav(
    store: &WorkspaceStore,
    workspace: &str,
    selected_thread: Option<i64>,
) -> Result<Vec<WorkspaceChatNavItem>> {
    let threads = store.list_chat_threads(workspace)?;
    let provider_store = ProviderEventStore::new(store.db_path());
    let working_threads = threads
        .iter()
        .filter_map(|thread| {
            let events = match provider_store.list_for_chat_thread(thread.id) {
                Ok(events) => events,
                Err(err) => return Some(Err(err)),
            };
            provider_events_have_active_work(&events).then_some(Ok(thread.id))
        })
        .collect::<Result<HashSet<_>>>()?;
    Ok(threads
        .into_iter()
        .map(|thread| workspace_chat_nav_item(&thread, &working_threads, selected_thread))
        .collect())
}

pub(crate) fn provider_events_have_active_work(events: &[ProviderEventRecord]) -> bool {
    let terminal_keys = events
        .iter()
        .filter(|event| provider_event_phase_is_terminal(event.phase))
        .map(provider_event_activity_key)
        .collect::<HashSet<_>>();
    if events.iter().any(|event| {
        event.kind == ProviderEventKind::Turn
            && provider_event_phase_is_active(event.phase)
            && !terminal_keys.contains(&provider_event_activity_key(event))
    }) {
        return true;
    }

    provider_projection_from_records(events)
        .items
        .iter()
        .any(|item| {
            item.status == ProviderProjectionStatus::Running
                && provider_projection_item_is_relevant_chat_event(item)
        })
}

fn provider_event_phase_is_active(phase: ProviderEventPhase) -> bool {
    matches!(
        phase,
        ProviderEventPhase::Started | ProviderEventPhase::Delta | ProviderEventPhase::Progress
    )
}

fn provider_event_phase_is_terminal(phase: ProviderEventPhase) -> bool {
    matches!(
        phase,
        ProviderEventPhase::Completed
            | ProviderEventPhase::Failed
            | ProviderEventPhase::Declined
            | ProviderEventPhase::Interrupted
    )
}

fn provider_event_activity_key(event: &ProviderEventRecord) -> String {
    if let Some(item_id) = event.provider_item_id.as_deref() {
        return format!(
            "{}:{}:item:{item_id}",
            event.provider,
            event.provider_thread_id.as_deref().unwrap_or("-")
        );
    }
    if let Some(turn_id) = event.provider_turn_id.as_deref() {
        return format!(
            "{}:{}:turn:{turn_id}",
            event.provider,
            event.provider_thread_id.as_deref().unwrap_or("-")
        );
    }
    if let Some(event_id) = event.provider_event_id.as_deref() {
        return format!(
            "{}:{}:event:{event_id}",
            event.provider,
            event.provider_thread_id.as_deref().unwrap_or("-")
        );
    }
    event.identity_key.clone()
}

fn workspace_chat_nav_item(
    thread: &ChatThreadRecord,
    running_threads: &HashSet<i64>,
    selected_thread: Option<i64>,
) -> WorkspaceChatNavItem {
    let running = running_threads.contains(&thread.id);
    WorkspaceChatNavItem {
        thread_id: thread.id,
        title: thread.title.clone(),
        provider: thread.provider.clone(),
        status: thread.status.clone(),
        running,
        unread: running && selected_thread != Some(thread.id),
        updated_at: thread.updated_at.clone(),
    }
}

pub fn diff_background_sync(
    previous: &BackgroundSyncSnapshot,
    next: &BackgroundSyncSnapshot,
) -> Vec<RefreshEvent> {
    let previous_by_thread = snapshot_by_thread(previous);
    let next_by_thread = snapshot_by_thread(next);
    let mut lifecycle_workspaces = BTreeSet::new();
    let mut events = Vec::new();

    for (key, next_thread) in &next_by_thread {
        let Some(previous_thread) = previous_by_thread.get(key) else {
            lifecycle_workspaces.insert(next_thread.workspace.clone());
            continue;
        };

        if previous_thread.title != next_thread.title
            || previous_thread.provider != next_thread.provider
            || previous_thread.status != next_thread.status
            || previous_thread.running_session_id != next_thread.running_session_id
        {
            lifecycle_workspaces.insert(next_thread.workspace.clone());
        }

        if previous_thread.latest_message_id != next_thread.latest_message_id
            || previous_thread.latest_provider_sequence != next_thread.latest_provider_sequence
        {
            events.push(RefreshEvent::WorkspaceChatMessagesChanged {
                workspace: next_thread.workspace.clone(),
                thread_id: next_thread.thread_id,
            });
        }
    }

    for (key, previous_thread) in &previous_by_thread {
        if !next_by_thread.contains_key(key) {
            lifecycle_workspaces.insert(previous_thread.workspace.clone());
        }
    }

    events.extend(
        lifecycle_workspaces
            .into_iter()
            .map(|workspace| RefreshEvent::WorkspaceChatLifecycleChanged { workspace }),
    );

    events
}

pub(crate) fn coalesce_refresh_events(events: Vec<RefreshEvent>) -> Vec<RefreshEvent> {
    let mut seen_messages = BTreeSet::new();
    let mut seen_lifecycle = BTreeSet::new();
    let mut coalesced = Vec::new();

    for event in events {
        match &event {
            RefreshEvent::WorkspaceChatMessagesChanged {
                workspace,
                thread_id,
            } => {
                if seen_messages.insert((workspace.clone(), *thread_id)) {
                    coalesced.push(event);
                }
            }
            RefreshEvent::WorkspaceChatLifecycleChanged { workspace } => {
                if seen_lifecycle.insert(workspace.clone()) {
                    coalesced.push(event);
                }
            }
            _ => coalesced.push(event),
        }
    }

    coalesced
}

fn snapshot_by_thread(
    snapshot: &BackgroundSyncSnapshot,
) -> BTreeMap<(String, i64), &BackgroundThreadSnapshot> {
    snapshot
        .running_threads
        .iter()
        .map(|thread| ((thread.workspace.clone(), thread.thread_id), thread))
        .collect()
}

#[cfg(test)]
mod tests {
    use archductor_core::provider_events::{
        ProviderEventDraft, ProviderEventKind, ProviderEventPhase, ProviderEventStore,
    };
    use rusqlite::{params, Connection};
    use serde_json::json;

    use super::*;

    fn thread_snapshot() -> BackgroundThreadSnapshot {
        BackgroundThreadSnapshot {
            workspace: "berlin".into(),
            thread_id: 7,
            title: "Fix auth".into(),
            provider: "codex".into(),
            status: "running".into(),
            latest_message_id: Some(11),
            latest_provider_sequence: Some(99),
            running_session_id: Some(22),
            updated_at: "2026-07-18T12:00:00Z".into(),
        }
    }

    fn create_nav_fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("state.db");
        WorkspaceStore::open_app(&db_path).unwrap();
        let conn = Connection::open(&db_path).unwrap();
        let now = "1";
        conn.execute(
            "INSERT INTO repositories (
                id, name, root_path, default_branch, remote_name, workspace_parent_path, created_at, updated_at
             ) VALUES (1, 'repo', ?1, 'main', 'origin', ?2, ?3, ?3)",
            params![
                temp.path().join("repo").display().to_string(),
                temp.path().join("workspaces").display().to_string(),
                now
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO workspaces (
                id, repository_id, name, path, branch, base_ref, port_base, status, created_at, updated_at
             ) VALUES (1, 1, 'berlin', ?1, 'feature/berlin', 'main', 3000, 'active', ?2, ?2)",
            params![temp.path().join("workspaces/berlin").display().to_string(), now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chat_threads (
                id, workspace_id, provider, title, status, created_at, updated_at
             ) VALUES (7, 1, 'codex', 'Fix auth', 'active', ?1, ?1)",
            [now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO processes (
                id, workspace_id, chat_thread_id, kind, command, pid, log_path, status, started_at
             ) VALUES (11, 1, 7, 'session', 'codex', 1000, '/tmp/session.log', 'running', ?1)",
            [now],
        )
        .unwrap();
        (temp, db_path)
    }

    fn provider_event_draft(thread_id: i64, phase: ProviderEventPhase) -> ProviderEventDraft {
        ProviderEventDraft {
            provider: "codex".to_owned(),
            provider_event_id: Some("event-1".to_owned()),
            provider_item_id: Some("turn-1".to_owned()),
            provider_thread_id: Some("native-thread-1".to_owned()),
            provider_turn_id: Some("turn-1".to_owned()),
            parent_provider_item_id: None,
            parent_provider_thread_id: None,
            workspace_id: Some(1),
            chat_thread_id: Some(thread_id),
            process_id: Some(11),
            phase,
            kind: ProviderEventKind::Turn,
            provider_subtype: Some("turn".to_owned()),
            provider_sequence: Some(1),
            occurred_at_ms: 42,
            normalized_payload: json!({"title": "Turn", "body": "working"}),
            raw_json: json!({"method": "turn/started"}),
            schema_version: 1,
            adapter_version: "test".to_owned(),
        }
    }

    #[test]
    fn diff_reports_new_running_thread_as_lifecycle_change() {
        let previous = BackgroundSyncSnapshot::default();
        let next = BackgroundSyncSnapshot {
            running_threads: vec![thread_snapshot()],
        };

        let events = diff_background_sync(&previous, &next);

        assert!(
            events.contains(&RefreshEvent::WorkspaceChatLifecycleChanged {
                workspace: "berlin".into(),
            })
        );
    }

    #[test]
    fn diff_reports_message_marker_change_without_lifecycle_change() {
        let previous = BackgroundSyncSnapshot {
            running_threads: vec![thread_snapshot()],
        };
        let mut changed = thread_snapshot();
        changed.latest_message_id = Some(12);
        let next = BackgroundSyncSnapshot {
            running_threads: vec![changed],
        };

        let events = diff_background_sync(&previous, &next);

        assert_eq!(
            events,
            vec![RefreshEvent::WorkspaceChatMessagesChanged {
                workspace: "berlin".into(),
                thread_id: 7,
            }]
        );
    }

    #[test]
    fn diff_reports_title_change_as_lifecycle_change() {
        let previous = BackgroundSyncSnapshot {
            running_threads: vec![thread_snapshot()],
        };
        let mut changed = thread_snapshot();
        changed.title = "Fix login".into();
        let next = BackgroundSyncSnapshot {
            running_threads: vec![changed],
        };

        let events = diff_background_sync(&previous, &next);

        assert_eq!(
            events,
            vec![RefreshEvent::WorkspaceChatLifecycleChanged {
                workspace: "berlin".into(),
            }]
        );
    }

    #[test]
    fn diff_ignores_timestamp_only_thread_change() {
        let previous = BackgroundSyncSnapshot {
            running_threads: vec![thread_snapshot()],
        };
        let mut changed = thread_snapshot();
        changed.updated_at = "2026-07-18T12:00:01Z".into();
        let next = BackgroundSyncSnapshot {
            running_threads: vec![changed],
        };

        let events = diff_background_sync(&previous, &next);

        assert!(events.is_empty());
    }

    #[test]
    fn diff_coalesces_lifecycle_changes_by_workspace() {
        let mut first = thread_snapshot();
        first.thread_id = 7;
        let mut second = thread_snapshot();
        second.thread_id = 8;
        second.title = "Fix review".into();
        let previous = BackgroundSyncSnapshot {
            running_threads: vec![first.clone(), second.clone()],
        };
        first.title = "Fix login".into();
        second.status = "idle".into();
        let next = BackgroundSyncSnapshot {
            running_threads: vec![first, second],
        };

        let events = diff_background_sync(&previous, &next);

        assert_eq!(
            events,
            vec![RefreshEvent::WorkspaceChatLifecycleChanged {
                workspace: "berlin".into(),
            }]
        );
    }

    #[test]
    fn coalesces_duplicate_chat_message_events_per_thread() {
        let events = vec![
            RefreshEvent::WorkspaceChatMessagesChanged {
                workspace: "berlin".to_string(),
                thread_id: 7,
            },
            RefreshEvent::WorkspaceChatMessagesChanged {
                workspace: "berlin".to_string(),
                thread_id: 7,
            },
        ];

        assert_eq!(coalesce_refresh_events(events).len(), 1);
    }

    #[test]
    fn preserves_distinct_chat_threads_when_coalescing() {
        let events = vec![
            RefreshEvent::WorkspaceChatMessagesChanged {
                workspace: "berlin".to_string(),
                thread_id: 7,
            },
            RefreshEvent::WorkspaceChatMessagesChanged {
                workspace: "berlin".to_string(),
                thread_id: 8,
            },
        ];

        assert_eq!(coalesce_refresh_events(events).len(), 2);
    }

    #[test]
    fn selected_running_thread_is_not_unread() {
        let thread = ChatThreadRecord {
            id: 7,
            workspace_id: 1,
            provider: "codex".to_owned(),
            title: "Fix auth".to_owned(),
            status: "active".to_owned(),
            native_thread_id: None,
            harness_metadata: None,
            created_at: "then".to_owned(),
            updated_at: "now".to_owned(),
            archived_at: None,
        };
        let running_threads = HashSet::from([7]);

        let item = workspace_chat_nav_item(&thread, &running_threads, Some(7));

        assert!(item.running);
        assert!(!item.unread);
    }

    #[test]
    fn workspace_chat_nav_does_not_mark_idle_running_session_as_working() {
        let (_temp, db_path) = create_nav_fixture();
        let store = WorkspaceStore::open_app(&db_path).unwrap();

        let items = load_workspace_chat_nav(&store, "berlin", Some(7)).unwrap();

        assert_eq!(items.len(), 1);
        assert!(!items[0].running);
        assert!(!items[0].unread);
    }

    #[test]
    fn workspace_chat_nav_marks_active_provider_turn_as_working() {
        let (_temp, db_path) = create_nav_fixture();
        ProviderEventStore::new(&db_path)
            .upsert_event(&provider_event_draft(7, ProviderEventPhase::Started))
            .unwrap();
        let store = WorkspaceStore::open_app(&db_path).unwrap();

        let items = load_workspace_chat_nav(&store, "berlin", Some(8)).unwrap();

        assert_eq!(items.len(), 1);
        assert!(items[0].running);
        assert!(items[0].unread);
    }

    #[test]
    fn workspace_chat_nav_clears_working_after_terminal_provider_turn() {
        let (_temp, db_path) = create_nav_fixture();
        let provider_store = ProviderEventStore::new(&db_path);
        provider_store
            .upsert_event(&provider_event_draft(7, ProviderEventPhase::Started))
            .unwrap();
        provider_store
            .upsert_event(&provider_event_draft(7, ProviderEventPhase::Completed))
            .unwrap();
        let store = WorkspaceStore::open_app(&db_path).unwrap();

        let items = load_workspace_chat_nav(&store, "berlin", Some(8)).unwrap();

        assert_eq!(items.len(), 1);
        assert!(!items[0].running);
        assert!(!items[0].unread);
    }

    #[test]
    fn non_selected_running_thread_is_unread() {
        let thread = ChatThreadRecord {
            id: 7,
            workspace_id: 1,
            provider: "codex".to_owned(),
            title: "Fix auth".to_owned(),
            status: "active".to_owned(),
            native_thread_id: None,
            harness_metadata: None,
            created_at: "then".to_owned(),
            updated_at: "now".to_owned(),
            archived_at: None,
        };
        let running_threads = HashSet::from([7]);

        let item = workspace_chat_nav_item(&thread, &running_threads, Some(8));

        assert!(item.running);
        assert!(item.unread);
    }
}
