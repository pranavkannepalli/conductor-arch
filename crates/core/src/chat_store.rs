use std::path::PathBuf;

use anyhow::Result;

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

    pub fn append_message_once_for_provider_input(
        &self,
        provider_input_id: &str,
        thread_id: i64,
        role: &str,
        content: &str,
    ) -> Result<ChatMessageRecord> {
        let source = format!("provider_input:{provider_input_id}");
        self.open()?
            .append_chat_message_once_for_source(thread_id, role, content, &source)
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
        let legacy_pipeline = concat!("persist_codex", "_pipeline_update");

        assert!(source.contains("pub struct ChatStore"));
        assert!(source.contains("append_message"));
        assert!(source.contains("resolve_codex_native_thread_id_for_process"));
        assert!(
            !source.contains(legacy_pipeline),
            "ChatStore should not expose the old Codex PTY semantic pipeline"
        );
    }
}
