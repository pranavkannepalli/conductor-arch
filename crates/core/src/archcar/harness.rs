use anyhow::{anyhow, Result};

use crate::codex_tui::{
    codex_screen_ready_for_input, detect_directory_trust_prompt, parse_codex_screen_messages,
    ScreenMessage,
};
use crate::workspace::{SessionHarnessOptions, SessionKind, WorkspaceStore};

pub trait HarnessController: Send + Sync {
    fn kind(&self) -> SessionKind;
    fn supports_auto_spawn(&self) -> bool;
    fn build_launch(
        &self,
        store: &WorkspaceStore,
        workspace: &str,
        harness: SessionHarnessOptions,
    ) -> Result<crate::workspace::SessionLaunch>;
    fn detect_ready(&self, screen: &str) -> bool;
    fn startup_input(&self, screen: &str) -> Option<String>;
    fn parse_messages(&self, screen: &str) -> Vec<ScreenMessage>;
}

pub fn controller_for_kind(kind: SessionKind) -> Box<dyn HarnessController> {
    match kind {
        SessionKind::Codex => Box::new(CodexHarnessController),
        SessionKind::Claude => Box::new(ClaudeHarnessController),
        SessionKind::Shell => Box::new(ShellHarnessController),
    }
}

pub struct CodexHarnessController;

impl HarnessController for CodexHarnessController {
    fn kind(&self) -> SessionKind {
        SessionKind::Codex
    }

    fn supports_auto_spawn(&self) -> bool {
        true
    }

    fn build_launch(
        &self,
        store: &WorkspaceStore,
        workspace: &str,
        harness: SessionHarnessOptions,
    ) -> Result<crate::workspace::SessionLaunch> {
        store.session_launch_with_options(workspace, SessionKind::Codex, harness)
    }

    fn detect_ready(&self, screen: &str) -> bool {
        codex_screen_ready_for_input(screen)
    }

    fn startup_input(&self, screen: &str) -> Option<String> {
        detect_directory_trust_prompt(screen).then(|| "1".to_owned())
    }

    fn parse_messages(&self, screen: &str) -> Vec<ScreenMessage> {
        parse_codex_screen_messages(screen)
    }
}

pub struct ClaudeHarnessController;

impl HarnessController for ClaudeHarnessController {
    fn kind(&self) -> SessionKind {
        SessionKind::Claude
    }

    fn supports_auto_spawn(&self) -> bool {
        false
    }

    fn build_launch(
        &self,
        _store: &WorkspaceStore,
        _workspace: &str,
        _harness: SessionHarnessOptions,
    ) -> Result<crate::workspace::SessionLaunch> {
        Err(anyhow!("claude harness not implemented in archcar yet"))
    }

    fn detect_ready(&self, _screen: &str) -> bool {
        false
    }

    fn startup_input(&self, _screen: &str) -> Option<String> {
        None
    }

    fn parse_messages(&self, _screen: &str) -> Vec<ScreenMessage> {
        Vec::new()
    }
}

pub struct ShellHarnessController;

impl HarnessController for ShellHarnessController {
    fn kind(&self) -> SessionKind {
        SessionKind::Shell
    }

    fn supports_auto_spawn(&self) -> bool {
        false
    }

    fn build_launch(
        &self,
        store: &WorkspaceStore,
        workspace: &str,
        harness: SessionHarnessOptions,
    ) -> Result<crate::workspace::SessionLaunch> {
        store.session_launch_with_options(workspace, SessionKind::Shell, harness)
    }

    fn detect_ready(&self, _screen: &str) -> bool {
        true
    }

    fn startup_input(&self, _screen: &str) -> Option<String> {
        None
    }

    fn parse_messages(&self, _screen: &str) -> Vec<ScreenMessage> {
        Vec::new()
    }
}

pub fn provider_name(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Codex => "codex",
        SessionKind::Claude => "claude",
        SessionKind::Shell => "shell",
    }
}

pub fn ensure_thread_for_kind(
    store: &WorkspaceStore,
    workspace: &str,
    kind: SessionKind,
) -> Result<crate::workspace::ChatThreadRecord> {
    let provider = provider_name(kind);
    if let Some(existing) = store
        .list_chat_threads(workspace)?
        .into_iter()
        .find(|thread| thread.provider == provider)
    {
        return Ok(existing);
    }
    let title = match kind {
        SessionKind::Codex => "Codex Chat 1",
        SessionKind::Claude => "Claude Chat 1",
        SessionKind::Shell => "Shell Chat 1",
    };
    store.create_chat_thread(workspace, provider, title, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_ready_detection_waits_for_boot_completion() {
        let controller = CodexHarnessController;
        assert!(!controller.detect_ready("• Booting MCP server\n\n› hi"));
        assert!(controller.detect_ready("› hi"));
    }

    #[test]
    fn claude_stub_reports_not_implemented() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("test.db");
        let store = WorkspaceStore::open(&db).unwrap();
        let err = ClaudeHarnessController
            .build_launch(&store, "berlin", SessionHarnessOptions::default())
            .unwrap_err();
        assert!(err.to_string().contains("not implemented"));
    }
}
