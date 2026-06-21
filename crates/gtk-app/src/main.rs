#![allow(dead_code)]
#![allow(clippy::ptr_arg, clippy::too_many_arguments)]

mod command_palette;
mod dashboard;
mod history;
mod projects;
mod refresh;
mod session_surface;
mod sidebar;
mod state;
mod terminal;
mod workspace_command_center;

use adw::prelude::*;
use adw::{Application, ApplicationWindow, HeaderBar};
use command_palette::{
    filter_palette_commands, palette_commands, Keybindings, PaletteCommand, PaletteTarget,
    ShortcutAction,
};
use gtk::{
    Box as GBox, Button, CssProvider, Entry, Label, Orientation, ScrolledWindow, Stack,
    STYLE_PROVIDER_PRIORITY_APPLICATION,
};
use linux_conductor_core::paths::AppPaths;
use linux_conductor_core::workspace::{ProcessStatus, WorkspaceStore, WorkspaceViewDefaults};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use refresh::{RefreshHub, RefreshScope};
use state::{AppPage, AppState, WorkspaceTab};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Sender};

const APP_ID: &str = "io.github.pranavkannepalli.linux-conductor";

#[derive(Debug, Clone, PartialEq, Eq)]
struct LaunchTarget {
    page: AppPage,
    workspace: Option<String>,
    workspace_tab: WorkspaceTab,
    explicit_workspace_tab: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ViewPreferences {
    theme: Option<ViewTheme>,
    accent: Option<AccentColor>,
    density: Option<ViewDensity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ViewTheme {
    System,
    Dark,
    Light,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AccentColor {
    Blue,
    Green,
    Amber,
    Rose,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ViewDensity {
    Compact,
    Comfortable,
}

impl ViewPreferences {
    fn from_defaults(defaults: WorkspaceViewDefaults) -> Self {
        Self {
            theme: defaults.theme.as_deref().and_then(ViewTheme::from_config),
            accent: defaults
                .accent_color
                .as_deref()
                .and_then(AccentColor::from_config),
            density: defaults
                .density
                .as_deref()
                .and_then(ViewDensity::from_config),
        }
    }

    fn css_classes(&self) -> Vec<&'static str> {
        let mut classes = Vec::new();
        match self.theme {
            Some(ViewTheme::Light) => classes.push("lc-theme-light"),
            Some(ViewTheme::Dark) => classes.push("lc-theme-dark"),
            Some(ViewTheme::System) | None => {}
        }
        match self.accent {
            Some(AccentColor::Blue) => classes.push("lc-accent-blue"),
            Some(AccentColor::Green) => classes.push("lc-accent-green"),
            Some(AccentColor::Amber) => classes.push("lc-accent-amber"),
            Some(AccentColor::Rose) => classes.push("lc-accent-rose"),
            None => {}
        }
        match self.density {
            Some(ViewDensity::Compact) => classes.push("lc-density-compact"),
            Some(ViewDensity::Comfortable) => classes.push("lc-density-comfortable"),
            None => {}
        }
        classes
    }
}

impl ViewTheme {
    fn from_config(value: &str) -> Option<Self> {
        match normalize_launch_token(value).as_str() {
            "system" | "auto" => Some(Self::System),
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            _ => None,
        }
    }
}

impl AccentColor {
    fn from_config(value: &str) -> Option<Self> {
        match normalize_launch_token(value).as_str() {
            "blue" | "default" => Some(Self::Blue),
            "green" => Some(Self::Green),
            "amber" | "yellow" => Some(Self::Amber),
            "rose" | "red" | "pink" => Some(Self::Rose),
            _ => None,
        }
    }
}

impl ViewDensity {
    fn from_config(value: &str) -> Option<Self> {
        match normalize_launch_token(value).as_str() {
            "compact" | "dense" => Some(Self::Compact),
            "comfortable" | "cozy" => Some(Self::Comfortable),
            _ => None,
        }
    }
}

const VIEW_PREFERENCE_CLASSES: &[&str] = &[
    "lc-theme-light",
    "lc-theme-dark",
    "lc-accent-blue",
    "lc-accent-green",
    "lc-accent-amber",
    "lc-accent-rose",
    "lc-density-compact",
    "lc-density-comfortable",
];

impl Default for LaunchTarget {
    fn default() -> Self {
        Self {
            page: AppPage::Dashboard,
            workspace: None,
            workspace_tab: WorkspaceTab::Chats,
            explicit_workspace_tab: false,
        }
    }
}

fn main() {
    let launch_target = parse_launch_target(std::env::args()).unwrap_or_default();

    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(move |app| build_ui(app, launch_target.clone()));
    app.run();
}

fn parse_launch_target<I, S>(args: I) -> Result<LaunchTarget, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut target = LaunchTarget::default();
    let args = args
        .into_iter()
        .map(|arg| arg.as_ref().to_owned())
        .collect::<Vec<_>>();
    let mut index = 1usize;
    while index < args.len() {
        match args[index].as_str() {
            "--workspace" => {
                index += 1;
                target.workspace = args.get(index).cloned().filter(|value| !value.is_empty());
                target.page = AppPage::Workspace;
            }
            "--tab" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--tab requires a value".to_owned())?;
                target.workspace_tab = parse_workspace_tab(value)?;
                target.explicit_workspace_tab = true;
                target.page = AppPage::Workspace;
            }
            "--page" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--page requires a value".to_owned())?;
                target.page = parse_app_page(value)?;
            }
            value if value.starts_with("linux-conductor://") => {
                target = parse_deep_link(value)?;
            }
            _ => {}
        }
        index += 1;
    }
    Ok(target)
}

fn parse_deep_link(value: &str) -> Result<LaunchTarget, String> {
    let rest = value
        .strip_prefix("linux-conductor://")
        .ok_or_else(|| "deep link must start with linux-conductor://".to_owned())?;
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut target = LaunchTarget::default();
    let parts = path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();

    match parts.as_slice() {
        ["workspace", name] | ["workspaces", name] => {
            target.page = AppPage::Workspace;
            target.workspace = Some(percent_decode(name));
        }
        ["dashboard"] => target.page = AppPage::Dashboard,
        ["projects"] | ["repositories"] => target.page = AppPage::Projects,
        ["history"] => target.page = AppPage::History,
        ["workspace"] | ["workspaces"] => target.page = AppPage::Workspace,
        [] => {}
        [page] => target.page = parse_app_page(page)?,
        _ => return Err(format!("unsupported deep link path: {path}")),
    }

    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "workspace" => {
                target.workspace = Some(percent_decode(raw_value));
                target.page = AppPage::Workspace;
            }
            "tab" => {
                target.workspace_tab = parse_workspace_tab(&percent_decode(raw_value))?;
                target.explicit_workspace_tab = true;
                target.page = AppPage::Workspace;
            }
            _ => {}
        }
    }
    Ok(target)
}

