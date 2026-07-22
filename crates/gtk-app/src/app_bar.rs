use crate::buttons::icon_button;
use crate::state::{AppPage, AppState, AppStateSnapshot, AppStateSubscription};
use crate::workspace_command_center::{
    workspace_pull_request_status_summary, PullRequestStatusSummary,
};
use archductor_core::repository::RepositoryStore;
use archductor_core::workspace::WorkspaceStore;
use gtk::prelude::*;
use gtk::{Align, Box as GBox, HeaderBar, Label, Orientation, Stack, Widget};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

pub(crate) const APP_BAR_HEIGHT: i32 = 56;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppBarContext {
    pub(crate) stack_key: &'static str,
    pub(crate) workspace_name: Option<String>,
    pub(crate) repository_name: Option<String>,
    pub(crate) branch_name: Option<String>,
    pub(crate) pr_label: Option<String>,
    pub(crate) pr_css_class: Option<&'static str>,
}

impl AppBarContext {
    pub(crate) fn for_page(page: &AppPage, workspace: Option<Self>) -> Self {
        match page {
            AppPage::Workspace | AppPage::Review => {
                workspace.unwrap_or_else(|| Self::page("dashboard"))
            }
            AppPage::Projects => Self::page("projects"),
            AppPage::Settings => Self::page("settings"),
            AppPage::History => Self::page("history"),
            AppPage::Dashboard => Self::page("dashboard"),
        }
    }

    fn page(stack_key: &'static str) -> Self {
        Self {
            stack_key,
            workspace_name: None,
            repository_name: None,
            branch_name: None,
            pr_label: None,
            pr_css_class: None,
        }
    }

    pub(crate) fn workspace(
        name: &str,
        repository: &str,
        branch: &str,
        pr: Option<(&str, &'static str)>,
    ) -> Self {
        Self {
            stack_key: "workspace",
            workspace_name: Some(name.to_owned()),
            repository_name: Some(repository.to_owned()),
            branch_name: Some(branch.to_owned()),
            pr_label: pr.map(|(label, _)| label.to_owned()),
            pr_css_class: pr.map(|(_, css)| css),
        }
    }
}

const PR_STATUS_CLASSES: [&str; 5] = [
    "ws-pr-status-muted",
    "ws-pr-status-pending",
    "ws-pr-status-ready",
    "ws-pr-status-failed",
    "ws-pr-status-merged",
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrPresentation<'a> {
    label: &'a str,
    css_class: Option<&'static str>,
    visible: bool,
}

impl<'a> PrPresentation<'a> {
    fn from_context(context: &'a AppBarContext) -> Self {
        Self {
            label: context.pr_label.as_deref().unwrap_or_default(),
            css_class: context.pr_css_class,
            visible: context.pr_label.is_some(),
        }
    }
}

pub(crate) struct AppBar {
    header_bar: HeaderBar,
    stack: Stack,
    back_button: gtk::Button,
    forward_button: gtk::Button,
    workspace_name: Label,
    repository_name: Label,
    branch_name: Label,
    pr_label: Label,
    database_path: PathBuf,
    state: AppState,
    _subscription: AppStateSubscription,
}

impl AppBar {
    pub(crate) fn new(state: &AppState, database_path: PathBuf) -> Self {
        let header_bar = HeaderBar::builder().show_title_buttons(true).build();
        header_bar.add_css_class("app-bar");
        header_bar.set_height_request(APP_BAR_HEIGHT);

        let back_button = icon_button("go-previous-symbolic", "Back");
        let forward_button = icon_button("go-next-symbolic", "Forward");
        let navigation = GBox::new(Orientation::Horizontal, 4);
        navigation.append(&back_button);
        navigation.append(&forward_button);

        let stack = Stack::new();
        stack.set_hexpand(true);
        stack.set_hhomogeneous(false);
        for (key, title) in [
            ("dashboard", "Dashboard"),
            ("projects", "Projects"),
            ("settings", "Settings"),
            ("history", "History"),
        ] {
            stack.add_named(&page_title(title), Some(key));
        }

        let workspace_name = compact_label("app-bar-title");
        let repository_name = compact_label("app-bar-subtitle");
        let branch_name = compact_label("app-bar-subtitle");
        let pr_label = compact_label("app-bar-pr-status");
        let workspace_context = GBox::new(Orientation::Horizontal, 8);
        workspace_context.add_css_class("app-bar-context");
        workspace_context.append(&workspace_name);
        workspace_context.append(&repository_name);
        workspace_context.append(&branch_name);
        workspace_context.append(&pr_label);
        stack.add_named(&workspace_context, Some("workspace"));

        let content = GBox::new(Orientation::Horizontal, 10);
        content.add_css_class("app-bar-context");
        content.set_hexpand(true);
        content.append(&navigation);
        content.append(&stack);
        header_bar.set_title_widget(Some(&content));

        {
            let state = state.clone();
            back_button.connect_clicked(move |_| {
                state.navigate_back();
            });
        }
        {
            let state = state.clone();
            forward_button.connect_clicked(move |_| {
                state.navigate_forward();
            });
        }

        let widgets = Rc::new(RefCell::new(None::<AppBarRefreshWidgets>));
        let subscription = {
            let widgets = Rc::clone(&widgets);
            let state = state.clone();
            state.clone().subscribe(move |_event, snapshot| {
                if let Some(widgets) = widgets.borrow().as_ref() {
                    widgets.refresh(
                        snapshot,
                        state.can_navigate_back(),
                        state.can_navigate_forward(),
                    );
                }
            })
        };

        let app_bar = Self {
            header_bar,
            stack,
            back_button,
            forward_button,
            workspace_name,
            repository_name,
            branch_name,
            pr_label,
            database_path,
            state: state.clone(),
            _subscription: subscription,
        };
        *widgets.borrow_mut() = Some(app_bar.refresh_widgets());
        app_bar.refresh(&state.snapshot());
        app_bar
    }

