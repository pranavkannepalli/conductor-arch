use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderInputState {
    Queued,
    Written,
    Acknowledged,
    Terminal,
    Failed,
}

impl ProviderInputState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Written => "written",
            Self::Acknowledged => "acknowledged",
            Self::Terminal => "terminal",
            Self::Failed => "failed",
        }
    }

    fn from_str(value: &str) -> Self {
        match value {
            "written" => Self::Written,
            "acknowledged" => Self::Acknowledged,
            "terminal" => Self::Terminal,
            "failed" => Self::Failed,
            _ => Self::Queued,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderInputInput {
    pub id: String,
    pub provider: String,
    pub thread_id: i64,
    pub process_id: i64,
    pub native_session_id: Option<String>,
    pub input_kind: String,
    pub delivery: String,
    pub provider_input: String,
    pub visible_input: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderInputRecord {
    pub id: String,
    pub provider: String,
    pub thread_id: i64,
    pub process_id: i64,
    pub native_session_id: Option<String>,
    pub input_kind: String,
    pub delivery: String,
    pub provider_input: String,
    pub visible_input: Option<String>,
    pub state: ProviderInputState,
    pub acknowledgement: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct ProviderInputStore {
    db_path: PathBuf,
}

impl ProviderInputStore {
    pub fn new(db_path: PathBuf) -> Self {
        Self { db_path }
    }

    pub fn enqueue(&self, input: ProviderInputInput) -> Result<ProviderInputRecord> {
        let conn = self.open()?;
        let now = timestamp();
        conn.execute(
            "INSERT OR IGNORE INTO provider_inputs (
                id, provider, thread_id, process_id, native_session_id, input_kind, delivery,
                provider_input, visible_input, state, acknowledgement, error, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'queued', NULL, NULL, ?10, ?10)",
            params![
                input.id,
                input.provider,
                input.thread_id,
                input.process_id,
                input.native_session_id,
                input.input_kind,
                input.delivery,
                input.provider_input,
                input.visible_input,
                now,
            ],
        )?;
        Ok(self
            .get(input.id.as_str())?
            .expect("provider input inserted"))
    }

    pub fn mark_written(&self, id: &str) -> Result<()> {
        self.update_state(id, ProviderInputState::Written, None, None)
    }

    pub fn mark_acknowledged(&self, id: &str, acknowledgement: Option<&str>) -> Result<()> {
        self.update_state(id, ProviderInputState::Acknowledged, acknowledgement, None)
    }

    pub fn mark_terminal(&self, id: &str) -> Result<()> {
        self.update_state(id, ProviderInputState::Terminal, None, None)
    }

    pub fn mark_failed(&self, id: &str, error: &str) -> Result<()> {
        self.update_state(id, ProviderInputState::Failed, None, Some(error))
    }

    pub fn get(&self, id: &str) -> Result<Option<ProviderInputRecord>> {
        self.open()?
            .query_row(
                "SELECT id, provider, thread_id, process_id, native_session_id, input_kind,
                    delivery, provider_input, visible_input, state, acknowledgement, error,
                    created_at, updated_at
                 FROM provider_inputs
                 WHERE id = ?1",
                params![id],
                row_to_provider_input,
            )
            .optional()
            .map_err(Into::into)
    }

    fn update_state(
        &self,
        id: &str,
        state: ProviderInputState,
        acknowledgement: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        let now = timestamp();
        self.open()?.execute(
            "UPDATE provider_inputs
             SET state = ?2,
                 acknowledgement = COALESCE(?3, acknowledgement),
                 error = COALESCE(?4, error),
                 updated_at = ?5
             WHERE id = ?1",
            params![id, state.as_str(), acknowledgement, error, now],
        )?;
        Ok(())
    }

    fn open(&self) -> Result<Connection> {
        Ok(Connection::open(&self.db_path)?)
    }
}

fn row_to_provider_input(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProviderInputRecord> {
    let state: String = row.get(9)?;
    Ok(ProviderInputRecord {
        id: row.get(0)?,
        provider: row.get(1)?,
        thread_id: row.get(2)?,
        process_id: row.get(3)?,
        native_session_id: row.get(4)?,
        input_kind: row.get(5)?,
        delivery: row.get(6)?,
        provider_input: row.get(7)?,
        visible_input: row.get(8)?,
        state: ProviderInputState::from_str(&state),
        acknowledgement: row.get(10)?,
        error: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

fn timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::migrate_workspace_db;

    #[test]
    fn provider_inputs_transition_through_delivery_states() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("state.db");
        let conn = Connection::open(&db_path).unwrap();
        migrate_workspace_db(&conn).unwrap();
        seed_process_rows(&conn);
        drop(conn);
        let store = ProviderInputStore::new(db_path);

        let queued = store.enqueue(fixture_input("input-1")).unwrap();
        assert_eq!(queued.state, ProviderInputState::Queued);

        store.mark_written(&queued.id).unwrap();
        store
            .mark_acknowledged(&queued.id, Some("native-user-id"))
            .unwrap();
        store.mark_terminal(&queued.id).unwrap();

        let terminal = store.get(&queued.id).unwrap().unwrap();
        assert_eq!(terminal.state, ProviderInputState::Terminal);
        assert_eq!(terminal.acknowledgement.as_deref(), Some("native-user-id"));
    }

    #[test]
    fn provider_inputs_record_failures_without_deleting_evidence() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("state.db");
        let conn = Connection::open(&db_path).unwrap();
        migrate_workspace_db(&conn).unwrap();
        seed_process_rows(&conn);
        drop(conn);
        let store = ProviderInputStore::new(db_path);

        let queued = store.enqueue(fixture_input("input-2")).unwrap();
        store.mark_failed(&queued.id, "stdin closed").unwrap();

        let failed = store.get(&queued.id).unwrap().unwrap();
        assert_eq!(failed.state, ProviderInputState::Failed);
        assert_eq!(failed.error.as_deref(), Some("stdin closed"));
        assert_eq!(failed.provider_input, "run tests");
    }

    #[test]
    fn claude_input_recovery_keeps_acknowledged_input_nonterminal() {
        let (store, _temp) = seeded_store();
        let queued = store.enqueue(fixture_input("claude-input-1")).unwrap();

        store.mark_written(&queued.id).unwrap();
        store
            .mark_acknowledged(&queued.id, Some("native-user-id"))
            .unwrap();

        let acknowledged = store.get(&queued.id).unwrap().unwrap();
        assert_eq!(acknowledged.state, ProviderInputState::Acknowledged);
        assert_eq!(
            acknowledged.acknowledgement.as_deref(),
            Some("native-user-id")
        );
    }

    #[test]
    fn codex_input_recovery_keeps_written_unacknowledged_input_resendable() {
        let (store, _temp) = seeded_store();
        let queued = store.enqueue(fixture_input("codex-input-1")).unwrap();

        store.mark_written(&queued.id).unwrap();

        let written = store.get(&queued.id).unwrap().unwrap();
        assert_eq!(written.state, ProviderInputState::Written);
        assert!(written.acknowledgement.is_none());
    }

    #[test]
    fn reconcile_startup_provider_inputs_survive_existing_database_migration() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("state.db");
        let conn = Connection::open(&db_path).unwrap();
        migrate_workspace_db(&conn).unwrap();
        migrate_workspace_db(&conn).unwrap();
        seed_process_rows(&conn);
        drop(conn);

        let store = ProviderInputStore::new(db_path);
        let queued = store.enqueue(fixture_input("startup-input-1")).unwrap();

        assert_eq!(
            store.get(&queued.id).unwrap().unwrap().state,
            ProviderInputState::Queued
        );
    }

    fn fixture_input(id: &str) -> ProviderInputInput {
        ProviderInputInput {
            id: id.to_owned(),
            provider: "claude".to_owned(),
            thread_id: 7,
            process_id: 11,
            native_session_id: Some("session-1".to_owned()),
            input_kind: "user".to_owned(),
            delivery: "auto".to_owned(),
            provider_input: "run tests".to_owned(),
            visible_input: Some("run tests".to_owned()),
        }
    }

    fn seeded_store() -> (ProviderInputStore, tempfile::TempDir) {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("state.db");
        let conn = Connection::open(&db_path).unwrap();
        migrate_workspace_db(&conn).unwrap();
        seed_process_rows(&conn);
        drop(conn);
        (ProviderInputStore::new(db_path), temp)
    }

    fn seed_process_rows(conn: &Connection) {
        conn.execute_batch(
            "
            INSERT INTO repositories
              (id, name, root_path, default_branch, remote_name, workspace_parent_path, created_at, updated_at)
            VALUES
              (1, 'demo', '/tmp/demo', 'main', 'origin', '/tmp', '1', '1');
            INSERT INTO workspaces
              (id, repository_id, name, path, branch, base_ref, port_base, status, created_at, updated_at)
            VALUES
              (1, 1, 'berlin', '/tmp/demo/berlin', 'lc/berlin', 'main', 42000, 'active', '1', '1');
            INSERT INTO chat_threads
              (id, workspace_id, provider, title, status, created_at, updated_at)
            VALUES
              (7, 1, 'claude', 'Claude', 'active', '1', '1');
            INSERT INTO processes
              (id, workspace_id, chat_thread_id, kind, command, pid, log_path, status, started_at)
            VALUES
              (11, 1, 7, 'session', 'claude', 123, '/tmp/claude.log', 'running', '1');
            ",
        )
        .unwrap();
    }
}