fn parse_app_page(value: &str) -> Result<AppPage, String> {
    match normalize_launch_token(value).as_str() {
        "dashboard" | "home" => Ok(AppPage::Dashboard),
        "projects" | "repositories" | "repos" => Ok(AppPage::Projects),
        "history" | "archive" => Ok(AppPage::History),
        "workspace" | "workspaces" => Ok(AppPage::Workspace),
        other => Err(format!("unknown page: {other}")),
    }
}

fn parse_workspace_tab(value: &str) -> Result<WorkspaceTab, String> {
    WorkspaceTab::from_config(value).ok_or_else(|| format!("unknown workspace tab: {value}"))
}

fn normalize_launch_token(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = &value[index + 1..index + 3];
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(if bytes[index] == b'+' {
            b' '
        } else {
            bytes[index]
        });
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn resolve_view_preferences(db_path: PathBuf, workspace: Option<&str>) -> ViewPreferences {
    workspace
        .and_then(|name| {
            WorkspaceStore::open(db_path)
                .and_then(|store| store.workspace_view_defaults(name))
                .ok()
        })
        .map(ViewPreferences::from_defaults)
        .unwrap_or_default()
}

fn resolve_keybindings(db_path: PathBuf, workspace: Option<&str>) -> Keybindings {
    workspace
        .and_then(|name| {
            WorkspaceStore::open(db_path)
                .and_then(|store| store.workspace_view_defaults(name))
                .ok()
        })
        .and_then(|defaults| defaults.keybindings)
        .as_deref()
        .map(|value| Keybindings::from_config(Some(value)))
        .unwrap_or_default()
}

fn apply_view_preferences(window: &ApplicationWindow, preferences: &ViewPreferences) {
    for class_name in VIEW_PREFERENCE_CLASSES {
        window.remove_css_class(class_name);
    }
    for class_name in preferences.css_classes() {
        window.add_css_class(class_name);
    }
}

fn build_ui(app: &Application, launch_target: LaunchTarget) {
    let paths = AppPaths::from_env();
    let initial_tab = if launch_target.explicit_workspace_tab {
        launch_target.workspace_tab.clone()
    } else {
        launch_target
            .workspace
            .as_deref()
            .and_then(|workspace| {
                WorkspaceStore::open(paths.database_path.clone())
                    .and_then(|store| store.workspace_view_defaults(workspace))
                    .ok()
            })
            .and_then(|defaults| defaults.default_visible_tab)
            .and_then(|tab| WorkspaceTab::from_config(&tab))
            .unwrap_or_else(|| launch_target.workspace_tab.clone())
    };
    let app_state = AppState::new(
        paths.clone(),
        launch_target.workspace.clone(),
        initial_tab,
        launch_target.page.clone(),
    );
    let refresh_hub = RefreshHub::default();

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Linux Conductor")
        .default_width(1280)
        .default_height(800)
        .build();
    let initial_view_preferences = resolve_view_preferences(
        paths.database_path.clone(),
        launch_target.workspace.as_deref(),
    );
    let current_keybindings = Rc::new(RefCell::new(resolve_keybindings(
        paths.database_path.clone(),
        launch_target.workspace.as_deref(),
    )));

    let css = CssProvider::new();
    css.load_from_data(APP_CSS);
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().unwrap(),
        &css,
        STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    apply_view_preferences(&window, &initial_view_preferences);

    let split = adw::OverlaySplitView::new();
    split.set_min_sidebar_width(220.0);
    split.set_max_sidebar_width(280.0);
    split.set_show_sidebar(true);

    let toast_overlay = adw::ToastOverlay::new();
    let (dashboard, refresh_dashboard) = dashboard::build_dashboard_panel(&app_state.paths);
    dashboard.set_hexpand(true);
    dashboard.set_vexpand(true);

    let (workspace_detail, refresh_workspace_detail) =
        workspace_command_center::build_workspace_command_center(
            &app_state,
            refresh_hub.clone(),
            toast_overlay.clone(),
        );
    let (projects_page, refresh_projects) = projects::build_projects_page(
        &app_state.paths,
        refresh_dashboard.clone(),
        refresh_workspace_detail.clone(),
    );
    let (history_page, refresh_history) =
        history::build_history_page(app_state.workspace_database_path());

    let main_stack = Stack::new();
    main_stack.set_hexpand(true);
    main_stack.set_vexpand(true);
    main_stack.add_named(&dashboard, Some("dashboard"));
    main_stack.add_named(&projects_page, Some("projects"));
    main_stack.add_named(&history_page, Some("history"));
    main_stack.add_named(&workspace_detail, Some("workspace"));
    main_stack.set_visible_child_name(match app_state.snapshot().active_page {
        AppPage::Workspace => "workspace",
        _ => "dashboard",
    });

    let refresh_view_preferences: Rc<dyn Fn()> = {
        let state_for_view = app_state.clone();
        let window_for_view = window.clone();
        let db_path_for_view = app_state.workspace_database_path();
        let keybindings_for_view = Rc::clone(&current_keybindings);
        Rc::new(move || {
            let workspace = state_for_view.selected_workspace();
            let preferences =
                resolve_view_preferences(db_path_for_view.clone(), workspace.as_deref());
            apply_view_preferences(&window_for_view, &preferences);
            *keybindings_for_view.borrow_mut() =
                resolve_keybindings(db_path_for_view.clone(), workspace.as_deref());
        })
    };

    let (sidebar, refresh_sidebar) = sidebar::build_app_sidebar(
        &app_state,
        refresh_hub.clone(),
        main_stack.clone(),
        refresh_workspace_detail.clone(),
        refresh_view_preferences.clone(),
    );

    refresh_hub.set_dashboard(refresh_dashboard.clone());
    refresh_hub.set_workspace(refresh_workspace_detail.clone());
    refresh_hub.set_projects(refresh_projects.clone());
    refresh_hub.set_history(refresh_history.clone());
    refresh_hub.set_sidebar(refresh_sidebar.clone());

    split.set_sidebar(Some(&sidebar));
    toast_overlay.set_child(Some(&main_stack));
    split.set_content(Some(&toast_overlay));

    // Header bar
    let header = HeaderBar::new();
    header.set_show_end_title_buttons(true);
    let toggle_btn = Button::from_icon_name("sidebar-show-symbolic");
    toggle_btn.set_tooltip_text(Some("Toggle sidebar"));
    let split_clone = split.clone();
    toggle_btn.connect_clicked(move |_| {
        split_clone.set_show_sidebar(!split_clone.shows_sidebar());
    });
    // Refresh button
    let refresh_btn = Button::from_icon_name("view-refresh-symbolic");
    refresh_btn.set_tooltip_text(Some("Refresh workspace state"));
    let hub_refresh = refresh_hub.clone();
    refresh_btn.connect_clicked(move |_| {
        hub_refresh.refresh(RefreshScope::All);
    });
    let palette_btn = Button::from_icon_name("edit-find-symbolic");
    palette_btn.set_tooltip_text(Some("Command palette"));

    let open_palette: Rc<dyn Fn()> = {
        let window_for_palette = window.clone();
        let state_for_palette = app_state.clone();
        let main_stack_for_palette = main_stack.clone();
        let split_for_palette = split.clone();
        let hub_for_palette = refresh_hub.clone();
        let refresh_workspace_for_palette = refresh_workspace_detail.clone();
        let keybindings_for_palette = Rc::clone(&current_keybindings);
        Rc::new(move || {
            let keybindings = keybindings_for_palette.borrow().clone();
            let custom_commands = state_for_palette
                .selected_workspace()
                .and_then(|workspace| {
                    WorkspaceStore::open(state_for_palette.workspace_database_path())
                        .and_then(|store| store.workspace_view_defaults(&workspace))
                        .ok()
                })
                .map(|defaults| defaults.command_palette_presets)
                .unwrap_or_default();
            let commands = palette_commands(
                state_for_palette.snapshot().selected_workspace.is_some(),
                &keybindings,
                &custom_commands,
            );
            let state_for_action = state_for_palette.clone();
            let stack_for_action = main_stack_for_palette.clone();
            let split_for_action = split_for_palette.clone();
            let hub_for_action = hub_for_palette.clone();
            let refresh_workspace_for_action = refresh_workspace_for_palette.clone();
            show_command_palette(
                &window_for_palette,
                commands,
                Rc::new(move |target| {
                    apply_palette_target(
                        target,
                        &state_for_action,
                        &stack_for_action,
                        &split_for_action,
                        &hub_for_action,
                        &refresh_workspace_for_action,
                    );
                }),
            );
        })
    };
    let open_palette_button = open_palette.clone();
    palette_btn.connect_clicked(move |_| {
        open_palette_button();
    });
    header.pack_start(&toggle_btn);
    header.pack_end(&palette_btn);
    header.pack_end(&refresh_btn);

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&split));

    window.set_content(Some(&toolbar_view));
    window.present();

    if reconcile_runtime_state(&app_state.workspace_database_path())
        .map(|report| report.changed())
        .unwrap_or(false)
    {
        refresh_hub.refresh(RefreshScope::All);
    }

    let spotlight_event_tx = {
        let (tx, rx) = mpsc::channel();
        let hub_spotlight_events = refresh_hub.clone();
        let db_path_spotlight_events = app_state.workspace_database_path();
        glib::timeout_add_seconds_local(1, move || {
            if rx.try_iter().next().is_some()
                && reconcile_runtime_state(&db_path_spotlight_events)
                    .map(|report| report.changed())
                    .unwrap_or(false)
            {
                hub_spotlight_events.refresh(RefreshScope::All);
            }
            glib::ControlFlow::Continue
        });
        tx
    };

    let spotlight_watcher = Rc::new(RefCell::new(None));
    let _ = refresh_spotlight_file_watcher(
        &app_state.workspace_database_path(),
        &spotlight_event_tx,
        &spotlight_watcher,
    );

    {
        let db_path_on_close = app_state.workspace_database_path();
        let hub_on_close = refresh_hub.clone();
        let spotlight_watcher_on_close = spotlight_watcher.clone();
        window.connect_destroy(move |_| {
            *spotlight_watcher_on_close.borrow_mut() = None;
            if reconcile_runtime_state(&db_path_on_close)
                .map(|report| report.changed())
                .unwrap_or(false)
            {
                hub_on_close.refresh(RefreshScope::All);
            }
        });
    }

    {
        let db_path_on_focus = app_state.workspace_database_path();
        let hub_on_focus = refresh_hub.clone();
        window.connect_is_active_notify(move |window| {
            if !window.is_active() {
                return;
            }
            if reconcile_runtime_state(&db_path_on_focus)
                .map(|report| report.changed())
                .unwrap_or(false)
            {
                hub_on_focus.refresh(RefreshScope::All);
            }
        });
    }

    // Keyboard shortcuts are resolved from customization.view.keybindings.
    let evk = gtk::EventControllerKey::new();
    let hub_kb = refresh_hub.clone();
    let split_kb = split.clone();
    let open_palette_kb = open_palette.clone();
    let keybindings_kb = Rc::clone(&current_keybindings);
    let state_kb = app_state.clone();
    let stack_kb = main_stack.clone();
    let refresh_workspace_kb = refresh_workspace_detail.clone();
    evk.connect_key_pressed(move |_, keyval, _, modifiers| {
        if let Some(key) = keyval.to_unicode() {
            let action = keybindings_kb.borrow().action_for_event(
                key,
                modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK),
                modifiers.contains(gtk::gdk::ModifierType::ALT_MASK),
                modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK),
                modifiers.contains(gtk::gdk::ModifierType::META_MASK)
                    || modifiers.contains(gtk::gdk::ModifierType::SUPER_MASK),
            );
            match action {
                Some(ShortcutAction::Refresh) => {
                    hub_kb.refresh(RefreshScope::All);
                    return gtk::glib::Propagation::Stop;
                }
                Some(ShortcutAction::ToggleSidebar) => {
                    split_kb.set_show_sidebar(!split_kb.shows_sidebar());
                    return gtk::glib::Propagation::Stop;
                }
                Some(ShortcutAction::CommandPalette) => {
                    open_palette_kb();
                    return gtk::glib::Propagation::Stop;
                }
                Some(ShortcutAction::NavigateTab(tab)) => {
                    apply_palette_target(
                        PaletteTarget::WorkspaceTab(tab),
                        &state_kb,
                        &stack_kb,
                        &split_kb,
                        &hub_kb,
                        &refresh_workspace_kb,
                    );
                    return gtk::glib::Propagation::Stop;
                }
                None => {}
            }
        }
        gtk::glib::Propagation::Proceed
    });
    window.add_controller(evk);

    // Auto-refresh panels every 5 seconds
    let hub_auto = refresh_hub.clone();
    glib::timeout_add_seconds_local(5, move || {
        hub_auto.refresh(RefreshScope::Sidebar);
        hub_auto.refresh(RefreshScope::Dashboard);
        hub_auto.refresh(RefreshScope::Projects);
        hub_auto.refresh(RefreshScope::Workspace);
        glib::ControlFlow::Continue
    });

    // Notification tracker: fires toasts when sessions stop or checks fail.
    // State is (workspace_name, prev_active_sessions, prev_session_was_running).
    let notification_prev: Rc<RefCell<Option<(String, usize, bool)>>> = Rc::new(RefCell::new(None));
    {
        let db_path_notif = app_state.workspace_database_path();
        let state_notif = app_state.clone();
        let toast_notif = toast_overlay.clone();
        let prev_notif = notification_prev.clone();
        glib::timeout_add_seconds_local(5, move || {
            let Some(workspace) = state_notif.selected_workspace() else {
                *prev_notif.borrow_mut() = None;
                return glib::ControlFlow::Continue;
            };
            let rules = WorkspaceStore::open(db_path_notif.clone())
                .and_then(|store| store.workspace_view_defaults(&workspace))
                .map(|defaults| defaults.notification_rules)
                .unwrap_or_default();
            let notify_session_stop = rules.iter().any(|r| {
                matches!(
                    r.to_ascii_lowercase().replace('_', "").as_str(),
                    "sessionstopped" | "sessionstop" | "onsessionstop" | "session"
                )
            });
            let notify_check_fail = rules.iter().any(|r| {
                matches!(
                    r.to_ascii_lowercase().replace('_', "").as_str(),
                    "checksfailed" | "checkfail" | "oncheckfail" | "checks"
                )
            });
            if !notify_session_stop && !notify_check_fail {
                return glib::ControlFlow::Continue;
            }
            let Ok(summary) = WorkspaceStore::open(db_path_notif.clone())
                .and_then(|store| store.checks_summary(&workspace))
            else {
                return glib::ControlFlow::Continue;
            };
            let current_active = summary.active_sessions;
            let session_running = summary.session_status == Some(ProcessStatus::Running);
            let mut prev = prev_notif.borrow_mut();
            if let Some((ref prev_ws, _prev_active, prev_running)) = *prev {
                if prev_ws == &workspace && notify_session_stop && prev_running && !session_running
                {
                    let toast = adw::Toast::new("Agent session stopped.");
                    toast.set_timeout(4);
                    toast_notif.add_toast(toast);
                }
            }
            let _ = notify_check_fail;
            *prev = Some((workspace, current_active, session_running));
            glib::ControlFlow::Continue
        });
    }

    let db_path_runtime_auto = app_state.workspace_database_path();
    let hub_runtime_auto = refresh_hub.clone();
    let spotlight_watcher_auto = spotlight_watcher.clone();
    let spotlight_event_tx_auto = spotlight_event_tx.clone();
    glib::timeout_add_seconds_local(5, move || {
        if reconcile_runtime_state(&db_path_runtime_auto)
            .map(|report| report.changed())
            .unwrap_or(false)
        {
            hub_runtime_auto.refresh(RefreshScope::All);
        }
        let _ = refresh_spotlight_file_watcher(
            &db_path_runtime_auto,
            &spotlight_event_tx_auto,
            &spotlight_watcher_auto,
        );
        glib::ControlFlow::Continue
    });
}