    pub(crate) fn widget(&self) -> HeaderBar {
        self.header_bar.clone()
    }

    pub(crate) fn content_widget(&self) -> Widget {
        self.header_bar.clone().upcast()
    }

    pub(crate) fn set_page_header(&self, key: &str, widget: &Widget) {
        if let Some(existing) = self.stack.child_by_name(key) {
            self.stack.remove(&existing);
        }
        self.stack.add_named(widget, Some(key));
    }

    pub(crate) fn refresh(&self, snapshot: &AppStateSnapshot) {
        let workspace = self.workspace_context(snapshot);
        self.apply_context(AppBarContext::for_page(&snapshot.active_page, workspace));
        self.back_button
            .set_sensitive(self.state.can_navigate_back());
        self.forward_button
            .set_sensitive(self.state.can_navigate_forward());
    }

    pub(crate) fn refresh_workspace(&self) {
        self.refresh(&self.state.snapshot());
    }

    fn workspace_context(&self, snapshot: &AppStateSnapshot) -> Option<AppBarContext> {
        let name = snapshot.selected_workspace.as_deref()?;
        let store = WorkspaceStore::open_app(self.database_path.clone()).ok()?;
        let workspace = store.get_workspace_record_by_name(name).ok()?;
        let repository = RepositoryStore::open(&self.database_path)
            .ok()?
            .list()
            .ok()?
            .into_iter()
            .find(|repository| repository.id == workspace.repository_id)?;
        let pr = store.pull_request(name).ok().flatten().map(|pr| {
            let status: PullRequestStatusSummary =
                workspace_pull_request_status_summary(&store, name, &pr);
            (
                format!("PR #{} {}", pr.number, status.label),
                status.css_class,
            )
        });
        AppBarContext::workspace(
            &workspace.name,
            &repository.name,
            &workspace.branch,
            pr.as_ref().map(|(label, css)| (label.as_str(), *css)),
        )
        .into()
    }

    fn apply_context(&self, context: AppBarContext) {
        apply_context_widgets(
            &self.stack,
            &self.workspace_name,
            &self.repository_name,
            &self.branch_name,
            &self.pr_label,
            &context,
        );
    }

