use std::cell::RefCell;
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
    WorkspaceRuntimeChanged { workspace: String },
    WorkspaceReviewChanged { workspace: String },
    WorkspaceChatLifecycleChanged { workspace: String },
    WorkspaceChatMessagesChanged { workspace: String, thread_id: i64 },
    TerminalChanged { workspace: String },
}

type RefreshHandler = Rc<dyn Fn()>;
type RefreshEventHandler = Rc<dyn Fn(&RefreshEvent)>;

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
            }
        }
    }

    pub fn refresh_workspace(&self, target: WorkspaceRefreshTarget) {
        match target {
            WorkspaceRefreshTarget::Shell => self.run(&self.workspace_shell),
            WorkspaceRefreshTarget::ChatSurface => {
                self.run_event(&self.workspace_chat_surface, &RefreshEvent::Manual)
            }
            WorkspaceRefreshTarget::ChatTabs => self.run_event_or_shell(
                &self.workspace_chat_tabs,
                &RefreshEvent::Manual,
                &self.workspace_shell,
            ),
            WorkspaceRefreshTarget::Runtime => self.run_event_or_shell(
                &self.workspace_runtime,
                &RefreshEvent::Manual,
                &self.workspace_shell,
            ),
            WorkspaceRefreshTarget::Review => self.run_event_or_shell(
                &self.workspace_review,
                &RefreshEvent::Manual,
                &self.workspace_shell,
            ),
        }
    }

    fn refresh_workspace_event(&self, target: WorkspaceRefreshTarget, event: &RefreshEvent) {
        match target {
            WorkspaceRefreshTarget::ChatSurface => {
                self.run_event(&self.workspace_chat_surface, event)
            }
            WorkspaceRefreshTarget::ChatTabs => {
                self.run_event_or_shell(&self.workspace_chat_tabs, event, &self.workspace_shell)
            }
            WorkspaceRefreshTarget::Runtime => {
                self.run_event_or_shell(&self.workspace_runtime, event, &self.workspace_shell)
            }
            WorkspaceRefreshTarget::Review => {
                self.run_event_or_shell(&self.workspace_review, event, &self.workspace_shell)
            }
            _ => self.refresh_workspace(target),
        }
    }

    pub fn refresh(&self, scope: RefreshScope) {
        match scope {
            RefreshScope::All => {
                self.run(&self.sidebar);
                self.run(&self.dashboard);
                self.run(&self.projects);
                self.run(&self.history);
                self.run(&self.workspace_shell);
            }
            RefreshScope::Sidebar => self.run(&self.sidebar),
            RefreshScope::Dashboard => self.run(&self.dashboard),
            RefreshScope::Projects => self.run(&self.projects),
            RefreshScope::History => self.run(&self.history),
            RefreshScope::Workspace => self.run(&self.workspace_shell),
        }
    }

    fn run(&self, slot: &Rc<RefCell<Option<RefreshHandler>>>) {
        let handler = slot.borrow().as_ref().cloned();
        if let Some(handler) = handler {
            handler();
        }
    }

    fn run_event_or_shell(
        &self,
        slot: &Rc<RefCell<Option<RefreshEventHandler>>>,
        event: &RefreshEvent,
        shell: &Rc<RefCell<Option<RefreshHandler>>>,
    ) {
        let handler = slot.borrow().as_ref().cloned();
        if let Some(handler) = handler {
            handler(event);
        } else {
            self.run(shell);
        }
    }

    fn run_event(&self, slot: &Rc<RefCell<Option<RefreshEventHandler>>>, event: &RefreshEvent) {
        let handler = slot.borrow().as_ref().cloned();
        if let Some(handler) = handler {
            handler(event);
        }
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
        }

        fn values(&self) -> (u32, u32, u32, u32, u32, u32, u32, u32, u32) {
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

        assert_eq!(counts.values(), (1, 1, 0, 1, 0, 0, 0, 1, 0));
    }

    #[test]
    fn chat_message_refresh_event_only_refreshes_chat_surface() {
        let hub = RefreshHub::default();
        let counts = RefreshCounts::default();
        counts.install(&hub);

        hub.refresh_event(RefreshEvent::WorkspaceChatMessagesChanged {
            workspace: "demo".to_owned(),
            thread_id: 7,
        });

        assert_eq!(counts.values(), (0, 0, 0, 0, 0, 1, 0, 0, 0));
    }

    #[test]
    fn chat_lifecycle_refresh_event_updates_nav_summaries_and_tabs() {
        let hub = RefreshHub::default();
        let counts = RefreshCounts::default();
        counts.install(&hub);

        hub.refresh_event(RefreshEvent::WorkspaceChatLifecycleChanged {
            workspace: "demo".to_owned(),
        });

        assert_eq!(counts.values(), (1, 1, 0, 1, 0, 1, 1, 0, 0));
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

        assert_eq!(counts.values(), (0, 1, 0, 1, 0, 0, 0, 0, 1));
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

        assert_eq!(counts.values(), (1, 1, 1, 0, 0, 0, 0, 0, 0));
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
