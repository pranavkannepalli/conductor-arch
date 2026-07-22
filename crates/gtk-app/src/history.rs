use archductor_core::paths::AppPaths;
use gtk::prelude::*;
use gtk::{
    Box as GBox, Button, Label, ListBox, Orientation, PolicyType, ScrolledWindow, Separator, Stack,
    TextView,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;
use tracing::error;

use crate::detail_row;
use crate::history_data::{
    history_recent_sessions, history_recent_workspaces, history_session_messages, HistoryTab,
    WorkspaceHistoryEntry, WorkspaceHistoryFilter,
};
pub(crate) use crate::history_data::{sessions_for_workspace_path, ChatSummary};
use crate::motion::{append_revealed_to_list, clear_box, clear_list};
use crate::tabs::{set_standard_tab_active, standard_tab, standard_tab_strip};
use crate::toast::ToastManager;

const HISTORY_SUBTITLE: &str = "Review past workspaces and saved agent chats.";

pub(crate) fn build_history_page(
    paths: &AppPaths,
    open_workspace: Rc<dyn Fn(String)>,
    toast_manager: ToastManager,
) -> (GBox, impl Fn() + Clone + 'static) {
    let root = GBox::new(Orientation::Vertical, 0);
    root.add_css_class("dashboard");
    root.add_css_class("page-shell");

    let header = GBox::new(Orientation::Vertical, 8);
    header.add_css_class("dashboard-header");
    header.add_css_class("page-header");
    let title = Label::new(Some("History"));
    title.add_css_class("dashboard-title");
    title.set_xalign(0.0);
    let subtitle = Label::new(Some(HISTORY_SUBTITLE));
    subtitle.add_css_class("card-meta");
    subtitle.set_xalign(0.0);
    header.append(&title);
    header.append(&subtitle);

    let (tabs_scroll, tabs) = standard_tab_strip();
    tabs_scroll.add_css_class("history-tabs");
    let workspaces_tab = standard_tab("Workspaces");
    let chats_tab = standard_tab("Chats");
    set_standard_tab_active(&workspaces_tab, true);
    tabs.append(&workspaces_tab);
    tabs.append(&chats_tab);
    header.append(&tabs_scroll);
    root.append(&header);

    let (workspaces_view, refresh_workspaces) =
        build_workspace_history_view(paths.database_path.clone(), open_workspace, toast_manager);
    let (chats_view, refresh_chats) = build_chat_history_view(paths.database_path.clone());
    let stack = Stack::new();
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    stack.add_named(&workspaces_view, Some(HistoryTab::Workspaces.stack_name()));
    stack.add_named(&chats_view, Some(HistoryTab::Chats.stack_name()));
    stack.set_visible_child_name(HistoryTab::default().stack_name());
    root.append(&stack);

    {
        let stack = stack.clone();
        let workspaces_tab = workspaces_tab.clone();
        let chats_tab = chats_tab.clone();
        workspaces_tab.clone().connect_clicked(move |_| {
            stack.set_visible_child_name(HistoryTab::Workspaces.stack_name());
            set_standard_tab_active(&workspaces_tab, true);
            set_standard_tab_active(&chats_tab, false);
        });
    }
    {
        let stack = stack.clone();
        let workspaces_tab = workspaces_tab.clone();
        let chats_tab = chats_tab.clone();
        chats_tab.clone().connect_clicked(move |_| {
            stack.set_visible_child_name(HistoryTab::Chats.stack_name());
            set_standard_tab_active(&workspaces_tab, false);
            set_standard_tab_active(&chats_tab, true);
        });
    }

    let refresh = move || {
        refresh_workspaces();
        refresh_chats();
    };
    refresh();
    (root, refresh)
}

fn build_workspace_history_view(
    database_path: PathBuf,
    open_workspace: Rc<dyn Fn(String)>,
    toast_manager: ToastManager,
) -> (GBox, Rc<dyn Fn()>) {
    let view = GBox::new(Orientation::Vertical, 0);
    view.add_css_class("history-page-body");
    let (filter_scroll, filter_tabs) = standard_tab_strip();
    filter_scroll.add_css_class("history-filter-tabs");
    view.append(&filter_scroll);

    let split = history_split_pane();
    let list = history_list();
    let list_scroll = history_list_scroll(&list);
    let details = GBox::new(Orientation::Vertical, 12);
    details.add_css_class("history-detail");
    let detail_scroll = ScrolledWindow::new();
    detail_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    detail_scroll.set_hexpand(true);
    detail_scroll.set_child(Some(&details));
    split.append(&list_scroll);
    split.append(&Separator::new(Orientation::Vertical));
    split.append(&detail_scroll);
    view.append(&split);

    let rows = Rc::new(RefCell::new(HashMap::<i32, WorkspaceHistoryEntry>::new()));
    let workspaces = Rc::new(RefCell::new(Vec::<WorkspaceHistoryEntry>::new()));
    let filter = Rc::new(RefCell::new(WorkspaceHistoryFilter::All));
    let generation = Rc::new(RefCell::new(0u64));

    {
        let rows = rows.clone();
        let details = details.clone();
        let open_workspace = open_workspace.clone();
        list.connect_row_selected(move |_, row| {
            guarded_gtk_callback((), || {
                let Some(workspace) = row.and_then(|row| rows.borrow().get(&row.index()).cloned())
                else {
                    return;
                };
                render_workspace_detail(&details, &workspace, open_workspace.clone());
            });
        });
    }

    render_workspace_filter_tabs(
        &filter_tabs,
        &list,
        &details,
        rows.clone(),
        workspaces.clone(),
        filter.clone(),
    );
    render_workspace_placeholder(&details);

    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        clear_list(&list);
        rows.borrow_mut().clear();
        workspaces.borrow_mut().clear();
        render_workspace_placeholder(&details);
        append_history_status(&list, "Loading workspace history...");

        *generation.borrow_mut() += 1;
        let request_generation = *generation.borrow();
        let (tx, rx) = mpsc::channel();
        let db_path = database_path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(history_recent_workspaces(&db_path));
        });

        let list = list.clone();
        let details = details.clone();
        let rows = rows.clone();
        let workspaces = workspaces.clone();
        let filter = filter.clone();
        let filter_tabs = filter_tabs.clone();
        let generation = generation.clone();
        let toast_manager = toast_manager.clone();
        // PER-190: This page-owned poll stops after the workspace worker replies
        // or disconnects; remove it when a GLib main-context bridge replaces mpsc.
        glib::timeout_add_local(Duration::from_millis(100), move || match rx.try_recv() {
            Ok(Ok(loaded)) => {
                if *generation.borrow() == request_generation {
                    *workspaces.borrow_mut() = loaded;
                    render_workspace_filter_tabs(
                        &filter_tabs,
                        &list,
                        &details,
                        rows.clone(),
                        workspaces.clone(),
                        filter.clone(),
                    );
                    render_workspace_list(
                        &list,
                        &details,
                        &rows,
                        &workspaces.borrow(),
                        *filter.borrow(),
                    );
                }
                glib::ControlFlow::Break
            }
            Ok(Err(err)) => {
                if *generation.borrow() == request_generation {
                    clear_list(&list);
                    let message = format!("Could not load workspace history: {err:#}");
                    toast_manager.error(message.clone());
                    append_history_status(&list, &message);
                }
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => {
                if *generation.borrow() == request_generation {
                    clear_list(&list);
                    append_history_status(&list, "Could not load workspace history.");
                }
                glib::ControlFlow::Break
            }
        });
    });
    (view, refresh)
}