fn show_command_palette(
    parent: &ApplicationWindow,
    commands: Vec<PaletteCommand>,
    apply: Rc<dyn Fn(PaletteTarget)>,
) {
    let dialog = gtk::Window::builder()
        .title("Command Palette")
        .transient_for(parent)
        .modal(true)
        .default_width(420)
        .default_height(360)
        .build();
    let body = GBox::new(Orientation::Vertical, 8);
    body.add_css_class("command-palette");
    body.set_margin_top(12);
    body.set_margin_bottom(12);
    body.set_margin_start(12);
    body.set_margin_end(12);
    let title = Label::new(Some("Command Palette"));
    title.add_css_class("section-title");
    title.set_xalign(0.0);
    body.append(&title);

    let search = Entry::new();
    search.set_placeholder_text(Some("Search commands"));
    search.set_tooltip_text(Some(
        "Filter commands by name, shortcut, or workflow keyword.",
    ));
    body.append(&search);

    let list = GBox::new(Orientation::Vertical, 6);
    let scroll = ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_vexpand(true);
    scroll.set_child(Some(&list));
    body.append(&scroll);

    render_command_palette_results(&list, &commands, "", &dialog, &apply);
    search.grab_focus();
    search.connect_changed({
        let list = list.clone();
        let commands = commands.clone();
        let dialog = dialog.clone();
        let apply = apply.clone();
        move |entry| {
            render_command_palette_results(
                &list,
                &commands,
                entry.text().as_ref(),
                &dialog,
                &apply,
            );
        }
    });

    dialog.set_child(Some(&body));
    dialog.present();
}

