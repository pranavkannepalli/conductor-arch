#![allow(dead_code)]
#![allow(clippy::ptr_arg, clippy::too_many_arguments)]

mod archcar_async;
mod buttons;
mod command_palette;
mod dashboard;
mod history;
mod logger;
mod projects;
mod refresh;
mod session_surface;
mod settings;
mod setup;
mod sidebar;
mod state;
mod terminal;
mod theme;
mod toast;
mod workspace_command_center;

use crate::buttons::{icon_button, text_button};
use adw::prelude::*;
use adw::{Application, ApplicationWindow, ColorScheme, StyleManager};
use command_palette::{
    filter_palette_commands, palette_commands, Keybindings, PaletteCommand, PaletteTarget,
    ShortcutAction,
};
use gtk::{
    Align, Box as GBox, CssProvider, Entry, Label, Orientation, Overlay, ScrolledWindow, Stack,
    STYLE_PROVIDER_PRIORITY_APPLICATION,
};
use linux_archductor_core::archcar::server::{
    reconcile_managed_sessions_on_startup, ArchcarServer,
};
use linux_archductor_core::paths::AppPaths;
use linux_archductor_core::workspace::{ProcessStatus, WorkspaceStore, WorkspaceViewDefaults};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use refresh::{RefreshHub, RefreshScope};
use state::{AppPage, AppState, WorkspaceTab};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Sender};
use std::time::Instant;
use toast::{ToastManager, ToastMessage};

const APP_ID: &str = "io.github.pranavkannepalli.linux-archductor";

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
    if std::env::args().any(|arg| arg == "--archcar-serve") {
        let paths = AppPaths::from_env();
        if let Err(err) = reconcile_managed_sessions_on_startup(&paths)
            .and_then(|_| ArchcarServer::bind(paths))
            .and_then(|server| server.serve())
        {
            eprintln!("archcar serve failed: {err:#}");
            std::process::exit(1);
        }
        return;
    }

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
            value if value.starts_with("linux-archductor://") => {
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
        .strip_prefix("linux-archductor://")
        .ok_or_else(|| "deep link must start with linux-archductor://".to_owned())?;
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
        "settings" | "config" => Ok(AppPage::Settings),
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
    let style_manager = StyleManager::default();
    match preferences.theme {
        Some(ViewTheme::Light) => style_manager.set_color_scheme(ColorScheme::ForceLight),
        Some(ViewTheme::Dark) => style_manager.set_color_scheme(ColorScheme::ForceDark),
        Some(ViewTheme::System) | None => style_manager.set_color_scheme(ColorScheme::Default),
    }
    for class_name in VIEW_PREFERENCE_CLASSES {
        window.remove_css_class(class_name);
    }
    for class_name in preferences.css_classes() {
        window.add_css_class(class_name);
    }
}