fn render_workspace_filter_tabs(
    tabs: &GBox,
    list: &ListBox,
    details: &GBox,
    rows: Rc<RefCell<HashMap<i32, WorkspaceHistoryEntry>>>,
    workspaces: Rc<RefCell<Vec<WorkspaceHistoryEntry>>>,
    selected: Rc<RefCell<WorkspaceHistoryFilter>>,
) {
    clear_box(tabs);
    for filter in WorkspaceHistoryFilter::ALL {
        let tab = standard_tab(filter.label());
        set_standard_tab_active(&tab, *selected.borrow() == filter);
        let tabs_for_click = tabs.clone();
        let list = list.clone();
        let details = details.clone();
        let rows = rows.clone();
        let workspaces = workspaces.clone();
        let selected = selected.clone();
        tab.connect_clicked(move |_| {
            *selected.borrow_mut() = filter;
            render_workspace_filter_tabs(
                &tabs_for_click,
                &list,
                &details,
                rows.clone(),
                workspaces.clone(),
                selected.clone(),
            );
            render_workspace_list(&list, &details, &rows, &workspaces.borrow(), filter);
        });
        tabs.append(&tab);
    }
}

fn render_workspace_list(
    list: &ListBox,
    details: &GBox,
    rows: &Rc<RefCell<HashMap<i32, WorkspaceHistoryEntry>>>,
    workspaces: &[WorkspaceHistoryEntry],
    filter: WorkspaceHistoryFilter,
) {
    clear_list(list);
    rows.borrow_mut().clear();
    render_workspace_placeholder(details);
    let mut visible_index = 0i32;
    for workspace in workspaces
        .iter()
        .filter(|workspace| filter.matches(&workspace.state))
    {
        append_revealed_to_list(list, &workspace_history_row(workspace));
        rows.borrow_mut().insert(visible_index, workspace.clone());
        visible_index += 1;
    }
    if visible_index == 0 {
        append_history_status(list, "No workspace history in this scope.");
    }
}

