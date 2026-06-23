use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use linux_conductor_core::paths::AppPaths;

#[derive(Debug, Clone, PartialEq, Eq)]
struct NavigationEntry {
    selected_workspace: Option<String>,
    active_page: AppPage,
    active_workspace_tab: WorkspaceTab,
}

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
    navigation_back: Vec<NavigationEntry>,
    navigation_forward: Vec<NavigationEntry>,
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
                navigation_back: Vec::new(),
                navigation_forward: Vec::new(),
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

    pub fn navigate_to_page(&self, page: AppPage) {
        let mut state = self.inner.borrow_mut();
        if state.active_page == page {
            return;
        }
        push_navigation_entry(&mut state);
        state.active_page = page;
    }

    pub fn navigate_to_workspace(&self, workspace: Option<String>) {
        let mut state = self.inner.borrow_mut();
        if state.selected_workspace == workspace && state.active_page == AppPage::Workspace {
            return;
        }
        push_navigation_entry(&mut state);
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

    pub fn navigate_to_workspace_with_default_tab(
        &self,
        workspace: Option<String>,
        default_tab: Option<WorkspaceTab>,
    ) {
        let mut state = self.inner.borrow_mut();
        if state.selected_workspace == workspace && state.active_page == AppPage::Workspace {
            return;
        }
        push_navigation_entry(&mut state);
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

    pub fn navigate_to_workspace_tab(&self, tab: WorkspaceTab) {
        let mut state = self.inner.borrow_mut();
        if state.active_page == AppPage::Workspace && state.active_workspace_tab == tab {
            return;
        }
        push_navigation_entry(&mut state);
        state.active_page = AppPage::Workspace;
        state.active_workspace_tab = tab;
    }

    pub fn set_active_workspace_tab(&self, tab: WorkspaceTab) {
        self.inner.borrow_mut().active_workspace_tab = tab;
    }

    pub fn navigate_back(&self) -> bool {
        let mut state = self.inner.borrow_mut();
        let Some(entry) = state.navigation_back.pop() else {
            return false;
        };
        let current = snapshot_navigation_entry(&state);
        state.navigation_forward.push(current);
        apply_navigation_entry(&mut state, entry);
        true
    }

    pub fn navigate_forward(&self) -> bool {
        let mut state = self.inner.borrow_mut();
        let Some(entry) = state.navigation_forward.pop() else {
            return false;
        };
        let current = snapshot_navigation_entry(&state);
        state.navigation_back.push(current);
        apply_navigation_entry(&mut state, entry);
        true
    }

    pub fn can_navigate_back(&self) -> bool {
        !self.inner.borrow().navigation_back.is_empty()
    }

    pub fn can_navigate_forward(&self) -> bool {
        !self.inner.borrow().navigation_forward.is_empty()
    }

    pub fn workspace_database_path(&self) -> PathBuf {
        self.paths.database_path.clone()
    }

    pub fn snapshot(&self) -> AppStateSnapshot {
        self.inner.borrow().clone()
    }
}

fn snapshot_navigation_entry(state: &AppStateSnapshot) -> NavigationEntry {
    NavigationEntry {
        selected_workspace: state.selected_workspace.clone(),
        active_page: state.active_page.clone(),
        active_workspace_tab: state.active_workspace_tab.clone(),
    }
}

fn push_navigation_entry(state: &mut AppStateSnapshot) {
    let entry = snapshot_navigation_entry(state);
    if state.navigation_back.last() != Some(&entry) {
        state.navigation_back.push(entry);
    }
    state.navigation_forward.clear();
}

fn apply_navigation_entry(state: &mut AppStateSnapshot, entry: NavigationEntry) {
    let workspace_changed = state.selected_workspace != entry.selected_workspace;
    if workspace_changed {
        state.selected_agent_session = None;
        state.staged_review_prompt = None;
    }
    state.selected_workspace = entry.selected_workspace;
    state.active_page = entry.active_page;
    state.active_workspace_tab = entry.active_workspace_tab;
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

    #[test]
    fn navigation_history_moves_back_and_forward() {
        let state = AppState::new(
            AppPaths::from_env(),
            None,
            WorkspaceTab::Chats,
            AppPage::Dashboard,
        );

        state.navigate_to_page(AppPage::History);
        state.navigate_to_workspace(Some("berlin".to_owned()));
        assert!(state.can_navigate_back());
        assert!(!state.can_navigate_forward());

        assert!(state.navigate_back());
        assert_eq!(state.snapshot().active_page, AppPage::History);
        assert!(state.can_navigate_forward());

        assert!(state.navigate_forward());
        assert_eq!(
            state.snapshot().selected_workspace.as_deref(),
            Some("berlin")
        );
        assert_eq!(state.snapshot().active_page, AppPage::Workspace);
    }
}