fn render_command_palette_results(
    list: &GBox,
    commands: &[PaletteCommand],
    query: &str,
    dialog: &gtk::Window,
    apply: &Rc<dyn Fn(PaletteTarget)>,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    let matches = filter_palette_commands(commands, query);
    if matches.is_empty() {
        let empty = Label::new(Some("No matching commands."));
        empty.add_css_class("card-meta");
        empty.set_xalign(0.0);
        list.append(&empty);
        return;
    }

    for command in matches {
        let label = match &command.shortcut {
            Some(shortcut) => format!("{}  {}", command.label, shortcut),
            None => command.label.to_owned(),
        };
        let button = Button::with_label(&label);
        button.set_halign(gtk::Align::Fill);
        button.set_hexpand(true);
        let target = command.target.clone();
        let apply_command = apply.clone();
        let dialog_to_close = dialog.clone();
        button.connect_clicked(move |_| {
            apply_command(target.clone());
            dialog_to_close.close();
        });
        list.append(&button);
    }
}

fn apply_palette_target(
    target: PaletteTarget,
    state: &AppState,
    main_stack: &Stack,
    split: &adw::OverlaySplitView,
    refresh_hub: &RefreshHub,
    refresh_workspace: &impl Fn(),
) {
    match target {
        PaletteTarget::Page(page) => {
            state.set_active_page(page.clone());
            main_stack.set_visible_child_name(page_stack_name(&page));
            if page == AppPage::Workspace {
                refresh_workspace();
            }
        }
        PaletteTarget::WorkspaceTab(tab) => {
            state.set_active_page(AppPage::Workspace);
            state.set_active_workspace_tab(tab);
            main_stack.set_visible_child_name("workspace");
            refresh_workspace();
        }
        PaletteTarget::Refresh => refresh_hub.refresh(RefreshScope::All),
        PaletteTarget::ToggleSidebar => split.set_show_sidebar(!split.shows_sidebar()),
        PaletteTarget::RunCommand(cmd) => {
            if let Some(workspace) = state.selected_workspace() {
                let _ = WorkspaceStore::open(state.workspace_database_path())
                    .and_then(|store| store.terminal_command(&workspace, &cmd));
                refresh_hub.refresh(RefreshScope::Workspace);
            }
        }
    }
}

