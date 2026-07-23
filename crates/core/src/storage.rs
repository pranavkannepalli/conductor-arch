use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

pub(crate) fn migrate_workspace_db(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS repositories (
          id INTEGER PRIMARY KEY,
          name TEXT NOT NULL,
          root_path TEXT NOT NULL UNIQUE,
          default_branch TEXT NOT NULL,
          remote_name TEXT NOT NULL DEFAULT 'origin',
          workspace_parent_path TEXT NOT NULL,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS workspaces (
          id INTEGER PRIMARY KEY,
          repository_id INTEGER NOT NULL REFERENCES repositories(id),
          name TEXT NOT NULL,
          path TEXT NOT NULL UNIQUE,
          branch TEXT NOT NULL,
          base_ref TEXT NOT NULL,
          port_base INTEGER NOT NULL,
          status TEXT NOT NULL,
          archived_at TEXT,
          agent_metadata_applied_at TEXT,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS processes (
          id INTEGER PRIMARY KEY,
          workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
          chat_thread_id INTEGER REFERENCES chat_threads(id),
          kind TEXT NOT NULL,
          command TEXT NOT NULL,
          pid INTEGER NOT NULL,
          log_path TEXT NOT NULL,
          status TEXT NOT NULL,
          started_at TEXT NOT NULL,
          ended_at TEXT,
          session_harness_metadata TEXT,
          session_resume_id TEXT
        );

        CREATE TABLE IF NOT EXISTS pull_requests (
          id INTEGER PRIMARY KEY,
          workspace_id INTEGER NOT NULL UNIQUE REFERENCES workspaces(id),
          provider TEXT NOT NULL,
          number INTEGER NOT NULL,
          url TEXT NOT NULL,
          state TEXT NOT NULL,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS todos (
          id INTEGER PRIMARY KEY,
          workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
          text TEXT NOT NULL,
          status TEXT NOT NULL,
          source TEXT NOT NULL,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS review_comments (
          id INTEGER PRIMARY KEY,
          workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
          file_path TEXT NOT NULL,
          line_number INTEGER,
          body TEXT NOT NULL,
          status TEXT NOT NULL,
          github_thread_id TEXT,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS checkpoints (
          id INTEGER PRIMARY KEY,
          workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
          session_id INTEGER REFERENCES processes(id),
          git_ref TEXT NOT NULL,
          message TEXT NOT NULL,
          created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS spotlight_sessions (
          id INTEGER PRIMARY KEY,
          repository_id INTEGER NOT NULL REFERENCES repositories(id),
          workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
          workspace_name TEXT NOT NULL,
          patch_path TEXT NOT NULL,
          status TEXT NOT NULL,
          started_at TEXT NOT NULL,
          ended_at TEXT
        );

        CREATE TABLE IF NOT EXISTS linked_directories (
          id INTEGER PRIMARY KEY,
          workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
          target_workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
          created_at TEXT NOT NULL,
          UNIQUE(workspace_id, target_workspace_id)
        );

        CREATE TABLE IF NOT EXISTS chat_threads (
          id INTEGER PRIMARY KEY,
          workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
          provider TEXT NOT NULL,
          title TEXT NOT NULL,
          status TEXT NOT NULL,
          native_thread_id TEXT,
          harness_metadata TEXT,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          archived_at TEXT
        );

        CREATE TABLE IF NOT EXISTS chat_messages (
          id INTEGER PRIMARY KEY,
          thread_id INTEGER NOT NULL REFERENCES chat_threads(id),
          role TEXT NOT NULL,
          content TEXT NOT NULL,
          source TEXT NOT NULL,
          timeline_seq INTEGER,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS chat_queued_inputs (
          id INTEGER PRIMARY KEY,
          thread_id INTEGER NOT NULL REFERENCES chat_threads(id) ON DELETE CASCADE,
          input TEXT NOT NULL,
          visible_input TEXT,
          input_kind TEXT NOT NULL,
          session_kind TEXT NOT NULL,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_chat_queued_inputs_thread
          ON chat_queued_inputs(thread_id, id);

        CREATE TABLE IF NOT EXISTS chat_timeline_seq (
          id INTEGER PRIMARY KEY AUTOINCREMENT
        );

        CREATE TABLE IF NOT EXISTS codex_parse_cursors (
          process_id INTEGER PRIMARY KEY REFERENCES processes(id) ON DELETE CASCADE,
          fingerprint TEXT,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS chat_events (
          id INTEGER PRIMARY KEY,
          thread_id INTEGER NOT NULL REFERENCES chat_threads(id) ON DELETE CASCADE,
          process_id INTEGER NOT NULL REFERENCES processes(id) ON DELETE CASCADE,
          kind TEXT NOT NULL,
          title TEXT NOT NULL,
          body TEXT NOT NULL DEFAULT '',
          path TEXT,
          payload_json TEXT NOT NULL,
          timeline_seq INTEGER NOT NULL,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS session_events (
          id INTEGER PRIMARY KEY,
          process_id INTEGER NOT NULL REFERENCES processes(id) ON DELETE CASCADE,
          sequence INTEGER NOT NULL,
          occurred_at_ms INTEGER NOT NULL,
          source TEXT NOT NULL,
          raw_text TEXT,
          payload_json TEXT NOT NULL,
          created_at TEXT NOT NULL,
          UNIQUE(process_id, sequence)
        );

        CREATE TABLE IF NOT EXISTS pty_chunks (
          id INTEGER PRIMARY KEY,
          process_id INTEGER NOT NULL REFERENCES processes(id) ON DELETE CASCADE,
          sequence INTEGER NOT NULL,
          occurred_at_ms INTEGER NOT NULL,
          stream TEXT NOT NULL DEFAULT 'stdout_pty',
          text TEXT NOT NULL,
          created_at TEXT NOT NULL,
          UNIQUE(process_id, sequence)
        );

        CREATE TABLE IF NOT EXISTS provider_events (
          id INTEGER PRIMARY KEY,
          identity_key TEXT NOT NULL UNIQUE,
          provider TEXT NOT NULL,
          provider_event_id TEXT,
          provider_item_id TEXT,
          provider_thread_id TEXT,
          provider_turn_id TEXT,
          parent_provider_item_id TEXT,
          parent_provider_thread_id TEXT,
          workspace_id INTEGER REFERENCES workspaces(id) ON DELETE CASCADE,
          chat_thread_id INTEGER REFERENCES chat_threads(id) ON DELETE CASCADE,
          process_id INTEGER REFERENCES processes(id) ON DELETE CASCADE,
          phase TEXT NOT NULL,
          kind TEXT NOT NULL,
          provider_subtype TEXT,
          provider_sequence INTEGER,
          received_sequence INTEGER NOT NULL,
          timeline_seq INTEGER,
          occurred_at_ms INTEGER NOT NULL,
          normalized_payload_json TEXT NOT NULL,
          raw_json TEXT NOT NULL,
          schema_version INTEGER NOT NULL,
          adapter_version TEXT NOT NULL,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_provider_events_thread
          ON provider_events(provider, provider_thread_id, received_sequence, id);

        CREATE INDEX IF NOT EXISTS idx_provider_events_chat_thread
          ON provider_events(chat_thread_id, received_sequence, id);

        CREATE INDEX IF NOT EXISTS idx_provider_events_process
          ON provider_events(process_id, received_sequence, id);

        CREATE TABLE IF NOT EXISTS provider_inputs (
          id TEXT PRIMARY KEY,
          provider TEXT NOT NULL,
          thread_id INTEGER NOT NULL REFERENCES chat_threads(id) ON DELETE CASCADE,
          process_id INTEGER NOT NULL REFERENCES processes(id) ON DELETE CASCADE,
          native_session_id TEXT,
          input_kind TEXT NOT NULL,
          delivery TEXT NOT NULL,
          provider_input TEXT NOT NULL,
          visible_input TEXT,
          state TEXT NOT NULL,
          acknowledgement TEXT,
          error TEXT,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_provider_inputs_process
          ON provider_inputs(process_id, state, updated_at);

        CREATE INDEX IF NOT EXISTS idx_provider_inputs_thread
          ON provider_inputs(thread_id, state, updated_at);

        CREATE TABLE IF NOT EXISTS provider_interactions (
          id TEXT PRIMARY KEY,
          provider_key TEXT NOT NULL,
          workspace TEXT NOT NULL,
          thread_id INTEGER NOT NULL,
          session_id INTEGER NOT NULL,
          native_session_id TEXT,
          native_id TEXT NOT NULL,
          kind TEXT NOT NULL,
          title TEXT NOT NULL,
          detail TEXT NOT NULL,
          choices_json TEXT NOT NULL,
          native_request_json TEXT NOT NULL,
          request_fingerprint TEXT NOT NULL,
          status TEXT NOT NULL,
          resolution_json TEXT,
          native_response_json TEXT,
          error TEXT,
          created_at TEXT NOT NULL,
          resolved_at TEXT,
          consumed_at TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_provider_interactions_thread
          ON provider_interactions(thread_id, status, created_at);

        CREATE INDEX IF NOT EXISTS idx_provider_interactions_fingerprint
          ON provider_interactions(request_fingerprint, status, created_at);

        CREATE UNIQUE INDEX IF NOT EXISTS idx_provider_interactions_pending_fingerprint
          ON provider_interactions(request_fingerprint)
          WHERE status = 'pending';

        CREATE TABLE IF NOT EXISTS provider_event_raw_payloads (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          identity_key TEXT NOT NULL,
          provider TEXT NOT NULL,
          chat_thread_id INTEGER,
          process_id INTEGER,
          phase TEXT NOT NULL,
          kind TEXT NOT NULL,
          provider_sequence INTEGER,
          raw_sequence INTEGER NOT NULL,
          occurred_at_ms INTEGER NOT NULL,
          raw_json TEXT NOT NULL,
          created_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_provider_event_raw_payloads_identity
          ON provider_event_raw_payloads(identity_key, raw_sequence, id);

        CREATE INDEX IF NOT EXISTS idx_provider_event_raw_payloads_chat_thread
          ON provider_event_raw_payloads(chat_thread_id, raw_sequence, id);

        CREATE TABLE IF NOT EXISTS workspace_timeline (
          id INTEGER PRIMARY KEY,
          workspace_id INTEGER NOT NULL,
          workspace_name TEXT NOT NULL,
          kind TEXT NOT NULL,
          summary TEXT NOT NULL,
          created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS workspace_ui_state (
          workspace_id INTEGER PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE,
          changes_scope TEXT,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS workspace_lifecycle_jobs (
          id INTEGER PRIMARY KEY,
          kind TEXT NOT NULL,
          status TEXT NOT NULL,
          workspace_id INTEGER,
          payload_json TEXT NOT NULL,
          error TEXT,
          attempts INTEGER NOT NULL DEFAULT 0,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          started_at TEXT,
          finished_at TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_workspace_lifecycle_jobs_status
          ON workspace_lifecycle_jobs(status, id);
        ",
    )?;
    remove_chat_events_exact_unique_constraint(conn)?;
    ensure_column(
        conn,
        "processes",
        "exit_code",
        "ALTER TABLE processes ADD COLUMN exit_code INTEGER",
    )?;
    ensure_column(
        conn,
        "processes",
        "session_harness_metadata",
        "ALTER TABLE processes ADD COLUMN session_harness_metadata TEXT",
    )?;
    ensure_column(
        conn,
        "processes",
        "session_resume_id",
        "ALTER TABLE processes ADD COLUMN session_resume_id TEXT",
    )?;
    ensure_column(
        conn,
        "processes",
        "chat_thread_id",
        "ALTER TABLE processes ADD COLUMN chat_thread_id INTEGER REFERENCES chat_threads(id)",
    )?;
    ensure_column(
        conn,
        "chat_messages",
        "timeline_seq",
        "ALTER TABLE chat_messages ADD COLUMN timeline_seq INTEGER",
    )?;
    ensure_column(
        conn,
        "workspaces",
        "agent_metadata_applied_at",
        "ALTER TABLE workspaces ADD COLUMN agent_metadata_applied_at TEXT",
    )?;
    ensure_column(
        conn,
        "provider_events",
        "timeline_seq",
        "ALTER TABLE provider_events ADD COLUMN timeline_seq INTEGER",
    )?;
    Ok(())
}

fn remove_chat_events_exact_unique_constraint(conn: &Connection) -> Result<()> {
    let create_sql = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'chat_events'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .unwrap_or_default();
    if !create_sql.contains("UNIQUE(thread_id, process_id, kind, title, body, payload_json)") {
        return Ok(());
    }

    conn.execute_batch(
        "
        ALTER TABLE chat_events RENAME TO chat_events_with_exact_unique;
        CREATE TABLE chat_events (
          id INTEGER PRIMARY KEY,
          thread_id INTEGER NOT NULL REFERENCES chat_threads(id) ON DELETE CASCADE,
          process_id INTEGER NOT NULL REFERENCES processes(id) ON DELETE CASCADE,
          kind TEXT NOT NULL,
          title TEXT NOT NULL,
          body TEXT NOT NULL DEFAULT '',
          path TEXT,
          payload_json TEXT NOT NULL,
          timeline_seq INTEGER NOT NULL,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );
        INSERT INTO chat_events (
          id, thread_id, process_id, kind, title, body, path, payload_json, timeline_seq, created_at, updated_at
        )
        SELECT id, thread_id, process_id, kind, title, body, path, payload_json, timeline_seq, created_at, updated_at
        FROM chat_events_with_exact_unique;
        DROP TABLE chat_events_with_exact_unique;
        ",
    )?;
    Ok(())
}

fn ensure_column(conn: &Connection, table: &str, column: &str, alter_sql: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !names.iter().any(|name| name == column) {
        conn.execute(alter_sql, [])?;
    }
    Ok(())
}
