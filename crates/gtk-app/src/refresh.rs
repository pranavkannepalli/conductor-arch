use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone, Copy, Debug)]
pub enum RefreshScope {
    All,
    Sidebar,
    Dashboard,
    Projects,
    History,
    Workspace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceRefreshTarget {
    Shell,
    ChatSurface,
    ChatTabs,
    Runtime,
    Review,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefreshEvent {
    Manual,
    ProjectInventoryChanged,
    SettingsChanged,
    WorkspaceSelectionChanged,
    WorkspaceInventoryChanged,
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
    WorkspaceChatLifecycleChanged {
        workspace: String,
    },
    WorkspaceChatMessagesChanged {
        workspace: String,
        thread_id: i64,
    },
    TerminalChanged {
        workspace: String,
    },
}

type RefreshHandler = Rc<dyn Fn()>;
type RefreshEventHandler = Rc<dyn Fn(&RefreshEvent)>;

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

    pub fn set_workspace(&self, handler: impl Fn() + 'static) {
        self.set_workspace_shell(handler);
    }

    pub fn set_workspace_shell(&self, handler: impl Fn() + 'static) {
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

    pub fn refresh_event(&self, event: RefreshEvent) {
        match event {
            RefreshEvent::Manual => self.refresh(RefreshScope::All),
            RefreshEvent::ProjectInventoryChanged => {
                self.refresh(RefreshScope::Projects);
                self.refresh(RefreshScope::Sidebar);
                self.refresh(RefreshScope::Dashboard);
            }
            RefreshEvent::SettingsChanged => {
                self.refresh(RefreshScope::Projects);
                self.refresh_workspace(WorkspaceRefreshTarget::Shell);
            }
            RefreshEvent::WorkspaceSelectionChanged => {
                self.refresh(RefreshScope::Sidebar);
                self.refresh_workspace(WorkspaceRefreshTarget::Shell);
            }
            RefreshEvent::WorkspaceInventoryChanged => {
                self.refresh(RefreshScope::Sidebar);
                self.refresh(RefreshScope::Dashboard);
                self.refresh(RefreshScope::History);
                self.refresh_workspace(WorkspaceRefreshTarget::Shell);
            }
            RefreshEvent::WorkspaceMetadataChanged { .. } => {
                self.run_event(
                    RefreshMetricTarget::WorkspaceNavRow,
                    &self.workspace_nav_row,
                    &event,
                );
            }
            RefreshEvent::WorkspaceRuntimeChanged { .. } | RefreshEvent::TerminalChanged { .. } => {
                self.refresh(RefreshScope::Sidebar);
                self.refresh(RefreshScope::Dashboard);
                self.refresh(RefreshScope::History);
                self.refresh_workspace_event(WorkspaceRefreshTarget::Runtime, &event);
            }
            RefreshEvent::WorkspaceChatLifecycleChanged { .. } => {
                self.refresh(RefreshScope::Sidebar);
                self.refresh(RefreshScope::Dashboard);
                self.refresh(RefreshScope::History);
                self.refresh_workspace_event(WorkspaceRefreshTarget::ChatTabs, &event);
                self.refresh_workspace_event(WorkspaceRefreshTarget::ChatSurface, &event);
            }
            RefreshEvent::WorkspaceReviewChanged { .. } => {
                self.refresh(RefreshScope::Dashboard);
                self.refresh(RefreshScope::History);
                self.refresh_workspace_event(WorkspaceRefreshTarget::Review, &event);
            }
            RefreshEvent::WorkspaceChatMessagesChanged { .. } => {
                self.refresh_workspace_event(WorkspaceRefreshTarget::ChatSurface, &event);
                self.run_event(
                    RefreshMetricTarget::WorkspaceChatTabs,
                    &self.workspace_chat_tabs,
                    &event,
                );
            }
        }
    }

    pub fn refresh_workspace(&self, target: WorkspaceRefreshTarget) {
        match target {
            WorkspaceRefreshTarget::Shell => {
                self.run(RefreshMetricTarget::WorkspaceShell, &self.workspace_shell)
            }
            WorkspaceRefreshTarget::ChatSurface => self.run_event(
                RefreshMetricTarget::WorkspaceChatSurface,
                &self.workspace_chat_surface,
                &RefreshEvent::Manual,
            ),
            WorkspaceRefreshTarget::ChatTabs => self.run_event_or_shell(
                RefreshMetricTarget::WorkspaceChatTabs,
                &self.workspace_chat_tabs,
                &RefreshEvent::Manual,
                &self.workspace_shell,
            ),
            WorkspaceRefreshTarget::Runtime => self.run_event_or_shell(
                RefreshMetricTarget::WorkspaceRuntime,
                &self.workspace_runtime,
                &RefreshEvent::Manual,
                &self.workspace_shell,
            ),
            WorkspaceRefreshTarget::Review => self.run_event_or_shell(
                RefreshMetricTarget::WorkspaceReview,
                &self.workspace_review,
                &RefreshEvent::Manual,
                &self.workspace_shell,
            ),
        }
    }

    fn refresh_workspace_event(&self, target: WorkspaceRefreshTarget, event: &RefreshEvent) {
        match target {
            WorkspaceRefreshTarget::ChatSurface => self.run_event(
                RefreshMetricTarget::WorkspaceChatSurface,
                &self.workspace_chat_surface,
                event,
            ),
            WorkspaceRefreshTarget::ChatTabs => self.run_event_or_shell(
                RefreshMetricTarget::WorkspaceChatTabs,
                &self.workspace_chat_tabs,
                event,
                &self.workspace_shell,
            ),
            WorkspaceRefreshTarget::Runtime => self.run_event_or_shell(
                RefreshMetricTarget::WorkspaceRuntime,
                &self.workspace_runtime,
                event,
                &self.workspace_shell,
            ),
            WorkspaceRefreshTarget::Review => self.run_event_or_shell(
                RefreshMetricTarget::WorkspaceReview,
                &self.workspace_review,
                event,
                &self.workspace_shell,
            ),
            _ => self.refresh_workspace(target),
        }
    }

    pub fn refresh(&self, scope: RefreshScope) {
        match scope {
            RefreshScope::All => {
                self.run(RefreshMetricTarget::Sidebar, &self.sidebar);
                self.run(RefreshMetricTarget::Dashboard, &self.dashboard);
                self.run(RefreshMetricTarget::Projects, &self.projects);
                self.run(RefreshMetricTarget::History, &self.history);
                self.run(RefreshMetricTarget::WorkspaceShell, &self.workspace_shell);
            }
            RefreshScope::Sidebar => self.run(RefreshMetricTarget::Sidebar, &self.sidebar),
            RefreshScope::Dashboard => self.run(RefreshMetricTarget::Dashboard, &self.dashboard),
            RefreshScope::Projects => self.run(RefreshMetricTarget::Projects, &self.projects),
            RefreshScope::History => self.run(RefreshMetricTarget::History, &self.history),
            RefreshScope::Workspace => {
                self.run(RefreshMetricTarget::WorkspaceShell, &self.workspace_shell)
            }
        }
    }

    fn run(&self, target: RefreshMetricTarget, slot: &Rc<RefCell<Option<RefreshHandler>>>) {
        let handler = slot.borrow().as_ref().cloned();
        if let Some(handler) = handler {
            self.metrics.record(target);
            handler();
        }
    }

    fn run_event_or_shell(
        &self,
        target: RefreshMetricTarget,
        slot: &Rc<RefCell<Option<RefreshEventHandler>>>,
        event: &RefreshEvent,
        shell: &Rc<RefCell<Option<RefreshHandler>>>,
    ) {
        let handler = slot.borrow().as_ref().cloned();
        if let Some(handler) = handler {
            self.metrics.record(target);
            handler(event);
        } else {
            self.run(RefreshMetricTarget::WorkspaceShell, shell);
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
            hub.set_workspace(move || workspace.set(workspace.get() + 1));

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
        hub.set_workspace(move || {
            hub_for_handler.set_workspace(|| {});
        });

        hub.refresh(RefreshScope::Workspace);
    }

    #[test]
    fn runtime_refresh_event_skips_projects() {
        let hub = RefreshHub::default();
        let counts = RefreshCounts::default();
        counts.install(&hub);

        hub.refresh_event(RefreshEvent::WorkspaceRuntimeChanged {
            workspace: "demo".to_owned(),
        });

        assert_eq!(counts.values(), (1, 1, 0, 1, 0, 0, 0, 1, 0, 0));
    }

    #[test]
    fn chat_message_refresh_event_updates_chat_surface_and_tabs_only() {
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
    fn refresh_metrics_count_chat_message_surface_and_tab_refreshes() {
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

        assert_eq!(counts.values(), (1, 1, 0, 1, 0, 1, 1, 0, 0, 0));
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
        assert_eq!(metrics.total, 5);
        assert_eq!(metrics.sidebar, 1);
        assert_eq!(metrics.dashboard, 1);
        assert_eq!(metrics.history, 1);
        assert_eq!(metrics.workspace_chat_surface, 1);
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
    fn chat_lifecycle_event_is_passed_to_chat_surface_handler() {
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

        assert_eq!(*seen.borrow(), Some(event));
    }

    #[test]
    fn review_refresh_event_updates_review_surface_without_shell_rebuild() {
        let hub = RefreshHub::default();
        let counts = RefreshCounts::default();
        counts.install(&hub);

        hub.refresh_event(RefreshEvent::WorkspaceReviewChanged {
            workspace: "demo".to_owned(),
        });

        assert_eq!(counts.values(), (0, 1, 0, 1, 0, 0, 0, 0, 1, 0));
    }

    #[test]
    fn unregistered_granular_workspace_handlers_fall_back_to_shell() {
        let hub = RefreshHub::default();
        let shell_count = Rc::new(Cell::new(0));
        let shell_count_for_handler = Rc::clone(&shell_count);
        hub.set_workspace(move || shell_count_for_handler.set(shell_count_for_handler.get() + 1));

        hub.refresh_event(RefreshEvent::WorkspaceRuntimeChanged {
            workspace: "demo".to_owned(),
        });
        hub.refresh_event(RefreshEvent::WorkspaceReviewChanged {
            workspace: "demo".to_owned(),
        });
        hub.refresh_event(RefreshEvent::WorkspaceChatLifecycleChanged {
            workspace: "demo".to_owned(),
        });

        assert_eq!(shell_count.get(), 3);
    }

    #[test]
    fn unregistered_chat_message_handler_does_not_rebuild_shell() {
        let hub = RefreshHub::default();
        let shell_count = Rc::new(Cell::new(0));
        let shell_count_for_handler = Rc::clone(&shell_count);
        hub.set_workspace(move || shell_count_for_handler.set(shell_count_for_handler.get() + 1));

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
        for (path, source) in [
            ("sidebar.rs", include_str!("sidebar.rs")),
            (
                "workspace_command_center.rs",
                include_str!("workspace_command_center.rs"),
            ),
            ("projects.rs", include_str!("projects.rs")),
        ] {
            assert!(
                !source.contains("RefreshScope::All"),
                "{path} contains a routine RefreshScope::All call"
            );
        }

        let main_source = include_str!("main.rs");
        assert_eq!(main_source.matches("RefreshScope::All").count(), 3);
        assert!(main_source.contains("Some(ShortcutAction::Refresh)"));
        assert!(main_source.contains("PaletteTarget::Refresh =>"));
    }
}
