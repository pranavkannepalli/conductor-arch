use std::path::PathBuf;

use anyhow::Result;

use crate::chat_store::ChatStore;
use crate::provider_events::{
    ProviderEventDraft, ProviderEventKind, ProviderEventRecord, ProviderEventStore,
};
use crate::session_pipeline::PtyChunkInput;
use crate::workspace::{SessionKind, WorkspaceStore};

#[derive(Debug, Clone)]
pub struct RuntimeSessionStore {
    db_path: PathBuf,
    chat_store: ChatStore,
    provider_event_store: ProviderEventStore,
}

impl RuntimeSessionStore {
    pub fn new(db_path: PathBuf) -> Self {
        Self {
            chat_store: ChatStore::new(db_path.clone()),
            provider_event_store: ProviderEventStore::new(db_path.clone()),
            db_path,
        }
    }

    pub fn append_input_and_audit_log(
        &self,
        thread_id: i64,
        process_id: i64,
        role: &str,
        input: &str,
        source: &str,
        audit_log: &str,
    ) -> Result<()> {
        let store = self.open()?;
        self.chat_store
            .append_message(thread_id, role, input, source)?;
        if !audit_log.is_empty() {
            store.append_session_process_output(process_id, audit_log)?;
        }
        Ok(())
    }

    pub fn append_raw_output(
        &self,
        process_id: i64,
        kind: SessionKind,
        raw: &str,
    ) -> Result<Option<PtyChunkInput>> {
        if raw.is_empty() {
            return Ok(None);
        }
        let store = self.open()?;
        let chunk = store.append_pty_chunk(process_id, "stdout_pty", raw)?;
        store.append_session_process_output(process_id, &format_session_raw_output(kind, raw))?;
        Ok(Some(PtyChunkInput {
            sequence: chunk.sequence,
            text: chunk.text,
        }))
    }

    pub fn append_screen_output(
        &self,
        process_id: i64,
        kind: SessionKind,
        screen: &str,
    ) -> Result<()> {
        self.open()?
            .append_session_process_output(process_id, &format_session_screen_output(kind, screen))
    }

    pub fn append_provider_event(&self, draft: &ProviderEventDraft) -> Result<ProviderEventRecord> {
        self.provider_event_store.upsert_event(draft)
    }

    pub fn count_runtime_input_provider_events(&self, process_id: i64) -> Result<u64> {
        self.provider_event_store.count_for_process_subtypes(
            process_id,
            ProviderEventKind::UserInput,
            &["user_input", "review_prompt", "control_command"],
        )
    }

    pub fn resolve_codex_native_thread_id_for_process(
        &self,
        process_id: i64,
    ) -> Result<Option<String>> {
        self.chat_store
            .resolve_codex_native_thread_id_for_process(process_id)
    }

    pub fn mark_session_process_exited(
        &self,
        process_id: i64,
        exit_code: Option<i32>,
    ) -> Result<()> {
        self.open()?
            .mark_session_process_exited(process_id, exit_code)?;
        Ok(())
    }

    fn open(&self) -> Result<WorkspaceStore> {
        WorkspaceStore::open(&self.db_path)
    }
}

pub(crate) fn format_session_raw_output(kind: SessionKind, raw: &str) -> String {
    match kind {
        SessionKind::Codex => crate::workspace::format_codex_raw_output(raw),
        _ => raw.to_owned(),
    }
}

pub(crate) fn format_session_screen_output(kind: SessionKind, screen: &str) -> String {
    match kind {
        SessionKind::Codex => crate::workspace::format_codex_screen_snapshot(screen),
        _ => screen.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn runtime_session_store_uses_chat_store_boundary_for_chat_and_pipeline_state() {
        let source = include_str!("runtime_session_store.rs");
        let broad_chat_append = concat!("store.", "append_chat_message(");

        assert!(source.contains("ChatStore"));
        assert!(
            !source.contains(broad_chat_append),
            "runtime session persistence should not write chat rows through broad WorkspaceStore"
        );
    }

    #[test]
    fn runtime_provider_semantics_use_provider_event_store_not_legacy_codex_pipeline() {
        let source = include_str!("runtime_session_store.rs");
        let legacy_pipeline = concat!("persist_codex", "_pipeline_update");

        assert!(
            source.contains("ProviderEventStore"),
            "runtime semantic provider persistence should use ProviderEventStore"
        );
        assert!(
            !source.contains(legacy_pipeline),
            "runtime semantic provider persistence must not call the old Codex PTY pipeline"
        );
    }
}