fn workspace_history_row(workspace: &WorkspaceHistoryEntry) -> GBox {
    let row = GBox::new(Orientation::Vertical, 3);
    row.add_css_class("history-row");
    let title = Label::new(Some(&workspace.name));
    title.add_css_class("workspace-name");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let meta = Label::new(Some(&format!(
        "{} · {} · {}",
        workspace.repository_name, workspace.branch, workspace.state
    )));
    meta.add_css_class("workspace-meta");
    meta.set_xalign(0.0);
    meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let status = Label::new(Some(&format!(
        "updated {} · +{} -{}",
        workspace.updated_at, workspace.diff_additions, workspace.diff_deletions
    )));
    status.add_css_class("card-meta");
    status.set_xalign(0.0);
    status.set_ellipsize(gtk::pango::EllipsizeMode::End);
    row.append(&title);
    row.append(&meta);
    row.append(&status);
    row
}

fn render_workspace_detail(
    details: &GBox,
    workspace: &WorkspaceHistoryEntry,
    open_workspace: Rc<dyn Fn(String)>,
) {
    clear_box(details);
    let title = Label::new(Some(&workspace.name));
    title.add_css_class("dashboard-title");
    title.set_xalign(0.0);
    details.append(&title);
    details.append(&detail_row(
        "State",
        &format!("{} · {}", workspace.state, workspace.status),
    ));
    details.append(&detail_row("Project", &workspace.repository_name));
    details.append(&detail_row(
        "Branch",
        &format!("{}\nBase: {}", workspace.branch, workspace.base_ref),
    ));
    details.append(&detail_row(
        "Changes",
        &format!(
            "+{} -{} · run {}",
            workspace.diff_additions,
            workspace.diff_deletions,
            if workspace.run_running {
                "running"
            } else {
                "idle"
            }
        ),
    ));
    details.append(&detail_row(
        "Todos",
        &format!("{} open", workspace.open_todos),
    ));
    details.append(&detail_row(
        "Sessions",
        &format!("{} active", workspace.active_sessions),
    ));
    details.append(&detail_row(
        "Pull Request",
        &workspace
            .pull_request
            .map(|number| format!("PR #{number}"))
            .unwrap_or_else(|| "No pull request".to_owned()),
    ));
    details.append(&detail_row(
        "Dates",
        &format!(
            "Created: {}\nUpdated: {}\nArchived: {}",
            workspace.created_at,
            workspace.updated_at,
            workspace.archived_at.as_deref().unwrap_or("Not archived")
        ),
    ));
    details.append(&detail_row("Path", &workspace.path));

    if history_workspace_is_openable(&workspace.status) {
        let open = Button::with_label("Open Workspace");
        open.add_css_class("suggested-action");
        open.set_halign(gtk::Align::Start);
        let workspace_name = workspace.name.clone();
        open.connect_clicked(move |_| open_workspace(workspace_name.clone()));
        details.append(&open);
    }
}