    fn refresh_widgets(&self) -> AppBarRefreshWidgets {
        AppBarRefreshWidgets {
            stack: self.stack.clone(),
            back_button: self.back_button.clone(),
            forward_button: self.forward_button.clone(),
            workspace_name: self.workspace_name.clone(),
            repository_name: self.repository_name.clone(),
            branch_name: self.branch_name.clone(),
            pr_label: self.pr_label.clone(),
            database_path: self.database_path.clone(),
        }
    }
}

struct AppBarRefreshWidgets {
    stack: Stack,
    back_button: gtk::Button,
    forward_button: gtk::Button,
    workspace_name: Label,
    repository_name: Label,
    branch_name: Label,
    pr_label: Label,
    database_path: PathBuf,
}

impl AppBarRefreshWidgets {
    fn refresh(&self, snapshot: &AppStateSnapshot, can_back: bool, can_forward: bool) {
        let workspace = workspace_context(&self.database_path, snapshot);
        let context = AppBarContext::for_page(&snapshot.active_page, workspace);
        apply_context_widgets(
            &self.stack,
            &self.workspace_name,
            &self.repository_name,
            &self.branch_name,
            &self.pr_label,
            &context,
        );
        self.back_button.set_sensitive(can_back);
        self.forward_button.set_sensitive(can_forward);
    }
}

fn apply_context_widgets(
    stack: &Stack,
    workspace_name: &Label,
    repository_name: &Label,
    branch_name: &Label,
    pr_label: &Label,
    context: &AppBarContext,
) {
    stack.set_visible_child_name(context.stack_key);
    workspace_name.set_label(context.workspace_name.as_deref().unwrap_or_default());
    repository_name.set_label(context.repository_name.as_deref().unwrap_or_default());
    branch_name.set_label(context.branch_name.as_deref().unwrap_or_default());

    let pr = PrPresentation::from_context(context);
    pr_label.set_label(pr.label);
    for class in PR_STATUS_CLASSES {
        pr_label.remove_css_class(class);
    }
    if let Some(class) = pr.css_class {
        pr_label.add_css_class(class);
    }
    pr_label.set_visible(pr.visible);
}

fn workspace_context(
    database_path: &PathBuf,
    snapshot: &AppStateSnapshot,
) -> Option<AppBarContext> {
    let name = snapshot.selected_workspace.as_deref()?;
    let store = WorkspaceStore::open_app(database_path.clone()).ok()?;
    let workspace = store.get_workspace_record_by_name(name).ok()?;
    let repository = RepositoryStore::open(database_path)
        .ok()?
        .list()
        .ok()?
        .into_iter()
        .find(|repository| repository.id == workspace.repository_id)?;
    let pr = store.pull_request(name).ok().flatten().map(|pr| {
        let status = workspace_pull_request_status_summary(&store, name, &pr);
        (
            format!("PR #{} {}", pr.number, status.label),
            status.css_class,
        )
    });
    Some(AppBarContext::workspace(
        &workspace.name,
        &repository.name,
        &workspace.branch,
        pr.as_ref().map(|(label, css)| (label.as_str(), *css)),
    ))
}

fn page_title(title: &str) -> GBox {
    let context = GBox::new(Orientation::Horizontal, 0);
    context.add_css_class("app-bar-context");
    let label = compact_label("app-bar-title");
    label.set_label(title);
    context.append(&label);
    context
}

fn compact_label(css_class: &str) -> Label {
    let label = Label::new(None);
    label.add_css_class(css_class);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_single_line_mode(true);
    label.set_halign(Align::Start);
    label
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppPage;

    #[test]
    fn non_workspace_pages_project_their_header_stack_keys() {
        assert_eq!(
            AppBarContext::for_page(&AppPage::Dashboard, None).stack_key,
            "dashboard"
        );
        assert_eq!(
            AppBarContext::for_page(&AppPage::Projects, None).stack_key,
            "projects"
        );
        assert_eq!(
            AppBarContext::for_page(&AppPage::Settings, None).stack_key,
            "settings"
        );
        assert_eq!(
            AppBarContext::for_page(&AppPage::History, None).stack_key,
            "history"
        );
    }

    #[test]
    fn workspace_projection_keeps_name_repository_branch_and_pr_compact() {
        let context = AppBarContext::workspace(
            "equal-height",
            "archductor",
            "feature/equal-height",
            Some(("PR #42 ready", "ws-pr-status-ready")),
        );
        assert_eq!(context.stack_key, "workspace");
        assert_eq!(context.workspace_name.as_deref(), Some("equal-height"));
        assert_eq!(context.repository_name.as_deref(), Some("archductor"));
        assert_eq!(context.branch_name.as_deref(), Some("feature/equal-height"));
        assert_eq!(context.pr_label.as_deref(), Some("PR #42 ready"));
    }

    #[test]
    fn pr_presentation_replaces_status_class_and_clears_without_pr() {
        let ready_context = AppBarContext::workspace(
            "equal-height",
            "archductor",
            "feature/equal-height",
            Some(("PR #42 ready", "ws-pr-status-ready")),
        );
        let ready = PrPresentation::from_context(&ready_context);
        assert_eq!(ready.css_class, Some("ws-pr-status-ready"));
        assert!(ready.visible);

        let failed_context = AppBarContext::workspace(
            "equal-height",
            "archductor",
            "feature/equal-height",
            Some(("PR #42 failed", "ws-pr-status-failed")),
        );
        let failed = PrPresentation::from_context(&failed_context);
        assert_eq!(failed.css_class, Some("ws-pr-status-failed"));
        assert_ne!(failed.css_class, ready.css_class);

        let no_pr_context =
            AppBarContext::workspace("equal-height", "archductor", "feature/equal-height", None);
        let no_pr = PrPresentation::from_context(&no_pr_context);
        assert_eq!(no_pr.css_class, None);
        assert!(!no_pr.visible);
        assert_eq!(no_pr.label, "");
    }

    #[test]
    fn build_ui_retains_app_bar_for_window_lifetime() {
        let source = include_str!("main.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("production source exists");

        assert!(production.contains("Rc::new(AppBar::new("));
        assert!(production.contains("window.connect_destroy(move |_|"));
        assert!(production.contains("app_bar_lifetime"));
    }
}
