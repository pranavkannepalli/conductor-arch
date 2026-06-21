use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use linux_conductor_core::paths::AppPaths;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppPage {
    Dashboard,
    Projects,
    Workspace,
    History,
    Settings,
    Review,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceTab {
    Chats,
    Changes,
    Review,
    Checkpoints,
    Checks,
    Todos,
    Processes,
    Terminal,
}

impl WorkspaceTab {
    pub fn from_config(value: &str) -> Option<Self> {
        match normalize_tab_token(value).as_str() {
            "chat" | "chats" | "chatterminal" | "session" | "sessions" => Some(Self::Chats),
            "changes" | "change" | "diff" | "files" => Some(Self::Changes),
            "review" | "comments" => Some(Self::Review),
            "checks" | "ci" | "pr" | "pullrequest" => Some(Self::Checks),
            "todos" | "todo" | "tasks" => Some(Self::Todos),
            "processes" | "runs" | "process" => Some(Self::Processes),
            "terminal" | "term" | "shell" | "bigterminal" => Some(Self::Terminal),
            "checkpoints" | "checkpoint" | "restore" => Some(Self::Checkpoints),
            _ => None,
        }
    }
}

fn normalize_tab_token(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[derive(Debug, Clone)]
pub struct AppStateSnapshot {
    pub selected_workspace: Option<String>,
    pub selected_project: Option<String>,
    pub active_page: AppPage,
    pub active_workspace_tab: WorkspaceTab,
    pub selected_agent_session: Option<i64>,
    pub staged_review_prompt: Option<String>,
    pub running_processes: Vec<i64>,
    pub attention_state: AttentionState,
    pub settings_layer: SettingsLayer,
}

#[derive(Debug, Clone, Default)]
pub struct AttentionState {
    pub failed_checks: usize,
    pub open_todos: usize,
    pub open_comments: usize,
    pub conflicts: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsLayer {
    BuiltInDefaults,
    UserShared,
    RepositoryShared,
    LocalProjectOverride,
    Managed,
}

#[derive(Debug, Clone)]
pub struct AppState {
    inner: Rc<RefCell<AppStateSnapshot>>,
    pub paths: AppPaths,
}

impl AppState {
    pub fn new(
        paths: AppPaths,
        initial_workspace: Option<String>,
        initial_tab: WorkspaceTab,
        initial_page: AppPage,
    ) -> Self {
        let active_page = if initial_workspace.is_some() {
            AppPage::Workspace
        } else {
            initial_page
        };
        Self {
            inner: Rc::new(RefCell::new(AppStateSnapshot {
                selected_workspace: initial_workspace,
                selected_project: None,
                active_page,
                active_workspace_tab: initial_tab,
                selected_agent_session: None,
                staged_review_prompt: None,
                running_processes: Vec::new(),
                attention_state: AttentionState::default(),
                settings_layer: SettingsLayer::BuiltInDefaults,
            })),
            paths,
        }
    }

    pub fn selected_workspace(&self) -> Option<String> {
        self.inner.borrow().selected_workspace.clone()
    }

    pub fn set_selected_workspace(&self, workspace: Option<String>) {
        let mut state = self.inner.borrow_mut();
        if state.selected_workspace != workspace {
            state.selected_agent_session = None;
            state.staged_review_prompt = None;
        }
        state.selected_workspace = workspace;
        state.active_page = AppPage::Workspace;
    }

    pub fn set_selected_workspace_with_default_tab(
        &self,
        workspace: Option<String>,
        default_tab: Option<WorkspaceTab>,
    ) {
        let mut state = self.inner.borrow_mut();
        if state.selected_workspace != workspace {
            state.selected_agent_session = None;
            state.staged_review_prompt = None;
            if let Some(tab) = default_tab {
                state.active_workspace_tab = tab;
            }
        }
        state.selected_workspace = workspace;
        state.active_page = AppPage::Workspace;
    }

    pub fn selected_agent_session(&self) -> Option<i64> {
        self.inner.borrow().selected_agent_session
    }

    pub fn set_selected_agent_session(&self, session_id: Option<i64>) {
        self.inner.borrow_mut().selected_agent_session = session_id;
    }

    pub fn staged_review_prompt(&self) -> Option<String> {
        self.inner.borrow().staged_review_prompt.clone()
    }

    pub fn set_staged_review_prompt(&self, prompt: Option<String>) {
        self.inner.borrow_mut().staged_review_prompt = prompt;
    }

    pub fn set_active_page(&self, page: AppPage) {
        self.inner.borrow_mut().active_page = page;
    }

    pub fn set_active_workspace_tab(&self, tab: WorkspaceTab) {
        self.inner.borrow_mut().active_workspace_tab = tab;
    }

    pub fn workspace_database_path(&self) -> PathBuf {
        self.paths.database_path.clone()
    }

    pub fn snapshot(&self) -> AppStateSnapshot {
        self.inner.borrow().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_tab_from_config_accepts_view_default_aliases() {
        assert_eq!(
            WorkspaceTab::from_config("checks"),
            Some(WorkspaceTab::Checks)
        );
        assert_eq!(
            WorkspaceTab::from_config("big-terminal"),
            Some(WorkspaceTab::Terminal)
        );
        assert_eq!(
            WorkspaceTab::from_config("review"),
            Some(WorkspaceTab::Review)
        );
        assert_eq!(WorkspaceTab::from_config("missing"), None);
    }

    #[test]
    fn selected_workspace_default_tab_only_applies_when_workspace_changes() {
        let state = AppState::new(
            AppPaths::from_env(),
            Some("berlin".to_owned()),
            WorkspaceTab::Terminal,
            AppPage::Workspace,
        );

        state.set_selected_workspace_with_default_tab(
            Some("berlin".to_owned()),
            Some(WorkspaceTab::Checks),
        );
        assert_eq!(
            state.snapshot().active_workspace_tab,
            WorkspaceTab::Terminal
        );

        state.set_selected_workspace_with_default_tab(
            Some("tokyo".to_owned()),
            Some(WorkspaceTab::Checks),
        );
        assert_eq!(state.snapshot().active_workspace_tab, WorkspaceTab::Checks);
    }
}
