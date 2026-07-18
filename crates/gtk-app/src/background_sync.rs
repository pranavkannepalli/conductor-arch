use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use archductor_core::workspace::WorkspaceStore;

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

pub fn diff_background_sync(
    previous: &BackgroundSyncSnapshot,
    next: &BackgroundSyncSnapshot,
) -> Vec<RefreshEvent> {
    let previous_by_thread = snapshot_by_thread(previous);
    let next_by_thread = snapshot_by_thread(next);
    let mut events = Vec::new();

    for (key, next_thread) in &next_by_thread {
        let Some(previous_thread) = previous_by_thread.get(key) else {
            events.push(RefreshEvent::WorkspaceChatLifecycleChanged {
                workspace: next_thread.workspace.clone(),
            });
            continue;
        };

        if previous_thread.title != next_thread.title
            || previous_thread.provider != next_thread.provider
            || previous_thread.status != next_thread.status
            || previous_thread.running_session_id != next_thread.running_session_id
            || previous_thread.updated_at != next_thread.updated_at
        {
            events.push(RefreshEvent::WorkspaceChatLifecycleChanged {
                workspace: next_thread.workspace.clone(),
            });
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
            events.push(RefreshEvent::WorkspaceChatLifecycleChanged {
                workspace: previous_thread.workspace.clone(),
            });
        }
    }

    events
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

    #[test]
    fn diff_reports_new_running_thread_as_lifecycle_change() {
        let previous = BackgroundSyncSnapshot::default();
        let next = BackgroundSyncSnapshot {
            running_threads: vec![thread_snapshot()],
        };

        let events = diff_background_sync(&previous, &next);

        assert!(events.contains(&RefreshEvent::WorkspaceChatLifecycleChanged {
            workspace: "berlin".into(),
        }));
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
}
