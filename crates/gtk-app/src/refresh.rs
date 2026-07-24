use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

#[derive(Clone, Copy, Debug)]
pub enum RefreshScope {
    Sidebar,
    Dashboard,
    Projects,
    History,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkspaceRefreshTarget {
    ChatSurface,
    ChatTabs,
    Runtime,
    Review,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefreshEvent {
    ProjectInventoryChanged,
    SettingsChanged,
    WorkspaceSelectionChanged,
    WorkspaceInventoryChanged,
    WorkspaceHeaderChanged {
        workspace: String,
    },
    WorkspaceStatusChanged {
        workspace: String,
    },
    WorkspaceDiffStatsChanged {
        workspace: String,
        additions: u64,
        deletions: u64,
    },
    WorkspaceBranchChanged {
        workspace: String,
    },
    WorkspaceLifecycleChanged {
        workspace: String,
    },
    WorkspaceMetadataChanged {
        old_workspace: String,
        workspace: String,
        branch: Option<String>,
    },
    WorkspaceRuntimeChanged {
        workspace: String,
    },
    WorkspaceReviewChanged {
        workspace: String,
    },
    WorkspaceGitReviewChanged {
        workspace: String,
    },
    WorkspaceChatLifecycleChanged {
        workspace: String,
    },
    WorkspaceChatMessagesChanged {
        workspace: String,
        thread_id: i64,
    },
    ChatMessageAppended {
        workspace: String,
        thread_id: i64,
        message_id: i64,
    },
    ChatMessageUpdated {
        workspace: String,
        thread_id: i64,
        message_id: i64,
    },
    ChatTimelineTailChanged {
        workspace: String,
        thread_id: i64,
    },
    ChatComposerChanged {
        target: String,
    },
    ChatQueueChanged {
        target: String,
    },
    ChatTabChanged {
        workspace: String,
        thread_id: i64,
    },
    ChatSessionStatusChanged {
        workspace: String,
        thread_id: i64,
        session_id: i64,
    },
    RightPanelFileListChanged {
        workspace: String,
    },
    RightPanelSelectedFileChanged {
        workspace: String,
        path: String,
    },
    RightPanelDiffPreviewChanged {
        workspace: String,
        path: String,
    },
    ReviewCommentsChanged {
        workspace: String,
    },
    TodosChanged {
        workspace: String,
    },
    TerminalBufferChanged {
        workspace: String,
        terminal_id: i64,
    },
    RuntimeProcessChanged {
        workspace: String,
        process_id: i64,
    },
    SettingsSectionChanged {
        scope: String,
        section: String,
    },
    TerminalChanged {
        workspace: String,
    },
}

type RefreshHandler = Rc<dyn Fn()>;
type RefreshEventHandler = Rc<dyn Fn(&RefreshEvent)>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefreshFilter {
    WorkspaceDiffStats { workspace: String },
}

impl RefreshFilter {
    pub fn workspace_diff_stats(workspace: impl Into<String>) -> Self {
        Self::WorkspaceDiffStats {
            workspace: workspace.into(),
        }
    }

    fn matches(&self, event: &RefreshEvent) -> bool {
        match (self, event) {
            (
                Self::WorkspaceDiffStats { workspace: target },
                RefreshEvent::WorkspaceDiffStatsChanged { workspace, .. },
            ) => workspace == target,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceDiffStatsRefresh {
    pub workspace: String,
    pub additions: u64,
    pub deletions: u64,
}

struct RefreshListener {
    id: u64,
    filter: RefreshFilter,
    handler: RefreshEventHandler,
}

pub struct RefreshSubscription {
    id: u64,
    listeners: Weak<RefCell<Vec<RefreshListener>>>,
}

impl Drop for RefreshSubscription {
    fn drop(&mut self) {
        if let Some(listeners) = self.listeners.upgrade() {
            listeners
                .borrow_mut()
                .retain(|listener| listener.id != self.id);
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum RefreshMetricTarget {
    Sidebar,
    Dashboard,
    Projects,
    History,
    WorkspaceShell,
    WorkspaceChatSurface,
    WorkspaceChatTabs,
    WorkspaceRuntime,
    WorkspaceReview,
    WorkspaceNavRow,
    RightPanelFileList,
    RightPanelDiffPreview,
}

impl RefreshMetricTarget {
    fn label(self) -> &'static str {
        match self {
            Self::Sidebar => "sidebar",
            Self::Dashboard => "dashboard",
            Self::Projects => "projects",
            Self::History => "history",
            Self::WorkspaceShell => "workspace_shell",
            Self::WorkspaceChatSurface => "workspace_chat_surface",
            Self::WorkspaceChatTabs => "workspace_chat_tabs",
            Self::WorkspaceRuntime => "workspace_runtime",
            Self::WorkspaceReview => "workspace_review",
            Self::WorkspaceNavRow => "workspace_nav_row",
            Self::RightPanelFileList => "right_panel_file_list",
            Self::RightPanelDiffPreview => "right_panel_diff_preview",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RefreshMetricSnapshot {
    total: u64,
    sidebar: u64,
    dashboard: u64,
    projects: u64,
    history: u64,
    workspace_shell: u64,
    workspace_chat_surface: u64,
    workspace_chat_tabs: u64,
    workspace_runtime: u64,
    workspace_review: u64,
    workspace_nav_row: u64,
    right_panel_file_list: u64,
    right_panel_diff_preview: u64,
}

#[derive(Default)]
struct RefreshMetrics {
    enabled: Cell<Option<bool>>,
    total: Cell<u64>,
    sidebar: Cell<u64>,
    dashboard: Cell<u64>,
    projects: Cell<u64>,
    history: Cell<u64>,
    workspace_shell: Cell<u64>,
    workspace_chat_surface: Cell<u64>,
    workspace_chat_tabs: Cell<u64>,
    workspace_runtime: Cell<u64>,
    workspace_review: Cell<u64>,
    workspace_nav_row: Cell<u64>,
    right_panel_file_list: Cell<u64>,
    right_panel_diff_preview: Cell<u64>,
}

impl RefreshMetrics {
    fn record(&self, target: RefreshMetricTarget) {
        self.total.set(self.total.get() + 1);
        match target {
            RefreshMetricTarget::Sidebar => self.sidebar.set(self.sidebar.get() + 1),
            RefreshMetricTarget::Dashboard => self.dashboard.set(self.dashboard.get() + 1),
            RefreshMetricTarget::Projects => self.projects.set(self.projects.get() + 1),
            RefreshMetricTarget::History => self.history.set(self.history.get() + 1),
            RefreshMetricTarget::WorkspaceShell => {
                self.workspace_shell.set(self.workspace_shell.get() + 1)
            }
            RefreshMetricTarget::WorkspaceChatSurface => self
                .workspace_chat_surface
                .set(self.workspace_chat_surface.get() + 1),
            RefreshMetricTarget::WorkspaceChatTabs => self
                .workspace_chat_tabs
                .set(self.workspace_chat_tabs.get() + 1),
            RefreshMetricTarget::WorkspaceRuntime => {
                self.workspace_runtime.set(self.workspace_runtime.get() + 1)
            }
            RefreshMetricTarget::WorkspaceReview => {
                self.workspace_review.set(self.workspace_review.get() + 1)
            }
            RefreshMetricTarget::WorkspaceNavRow => {
                self.workspace_nav_row.set(self.workspace_nav_row.get() + 1)
            }
            RefreshMetricTarget::RightPanelFileList => self
                .right_panel_file_list
                .set(self.right_panel_file_list.get() + 1),
            RefreshMetricTarget::RightPanelDiffPreview => self
                .right_panel_diff_preview
                .set(self.right_panel_diff_preview.get() + 1),
        }
        if self.enabled() {
            let snapshot = self.snapshot();
            tracing::info!(
                target = target.label(),
                total = snapshot.total,
                sidebar = snapshot.sidebar,
                dashboard = snapshot.dashboard,
                projects = snapshot.projects,
                history = snapshot.history,
                workspace_shell = snapshot.workspace_shell,
                workspace_chat_surface = snapshot.workspace_chat_surface,
                workspace_chat_tabs = snapshot.workspace_chat_tabs,
                workspace_runtime = snapshot.workspace_runtime,
                workspace_review = snapshot.workspace_review,
                workspace_nav_row = snapshot.workspace_nav_row,
                right_panel_file_list = snapshot.right_panel_file_list,
                right_panel_diff_preview = snapshot.right_panel_diff_preview,
                "gtk refresh metric"
            );
        }
    }

    fn enabled(&self) -> bool {
        if let Some(enabled) = self.enabled.get() {
            return enabled;
        }
        let enabled = archductor_core::env_flags::enabled("ARCHDUCTOR_GTK_REFRESH_METRICS");
        self.enabled.set(Some(enabled));
        enabled
    }

    fn snapshot(&self) -> RefreshMetricSnapshot {
        RefreshMetricSnapshot {
            total: self.total.get(),
            sidebar: self.sidebar.get(),
            dashboard: self.dashboard.get(),
            projects: self.projects.get(),
            history: self.history.get(),
            workspace_shell: self.workspace_shell.get(),
            workspace_chat_surface: self.workspace_chat_surface.get(),
            workspace_chat_tabs: self.workspace_chat_tabs.get(),
            workspace_runtime: self.workspace_runtime.get(),
            workspace_review: self.workspace_review.get(),
            workspace_nav_row: self.workspace_nav_row.get(),
            right_panel_file_list: self.right_panel_file_list.get(),
            right_panel_diff_preview: self.right_panel_diff_preview.get(),
        }
    }
}

/// Dumb UI fanout for page refresh callbacks.
///
/// PER-190: RefreshHub intentionally has no typed error channel; each page owns
/// its load/store error handling and renders failures in-place before or during
/// its registered callback. Replace this with typed refresh results only if
/// multiple pages need shared page-owned error handling semantics.
#[derive(Clone, Default)]
pub struct RefreshHub {
    sidebar: Rc<RefCell<Option<RefreshHandler>>>,
    dashboard: Rc<RefCell<Option<RefreshHandler>>>,
    projects: Rc<RefCell<Option<RefreshHandler>>>,
    history: Rc<RefCell<Option<RefreshHandler>>>,
    workspace_shell: Rc<RefCell<Option<RefreshHandler>>>,
    workspace_chat_surface: Rc<RefCell<Option<RefreshEventHandler>>>,
    workspace_chat_tabs: Rc<RefCell<Option<RefreshEventHandler>>>,
    workspace_runtime: Rc<RefCell<Option<RefreshEventHandler>>>,
    workspace_review: Rc<RefCell<Option<RefreshEventHandler>>>,
    workspace_nav_row: Rc<RefCell<Option<RefreshEventHandler>>>,
    right_panel_file_list: Rc<RefCell<Option<RefreshEventHandler>>>,
    right_panel_diff_preview: Rc<RefCell<Option<RefreshEventHandler>>>,
    listeners: Rc<RefCell<Vec<RefreshListener>>>,
    next_listener_id: Rc<Cell<u64>>,
    metrics: Rc<RefreshMetrics>,
}

impl RefreshHub {
    pub fn set_sidebar(&self, handler: impl Fn() + 'static) {
        *self.sidebar.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn set_dashboard(&self, handler: impl Fn() + 'static) {
        *self.dashboard.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn set_projects(&self, handler: impl Fn() + 'static) {
        *self.projects.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn set_history(&self, handler: impl Fn() + 'static) {
        *self.history.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn set_workspace_mount(&self, handler: impl Fn() + 'static) {
        *self.workspace_shell.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn set_workspace_chat_surface(&self, handler: impl Fn(&RefreshEvent) + 'static) {
        *self.workspace_chat_surface.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn set_workspace_chat_tabs(&self, handler: impl Fn(&RefreshEvent) + 'static) {
        *self.workspace_chat_tabs.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn set_workspace_runtime(&self, handler: impl Fn(&RefreshEvent) + 'static) {
        *self.workspace_runtime.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn set_workspace_review(&self, handler: impl Fn(&RefreshEvent) + 'static) {
        *self.workspace_review.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn set_workspace_nav_row(&self, handler: impl Fn(&RefreshEvent) + 'static) {
        *self.workspace_nav_row.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn set_right_panel_file_list(&self, handler: impl Fn(&RefreshEvent) + 'static) {
        *self.right_panel_file_list.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn set_right_panel_diff_preview(&self, handler: impl Fn(&RefreshEvent) + 'static) {
        *self.right_panel_diff_preview.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn subscribe(
        &self,
        filter: RefreshFilter,
        handler: impl Fn(&RefreshEvent) + 'static,
    ) -> RefreshSubscription {
        let id = self.next_listener_id.get();
        self.next_listener_id.set(id.saturating_add(1));
        self.listeners.borrow_mut().push(RefreshListener {
            id,
            filter,
            handler: Rc::new(handler),
        });
        RefreshSubscription {
            id,
            listeners: Rc::downgrade(&self.listeners),
        }
    }

    pub fn on_workspace_diff_stats(
        &self,
        workspace: impl Into<String>,
        handler: impl Fn(WorkspaceDiffStatsRefresh) + 'static,
    ) -> RefreshSubscription {
        self.subscribe(
            RefreshFilter::workspace_diff_stats(workspace),
            move |event| {
                if let RefreshEvent::WorkspaceDiffStatsChanged {
                    workspace,
                    additions,
                    deletions,
                } = event
                {
                    handler(WorkspaceDiffStatsRefresh {
                        workspace: workspace.clone(),
                        additions: *additions,
                        deletions: *deletions,
                    });
                }
            },
        )
    }

    pub fn refresh_event(&self, event: RefreshEvent) {
        self.run_listeners(&event);
        match event {
            RefreshEvent::ProjectInventoryChanged => {
                self.refresh(RefreshScope::Projects);
                self.refresh(RefreshScope::Sidebar);
                self.refresh(RefreshScope::Dashboard);
            }
            RefreshEvent::SettingsChanged => {
                self.refresh(RefreshScope::Projects);
                self.refresh_workspace_mount();
            }
            RefreshEvent::WorkspaceSelectionChanged => {
                self.refresh(RefreshScope::Sidebar);
                self.refresh_workspace_mount();
            }
            RefreshEvent::WorkspaceInventoryChanged => {
                self.refresh(RefreshScope::Sidebar);
                self.refresh(RefreshScope::Dashboard);
                self.refresh(RefreshScope::History);
                self.refresh_workspace_mount();
            }
            RefreshEvent::WorkspaceLifecycleChanged { .. } => {
                self.refresh(RefreshScope::Sidebar);
                self.refresh(RefreshScope::Dashboard);
                self.refresh(RefreshScope::History);
                self.refresh_workspace_mount();
            }
            RefreshEvent::WorkspaceHeaderChanged { .. }
            | RefreshEvent::WorkspaceStatusChanged { .. }
            | RefreshEvent::WorkspaceBranchChanged { .. }
            | RefreshEvent::WorkspaceMetadataChanged { .. } => {
                self.run_event(
                    RefreshMetricTarget::WorkspaceNavRow,
                    &self.workspace_nav_row,
                    &event,
                );
            }
            RefreshEvent::WorkspaceDiffStatsChanged { .. } => {}
            RefreshEvent::WorkspaceRuntimeChanged { .. } | RefreshEvent::TerminalChanged { .. } => {
                self.refresh_workspace_event(WorkspaceRefreshTarget::Runtime, &event);
            }
            RefreshEvent::WorkspaceChatLifecycleChanged { .. } => {
                self.refresh_workspace_event(WorkspaceRefreshTarget::ChatTabs, &event);
            }
            RefreshEvent::WorkspaceReviewChanged { .. } => {
                self.refresh_workspace_event(WorkspaceRefreshTarget::Review, &event);
            }
            RefreshEvent::WorkspaceGitReviewChanged { .. } => {
                self.refresh_workspace_event(WorkspaceRefreshTarget::Review, &event);
                self.run_event(
                    RefreshMetricTarget::WorkspaceNavRow,
                    &self.workspace_nav_row,
                    &event,
                );
            }
            RefreshEvent::WorkspaceChatMessagesChanged { .. }
            | RefreshEvent::ChatMessageAppended { .. }
            | RefreshEvent::ChatMessageUpdated { .. }
            | RefreshEvent::ChatTimelineTailChanged { .. } => {
                self.refresh_workspace_event(WorkspaceRefreshTarget::ChatSurface, &event);
                self.refresh_workspace_event(WorkspaceRefreshTarget::ChatTabs, &event);
            }
            RefreshEvent::ChatComposerChanged { .. }
            | RefreshEvent::ChatQueueChanged { .. }
            | RefreshEvent::ChatTabChanged { .. } => {}
            RefreshEvent::ChatSessionStatusChanged { .. } => {
                self.refresh_workspace_event(WorkspaceRefreshTarget::ChatTabs, &event);
            }
            RefreshEvent::RightPanelFileListChanged { .. } => self.run_event(
                RefreshMetricTarget::RightPanelFileList,
                &self.right_panel_file_list,
                &event,
            ),
            RefreshEvent::RightPanelSelectedFileChanged { .. }
            | RefreshEvent::RightPanelDiffPreviewChanged { .. } => self.run_event(
                RefreshMetricTarget::RightPanelDiffPreview,
                &self.right_panel_diff_preview,
                &event,
            ),
            RefreshEvent::ReviewCommentsChanged { .. }
            | RefreshEvent::TodosChanged { .. }
            | RefreshEvent::TerminalBufferChanged { .. }
            | RefreshEvent::RuntimeProcessChanged { .. }
            | RefreshEvent::SettingsSectionChanged { .. } => {}
        }
    }

    pub fn refresh_workspace_mount(&self) {
        self.run(RefreshMetricTarget::WorkspaceShell, &self.workspace_shell);
    }

    fn refresh_workspace_event(&self, target: WorkspaceRefreshTarget, event: &RefreshEvent) {
        match target {
            WorkspaceRefreshTarget::ChatSurface => self.run_event(
                RefreshMetricTarget::WorkspaceChatSurface,
                &self.workspace_chat_surface,
                event,
            ),
            WorkspaceRefreshTarget::ChatTabs => self.run_event(
                RefreshMetricTarget::WorkspaceChatTabs,
                &self.workspace_chat_tabs,
                event,
            ),
            WorkspaceRefreshTarget::Runtime => self.run_event(
                RefreshMetricTarget::WorkspaceRuntime,
                &self.workspace_runtime,
                event,
            ),
            WorkspaceRefreshTarget::Review => self.run_event(
                RefreshMetricTarget::WorkspaceReview,
                &self.workspace_review,
                event,
            ),
        }
    }

    pub fn refresh(&self, scope: RefreshScope) {
        match scope {
            RefreshScope::Sidebar => self.run(RefreshMetricTarget::Sidebar, &self.sidebar),
            RefreshScope::Dashboard => self.run(RefreshMetricTarget::Dashboard, &self.dashboard),
            RefreshScope::Projects => self.run(RefreshMetricTarget::Projects, &self.projects),
            RefreshScope::History => self.run(RefreshMetricTarget::History, &self.history),
        }
    }

    pub fn debug_full_refresh(&self) {
        if !archductor_core::env_flags::enabled("ARCHDUCTOR_GTK_DEBUG_FULL_REFRESH") {
            tracing::debug!(
                "ignored full GTK refresh because ARCHDUCTOR_GTK_DEBUG_FULL_REFRESH is disabled"
            );
            return;
        }
        self.run(RefreshMetricTarget::Sidebar, &self.sidebar);
        self.run(RefreshMetricTarget::Dashboard, &self.dashboard);
        self.run(RefreshMetricTarget::Projects, &self.projects);
        self.run(RefreshMetricTarget::History, &self.history);
        self.run(RefreshMetricTarget::WorkspaceShell, &self.workspace_shell);
    }

    fn run(&self, target: RefreshMetricTarget, slot: &Rc<RefCell<Option<RefreshHandler>>>) {
        let handler = slot.borrow().as_ref().cloned();
        if let Some(handler) = handler {
            self.metrics.record(target);
            handler();
        }
    }

    fn run_event(
        &self,
        target: RefreshMetricTarget,
        slot: &Rc<RefCell<Option<RefreshEventHandler>>>,
        event: &RefreshEvent,
    ) {
        let handler = slot.borrow().as_ref().cloned();
        if let Some(handler) = handler {
            self.metrics.record(target);
            handler(event);
        }
    }

    fn run_listeners(&self, event: &RefreshEvent) {
        let handlers: Vec<RefreshEventHandler> = self
            .listeners
            .borrow()
            .iter()
            .filter(|listener| listener.filter.matches(event))
            .map(|listener| Rc::clone(&listener.handler))
            .collect();
        for handler in handlers {
            handler(event);
        }
    }

    #[cfg(test)]
    fn refresh_metrics_snapshot(&self) -> RefreshMetricSnapshot {
        self.metrics.snapshot()
    }

    #[cfg(test)]
    fn set_refresh_metrics_enabled_for_test(&self, enabled: bool) {
        self.metrics.enabled.set(Some(enabled));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::fs;
    use std::path::{Path, PathBuf};

    #[derive(Default)]
    struct RefreshCounts {
        sidebar: Rc<Cell<u32>>,
        dashboard: Rc<Cell<u32>>,
        projects: Rc<Cell<u32>>,
        history: Rc<Cell<u32>>,
        workspace: Rc<Cell<u32>>,
        workspace_chat_surface: Rc<Cell<u32>>,
        workspace_chat_tabs: Rc<Cell<u32>>,
        workspace_runtime: Rc<Cell<u32>>,
        workspace_review: Rc<Cell<u32>>,
        workspace_nav_row: Rc<Cell<u32>>,
    }

    impl RefreshCounts {
        fn install(&self, hub: &RefreshHub) {
            let sidebar = Rc::clone(&self.sidebar);
            hub.set_sidebar(move || sidebar.set(sidebar.get() + 1));

            let dashboard = Rc::clone(&self.dashboard);
            hub.set_dashboard(move || dashboard.set(dashboard.get() + 1));

            let projects = Rc::clone(&self.projects);
            hub.set_projects(move || projects.set(projects.get() + 1));

            let history = Rc::clone(&self.history);
            hub.set_history(move || history.set(history.get() + 1));

            let workspace = Rc::clone(&self.workspace);
            hub.set_workspace_mount(move || workspace.set(workspace.get() + 1));

            let workspace_chat_surface = Rc::clone(&self.workspace_chat_surface);
            hub.set_workspace_chat_surface(move |_| {
                workspace_chat_surface.set(workspace_chat_surface.get() + 1)
            });

            let workspace_chat_tabs = Rc::clone(&self.workspace_chat_tabs);
            hub.set_workspace_chat_tabs(move |_| {
                workspace_chat_tabs.set(workspace_chat_tabs.get() + 1)
            });

            let workspace_runtime = Rc::clone(&self.workspace_runtime);
            hub.set_workspace_runtime(move |_| workspace_runtime.set(workspace_runtime.get() + 1));

            let workspace_review = Rc::clone(&self.workspace_review);
            hub.set_workspace_review(move |_| workspace_review.set(workspace_review.get() + 1));

            let workspace_nav_row = Rc::clone(&self.workspace_nav_row);
            hub.set_workspace_nav_row(move |_| workspace_nav_row.set(workspace_nav_row.get() + 1));
        }

        fn values(&self) -> (u32, u32, u32, u32, u32, u32, u32, u32, u32, u32) {
            (
                self.sidebar.get(),
                self.dashboard.get(),
                self.projects.get(),
                self.history.get(),
                self.workspace.get(),
                self.workspace_chat_surface.get(),
                self.workspace_chat_tabs.get(),
                self.workspace_runtime.get(),
                self.workspace_review.get(),
                self.workspace_nav_row.get(),
            )
        }
    }

    #[test]
    fn refresh_handler_can_replace_same_scope_without_refcell_panic() {
        let hub = RefreshHub::default();
        let hub_for_handler = hub.clone();
        hub.set_workspace_mount(move || {
            hub_for_handler.set_workspace_mount(|| {});
        });

        hub.refresh_workspace_mount();
    }

    #[test]
    fn runtime_refresh_event_skips_projects() {
        let hub = RefreshHub::default();
        let counts = RefreshCounts::default();
        counts.install(&hub);

        hub.refresh_event(RefreshEvent::WorkspaceRuntimeChanged {
            workspace: "demo".to_owned(),
        });

        assert_eq!(counts.values(), (0, 0, 0, 0, 0, 0, 0, 1, 0, 0));
    }

    #[test]
    fn chat_message_refresh_event_updates_chat_surface_only() {
        let hub = RefreshHub::default();
        let counts = RefreshCounts::default();
        counts.install(&hub);
        hub.set_refresh_metrics_enabled_for_test(false);

        hub.refresh_event(RefreshEvent::WorkspaceChatMessagesChanged {
            workspace: "demo".to_owned(),
            thread_id: 7,
        });

        assert_eq!(counts.values(), (0, 0, 0, 0, 0, 1, 1, 0, 0, 0));
    }

    #[test]
    fn chat_session_status_event_updates_chat_tabs_without_shell_rebuild() {
        let hub = RefreshHub::default();
        let counts = RefreshCounts::default();
        counts.install(&hub);

        hub.refresh_event(RefreshEvent::ChatSessionStatusChanged {
            workspace: "demo".to_owned(),
            thread_id: 7,
            session_id: 11,
        });

        assert_eq!(counts.values(), (0, 0, 0, 0, 0, 0, 1, 0, 0, 0));
    }

    #[test]
    fn workspace_metadata_refresh_updates_only_workspace_nav_row() {
        let hub = RefreshHub::default();
        let counts = RefreshCounts::default();
        counts.install(&hub);

        hub.refresh_event(RefreshEvent::WorkspaceMetadataChanged {
            old_workspace: "old-name".to_owned(),
            workspace: "new-name".to_owned(),
            branch: None,
        });

        assert_eq!(counts.values(), (0, 0, 0, 0, 0, 0, 0, 0, 0, 1));
    }

    #[test]
    fn workspace_diff_stats_subscription_receives_only_matching_workspace() {
        let hub = RefreshHub::default();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let seen_for_handler = Rc::clone(&seen);
        let _subscription = hub.on_workspace_diff_stats("a", move |stats| {
            seen_for_handler.borrow_mut().push(stats);
        });

        hub.refresh_event(RefreshEvent::WorkspaceDiffStatsChanged {
            workspace: "a".to_owned(),
            additions: 12,
            deletions: 3,
        });
        hub.refresh_event(RefreshEvent::WorkspaceDiffStatsChanged {
            workspace: "b".to_owned(),
            additions: 99,
            deletions: 88,
        });
        hub.refresh_event(RefreshEvent::WorkspaceMetadataChanged {
            old_workspace: "old-a".to_owned(),
            workspace: "a".to_owned(),
            branch: None,
        });
        hub.refresh_event(RefreshEvent::WorkspaceStatusChanged {
            workspace: "a".to_owned(),
        });
        hub.refresh_event(RefreshEvent::WorkspaceRuntimeChanged {
            workspace: "a".to_owned(),
        });
        hub.refresh_event(RefreshEvent::WorkspaceChatMessagesChanged {
            workspace: "a".to_owned(),
            thread_id: 7,
        });
        hub.refresh_event(RefreshEvent::RightPanelDiffPreviewChanged {
            workspace: "a".to_owned(),
            path: "src/lib.rs".to_owned(),
        });

        assert_eq!(
            *seen.borrow(),
            vec![WorkspaceDiffStatsRefresh {
                workspace: "a".to_owned(),
                additions: 12,
                deletions: 3,
            }]
        );
    }

    #[test]
    fn workspace_diff_stats_subscription_unregisters_on_drop() {
        let hub = RefreshHub::default();
        let seen = Rc::new(Cell::new(0));
        let seen_for_handler = Rc::clone(&seen);
        let subscription = hub.on_workspace_diff_stats("a", move |_| {
            seen_for_handler.set(seen_for_handler.get() + 1);
        });

        hub.refresh_event(RefreshEvent::WorkspaceDiffStatsChanged {
            workspace: "a".to_owned(),
            additions: 1,
            deletions: 2,
        });
        drop(subscription);
        hub.refresh_event(RefreshEvent::WorkspaceDiffStatsChanged {
            workspace: "a".to_owned(),
            additions: 3,
            deletions: 4,
        });

        assert_eq!(seen.get(), 1);
    }

    #[test]
    fn workspace_diff_stats_refresh_does_not_call_workspace_nav_row() {
        let hub = RefreshHub::default();
        let counts = RefreshCounts::default();
        counts.install(&hub);

        hub.refresh_event(RefreshEvent::WorkspaceDiffStatsChanged {
            workspace: "demo".to_owned(),
            additions: 4,
            deletions: 2,
        });

        assert_eq!(counts.values(), (0, 0, 0, 0, 0, 0, 0, 0, 0, 0));
        let metrics = hub.refresh_metrics_snapshot();
        assert_eq!(metrics.total, 0);
        assert_eq!(metrics.workspace_nav_row, 0);
        assert_eq!(metrics.sidebar, 0);
        assert_eq!(metrics.workspace_shell, 0);
    }

    #[test]
    fn refresh_metrics_count_chat_message_surface_only() {
        let hub = RefreshHub::default();
        let counts = RefreshCounts::default();
        counts.install(&hub);

        hub.refresh_event(RefreshEvent::WorkspaceChatMessagesChanged {
            workspace: "demo".to_owned(),
            thread_id: 7,
        });

        let metrics = hub.refresh_metrics_snapshot();
        assert_eq!(metrics.total, 2);
        assert_eq!(metrics.workspace_chat_surface, 1);
        assert_eq!(metrics.workspace_chat_tabs, 1);
        assert_eq!(metrics.workspace_shell, 0);
    }

    #[test]
    fn chat_lifecycle_refresh_event_updates_nav_summaries_and_tabs() {
        let hub = RefreshHub::default();
        let counts = RefreshCounts::default();
        counts.install(&hub);
        hub.set_refresh_metrics_enabled_for_test(false);

        hub.refresh_event(RefreshEvent::WorkspaceChatLifecycleChanged {
            workspace: "demo".to_owned(),
        });

        assert_eq!(counts.values(), (0, 0, 0, 0, 0, 0, 1, 0, 0, 0));
    }

    #[test]
    fn refresh_metrics_count_lifecycle_fanout_refreshes() {
        let hub = RefreshHub::default();
        let counts = RefreshCounts::default();
        counts.install(&hub);

        hub.refresh_event(RefreshEvent::WorkspaceChatLifecycleChanged {
            workspace: "demo".to_owned(),
        });

        let metrics = hub.refresh_metrics_snapshot();
        assert_eq!(metrics.total, 1);
        assert_eq!(metrics.sidebar, 0);
        assert_eq!(metrics.dashboard, 0);
        assert_eq!(metrics.history, 0);
        assert_eq!(metrics.workspace_chat_surface, 0);
        assert_eq!(metrics.workspace_chat_tabs, 1);
    }

    #[test]
    fn chat_lifecycle_event_is_passed_to_chat_tabs_handler() {
        let hub = RefreshHub::default();
        let seen = Rc::new(RefCell::new(None));
        let seen_for_handler = Rc::clone(&seen);
        hub.set_workspace_chat_tabs(move |event| {
            *seen_for_handler.borrow_mut() = Some(event.clone());
        });

        let event = RefreshEvent::WorkspaceChatLifecycleChanged {
            workspace: "demo".to_owned(),
        };
        hub.refresh_event(event.clone());

        assert_eq!(*seen.borrow(), Some(event));
    }

    #[test]
    fn chat_lifecycle_event_skips_chat_surface_handler() {
        let hub = RefreshHub::default();
        let seen = Rc::new(RefCell::new(None));
        let seen_for_handler = Rc::clone(&seen);
        hub.set_workspace_chat_surface(move |event| {
            *seen_for_handler.borrow_mut() = Some(event.clone());
        });

        let event = RefreshEvent::WorkspaceChatLifecycleChanged {
            workspace: "demo".to_owned(),
        };
        hub.refresh_event(event.clone());

        assert_eq!(*seen.borrow(), None);
    }

    #[test]
    fn review_refresh_event_updates_review_surface_without_shell_rebuild() {
        let hub = RefreshHub::default();
        let counts = RefreshCounts::default();
        counts.install(&hub);

        hub.refresh_event(RefreshEvent::WorkspaceReviewChanged {
            workspace: "demo".to_owned(),
        });

        assert_eq!(counts.values(), (0, 0, 0, 0, 0, 0, 0, 0, 1, 0));
    }

    #[test]
    fn git_review_refresh_event_updates_review_and_nav_without_global_summary_refresh() {
        let hub = RefreshHub::default();
        let counts = RefreshCounts::default();
        counts.install(&hub);

        hub.refresh_event(RefreshEvent::WorkspaceGitReviewChanged {
            workspace: "demo".to_owned(),
        });

        assert_eq!(counts.values(), (0, 0, 0, 0, 0, 0, 0, 0, 1, 1));
    }

    #[test]
    fn right_panel_diff_preview_event_updates_only_right_panel_child() {
        let hub = RefreshHub::default();
        let counts = RefreshCounts::default();
        counts.install(&hub);
        let right_panel_count = Rc::new(Cell::new(0));
        let right_panel_count_for_handler = Rc::clone(&right_panel_count);
        hub.set_right_panel_diff_preview(move |_| {
            right_panel_count_for_handler.set(right_panel_count_for_handler.get() + 1);
        });

        hub.refresh_event(RefreshEvent::RightPanelDiffPreviewChanged {
            workspace: "demo".to_owned(),
            path: "src/main.rs".to_owned(),
        });

        assert_eq!(right_panel_count.get(), 1);
        assert_eq!(counts.values(), (0, 0, 0, 0, 0, 0, 0, 0, 0, 0));
        let metrics = hub.refresh_metrics_snapshot();
        assert_eq!(metrics.total, 1);
        assert_eq!(metrics.right_panel_diff_preview, 1);
        assert_eq!(metrics.workspace_shell, 0);
        assert_eq!(metrics.workspace_chat_surface, 0);
    }

    #[test]
    fn unregistered_granular_workspace_handlers_do_not_fall_back_to_shell() {
        let hub = RefreshHub::default();
        let shell_count = Rc::new(Cell::new(0));
        let shell_count_for_handler = Rc::clone(&shell_count);
        hub.set_workspace_mount(move || {
            shell_count_for_handler.set(shell_count_for_handler.get() + 1)
        });

        hub.refresh_event(RefreshEvent::WorkspaceRuntimeChanged {
            workspace: "demo".to_owned(),
        });
        hub.refresh_event(RefreshEvent::WorkspaceReviewChanged {
            workspace: "demo".to_owned(),
        });
        hub.refresh_event(RefreshEvent::WorkspaceChatLifecycleChanged {
            workspace: "demo".to_owned(),
        });

        assert_eq!(shell_count.get(), 0);
    }

    #[test]
    fn unregistered_chat_message_handler_does_not_rebuild_shell() {
        let hub = RefreshHub::default();
        let shell_count = Rc::new(Cell::new(0));
        let shell_count_for_handler = Rc::clone(&shell_count);
        hub.set_workspace_mount(move || {
            shell_count_for_handler.set(shell_count_for_handler.get() + 1)
        });

        hub.refresh_event(RefreshEvent::WorkspaceChatMessagesChanged {
            workspace: "demo".to_owned(),
            thread_id: 7,
        });

        assert_eq!(shell_count.get(), 0);
    }

    #[test]
    fn project_inventory_refresh_event_updates_global_summaries() {
        let hub = RefreshHub::default();
        let counts = RefreshCounts::default();
        counts.install(&hub);

        hub.refresh_event(RefreshEvent::ProjectInventoryChanged);

        assert_eq!(counts.values(), (1, 1, 1, 0, 0, 0, 0, 0, 0, 0));
    }

    #[test]
    fn routine_sources_do_not_use_refresh_all() {
        let mut paths = Vec::new();
        collect_rust_sources(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut paths,
        );
        paths.sort();

        for path in paths {
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
            let production_source = source
                .split("#[cfg(test)]")
                .next()
                .expect("source should contain production code");
            let relative_path = path
                .strip_prefix(env!("CARGO_MANIFEST_DIR"))
                .unwrap_or(&path)
                .display();
            for forbidden in [
                concat!("RefreshScope", "::All"),
                concat!("RefreshScope", "::Workspace"),
                concat!("WorkspaceRefreshTarget", "::Shell"),
                concat!("RefreshEvent", "::Manual"),
                concat!("set_workspace", "("),
                concat!("set_workspace", "_shell"),
                concat!("run_event", "_or_shell"),
                concat!("refresh_workspace", "(WorkspaceRefreshTarget"),
            ] {
                assert!(
                    !production_source.contains(forbidden),
                    "{relative_path} contains obsolete broad refresh handler `{forbidden}`"
                );
            }
        }

        let refresh_source = include_str!("refresh.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("refresh source should contain production code");
        assert!(!refresh_source.contains(concat!("run_event", "_or_shell")));
        assert!(!refresh_source.contains(concat!("WorkspaceRefreshTarget", "::Shell")));
        assert!(refresh_source.contains("ARCHDUCTOR_GTK_DEBUG_FULL_REFRESH"));
    }

    fn collect_rust_sources(dir: &Path, paths: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap_or_else(|err| {
            panic!("failed to read source directory {}: {err}", dir.display())
        }) {
            let entry = entry.expect("source directory entry should be readable");
            let path = entry.path();
            if path.is_dir() {
                collect_rust_sources(&path, paths);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                paths.push(path);
            }
        }
    }
}