fn build_ui(app: &Application, launch_target: LaunchTarget) {
    let startup = Instant::now();
    let paths = AppPaths::from_env();
    if let Err(err) = logger::init_dev_logger(&paths) {
        eprintln!("failed to initialize dev logger: {err:#}");
    }
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        ?launch_target,
        "gtk startup: build_ui entered"
    );
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
        .title("Linux Archductor")
        .default_width(1280)
        .default_height(800)
        .build();
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: window built"
    );
    let initial_view_preferences = resolve_view_preferences(
        paths.database_path.clone(),
        launch_target.workspace.as_deref(),
    );
    let current_keybindings = Rc::new(RefCell::new(resolve_keybindings(
        paths.database_path.clone(),
        launch_target.workspace.as_deref(),
    )));
    let css = CssProvider::new();
    css.load_from_data(theme::app_css());
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().unwrap(),
        &css,
        STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    apply_view_preferences(&window, &initial_view_preferences);
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: styles applied"
    );

    let split = adw::OverlaySplitView::new();
    split.set_min_sidebar_width(120.0);
    split.set_max_sidebar_width(360.0);
    split.set_pin_sidebar(false);
    split.set_collapsed(false);
    let collapse_sidebar: Rc<dyn Fn()> = {
        let split = split.clone();
        Rc::new(move || split.set_collapsed(true))
    };

    let toast_overlay = adw::ToastOverlay::new();
    let toast_manager = ToastManager::new(&toast_overlay);
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: building dashboard"
    );
    let (dashboard, refresh_dashboard) = dashboard::build_dashboard_panel(&app_state.paths);
    dashboard.set_hexpand(true);
    dashboard.set_vexpand(true);
    let main_stack_handle: Rc<RefCell<Option<Stack>>> = Rc::new(RefCell::new(None));
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: dashboard built"
    );

    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: building workspace center"
    );
    let (workspace_detail, refresh_workspace_detail) =
        workspace_command_center::build_workspace_command_center(
            &app_state,
            refresh_hub.clone(),
            toast_overlay.clone(),
            collapse_sidebar.clone(),
        );
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: workspace center built"
    );
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: building projects page"
    );
    let (projects_page, refresh_projects) = projects::build_projects_page(
        &app_state.paths,
        refresh_dashboard.clone(),
        refresh_workspace_detail.clone(),
    );
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: projects page built"
    );
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: building settings page"
    );
    let (settings_page, refresh_settings) = settings::build_settings_page(&app_state.paths);
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: settings page built"
    );
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: building history page"
    );
    let (history_page, refresh_history) =
        history::build_history_page(app_state.workspace_database_path());
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: history page built"
    );

    let main_stack = Stack::new();
    main_stack.set_hexpand(true);
    main_stack.set_vexpand(true);
    *main_stack_handle.borrow_mut() = Some(main_stack.clone());
    main_stack.add_named(&dashboard, Some("dashboard"));
    main_stack.add_named(&projects_page, Some("projects"));
    main_stack.add_named(&settings_page, Some("settings"));
    main_stack.add_named(&history_page, Some("history"));
    main_stack.add_named(&workspace_detail, Some("workspace"));
    main_stack.set_visible_child_name(match app_state.snapshot().active_page {
        AppPage::Workspace => "workspace",
        AppPage::Projects => "projects",
        AppPage::Settings => "settings",
        AppPage::History => "history",
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
        window.clone(),
        split.clone(),
        refresh_workspace_detail.clone(),
        refresh_view_preferences.clone(),
    );
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: sidebar built"
    );

    refresh_hub.set_dashboard(refresh_dashboard.clone());
    refresh_hub.set_workspace(refresh_workspace_detail.clone());
    refresh_hub.set_projects({
        let refresh_projects = refresh_projects.clone();
        let refresh_settings = refresh_settings.clone();
        move || {
            refresh_projects();
            refresh_settings();
        }
    });
    refresh_hub.set_history(refresh_history.clone());
    refresh_hub.set_sidebar(refresh_sidebar.clone());

    split.set_sidebar(Some(&sidebar));
    toast_overlay.set_child(Some(&main_stack));

    let content_overlay = Overlay::new();
    content_overlay.set_child(Some(&toast_overlay));

    let reopen_sidebar_btn = icon_button("sidebar-show-symbolic", "Show sidebar");
    reopen_sidebar_btn.add_css_class("sidebar-reopen-button");
    reopen_sidebar_btn.set_tooltip_text(Some("Show sidebar"));
    reopen_sidebar_btn.set_halign(Align::Start);
    reopen_sidebar_btn.set_valign(Align::Start);
    reopen_sidebar_btn.set_margin_start(10);
    reopen_sidebar_btn.set_margin_top(10);
    {
        let split = split.clone();
        reopen_sidebar_btn.connect_clicked(move |_| {
            split.set_collapsed(false);
        });
    }
    content_overlay.add_overlay(&reopen_sidebar_btn);
    reopen_sidebar_btn.set_visible(split.is_collapsed());
    {
        let reopen_sidebar_btn = reopen_sidebar_btn.clone();
        split.connect_collapsed_notify(move |split| {
            reopen_sidebar_btn.set_visible(split.is_collapsed());
        });
    }

    split.set_content(Some(&content_overlay));

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
    window.set_content(Some(&split));
    window.present();
    setup::show_blocking_setup_if_needed(&window);
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: window presented"
    );

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
            if key.eq_ignore_ascii_case(&'t')
                && (modifiers.contains(gtk::gdk::ModifierType::META_MASK)
                    || modifiers.contains(gtk::gdk::ModifierType::SUPER_MASK))
            {
                apply_palette_target(
                    PaletteTarget::WorkspaceTab(WorkspaceTab::Terminal),
                    &state_kb,
                    &stack_kb,
                    &split_kb,
                    &hub_kb,
                    &refresh_workspace_kb,
                );
                return gtk::glib::Propagation::Stop;
            }
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

    // Notification tracker: fires toasts when sessions stop or checks fail.
    // State is (workspace_name, prev_active_sessions, prev_session_was_running).
    let notification_prev: Rc<RefCell<Option<(String, usize, bool)>>> = Rc::new(RefCell::new(None));
    {
        let db_path_notif = app_state.workspace_database_path();
        let state_notif = app_state.clone();
        let toast_notif = toast_manager.clone();
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
                    toast_notif.show(ToastMessage::success("Agent session stopped."));
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
        let button = text_button(&label);
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
            state.navigate_to_page(page.clone());
            main_stack.set_visible_child_name(page_stack_name(&page));
            refresh_hub.refresh(RefreshScope::Sidebar);
            if page == AppPage::Workspace {
                refresh_workspace();
            }
        }
        PaletteTarget::WorkspaceTab(tab) => {
            state.navigate_to_workspace_tab(tab);
            main_stack.set_visible_child_name("workspace");
            refresh_hub.refresh(RefreshScope::Sidebar);
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
        AppPage::Settings => "settings",
        AppPage::History => "history",
        AppPage::Workspace | AppPage::Review => "workspace",
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
        .and_then(|path| path.parent().map(|parent| parent.join("linux-archductor")))
        .filter(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from("linux-archductor"))
}

pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(crate) fn default_clone_parent() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("archductor")
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

#[cfg(test)]
mod tests {
    use super::*;
    use linux_archductor_core::repository::{AddRepository, RepositoryStore};
    use linux_archductor_core::workspace::{CreateWorkspace, ProcessStatus};
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
            "linux-archductor-gtk",
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
            "linux-archductor-gtk",
            "linux-archductor://workspace/berlin?tab=review",
        ])
        .unwrap();

        assert_eq!(target.workspace.as_deref(), Some("berlin"));
        assert_eq!(target.workspace_tab, WorkspaceTab::Review);
        assert_eq!(target.page, AppPage::Workspace);
    }

    #[test]
    fn launch_target_parses_page_deep_links_and_tab_aliases() {
        let history =
            parse_launch_target(["linux-archductor-gtk", "linux-archductor://history"]).unwrap();
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
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
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