fn render_workspace_placeholder(details: &GBox) {
    clear_box(details);
    let message = Label::new(Some("Select a workspace to inspect its current state."));
    message.add_css_class("empty-label");
    message.set_xalign(0.0);
    details.append(&message);
}

fn history_workspace_is_openable(status: &str) -> bool {
    !status.eq_ignore_ascii_case("archived")
}

fn build_chat_history_view(database_path: PathBuf) -> (GBox, Rc<dyn Fn()>) {
    let view = GBox::new(Orientation::Vertical, 0);
    view.add_css_class("history-page-body");
    let split = history_split_pane();
    let list = history_list();
    let list_scroll = history_list_scroll(&list);
    let transcript = TextView::new();
    transcript.set_editable(false);
    transcript.set_cursor_visible(false);
    transcript.set_monospace(false);
    transcript.set_wrap_mode(gtk::WrapMode::WordChar);
    transcript.add_css_class("history-view");
    transcript.add_css_class("history-transcript");
    transcript
        .buffer()
        .set_text("Select a saved chat to read its transcript.");
    let transcript_scroll = ScrolledWindow::new();
    transcript_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    transcript_scroll.set_hexpand(true);
    transcript_scroll.set_child(Some(&transcript));
    split.append(&list_scroll);
    split.append(&Separator::new(Orientation::Vertical));
    split.append(&transcript_scroll);
    view.append(&split);

    let rows = Rc::new(RefCell::new(HashMap::<i32, ChatSummary>::new()));
    let list_generation = Rc::new(RefCell::new(0u64));
    let transcript_generation = Rc::new(RefCell::new(0u64));

    {
        let rows = rows.clone();
        let transcript = transcript.clone();
        let transcript_generation = transcript_generation.clone();
        let database_path = database_path.clone();
        list.connect_row_selected(move |_, row| {
            let Some(session) = row.and_then(|row| rows.borrow().get(&row.index()).cloned()) else {
                return;
            };
            transcript.buffer().set_text("Loading chat transcript...");
            *transcript_generation.borrow_mut() += 1;
            let request_generation = *transcript_generation.borrow();
            let (tx, rx) = mpsc::channel();
            let db_path = database_path.clone();
            std::thread::spawn(move || {
                let _ = tx.send(history_session_messages(&db_path, &session.id));
            });
            let transcript = transcript.clone();
            let transcript_generation = transcript_generation.clone();
            // PER-190: This page-owned poll stops after the transcript worker
            // replies or disconnects; remove it with a GLib main-context bridge.
            glib::timeout_add_local(Duration::from_millis(100), move || match rx.try_recv() {
                Ok(messages) => {
                    if *transcript_generation.borrow() == request_generation {
                        transcript.buffer().set_text(&messages);
                    }
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if *transcript_generation.borrow() == request_generation {
                        transcript
                            .buffer()
                            .set_text("Could not load chat transcript.");
                    }
                    glib::ControlFlow::Break
                }
            });
        });
    }

    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        clear_list(&list);
        rows.borrow_mut().clear();
        transcript
            .buffer()
            .set_text("Select a saved chat to read its transcript.");
        *transcript_generation.borrow_mut() += 1;
        append_history_status(&list, "Loading saved chats...");

        *list_generation.borrow_mut() += 1;
        let request_generation = *list_generation.borrow();
        let (tx, rx) = mpsc::channel();
        let db_path = database_path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(history_recent_sessions(&db_path));
        });
        let list = list.clone();
        let rows = rows.clone();
        let list_generation = list_generation.clone();
        // PER-190: This page-owned poll stops after the chat-list worker replies
        // or disconnects; remove it when a GLib main-context bridge replaces mpsc.
        glib::timeout_add_local(Duration::from_millis(100), move || match rx.try_recv() {
            Ok(Ok(sessions)) => {
                if *list_generation.borrow() == request_generation {
                    clear_list(&list);
                    for (index, session) in sessions.into_iter().enumerate() {
                        append_revealed_to_list(&list, &session_summary_row(&session));
                        rows.borrow_mut()
                            .insert(i32::try_from(index).unwrap_or(i32::MAX), session);
                    }
                    if rows.borrow().is_empty() {
                        append_history_status(&list, "No saved chats yet.");
                    }
                }
                glib::ControlFlow::Break
            }
            Ok(Err(err)) => {
                if *list_generation.borrow() == request_generation {
                    clear_list(&list);
                    append_history_status(&list, &format!("Could not load saved chats: {err:#}"));
                }
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => {
                if *list_generation.borrow() == request_generation {
                    clear_list(&list);
                    append_history_status(&list, "Could not load saved chats.");
                }
                glib::ControlFlow::Break
            }
        });
    });
    (view, refresh)
}

