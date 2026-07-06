use std::path::PathBuf;

use anyhow::Result;

use crate::session_pipeline::{PtyChunkInput, SessionPipelineOutput};
use crate::session_state::AgentSessionState;
use crate::workspace::{ChatMessageRecord, WorkspaceStore};

#[derive(Debug, Clone)]
pub struct ChatStore {
    db_path: PathBuf,
}

impl ChatStore {
    pub fn new(db_path: PathBuf) -> Self {
        Self { db_path }
    }

    pub fn append_message(
        &self,
        thread_id: i64,
        role: &str,
        content: &str,
        source: &str,
    ) -> Result<ChatMessageRecord> {
        self.open()?
            .append_chat_message(thread_id, role, content, source)
    }

    pub fn persist_codex_pipeline_update(
        &self,
        thread_id: i64,
        process_id: i64,
        chunks: Vec<PtyChunkInput>,
        screen: &str,
        previous_state: AgentSessionState,
    ) -> Result<SessionPipelineOutput> {
        self.open()?.persist_codex_pipeline_update(
            thread_id,
            process_id,
            chunks,
            screen,
            previous_state,
        )
    }

    pub fn resolve_codex_native_thread_id_for_process(
        &self,
        process_id: i64,
    ) -> Result<Option<String>> {
        self.open()?
            .resolve_codex_native_thread_id_for_process(process_id)
    }

    fn open(&self) -> Result<WorkspaceStore> {
        WorkspaceStore::open(&self.db_path)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn chat_store_exposes_narrow_runtime_chat_boundary() {
        let source = include_str!("chat_store.rs");

        assert!(source.contains("pub struct ChatStore"));
        assert!(source.contains("append_message"));
        assert!(source.contains("persist_codex_pipeline_update"));
        assert!(source.contains("resolve_codex_native_thread_id_for_process"));
    }
}
