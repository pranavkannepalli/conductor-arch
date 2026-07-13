use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderEventPhase {
    Started,
    Delta,
    Progress,
    Completed,
    Failed,
    Declined,
    Interrupted,
    Unknown,
}

impl ProviderEventPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Delta => "delta",
            Self::Progress => "progress",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Declined => "declined",
            Self::Interrupted => "interrupted",
            Self::Unknown => "unknown",
        }
    }

    fn from_str(value: &str) -> Self {
        match value {
            "started" => Self::Started,
            "delta" => Self::Delta,
            "progress" => Self::Progress,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "declined" => Self::Declined,
            "interrupted" => Self::Interrupted,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderEventKind {
    AccountAuth,
    ThreadSession,
    GoalTask,
    Turn,
    UserInput,
    AssistantOutput,
    PlanningReasoning,
    CommandProcess,
    TerminalRuntime,
    FileSystem,
    DiffFileChange,
    Tool,
    Mcp,
    SkillPluginHook,
    ApprovalPermission,
    SubagentCollaboration,
    WebBrowserMedia,
    EnvironmentConfigModel,
    LimitFailure,
    Unknown,
}

impl ProviderEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AccountAuth => "account_auth",
            Self::ThreadSession => "thread_session",
            Self::GoalTask => "goal_task",
            Self::Turn => "turn",
            Self::UserInput => "user_input",
            Self::AssistantOutput => "assistant_output",
            Self::PlanningReasoning => "planning_reasoning",
            Self::CommandProcess => "command_process",
            Self::TerminalRuntime => "terminal_runtime",
            Self::FileSystem => "file_system",
            Self::DiffFileChange => "diff_file_change",
            Self::Tool => "tool",
            Self::Mcp => "mcp",
            Self::SkillPluginHook => "skill_plugin_hook",
            Self::ApprovalPermission => "approval_permission",
            Self::SubagentCollaboration => "subagent_collaboration",
            Self::WebBrowserMedia => "web_browser_media",
            Self::EnvironmentConfigModel => "environment_config_model",
            Self::LimitFailure => "limit_failure",
            Self::Unknown => "unknown",
        }
    }

    fn from_str(value: &str) -> Self {
        match value {
            "account_auth" => Self::AccountAuth,
            "thread_session" => Self::ThreadSession,
            "goal_task" => Self::GoalTask,
            "turn" => Self::Turn,
            "user_input" => Self::UserInput,
            "assistant_output" => Self::AssistantOutput,
            "planning_reasoning" => Self::PlanningReasoning,
            "command_process" => Self::CommandProcess,
            "terminal_runtime" => Self::TerminalRuntime,
            "file_system" => Self::FileSystem,
            "diff_file_change" => Self::DiffFileChange,
            "tool" => Self::Tool,
            "mcp" => Self::Mcp,
            "skill_plugin_hook" => Self::SkillPluginHook,
            "approval_permission" => Self::ApprovalPermission,
            "subagent_collaboration" => Self::SubagentCollaboration,
            "web_browser_media" => Self::WebBrowserMedia,
            "environment_config_model" => Self::EnvironmentConfigModel,
            "limit_failure" => Self::LimitFailure,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderActionCategory {
    pub kind: ProviderEventKind,
    pub label: &'static str,
    pub examples: &'static [&'static str],
}

pub const PROVIDER_ACTION_CATEGORIES: &[ProviderActionCategory] = &[
    ProviderActionCategory {
        kind: ProviderEventKind::AccountAuth,
        label: "account/auth",
        examples: &[
            "login",
            "logout",
            "token refresh",
            "account update",
            "rate limits",
        ],
    },
    ProviderActionCategory {
        kind: ProviderEventKind::ThreadSession,
        label: "thread/session lifecycle",
        examples: &[
            "start", "resume", "fork", "archive", "delete", "close", "status",
        ],
    },
    ProviderActionCategory {
        kind: ProviderEventKind::GoalTask,
        label: "goal/task lifecycle",
        examples: &["set goal", "clear goal", "task progress", "todo update"],
    },
    ProviderActionCategory {
        kind: ProviderEventKind::Turn,
        label: "turn lifecycle",
        examples: &["start", "steer", "interrupt", "complete", "fail", "compact"],
    },
    ProviderActionCategory {
        kind: ProviderEventKind::UserInput,
        label: "user input",
        examples: &[
            "message",
            "review prompt",
            "control command",
            "user question response",
        ],
    },
    ProviderActionCategory {
        kind: ProviderEventKind::AssistantOutput,
        label: "assistant output",
        examples: &[
            "text delta",
            "final message",
            "partial message",
            "structured output",
        ],
    },
    ProviderActionCategory {
        kind: ProviderEventKind::PlanningReasoning,
        label: "planning/reasoning",
        examples: &["plan delta", "reasoning delta", "reasoning summary"],
    },
    ProviderActionCategory {
        kind: ProviderEventKind::CommandProcess,
        label: "command/process execution",
        examples: &[
            "command start",
            "stdout/stderr delta",
            "exit code",
            "process kill",
        ],
    },
    ProviderActionCategory {
        kind: ProviderEventKind::TerminalRuntime,
        label: "terminal/background runtime",
        examples: &[
            "terminal interaction",
            "background terminal",
            "realtime transcript",
        ],
    },
    ProviderActionCategory {
        kind: ProviderEventKind::FileSystem,
        label: "file system",
        examples: &["read file", "write file", "copy", "remove", "watch"],
    },
    ProviderActionCategory {
        kind: ProviderEventKind::DiffFileChange,
        label: "diff/file changes",
        examples: &["edit", "patch", "file change output", "turn diff"],
    },
    ProviderActionCategory {
        kind: ProviderEventKind::Tool,
        label: "tools",
        examples: &[
            "native tool",
            "dynamic tool",
            "tool progress",
            "tool result",
        ],
    },
    ProviderActionCategory {
        kind: ProviderEventKind::Mcp,
        label: "MCP",
        examples: &[
            "server status",
            "OAuth",
            "resource read",
            "tool call",
            "elicitation",
        ],
    },
    ProviderActionCategory {
        kind: ProviderEventKind::SkillPluginHook,
        label: "skills/plugins/hooks",
        examples: &[
            "skill list",
            "skill read",
            "plugin import",
            "hook started",
            "hook completed",
        ],
    },
    ProviderActionCategory {
        kind: ProviderEventKind::ApprovalPermission,
        label: "approvals/permissions",
        examples: &[
            "command approval",
            "file approval",
            "permission request",
            "guardian review",
        ],
    },
    ProviderActionCategory {
        kind: ProviderEventKind::SubagentCollaboration,
        label: "subagents/collaboration",
        examples: &[
            "spawn",
            "send input",
            "wait",
            "resume",
            "nested transcript",
            "finish",
        ],
    },
    ProviderActionCategory {
        kind: ProviderEventKind::WebBrowserMedia,
        label: "web/browser/media",
        examples: &[
            "web search",
            "web fetch",
            "image view",
            "browser integration",
            "audio",
        ],
    },
    ProviderActionCategory {
        kind: ProviderEventKind::EnvironmentConfigModel,
        label: "environment/config/model",
        examples: &[
            "environment info",
            "config read/write",
            "model list",
            "model reroute",
        ],
    },
    ProviderActionCategory {
        kind: ProviderEventKind::LimitFailure,
        label: "limits/failures",
        examples: &[
            "context exhaustion",
            "quota",
            "rate limit",
            "warning",
            "provider error",
        ],
    },
    ProviderActionCategory {
        kind: ProviderEventKind::Unknown,
        label: "unknown future events",
        examples: &["lossless raw payload", "generic inspectable card"],
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderEventContext {
    pub workspace_id: Option<i64>,
    pub chat_thread_id: Option<i64>,
    pub process_id: Option<i64>,
    pub occurred_at_ms: u64,
    pub schema_version: i64,
    pub adapter_version: String,
}

impl ProviderEventContext {
    pub fn runtime(
        workspace_id: Option<i64>,
        chat_thread_id: Option<i64>,
        process_id: Option<i64>,
        adapter_version: impl Into<String>,
    ) -> Self {
        Self {
            workspace_id,
            chat_thread_id,
            process_id,
            occurred_at_ms: unix_millis(),
            schema_version: 1,
            adapter_version: adapter_version.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderEventDraft {
    pub provider: String,
    pub provider_event_id: Option<String>,
    pub provider_item_id: Option<String>,
    pub provider_thread_id: Option<String>,
    pub provider_turn_id: Option<String>,
    pub parent_provider_item_id: Option<String>,
    pub parent_provider_thread_id: Option<String>,
    pub workspace_id: Option<i64>,
    pub chat_thread_id: Option<i64>,
    pub process_id: Option<i64>,
    pub phase: ProviderEventPhase,
    pub kind: ProviderEventKind,
    pub provider_subtype: Option<String>,
    pub provider_sequence: Option<i64>,
    pub occurred_at_ms: u64,
    pub normalized_payload: Value,
    pub raw_json: Value,
    pub schema_version: i64,
    pub adapter_version: String,
}

impl ProviderEventDraft {
    pub fn identity_key(&self) -> String {
        if let Some(item_id) = &self.provider_item_id {
            let phase_key = match self.phase {
                ProviderEventPhase::Started
                | ProviderEventPhase::Delta
                | ProviderEventPhase::Progress => "stream",
                _ => self.phase.as_str(),
            };
            return format!(
                "{}:{}:{}:{}",
                self.provider,
                self.provider_thread_id.as_deref().unwrap_or("-"),
                item_id,
                phase_key
            );
        }
        if let Some(event_id) = &self.provider_event_id {
            return format!("{}:event:{event_id}", self.provider);
        }
        format!(
            "{}:raw:{}:{}:{}:{}:{}",
            self.provider,
            self.provider_thread_id.as_deref().unwrap_or("-"),
            self.provider_sequence.unwrap_or(0),
            self.provider_subtype.as_deref().unwrap_or("unknown"),
            self.occurred_at_ms,
            stable_text_hash(&self.raw_json.to_string())
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderEventRecord {
    pub id: i64,
    pub identity_key: String,
    pub provider: String,
    pub provider_event_id: Option<String>,
    pub provider_item_id: Option<String>,
    pub provider_thread_id: Option<String>,
    pub provider_turn_id: Option<String>,
    pub parent_provider_item_id: Option<String>,
    pub parent_provider_thread_id: Option<String>,
    pub workspace_id: Option<i64>,
    pub chat_thread_id: Option<i64>,
    pub process_id: Option<i64>,
    pub phase: ProviderEventPhase,
    pub kind: ProviderEventKind,
    pub provider_subtype: Option<String>,
    pub provider_sequence: Option<i64>,
    pub received_sequence: i64,
    pub occurred_at_ms: u64,
    pub normalized_payload: Value,
    pub raw_json: Value,
    pub schema_version: i64,
    pub adapter_version: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderTimelineItem {
    pub canonical_id: String,
    pub kind: ProviderEventKind,
    pub phase: ProviderEventPhase,
    pub title: String,
    pub body: String,
    pub provider_subtype: Option<String>,
    pub parent_canonical_id: Option<String>,
    pub raw_json: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderRawPayloadRecord {
    pub id: i64,
    pub identity_key: String,
    pub provider: String,
    pub chat_thread_id: Option<i64>,
    pub process_id: Option<i64>,
    pub phase: ProviderEventPhase,
    pub kind: ProviderEventKind,
    pub provider_sequence: Option<i64>,
    pub raw_sequence: i64,
    pub occurred_at_ms: u64,
    pub raw_json: Value,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct ProviderEventStore {
    db_path: PathBuf,
}

impl ProviderEventStore {
    pub fn new(db_path: impl Into<PathBuf>) -> Self {
        Self {
            db_path: db_path.into(),
        }
    }

    pub fn upsert_event(&self, draft: &ProviderEventDraft) -> Result<ProviderEventRecord> {
        let mut conn = self.open()?;
        let identity_key = draft.identity_key();
        let raw_json = serde_json::to_string(&draft.raw_json)?;
        let now = timestamp();
        let tx = conn.transaction()?;
        let received_sequence = next_received_sequence(&tx)?;
        let raw_sequence = next_raw_sequence(&tx)?;
        let normalized_payload = merge_existing_streaming_payload(&tx, &identity_key, draft)?;
        let normalized_payload_json = serde_json::to_string(&normalized_payload)?;
        tx.execute(
            "INSERT INTO provider_events (
                identity_key, provider, provider_event_id, provider_item_id,
                provider_thread_id, provider_turn_id, parent_provider_item_id,
                parent_provider_thread_id, workspace_id, chat_thread_id, process_id,
                phase, kind, provider_subtype, provider_sequence, received_sequence,
                occurred_at_ms, normalized_payload_json, raw_json, schema_version,
                adapter_version, created_at, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?22
             )
             ON CONFLICT(identity_key) DO UPDATE SET
                provider_event_id = excluded.provider_event_id,
                provider_item_id = excluded.provider_item_id,
                provider_thread_id = excluded.provider_thread_id,
                provider_turn_id = excluded.provider_turn_id,
                parent_provider_item_id = excluded.parent_provider_item_id,
                parent_provider_thread_id = excluded.parent_provider_thread_id,
                workspace_id = excluded.workspace_id,
                chat_thread_id = excluded.chat_thread_id,
                process_id = excluded.process_id,
                phase = excluded.phase,
                kind = excluded.kind,
                provider_subtype = excluded.provider_subtype,
                provider_sequence = excluded.provider_sequence,
                occurred_at_ms = excluded.occurred_at_ms,
                normalized_payload_json = excluded.normalized_payload_json,
                raw_json = excluded.raw_json,
                schema_version = excluded.schema_version,
                adapter_version = excluded.adapter_version,
                updated_at = excluded.updated_at",
            params![
                identity_key,
                draft.provider,
                draft.provider_event_id,
                draft.provider_item_id,
                draft.provider_thread_id,
                draft.provider_turn_id,
                draft.parent_provider_item_id,
                draft.parent_provider_thread_id,
                draft.workspace_id,
                draft.chat_thread_id,
                draft.process_id,
                draft.phase.as_str(),
                draft.kind.as_str(),
                draft.provider_subtype,
                draft.provider_sequence,
                received_sequence,
                draft.occurred_at_ms as i64,
                normalized_payload_json,
                raw_json,
                draft.schema_version,
                draft.adapter_version,
                now
            ],
        )?;
        tx.execute(
            "INSERT INTO provider_event_raw_payloads (
                identity_key, provider, chat_thread_id, process_id, phase, kind,
                provider_sequence, raw_sequence, occurred_at_ms, raw_json, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                identity_key,
                draft.provider,
                draft.chat_thread_id,
                draft.process_id,
                draft.phase.as_str(),
                draft.kind.as_str(),
                draft.provider_sequence,
                raw_sequence,
                draft.occurred_at_ms as i64,
                raw_json,
                now
            ],
        )?;
        tx.commit()?;
        self.get_by_identity_key(&identity_key)?
            .ok_or_else(|| anyhow!("provider event upsert did not return a row"))
    }

    pub fn get_by_identity_key(&self, identity_key: &str) -> Result<Option<ProviderEventRecord>> {
        let conn = self.open()?;
        conn.query_row(
            provider_event_select_sql("WHERE identity_key = ?1").as_str(),
            [identity_key],
            row_to_provider_event,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_for_chat_thread(&self, chat_thread_id: i64) -> Result<Vec<ProviderEventRecord>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            provider_event_select_sql(
                "WHERE chat_thread_id = ?1 ORDER BY received_sequence ASC, id ASC",
            )
            .as_str(),
        )?;
        let rows = stmt
            .query_map([chat_thread_id], row_to_provider_event)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn list_raw_payloads_for_identity(
        &self,
        identity_key: &str,
    ) -> Result<Vec<ProviderRawPayloadRecord>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT id, identity_key, provider, chat_thread_id, process_id, phase, kind,
                    provider_sequence, raw_sequence, occurred_at_ms, raw_json, created_at
             FROM provider_event_raw_payloads
             WHERE identity_key = ?1
             ORDER BY raw_sequence ASC, id ASC",
        )?;
        let rows = stmt
            .query_map([identity_key], row_to_provider_raw_payload)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn max_provider_sequence_for_process_subtypes(
        &self,
        process_id: i64,
        kind: ProviderEventKind,
        subtypes: &[&str],
    ) -> Result<Option<u64>> {
        let conn = self.open()?;
        let mut max_sequence = None;
        for subtype in subtypes {
            let subtype_max: Option<i64> = conn.query_row(
                "SELECT MAX(provider_sequence)
                 FROM provider_events
                 WHERE process_id = ?1 AND kind = ?2 AND provider_subtype = ?3",
                params![process_id, kind.as_str(), subtype],
                |row| row.get(0),
            )?;
            if let Some(subtype_max) = subtype_max {
                let subtype_max = subtype_max as u64;
                max_sequence =
                    Some(max_sequence.map_or(subtype_max, |current: u64| current.max(subtype_max)));
            }
        }
        Ok(max_sequence)
    }

    pub fn project_timeline_for_chat_thread(
        &self,
        chat_thread_id: i64,
    ) -> Result<Vec<ProviderTimelineItem>> {
        self.list_for_chat_thread(chat_thread_id)
            .map(project_timeline)
    }

    fn open(&self) -> Result<Connection> {
        open_migrated_connection(&self.db_path)
    }
}

pub fn project_timeline(events: Vec<ProviderEventRecord>) -> Vec<ProviderTimelineItem> {
    events.into_iter().map(project_timeline_item).collect()
}

fn project_timeline_item(event: ProviderEventRecord) -> ProviderTimelineItem {
    let title = event
        .normalized_payload
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            event.provider_subtype.clone().unwrap_or_else(|| {
                PROVIDER_ACTION_CATEGORIES
                    .iter()
                    .find(|category| category.kind == event.kind)
                    .map(|category| category.label.to_owned())
                    .unwrap_or_else(|| "unknown event".to_owned())
            })
        });
    let body = event
        .normalized_payload
        .get("body")
        .or_else(|| event.normalized_payload.get("text"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let canonical_id = provider_record_canonical_id(&event);
    let parent_canonical_id = event.parent_provider_item_id.as_ref().map(|parent| {
        provider_canonical_id(
            &event.provider,
            event
                .parent_provider_thread_id
                .as_deref()
                .or(event.provider_thread_id.as_deref()),
            parent,
        )
    });
    ProviderTimelineItem {
        canonical_id,
        kind: event.kind,
        phase: event.phase,
        title,
        body,
        provider_subtype: event.provider_subtype,
        parent_canonical_id,
        raw_json: event.raw_json,
    }
}

fn merge_existing_streaming_payload(
    tx: &Connection,
    identity_key: &str,
    draft: &ProviderEventDraft,
) -> Result<Value> {
    if !matches!(
        draft.phase,
        ProviderEventPhase::Started | ProviderEventPhase::Delta | ProviderEventPhase::Progress
    ) {
        return Ok(draft.normalized_payload.clone());
    }

    let existing: Option<String> = tx
        .query_row(
            "SELECT normalized_payload_json FROM provider_events WHERE identity_key = ?1",
            [identity_key],
            |row| row.get(0),
        )
        .optional()?;

    let Some(existing) = existing else {
        return Ok(draft.normalized_payload.clone());
    };
    let existing = serde_json::from_str(&existing).unwrap_or(Value::Null);
    Ok(merge_streaming_payload(
        existing,
        draft.normalized_payload.clone(),
        draft.phase,
    ))
}

fn merge_streaming_payload(
    existing: Value,
    mut incoming: Value,
    phase: ProviderEventPhase,
) -> Value {
    for key in ["body", "text"] {
        let Some(existing_text) = existing.get(key).and_then(Value::as_str) else {
            continue;
        };
        let Some(incoming_text) = incoming.get(key).and_then(Value::as_str) else {
            continue;
        };
        if incoming_text.len() > existing_text.len() && incoming_text.starts_with(existing_text) {
            continue;
        }
        if phase != ProviderEventPhase::Delta && existing_text.starts_with(incoming_text) {
            incoming[key] = Value::String(existing_text.to_owned());
            continue;
        }
        incoming[key] = Value::String(format!("{existing_text}{incoming_text}"));
    }
    incoming
}

fn provider_record_canonical_id(event: &ProviderEventRecord) -> String {
    event
        .provider_item_id
        .as_ref()
        .map(|item| {
            provider_canonical_id(&event.provider, event.provider_thread_id.as_deref(), item)
        })
        .unwrap_or_else(|| event.identity_key.clone())
}

fn provider_canonical_id(provider: &str, thread_id: Option<&str>, item_id: &str) -> String {
    format!("{}:{}:{}", provider, thread_id.unwrap_or("-"), item_id)
}

fn open_migrated_connection(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create data directory {}", parent.display()))?;
    }
    let conn =
        Connection::open(path).with_context(|| format!("open database {}", path.display()))?;
    crate::storage::migrate_workspace_db(&conn)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS provider_event_raw_payloads (
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
          ON provider_event_raw_payloads(chat_thread_id, raw_sequence, id);",
    )?;
    Ok(conn)
}

fn next_received_sequence(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT COALESCE(MAX(received_sequence), 0) + 1 FROM provider_events",
        [],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

fn next_raw_sequence(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT COALESCE(MAX(raw_sequence), 0) + 1 FROM provider_event_raw_payloads",
        [],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

fn provider_event_select_sql(where_clause: &str) -> String {
    format!(
        "SELECT id, identity_key, provider, provider_event_id, provider_item_id,
            provider_thread_id, provider_turn_id, parent_provider_item_id,
            parent_provider_thread_id, workspace_id, chat_thread_id, process_id,
            phase, kind, provider_subtype, provider_sequence, received_sequence,
            occurred_at_ms, normalized_payload_json, raw_json, schema_version,
            adapter_version, created_at, updated_at
         FROM provider_events {where_clause}"
    )
}

fn row_to_provider_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProviderEventRecord> {
    let normalized_payload_json: String = row.get(18)?;
    let raw_json: String = row.get(19)?;
    let phase: String = row.get(12)?;
    let kind: String = row.get(13)?;
    let occurred_at_ms: i64 = row.get(17)?;
    Ok(ProviderEventRecord {
        id: row.get(0)?,
        identity_key: row.get(1)?,
        provider: row.get(2)?,
        provider_event_id: row.get(3)?,
        provider_item_id: row.get(4)?,
        provider_thread_id: row.get(5)?,
        provider_turn_id: row.get(6)?,
        parent_provider_item_id: row.get(7)?,
        parent_provider_thread_id: row.get(8)?,
        workspace_id: row.get(9)?,
        chat_thread_id: row.get(10)?,
        process_id: row.get(11)?,
        phase: ProviderEventPhase::from_str(&phase),
        kind: ProviderEventKind::from_str(&kind),
        provider_subtype: row.get(14)?,
        provider_sequence: row.get(15)?,
        received_sequence: row.get(16)?,
        occurred_at_ms: occurred_at_ms.max(0) as u64,
        normalized_payload: serde_json::from_str(&normalized_payload_json).unwrap_or(Value::Null),
        raw_json: serde_json::from_str(&raw_json).unwrap_or(Value::Null),
        schema_version: row.get(20)?,
        adapter_version: row.get(21)?,
        created_at: row.get(22)?,
        updated_at: row.get(23)?,
    })
}

fn row_to_provider_raw_payload(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ProviderRawPayloadRecord> {
    let phase: String = row.get(5)?;
    let kind: String = row.get(6)?;
    let occurred_at_ms: i64 = row.get(9)?;
    let raw_json: String = row.get(10)?;
    Ok(ProviderRawPayloadRecord {
        id: row.get(0)?,
        identity_key: row.get(1)?,
        provider: row.get(2)?,
        chat_thread_id: row.get(3)?,
        process_id: row.get(4)?,
        phase: ProviderEventPhase::from_str(&phase),
        kind: ProviderEventKind::from_str(&kind),
        provider_sequence: row.get(7)?,
        raw_sequence: row.get(8)?,
        occurred_at_ms: occurred_at_ms.max(0) as u64,
        raw_json: serde_json::from_str(&raw_json).unwrap_or(Value::Null),
        created_at: row.get(11)?,
    })
}

fn timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_owned())
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn stable_text_hash(value: &str) -> u64 {
    value.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn create_parent_rows(store: &ProviderEventStore, root: &Path) {
        let conn = store.open().unwrap();
        let now = "1";
        conn.execute(
            "INSERT INTO repositories (
                id, name, root_path, default_branch, remote_name, workspace_parent_path, created_at, updated_at
             ) VALUES (1, 'repo', ?1, 'main', 'origin', ?2, ?3, ?3)",
            params![
                root.join("repo").display().to_string(),
                root.join("workspaces").display().to_string(),
                now
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO workspaces (
                id, repository_id, name, path, branch, base_ref, port_base, status, created_at, updated_at
             ) VALUES (1, 1, 'berlin', ?1, 'feature/berlin', 'main', 3000, 'active', ?2, ?2)",
            params![root.join("workspaces/berlin").display().to_string(), now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chat_threads (
                id, workspace_id, provider, title, status, created_at, updated_at
             ) VALUES (7, 1, 'codex', 'Codex', 'active', ?1, ?1)",
            [now],
        )
        .unwrap();
    }

    fn draft(kind: ProviderEventKind, phase: ProviderEventPhase) -> ProviderEventDraft {
        ProviderEventDraft {
            provider: "codex".to_owned(),
            provider_event_id: Some("evt-1".to_owned()),
            provider_item_id: Some("item-1".to_owned()),
            provider_thread_id: Some("thread-1".to_owned()),
            provider_turn_id: Some("turn-1".to_owned()),
            parent_provider_item_id: None,
            parent_provider_thread_id: None,
            workspace_id: None,
            chat_thread_id: Some(7),
            process_id: None,
            phase,
            kind,
            provider_subtype: Some("agent_message_delta".to_owned()),
            provider_sequence: Some(1),
            occurred_at_ms: 42,
            normalized_payload: json!({"title": "Assistant", "text": "hello"}),
            raw_json: json!({"method": "agent/message/delta", "params": {"delta": "hello"}}),
            schema_version: 1,
            adapter_version: "test-adapter".to_owned(),
        }
    }

    #[test]
    fn provider_event_categories_cover_requested_action_taxonomy() {
        assert_eq!(PROVIDER_ACTION_CATEGORIES.len(), 20);
        for kind in [
            ProviderEventKind::AccountAuth,
            ProviderEventKind::ThreadSession,
            ProviderEventKind::GoalTask,
            ProviderEventKind::Turn,
            ProviderEventKind::UserInput,
            ProviderEventKind::AssistantOutput,
            ProviderEventKind::PlanningReasoning,
            ProviderEventKind::CommandProcess,
            ProviderEventKind::TerminalRuntime,
            ProviderEventKind::FileSystem,
            ProviderEventKind::DiffFileChange,
            ProviderEventKind::Tool,
            ProviderEventKind::Mcp,
            ProviderEventKind::SkillPluginHook,
            ProviderEventKind::ApprovalPermission,
            ProviderEventKind::SubagentCollaboration,
            ProviderEventKind::WebBrowserMedia,
            ProviderEventKind::EnvironmentConfigModel,
            ProviderEventKind::LimitFailure,
            ProviderEventKind::Unknown,
        ] {
            assert!(
                PROVIDER_ACTION_CATEGORIES
                    .iter()
                    .any(|category| category.kind == kind),
                "missing category for {kind:?}"
            );
        }
    }

    #[test]
    fn upsert_updates_streaming_item_by_provider_identity() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProviderEventStore::new(temp.path().join("state.db"));
        create_parent_rows(&store, temp.path());
        let first = draft(
            ProviderEventKind::AssistantOutput,
            ProviderEventPhase::Delta,
        );
        let mut second = first.clone();
        second.normalized_payload = json!({"title": "Assistant", "text": "hello world"});
        second.raw_json = json!({"method": "agent/message/delta", "params": {"delta": " world"}});

        let inserted = store.upsert_event(&first).unwrap();
        let updated = store.upsert_event(&second).unwrap();

        assert_eq!(inserted.id, updated.id);
        assert_eq!(updated.normalized_payload["text"], "hello world");
        assert_eq!(store.list_for_chat_thread(7).unwrap().len(), 1);
    }

    #[test]
    fn upsert_preserves_raw_payload_history_for_streaming_identity() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProviderEventStore::new(temp.path().join("state.db"));
        create_parent_rows(&store, temp.path());
        let first = draft(
            ProviderEventKind::AssistantOutput,
            ProviderEventPhase::Started,
        );
        let mut second = first.clone();
        second.phase = ProviderEventPhase::Delta;
        second.provider_event_id = Some("evt-2".to_owned());
        second.provider_sequence = Some(2);
        second.normalized_payload = json!({"title": "Assistant", "text": "hello world"});
        second.raw_json = json!({"method": "agent/message/delta", "params": {"delta": " world"}});
        let mut third = second.clone();
        third.phase = ProviderEventPhase::Progress;
        third.provider_event_id = Some("evt-3".to_owned());
        third.provider_sequence = Some(3);
        third.raw_json = json!({"method": "agent/message/progress", "params": {"tokens": 12}});

        store.upsert_event(&first).unwrap();
        let latest = store.upsert_event(&second).unwrap();
        store.upsert_event(&third).unwrap();

        let rows = store.list_for_chat_thread(7).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].identity_key, first.identity_key());
        assert_eq!(rows[0].raw_json["params"]["tokens"], 12);
        assert_eq!(rows[0].normalized_payload["text"], "hello world");

        let raw_payloads = store
            .list_raw_payloads_for_identity(&latest.identity_key)
            .unwrap();
        assert_eq!(raw_payloads.len(), 3);
        assert_eq!(raw_payloads[0].raw_json["params"]["delta"], "hello");
        assert_eq!(raw_payloads[1].raw_json["params"]["delta"], " world");
        assert_eq!(raw_payloads[2].raw_json["params"]["tokens"], 12);
    }

    #[test]
    fn streaming_deltas_append_repeated_identical_chunks() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProviderEventStore::new(temp.path().join("state.db"));
        create_parent_rows(&store, temp.path());
        let mut first = draft(
            ProviderEventKind::AssistantOutput,
            ProviderEventPhase::Delta,
        );
        first.normalized_payload = json!({"title": "Assistant", "text": "ha"});
        first.raw_json = json!({"method": "agent/message/delta", "params": {"delta": "ha"}});
        let mut second = first.clone();
        second.provider_event_id = Some("evt-2".to_owned());
        second.provider_sequence = Some(2);
        second.raw_json = json!({"method": "agent/message/delta", "params": {"delta": "ha"}});

        store.upsert_event(&first).unwrap();
        let latest = store.upsert_event(&second).unwrap();

        assert_eq!(latest.normalized_payload["text"], "haha");
        assert_eq!(
            store
                .list_raw_payloads_for_identity(&latest.identity_key)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn stale_cumulative_snapshots_do_not_duplicate_existing_text() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProviderEventStore::new(temp.path().join("state.db"));
        create_parent_rows(&store, temp.path());
        let mut first = draft(
            ProviderEventKind::AssistantOutput,
            ProviderEventPhase::Delta,
        );
        first.normalized_payload = json!({"title": "Assistant", "text": "hello world"});
        first.raw_json =
            json!({"method": "agent/message/delta", "params": {"delta": "hello world"}});
        let mut stale = first.clone();
        stale.phase = ProviderEventPhase::Progress;
        stale.provider_event_id = Some("evt-2".to_owned());
        stale.provider_sequence = Some(2);
        stale.normalized_payload = json!({"title": "Assistant", "text": "hello"});
        stale.raw_json = json!({"method": "agent/message/progress", "params": {"text": "hello"}});

        store.upsert_event(&first).unwrap();
        let latest = store.upsert_event(&stale).unwrap();

        assert_eq!(latest.normalized_payload["text"], "hello world");
    }

    #[test]
    fn idless_raw_events_do_not_collapse_when_sequence_is_absent() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProviderEventStore::new(temp.path().join("state.db"));
        create_parent_rows(&store, temp.path());
        let mut first = draft(ProviderEventKind::Unknown, ProviderEventPhase::Unknown);
        first.provider_event_id = None;
        first.provider_item_id = None;
        first.provider_sequence = None;
        first.provider_subtype = Some("status".to_owned());
        first.occurred_at_ms = 10;
        first.raw_json = json!({"method": "status", "params": {"message": "one"}});
        let mut second = first.clone();
        second.occurred_at_ms = 11;
        second.raw_json = json!({"method": "status", "params": {"message": "two"}});

        store.upsert_event(&first).unwrap();
        store.upsert_event(&second).unwrap();

        let rows = store.list_for_chat_thread(7).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn projected_parent_ids_match_projected_item_ids() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProviderEventStore::new(temp.path().join("state.db"));
        create_parent_rows(&store, temp.path());
        let parent = draft(ProviderEventKind::Tool, ProviderEventPhase::Started);
        let mut child = draft(ProviderEventKind::Tool, ProviderEventPhase::Completed);
        child.provider_item_id = Some("child-1".to_owned());
        child.parent_provider_item_id = Some("item-1".to_owned());

        store.upsert_event(&parent).unwrap();
        store.upsert_event(&child).unwrap();

        let timeline = store.project_timeline_for_chat_thread(7).unwrap();
        let parent_id = timeline
            .iter()
            .find(|item| item.body == "hello")
            .map(|item| item.canonical_id.clone())
            .unwrap();
        let child_parent = timeline
            .iter()
            .find(|item| item.canonical_id.ends_with(":child-1"))
            .and_then(|item| item.parent_canonical_id.clone())
            .unwrap();
        assert_eq!(child_parent, parent_id);
    }

    #[test]
    fn completed_phase_is_a_distinct_final_record_for_same_item() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProviderEventStore::new(temp.path().join("state.db"));
        create_parent_rows(&store, temp.path());
        let delta = draft(
            ProviderEventKind::AssistantOutput,
            ProviderEventPhase::Delta,
        );
        let mut completed = delta.clone();
        completed.phase = ProviderEventPhase::Completed;
        completed.normalized_payload = json!({"title": "Assistant", "text": "final"});

        store.upsert_event(&delta).unwrap();
        store.upsert_event(&completed).unwrap();

        let rows = store.list_for_chat_thread(7).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows
            .iter()
            .any(|row| row.phase == ProviderEventPhase::Delta));
        assert!(rows
            .iter()
            .any(|row| row.phase == ProviderEventPhase::Completed));
    }

    #[test]
    fn unknown_events_are_preserved_losslessly_and_projectable() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProviderEventStore::new(temp.path().join("state.db"));
        create_parent_rows(&store, temp.path());
        let mut event = draft(ProviderEventKind::Unknown, ProviderEventPhase::Unknown);
        event.provider_event_id = Some("future-event".to_owned());
        event.provider_item_id = None;
        event.provider_subtype = Some("future/providerThing".to_owned());
        event.normalized_payload = json!({"title": "Unknown provider event"});
        event.raw_json = json!({"method": "future/providerThing", "params": {"new": true}});

        let stored = store.upsert_event(&event).unwrap();
        let timeline = store.project_timeline_for_chat_thread(7).unwrap();

        assert_eq!(stored.raw_json["params"]["new"], true);
        assert_eq!(timeline[0].kind, ProviderEventKind::Unknown);
        assert_eq!(timeline[0].title, "Unknown provider event");
    }

    #[test]
    fn projection_classifies_tool_output_as_tool_not_assistant() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProviderEventStore::new(temp.path().join("state.db"));
        create_parent_rows(&store, temp.path());
        let mut event = draft(ProviderEventKind::Tool, ProviderEventPhase::Completed);
        event.provider_item_id = Some("tool-1".to_owned());
        event.provider_subtype = Some("command_execution".to_owned());
        event.normalized_payload = json!({
            "title": "Ran git status --short",
            "body": "M crates/core/src/provider_events.rs"
        });

        store.upsert_event(&event).unwrap();
        let timeline = store.project_timeline_for_chat_thread(7).unwrap();

        assert_eq!(timeline[0].kind, ProviderEventKind::Tool);
        assert_ne!(timeline[0].kind, ProviderEventKind::AssistantOutput);
    }
}
