use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use linux_archductor_core::paths::AppPaths;

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
    PtyInspector,
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
    pub selected_chat_thread: Option<i64>,
    pub selected_agent_session: Option<i64>,
    pub staged_review_prompt: Option<String>,
    pub pending_chat_prompt: Option<String>,
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
                selected_chat_thread: None,
                selected_agent_session: None,
                staged_review_prompt: None,
                pending_chat_prompt: None,
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
            state.selected_chat_thread = None;
            state.selected_agent_session = None;
            state.staged_review_prompt = None;
            state.pending_chat_prompt = None;
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
            state.selected_chat_thread = None;
            state.selected_agent_session = None;
            state.staged_review_prompt = None;
            state.pending_chat_prompt = None;
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
            state.selected_chat_thread = None;
            state.selected_agent_session = None;
            state.staged_review_prompt = None;
            state.pending_chat_prompt = None;
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
            state.selected_chat_thread = None;
            state.selected_agent_session = None;
            state.staged_review_prompt = None;
            state.pending_chat_prompt = None;
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

    pub fn selected_chat_thread(&self) -> Option<i64> {
        self.inner.borrow().selected_chat_thread
    }

    pub fn set_selected_chat_thread(&self, thread_id: Option<i64>) {
        self.inner.borrow_mut().selected_chat_thread = thread_id;
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

    pub fn queue_pending_chat_prompt(&self, prompt: String) {
        let prompt = prompt.trim().to_owned();
        if prompt.is_empty() {
            return;
        }
        self.inner.borrow_mut().pending_chat_prompt = Some(prompt);
    }

    pub fn take_pending_chat_prompt(&self) -> Option<String> {
        self.inner.borrow_mut().pending_chat_prompt.take()
    }

    pub fn set_active_page(&self, page: AppPage) {
        self.inner.borrow_mut().active_page = page;
    }

    pub fn remove_workspace_from_navigation(&self, workspace: &str, fallback_page: AppPage) {
        let mut state = self.inner.borrow_mut();
        if state.selected_workspace.as_deref() == Some(workspace) {
            state.selected_workspace = None;
            state.selected_chat_thread = None;
            state.selected_agent_session = None;
            state.staged_review_prompt = None;
            state.pending_chat_prompt = None;
            if matches!(state.active_page, AppPage::Workspace | AppPage::Review) {
                state.active_page = fallback_page;
            }
        }
        state.navigation_back =
            sanitize_removed_workspace_navigation(&state.navigation_back, workspace);
        state.navigation_forward =
            sanitize_removed_workspace_navigation(&state.navigation_forward, workspace);
    }

    pub fn rename_workspace_in_navigation(&self, old_name: &str, new_name: &str) {
        let mut state = self.inner.borrow_mut();
        if state.selected_workspace.as_deref() == Some(old_name) {
            state.selected_workspace = Some(new_name.to_owned());
        }
        for entry in state.navigation_back.iter_mut() {
            if entry.selected_workspace.as_deref() == Some(old_name) {
                entry.selected_workspace = Some(new_name.to_owned());
            }
        }
        for entry in state.navigation_forward.iter_mut() {
            if entry.selected_workspace.as_deref() == Some(old_name) {
                entry.selected_workspace = Some(new_name.to_owned());
            }
        }
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

fn sanitize_removed_workspace_navigation(
    entries: &[NavigationEntry],
    workspace: &str,
) -> Vec<NavigationEntry> {
    compact_adjacent_navigation_entries(
        entries
            .iter()
            .filter_map(|entry| {
                if entry.selected_workspace.as_deref() != Some(workspace) {
                    return Some(entry.clone());
                }
                if matches!(entry.active_page, AppPage::Workspace | AppPage::Review) {
                    return None;
                }
                let mut entry = entry.clone();
                entry.selected_workspace = None;
                Some(entry)
            })
            .collect(),
    )
}

fn compact_adjacent_navigation_entries(entries: Vec<NavigationEntry>) -> Vec<NavigationEntry> {
    entries
        .into_iter()
        .fold(Vec::new(), |mut compacted, entry| {
            if compacted.last() != Some(&entry) {
                compacted.push(entry);
            }
            compacted
        })
}

fn apply_navigation_entry(state: &mut AppStateSnapshot, entry: NavigationEntry) {
    let workspace_changed = state.selected_workspace != entry.selected_workspace;
    if workspace_changed {
        state.selected_chat_thread = None;
        state.selected_agent_session = None;
        state.staged_review_prompt = None;
        state.pending_chat_prompt = None;
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

    #[test]
    fn changing_workspace_clears_selected_chat_thread() {
        let state = AppState::new(
            AppPaths::from_env(),
            Some("berlin".to_owned()),
            WorkspaceTab::Chats,
            AppPage::Workspace,
        );

        state.set_selected_chat_thread(Some(42));
        state.navigate_to_workspace(Some("tokyo".to_owned()));

        assert_eq!(state.selected_chat_thread(), None);
    }

    #[test]
    fn pending_chat_prompt_is_one_shot() {
        let state = AppState::new(
            AppPaths::from_env(),
            Some("berlin".to_owned()),
            WorkspaceTab::Chats,
            AppPage::Workspace,
        );

        state.queue_pending_chat_prompt("  Create a PR  ".to_owned());

        assert_eq!(
            state.take_pending_chat_prompt().as_deref(),
            Some("Create a PR")
        );
        assert_eq!(state.take_pending_chat_prompt(), None);
    }

    #[test]
    fn removed_workspace_is_cleared_from_active_state_and_history() {
        let state = AppState::new(
            AppPaths::from_env(),
            None,
            WorkspaceTab::Chats,
            AppPage::Dashboard,
        );

        state.navigate_to_workspace(Some("berlin".to_owned()));
        state.navigate_to_page(AppPage::History);
        state.navigate_to_workspace(Some("tokyo".to_owned()));
        state.navigate_to_page(AppPage::Dashboard);

        state.remove_workspace_from_navigation("berlin", AppPage::Dashboard);

        while state.navigate_back() {
            assert_ne!(
                state.snapshot().selected_workspace.as_deref(),
                Some("berlin")
            );
        }
        while state.navigate_forward() {
            assert_ne!(
                state.snapshot().selected_workspace.as_deref(),
                Some("berlin")
            );
        }
    }

    #[test]
    fn removing_selected_workspace_does_not_redirect_global_pages() {
        let state = AppState::new(
            AppPaths::from_env(),
            Some("berlin".to_owned()),
            WorkspaceTab::Chats,
            AppPage::Workspace,
        );
        state.navigate_to_page(AppPage::History);

        state.remove_workspace_from_navigation("berlin", AppPage::Dashboard);
        let snapshot = state.snapshot();

        assert_eq!(snapshot.selected_workspace, None);
        assert_eq!(snapshot.active_page, AppPage::History);
    }

    #[test]
    fn removed_workspace_preserves_global_navigation_entries() {
        let state = AppState::new(
            AppPaths::from_env(),
            Some("berlin".to_owned()),
            WorkspaceTab::Chats,
            AppPage::Workspace,
        );

        state.navigate_to_page(AppPage::History);
        state.navigate_to_page(AppPage::Settings);
        state.remove_workspace_from_navigation("berlin", AppPage::Dashboard);

        assert!(state.navigate_back());
        let snapshot = state.snapshot();
        assert_eq!(snapshot.active_page, AppPage::History);
        assert_eq!(snapshot.selected_workspace, None);
    }

    #[test]
    fn removed_workspace_compacts_adjacent_global_navigation_entries() {
        let history = NavigationEntry {
            selected_workspace: None,
            active_page: AppPage::History,
            active_workspace_tab: WorkspaceTab::Chats,
        };
        let settings = NavigationEntry {
            selected_workspace: None,
            active_page: AppPage::Settings,
            active_workspace_tab: WorkspaceTab::Chats,
        };
        let berlin_history = NavigationEntry {
            selected_workspace: Some("berlin".to_owned()),
            ..history.clone()
        };
        let berlin_settings = NavigationEntry {
            selected_workspace: Some("berlin".to_owned()),
            ..settings.clone()
        };

        assert_eq!(
            sanitize_removed_workspace_navigation(
                &[
                    history.clone(),
                    berlin_history,
                    settings.clone(),
                    berlin_settings
                ],
                "berlin",
            ),
            vec![history, settings]
        );
    }

    #[test]
    fn renamed_workspace_updates_active_state_and_history() {
        let state = AppState::new(
            AppPaths::from_env(),
            Some("berlin".to_owned()),
            WorkspaceTab::Chats,
            AppPage::Workspace,
        );

        state.navigate_to_page(AppPage::History);
        state.navigate_to_workspace(Some("berlin".to_owned()));
        state.rename_workspace_in_navigation("berlin", "tokyo");

        assert_eq!(
            state.snapshot().selected_workspace.as_deref(),
            Some("tokyo")
        );
        assert!(state.navigate_back());
        assert_eq!(
            state.snapshot().selected_workspace.as_deref(),
            Some("tokyo")
        );
    }

    #[test]
    fn removing_selected_workspace_clears_active_state() {
        let state = AppState::new(
            AppPaths::from_env(),
            Some("berlin".to_owned()),
            WorkspaceTab::Chats,
            AppPage::Workspace,
        );

        state.set_selected_chat_thread(Some(42));
        state.set_selected_agent_session(Some(99));
        state.set_staged_review_prompt(Some("review".to_owned()));
        state.queue_pending_chat_prompt("fix bug".to_owned());

        state.remove_workspace_from_navigation("berlin", AppPage::Dashboard);
        let snapshot = state.snapshot();

        assert_eq!(snapshot.selected_workspace, None);
        assert_eq!(snapshot.selected_chat_thread, None);
        assert_eq!(snapshot.selected_agent_session, None);
        assert_eq!(snapshot.staged_review_prompt, None);
        assert_eq!(snapshot.pending_chat_prompt, None);
        assert_eq!(snapshot.active_page, AppPage::Dashboard);
    }
}