fn page_stack_name(page: &AppPage) -> &'static str {
    match page {
        AppPage::Dashboard => "dashboard",
        AppPage::Projects => "projects",
        AppPage::History => "history",
        AppPage::Workspace | AppPage::Settings | AppPage::Review => "workspace",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RuntimeReconciliationReport {
    spotlight_sessions_synced: usize,
    terminal_processes_reconciled: usize,
}

impl RuntimeReconciliationReport {
    fn changed(self) -> bool {
        self.spotlight_sessions_synced > 0 || self.terminal_processes_reconciled > 0
    }
}

fn reconcile_runtime_state(db_path: &Path) -> anyhow::Result<RuntimeReconciliationReport> {
    let store = WorkspaceStore::open(db_path)?;
    let synced = store.spotlight_sync_active_sessions()?;
    let reconciled = store.reconcile_terminal_processes()?;
    Ok(RuntimeReconciliationReport {
        spotlight_sessions_synced: synced.len(),
        terminal_processes_reconciled: reconciled.len(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SpotlightWatchTargetKey {
    session_id: i64,
    workspace_path: PathBuf,
}

struct SpotlightFileWatcher {
    targets: Vec<SpotlightWatchTargetKey>,
    _watcher: RecommendedWatcher,
}

fn refresh_spotlight_file_watcher(
    db_path: &Path,
    event_tx: &Sender<()>,
    current: &Rc<RefCell<Option<SpotlightFileWatcher>>>,
) -> anyhow::Result<()> {
    let store = WorkspaceStore::open(db_path)?;
    let targets = store.spotlight_watch_targets()?;
    let target_keys = targets
        .iter()
        .map(|target| SpotlightWatchTargetKey {
            session_id: target.session_id,
            workspace_path: target.workspace_path.clone(),
        })
        .collect::<Vec<_>>();

    if current
        .borrow()
        .as_ref()
        .map(|watcher| watcher.targets == target_keys)
        .unwrap_or(false)
    {
        return Ok(());
    }

    if targets.is_empty() {
        *current.borrow_mut() = None;
        return Ok(());
    }

    let tx = event_tx.clone();
    let mut watcher = RecommendedWatcher::new(
        move |event: notify::Result<notify::Event>| {
            if event.is_ok() {
                let _ = tx.send(());
            }
        },
        Config::default(),
    )?;
    for target in &targets {
        watcher.watch(&target.workspace_path, RecursiveMode::Recursive)?;
    }

    *current.borrow_mut() = Some(SpotlightFileWatcher {
        targets: target_keys,
        _watcher: watcher,
    });
    Ok(())
}

pub(crate) fn title_case_workspace(name: &str) -> String {
    name.split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn detail_row(label: &str, value: &str) -> GBox {
    let row = GBox::new(Orientation::Horizontal, 12);
    row.add_css_class("detail-row");
    let label_widget = Label::new(Some(label));
    label_widget.add_css_class("detail-label");
    label_widget.set_xalign(0.0);
    label_widget.set_width_chars(18);
    let value_widget = Label::new(Some(value));
    value_widget.add_css_class("detail-value");
    value_widget.set_xalign(0.0);
    value_widget.set_wrap(true);
    value_widget.set_hexpand(true);
    row.append(&label_widget);
    row.append(&value_widget);
    row
}

pub(crate) fn cli_binary() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join("linux-conductor")))
        .filter(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from("linux-conductor"))
}

pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(crate) fn default_clone_parent() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("conductor")
        .join("repos")
}

pub(crate) fn repo_name_from_url(url: &str) -> String {
    url.trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("repository")
        .trim_end_matches(".git")
        .to_owned()
}

pub(crate) fn spawn_terminal_command(cmd: &str) {
    let full_cmd = format!("{cmd}; echo; echo '--- Press Enter to close ---'; read");

    #[cfg(target_os = "macos")]
    {
        let escaped = full_cmd.replace('\\', "\\\\").replace('"', "\\\"");
        if std::process::Command::new("osascript")
            .arg("-e")
            .arg(format!(
                "tell application \"Terminal\" to do script \"{}\"",
                escaped
            ))
            .arg("-e")
            .arg("tell application \"Terminal\" to activate")
            .spawn()
            .is_ok()
        {
            return;
        }
    }

    // Respect $TERMINAL env var if set
    if let Ok(term) = std::env::var("TERMINAL") {
        if std::process::Command::new(&term)
            .args(["-e", "bash", "-c", &full_cmd])
            .spawn()
            .is_ok()
        {
            return;
        }
    }

    let terminals: &[(&str, &[&str])] = &[
        ("gnome-terminal", &["--", "bash", "-c"]),
        ("xterm", &["-e", "bash", "-c"]),
        ("konsole", &["-e", "bash", "-c"]),
        ("xfce4-terminal", &["-e", "bash", "-c"]),
        ("tilix", &["-e", "bash", "-c"]),
        ("terminator", &["-e", "bash", "-c"]),
        ("alacritty", &["-e", "bash", "-c"]),
        ("kitty", &["bash", "-c"]),
        ("foot", &["bash", "-c"]),
        ("wezterm", &["start", "--", "bash", "-c"]),
        ("xterm", &["-e", "bash", "-c"]),
    ];

    for (term, prefix_args) in terminals {
        let mut command = std::process::Command::new(term);
        for arg in *prefix_args {
            command.arg(arg);
        }
        command.arg(&full_cmd);
        if command.spawn().is_ok() {
            return;
        }
    }

    eprintln!("No terminal emulator found. Run manually:\n  {cmd}");
}

// ── STYLES ────────────────────────────────────────────────────────────────

const APP_CSS: &str = r#"
window {
    background-color: #141111;
    color: #f5f0ed;
}

window.lc-theme-light,
.lc-theme-light .dashboard,
.lc-theme-light .history-view {
    background-color: #f7f5f2;
    color: #1f2933;
}

.lc-theme-light .sidebar {
    background-color: #ebe7e1;
    border-right-color: #d6cec4;
}

.lc-theme-light .dashboard-header {
    border-bottom-color: #d6cec4;
}

.lc-theme-light .workspace-card,
.lc-theme-light .command-panel,
.lc-theme-light .metric-card,
.lc-theme-light .detail-row {
    background-color: #ffffff;
    border-color: #d6cec4;
}

.lc-theme-light .workspace-name,
.lc-theme-light .dashboard-title,
.lc-theme-light .column-title,
.lc-theme-light .card-title,
.lc-theme-light .metric-value,
.lc-theme-light .detail-value {
    color: #1f2933;
}

.lc-theme-light .workspace-meta,
.lc-theme-light .card-meta,
.lc-theme-light .card-branch,
.lc-theme-light .card-diff,
.lc-theme-light .column-count,
.lc-theme-light .detail-label,
.lc-theme-light .project-tab,
.lc-theme-light .sidebar-header,
.lc-theme-light .repo-section-header {
    color: #667085;
}

.lc-theme-light .workspace-list row:selected,
.lc-theme-light .nav-row-active,
.lc-theme-light .nav-button-active,
.lc-theme-light .nav-button:hover {
    background-color: #ddd6cc;
    color: #111827;
}

.lc-theme-light .workspace-list row:hover {
    background-color: #e5ded5;
}

.lc-theme-light headerbar,
.lc-theme-light .diff-view,
.lc-theme-light .checks-view,
.lc-theme-light .composer-bar,
.lc-theme-light .status-container {
    background-color: #ebe7e1;
    color: #1f2933;
}

.lc-theme-light separator {
    background-color: #d6cec4;
}

.lc-accent-blue .section-title,
.lc-accent-blue .workspace-title,
.lc-accent-blue .project-tab-active,
.lc-accent-blue .card-activity,
.lc-accent-blue .composer-bar entry:focus {
    color: #2563eb;
    border-color: #2563eb;
}

.lc-accent-green .section-title,
.lc-accent-green .workspace-title,
.lc-accent-green .project-tab-active,
.lc-accent-green .card-activity,
.lc-accent-green .composer-bar entry:focus {
    color: #16a34a;
    border-color: #16a34a;
}

.lc-accent-amber .section-title,
.lc-accent-amber .workspace-title,
.lc-accent-amber .project-tab-active,
.lc-accent-amber .card-activity,
.lc-accent-amber .composer-bar entry:focus {
    color: #d97706;
    border-color: #d97706;
}

.lc-accent-rose .section-title,
.lc-accent-rose .workspace-title,
.lc-accent-rose .project-tab-active,
.lc-accent-rose .card-activity,
.lc-accent-rose .composer-bar entry:focus {
    color: #e11d48;
    border-color: #e11d48;
}

.lc-density-compact .nav-row,
.lc-density-compact .nav-row-active,
.lc-density-compact .nav-button,
.lc-density-compact .nav-button-active {
    padding: 6px 10px;
}

.lc-density-compact .project-row,
.lc-density-compact .history-row,
.lc-density-compact .detail-row,
.lc-density-compact .command-panel,
.lc-density-compact .metric-card,
.lc-density-compact .workspace-card {
    padding: 8px;
}

.lc-density-compact .detail-body,
.lc-density-compact .kanban-board {
    padding: 18px;
}

.lc-density-comfortable .nav-row,
.lc-density-comfortable .nav-row-active,
.lc-density-comfortable .nav-button,
.lc-density-comfortable .nav-button-active {
    padding: 12px 16px;
}

.lc-density-comfortable .project-row,
.lc-density-comfortable .history-row,
.lc-density-comfortable .detail-row,
.lc-density-comfortable .command-panel,
.lc-density-comfortable .metric-card,
.lc-density-comfortable .workspace-card {
    padding: 14px;
}

.lc-density-comfortable .detail-body,
.lc-density-comfortable .kanban-board {
    padding: 32px;
}

.sidebar {
    background-color: #171313;
    border-right: 1px solid #302b2b;
    padding-top: 12px;
}

.sidebar-header {
    font-size: 10px;
    font-weight: bold;
    color: #8b8582;
    text-transform: uppercase;
    letter-spacing: 1px;
}

.nav-row, .nav-row-active, .nav-button, .nav-button-active {
    font-size: 15px;
    padding: 10px 14px;
    margin: 2px 10px;
    border-radius: 8px;
    background: transparent;
    border: none;
    box-shadow: none;
    text-shadow: none;
}

.nav-row, .nav-button {
    color: #b8b0ac;
}

.nav-row-active, .nav-button-active, .nav-button:hover {
    color: #f5f0ed;
    background-color: #292523;
}

.projects-header {
    padding: 16px 14px 8px 14px;
}

.workspace-list {
    background-color: transparent;
}

.workspace-list row {
    border-radius: 8px;
    margin: 2px 10px;
    padding: 0;
}

.workspace-list row:selected {
    background-color: #292523;
}

.workspace-list row:hover {
    background-color: #211d1b;
}

.workspace-name {
    font-size: 14px;
    font-weight: 600;
    color: #f5f0ed;
}

.workspace-meta {
    font-size: 11px;
    color: #8b8582;
    font-family: monospace;
}

.workspace-status {
    font-size: 10px;
    color: #a6e3a1;
}

.workspace-status-archived {
    font-size: 10px;
    color: #45475a;
    font-style: italic;
}

.archive-banner {
    background-color: #2a2830;
    color: #f9e2af;
    font-size: 12px;
    border-left: 3px solid #f9e2af;
    padding: 6px 10px;
    border-radius: 4px;
}

.quick-stats {
    padding: 4px 0;
}

.stat-running {
    color: #a6e3a1;
    font-size: 12px;
    font-weight: bold;
}

.stat-stopped {
    color: #6c7086;
    font-size: 12px;
}

.stat-dim {
    color: #6c7086;
    font-size: 12px;
}

.workspace-title {
    font-size: 14px;
    font-weight: bold;
    color: #89b4fa;
}

.add-workspace-btn {
    background-color: transparent;
    color: #89b4fa;
    border: 1px solid #313244;
    border-radius: 6px;
    font-size: 12px;
}

.add-workspace-btn:hover {
    background-color: #313244;
}

.center-panel {
    background-color: #181825;
}

.dashboard {
    background-color: #141111;
}

.dashboard-header {
    padding: 20px 26px 0 26px;
    border-bottom: 1px solid #302b2b;
}

.dashboard-title {
    font-size: 17px;
    font-weight: 700;
    color: #f5f0ed;
}

.project-tabs {
    padding-bottom: 10px;
}

.project-tab, .project-tab-active {
    font-size: 13px;
    font-weight: 600;
}

.project-tab {
    color: #8b8582;
}

.project-tab-active {
    color: #f5f0ed;
    border-bottom: 2px solid #d7bfb4;
    padding-bottom: 10px;
}

.kanban-board {
    padding: 28px 28px;
}

.kanban-column {
    min-width: 235px;
}

.column-icon {
    color: #f5d90a;
    font-size: 14px;
}

.column-title {
    color: #f5f0ed;
    font-size: 15px;
    font-weight: 700;
}

.column-count {
    color: #8b8582;
    font-size: 13px;
}

.column-empty {
    color: #6f6966;
    font-size: 12px;
    padding: 18px 0;
}

.workspace-card {
    background-color: #292523;
    border: 1px solid #49413d;
    border-radius: 8px;
    padding: 12px;
    min-height: 116px;
}

.card-branch {
    color: #9d9691;
    font-size: 12px;
}

.card-diff {
    color: #8b8582;
    font-size: 12px;
}

.card-diff-hot {
    color: #4ade80;
    font-size: 12px;
    font-weight: 700;
}

.card-title {
    color: #f5f0ed;
    font-size: 15px;
    font-weight: 700;
}

.card-meta, .card-activity {
    color: #9d9691;
    font-size: 12px;
}

.card-activity {
    color: #d7bfb4;
}

.project-row {
    padding: 8px 10px;
}

.project-icon, .project-icon-hot {
    font-size: 15px;
}

.project-icon {
    color: #6f6966;
}

.project-icon-hot {
    color: #f5d90a;
}

.detail-body {
    padding: 24px 28px;
}

.command-center-strip {
    padding: 0;
}

.command-panel, .metric-card {
    background-color: #211d1b;
    border: 1px solid #3a3330;
    border-radius: 8px;
    padding: 12px;
}

.metric-value {
    color: #f5f0ed;
    font-size: 16px;
    font-weight: 700;
}

.detail-row {
    background-color: #211d1b;
    border: 1px solid #3a3330;
    border-radius: 8px;
    padding: 10px 12px;
}

.detail-label {
    color: #8b8582;
    font-size: 12px;
    font-weight: 700;
}

.detail-value {
    color: #f5f0ed;
    font-size: 13px;
}

.history-row {
    padding: 10px 12px;
}

.history-view {
    background-color: #141111;
    color: #f5f0ed;
    font-size: 13px;
    padding: 18px;
}

.terminal-tab-strip {
    padding: 2px 0;
}

.terminal-tab-strip button {
    font-size: 12px;
    border-radius: 6px;
    padding: 5px 10px;
}

.workspace-toolbar {
    background-color: #1e1e2e;
    padding: 8px 12px;
    border-bottom: 1px solid #313244;
}

.right-panel {
    background-color: #181825;
}

.panel-switcher {
    background-color: transparent;
}

.panel-switcher button {
    background-color: transparent;
    color: #6c7086;
    border: none;
    border-radius: 6px;
    padding: 4px 10px;
    font-size: 12px;
    font-weight: 500;
}

.panel-switcher button:hover {
    background-color: #313244;
    color: #cdd6f4;
}

.panel-switcher button:checked {
    background-color: #313244;
    color: #89b4fa;
    font-weight: 600;
}

.section-title {
    font-size: 11px;
    font-weight: bold;
    color: #89b4fa;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin-top: 8px;
}

.info-text {
    font-size: 12px;
    color: #585b70;
}

.status-name {
    font-size: 13px;
    font-weight: 600;
    color: #cdd6f4;
    font-family: monospace;
}

.status-detail {
    font-size: 12px;
    color: #a6adc8;
}

.diff-view, .checks-view {
    background-color: #1e1e2e;
    color: #cdd6f4;
    font-size: 12px;
    font-family: "JetBrains Mono", "Fira Code", "Cascadia Code", monospace;
}

.status-container {
    background-color: #1e1e2e;
    border-radius: 6px;
    padding: 8px;
}

.empty-label {
    font-size: 12px;
    color: #585b70;
}

headerbar {
    background-color: #1e1e2e;
    border-bottom: 1px solid #313244;
}

separator {
    background-color: #313244;
    min-width: 1px;
    min-height: 1px;
}

.repo-section-header {
    font-size: 9px;
    font-weight: bold;
    color: #6c7086;
    text-transform: uppercase;
    letter-spacing: 1px;
}

.run-dot-active {
    font-size: 9px;
    color: #a6e3a1;
}

.run-dot {
    font-size: 9px;
    color: #45475a;
}

.pr-badge {
    font-size: 9px;
    font-weight: bold;
    color: #89b4fa;
    background-color: #1e1e2e;
    border: 1px solid #45475a;
    border-radius: 4px;
    padding: 0 4px;
}

.composer-bar {
    background-color: #1e1e2e;
    border-top: 1px solid #313244;
}

.composer-bar entry {
    background-color: #181825;
    color: #cdd6f4;
    border: 1px solid #313244;
    border-radius: 6px;
    font-size: 13px;
}

.composer-bar entry:focus {
    border-color: #89b4fa;
}

.pill-button {
    background-color: #313244;
    color: #cdd6f4;
    border: 1px solid #45475a;
    border-radius: 6px;
    padding: 2px 10px;
    font-size: 12px;
}

.pill-button:hover {
    background-color: #45475a;
}

.conflict-badge {
    color: #f9e2af;
    font-size: 11px;
    font-weight: bold;
}

.session-badge {
    font-size: 9px;
    font-weight: bold;
    color: #a6e3a1;
    background-color: #1e1e2e;
    border: 1px solid #45475a;
    border-radius: 4px;
    padding: 0 3px;
}

.todo-count-badge {
    font-size: 9px;
    font-weight: bold;
    color: #f9e2af;
    background-color: #1e1e2e;
    border: 1px solid #45475a;
    border-radius: 4px;
    padding: 0 3px;
}

.workspace-path-label {
    color: #6c7086;
    font-size: 11px;
    font-family: monospace;
    margin-bottom: 2px;
}

.sidebar-search {
    background-color: #181825;
    color: #cdd6f4;
    border: 1px solid #313244;
    border-radius: 6px;
    font-size: 12px;
}

.sidebar-search:focus {
    border-color: #89b4fa;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use linux_conductor_core::repository::{AddRepository, RepositoryStore};
    use linux_conductor_core::workspace::{CreateWorkspace, ProcessStatus};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    #[test]
    fn startup_runtime_reconciliation_marks_stale_terminal_rows_exited() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        let db_path = temp.path().join("state.db");
        RepositoryStore::open(&db_path)
            .unwrap()
            .add(AddRepository {
                name: Some("demo".to_owned()),
                root_path: repo_path,
                default_branch: Some("main".to_owned()),
                remote_name: "origin".to_owned(),
                workspace_parent_path: Some(temp.path().join("workspaces/demo")),
            })
            .unwrap();

        let store = WorkspaceStore::open_with_logs(&db_path, temp.path().join("logs")).unwrap();
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        store
            .record_terminal_process("berlin", "shell", 999_999)
            .unwrap();

        let report = reconcile_runtime_state(&db_path).unwrap();

        assert_eq!(report.terminal_processes_reconciled, 1);
        assert_eq!(
            WorkspaceStore::open_with_logs(&db_path, temp.path().join("logs"))
                .unwrap()
                .list_terminals("berlin")
                .unwrap()[0]
                .status,
            ProcessStatus::Exited
        );
    }

    #[test]
    fn launch_target_parses_workspace_and_tab_args() {
        let target = parse_launch_target([
            "linux-conductor-gtk",
            "--workspace",
            "berlin",
            "--tab",
            "checks",
        ])
        .unwrap();

        assert_eq!(target.workspace.as_deref(), Some("berlin"));
        assert_eq!(target.workspace_tab, WorkspaceTab::Checks);
        assert_eq!(target.page, AppPage::Workspace);
    }

    #[test]
    fn launch_target_parses_workspace_deep_link() {
        let target = parse_launch_target([
            "linux-conductor-gtk",
            "linux-conductor://workspace/berlin?tab=review",
        ])
        .unwrap();

        assert_eq!(target.workspace.as_deref(), Some("berlin"));
        assert_eq!(target.workspace_tab, WorkspaceTab::Review);
        assert_eq!(target.page, AppPage::Workspace);
    }

    #[test]
    fn launch_target_parses_page_deep_links_and_tab_aliases() {
        let history =
            parse_launch_target(["linux-conductor-gtk", "linux-conductor://history"]).unwrap();
        let terminal = parse_workspace_tab("big-terminal").unwrap();

        assert_eq!(history.page, AppPage::History);
        assert_eq!(history.workspace, None);
        assert_eq!(terminal, WorkspaceTab::Terminal);
    }

    #[test]
    fn view_preferences_parse_known_theme_accent_and_density() {
        let preferences = ViewPreferences::from_defaults(WorkspaceViewDefaults {
            default_visible_tab: None,
            theme: Some("light".to_owned()),
            accent_color: Some("green".to_owned()),
            density: Some("compact".to_owned()),
            keybindings: None,
            terminal_font: None,
            terminal_scrollback: None,
            command_palette_presets: Vec::new(),
            agent_profile_names: Vec::new(),
            notification_rules: Vec::new(),
        });

        assert_eq!(
            preferences.css_classes(),
            vec!["lc-theme-light", "lc-accent-green", "lc-density-compact"]
        );
    }

    #[test]
    fn view_preferences_ignore_unknown_values() {
        let preferences = ViewPreferences::from_defaults(WorkspaceViewDefaults {
            default_visible_tab: None,
            theme: Some("noir".to_owned()),
            accent_color: Some("purple".to_owned()),
            density: Some("tiny".to_owned()),
            keybindings: None,
            terminal_font: None,
            terminal_scrollback: None,
            command_palette_presets: Vec::new(),
            agent_profile_names: Vec::new(),
            notification_rules: Vec::new(),
        });

        assert!(preferences.css_classes().is_empty());
    }

    fn init_repo(path: PathBuf) -> PathBuf {
        fs::create_dir(&path).unwrap();
        Command::new("git")
            .args(["init", "--initial-branch", "main"])
            .arg(&path)
            .status()
            .unwrap();
        fs::write(path.join("README.md"), "demo\n").unwrap();
        git(&path, ["add", "."]);
        git(
            &path,
            [
                "-c",
                "user.name=Linux Conductor",
                "-c",
                "user.email=linux-conductor@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "initial",
            ],
        );
        path
    }

    fn git<const N: usize>(repo_path: &Path, args: [&str; N]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
    }
}