pub(crate) fn session_summary_row(session: &ChatSummary) -> GBox {
    let row = GBox::new(Orientation::Vertical, 3);
    row.add_css_class("history-row");
    let title = Label::new(Some(&session.title));
    title.add_css_class("workspace-name");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let meta = Label::new(Some(&format!(
        "{} · {} · {} · {} · {} messages",
        session.source.label(),
        session.repository_name,
        session.workspace_name,
        session.agent_type,
        session.message_count
    )));
    meta.add_css_class("workspace-meta");
    meta.set_xalign(0.0);
    meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let status = Label::new(Some(&format!(
        "{} · {}",
        session.status, session.updated_at
    )));
    status.add_css_class("card-meta");
    status.set_xalign(0.0);
    status.set_ellipsize(gtk::pango::EllipsizeMode::End);
    row.append(&title);
    row.append(&meta);
    row.append(&status);
    row
}

fn history_split_pane() -> GBox {
    let split = GBox::new(Orientation::Horizontal, 0);
    split.add_css_class("history-split-pane");
    split.set_vexpand(true);
    split
}

fn history_list() -> ListBox {
    let list = ListBox::new();
    list.add_css_class("workspace-list");
    list.add_css_class("shell-list");
    list.add_css_class("history-list");
    list
}

fn history_list_scroll(list: &ListBox) -> ScrolledWindow {
    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    scroll.set_width_request(380);
    scroll.set_child(Some(list));
    scroll
}

fn append_history_status(list: &ListBox, message: &str) {
    let label = Label::new(Some(message));
    label.add_css_class("empty-label");
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_margin_start(18);
    label.set_margin_end(18);
    label.set_margin_top(18);
    append_revealed_to_list(list, &label);
}

fn guarded_gtk_callback<T, F>(fallback: T, callback: F) -> T
where
    F: FnOnce() -> T,
{
    match catch_unwind(AssertUnwindSafe(callback)) {
        Ok(value) => value,
        Err(_) => {
            error!("recovered panic inside history GTK callback");
            fallback
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{history_workspace_is_openable, HISTORY_SUBTITLE};

    #[test]
    fn archived_workspace_details_do_not_offer_open_action() {
        assert!(history_workspace_is_openable("active"));
        assert!(!history_workspace_is_openable("archived"));
    }

    #[test]
    fn history_header_explains_both_scopes() {
        assert_eq!(
            HISTORY_SUBTITLE,
            "Review past workspaces and saved agent chats."
        );
    }
}
