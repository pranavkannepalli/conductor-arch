#![allow(dead_code)]
#![allow(clippy::ptr_arg, clippy::too_many_arguments)]

mod archcar_async;
mod background_sync;
mod buttons;
mod command_palette;
mod dashboard;
mod file_component;
mod font_assets;
mod history;
mod history_data;
mod logger;
mod motion;
mod projects;
mod refresh;
mod session_surface;
mod settings;
mod setup;
mod sidebar;
mod state;
mod tabs;
mod terminal;
mod text;
mod theme;
mod toast;
mod window_chrome;
mod workspace_command_center;

use crate::buttons::{icon_button, text_button};
use adw::prelude::*;
use adw::Application;
use archductor_core::archcar::client::ArchcarClient;
use archductor_core::archcar::protocol::ArchcarRequest;
use archductor_core::archcar::server::{reconcile_managed_sessions_on_startup, ArchcarServer};
use archductor_core::paths::AppPaths;
use archductor_core::workspace::{
    ProcessKind, ProcessRecord, ProcessStatus, WorkspaceStore, WorkspaceViewDefaults,
};
use command_palette::{
    filter_palette_commands, palette_commands, Keybindings, PaletteCommand, PaletteTarget,
    ShortcutAction,
};
use gtk::{
    Align, ApplicationWindow, Box as GBox, CssProvider, Entry, Label, Orientation, Overflow,
    Overlay, ScrolledWindow, Stack, STYLE_PROVIDER_PRIORITY_APPLICATION,
};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use refresh::{RefreshEvent, RefreshHub, RefreshScope};
use state::{AppPage, AppState, AppStateEvent, WorkspaceTab};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::time::Instant;
use toast::{ToastManager, ToastMessage};

const APP_ID: &str = "io.github.pranavkannepalli.archductor";
const APP_SIDEBAR_DEFAULT_WIDTH_PX: f64 = 320.0;
pub(crate) const COLUMN_HEADER_HEIGHT: i32 = 52;
static NEXT_COLOR_SCOPE_ID: AtomicU64 = AtomicU64::new(1);

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
    colors: BTreeMap<String, String>,
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
            colors: defaults.colors,
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
        if !self.colors.is_empty() {
            classes.push("lc-custom-colors");
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
    "lc-custom-colors",
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
    font_assets::register_bundled_fonts();
    let paths = AppPaths::from_env();
    if std::env::args().any(|arg| arg == "--archcar-serve") {
        if let Err(err) = reconcile_managed_sessions_on_startup(&paths)
            .and_then(|_| ArchcarServer::bind(paths))
            .and_then(|server| server.serve())
        {
            eprintln!("archcar serve failed: {err:#}");
            std::process::exit(1);
        }
        return;
    }

    let debug_mode = debug_mode_enabled();
    let launch_target =
        parse_launch_target_with_debug_mode(std::env::args(), debug_mode).unwrap_or_default();

    let app_id = application_id();
    let app = Application::builder().application_id(&app_id).build();
    app.connect_shutdown(move |_| interrupt_running_managed_chats_on_shutdown(&paths));
    app.connect_activate(move |app| build_ui(app, launch_target.clone(), debug_mode));
    app.run_with_args(&["archductor-gtk"]);
}

fn application_id() -> String {
    dev_application_id_from_suffix(std::env::var("ARCHDUCTOR_DEV_INSTANCE").ok().as_deref())
}

fn dev_application_id_from_suffix(suffix: Option<&str>) -> String {
    let Some(suffix) = suffix else {
        return APP_ID.to_owned();
    };
    let sanitized = sanitize_dev_application_id_suffix(suffix);
    if sanitized.is_empty() {
        APP_ID.to_owned()
    } else {
        format!("{APP_ID}.dev.instance-{sanitized}")
    }
}

fn sanitize_dev_application_id_suffix(value: &str) -> String {
    let mut out = String::new();
    let mut last_was_separator = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_was_separator = false;
        } else if !last_was_separator && !out.is_empty() {
            out.push('-');
            last_was_separator = true;
        }
        if out.len() >= 80 {
            break;
        }
    }
    out.trim_matches('-').to_owned()
}

fn dev_instance_banner_text() -> Option<String> {
    let instance = std::env::var("ARCHDUCTOR_DEV_INSTANCE").ok()?;
    let instance = instance.trim();
    if instance.is_empty() {
        None
    } else {
        Some(format!("Dev worktree: {instance}"))
    }
}

fn build_dev_instance_banner(text: &str) -> GBox {
    let banner = GBox::new(Orientation::Horizontal, 0);
    banner.add_css_class("dev-instance-banner");
    banner.set_hexpand(true);

    let label = Label::new(Some(text));
    label.add_css_class("dev-instance-banner-label");
    label.set_hexpand(true);
    label.set_xalign(0.5);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_tooltip_text(Some(text));
    banner.append(&label);

    banner
}

fn parse_launch_target<I, S>(args: I) -> Result<LaunchTarget, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    parse_launch_target_with_debug_mode(args, false)
}

fn parse_launch_target_with_debug_mode<I, S>(
    args: I,
    debug_mode: bool,
) -> Result<LaunchTarget, String>
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
                target.page = parse_app_page_with_debug_mode(value, debug_mode)?;
            }
            value if value.starts_with("archductor://") => {
                target = parse_deep_link_with_debug_mode(value, debug_mode)?;
            }
            _ => {}
        }
        index += 1;
    }
    Ok(target)
}

fn parse_deep_link(value: &str) -> Result<LaunchTarget, String> {
    parse_deep_link_with_debug_mode(value, false)
}

fn parse_deep_link_with_debug_mode(value: &str, debug_mode: bool) -> Result<LaunchTarget, String> {
    let rest = value
        .strip_prefix("archductor://")
        .ok_or_else(|| "deep link must start with archductor://".to_owned())?;
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
        [page] => target.page = parse_app_page_with_debug_mode(page, debug_mode)?,
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
    parse_app_page_with_debug_mode(value, false)
}

fn parse_app_page_with_debug_mode(value: &str, _debug_mode: bool) -> Result<AppPage, String> {
    match normalize_launch_token(value).as_str() {
        "dashboard" | "home" => Ok(AppPage::Dashboard),
        "projects" | "repositories" | "repos" => Ok(AppPage::Projects),
        "settings" | "config" => Ok(AppPage::Settings),
        "history" | "archive" => Ok(AppPage::History),
        "workspace" | "workspaces" => Ok(AppPage::Workspace),
        other => Err(format!("unknown page: {other}")),
    }
}

fn debug_mode_enabled() -> bool {
    debug_mode_enabled_from_env(std::env::var("ARCHDUCTOR_DEBUG").ok().as_deref())
}

fn debug_mode_enabled_from_env(value: Option<&str>) -> bool {
    archductor_core::env_flags::explicit_truthy(value)
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
            WorkspaceStore::open_app(db_path)
                .and_then(|store| store.workspace_view_defaults(name))
                .ok()
        })
        .map(ViewPreferences::from_defaults)
        .unwrap_or_default()
}

fn resolve_workspace_default_tab(db_path: &Path, workspace: &str) -> WorkspaceTab {
    WorkspaceStore::open_app(db_path)
        .and_then(|store| store.workspace_view_defaults(workspace))
        .ok()
        .and_then(|defaults| defaults.default_visible_tab)
        .and_then(|tab| WorkspaceTab::from_config(&tab))
        .unwrap_or(WorkspaceTab::Chats)
}

fn resolve_keybindings(db_path: PathBuf, workspace: Option<&str>) -> Keybindings {
    workspace
        .and_then(|name| {
            WorkspaceStore::open_app(db_path)
                .and_then(|store| store.workspace_view_defaults(name))
                .ok()
        })
        .and_then(|defaults| defaults.keybindings)
        .as_deref()
        .map(|value| Keybindings::from_config(Some(value)))
        .unwrap_or_default()
}

fn apply_view_preferences(
    target: &impl IsA<gtk::Widget>,
    preferences: &ViewPreferences,
    colors_css: &CssProvider,
    color_scope_class: &str,
) {
    colors_css.load_from_data(&view_colors_css(color_scope_class, &preferences.colors));
    for class_name in VIEW_PREFERENCE_CLASSES {
        target.remove_css_class(class_name);
    }
    target.remove_css_class(color_scope_class);
    for class_name in preferences.css_classes() {
        target.add_css_class(class_name);
    }
    if !preferences.colors.is_empty() {
        target.add_css_class(color_scope_class);
    }
}

fn view_colors_css(scope_class: &str, colors: &BTreeMap<String, String>) -> String {
    if colors.is_empty() {
        return String::new();
    }
    let mut css = String::new();
    let color_suffix = scope_class.replace('-', "_");
    for (key, css_name, default) in VIEW_COLOR_TOKENS {
        let value = colors
            .get(*key)
            .filter(|value| is_config_hex_color(value))
            .map(String::as_str)
            .unwrap_or(default);
        css.push_str(&format!(
            "@define-color {css_name}-{color_suffix} {value};\n"
        ));
    }
    let mut scoped_css = CUSTOM_COLOR_CSS.replace(".lc-custom-colors", &format!(".{scope_class}"));
    let mut color_names = VIEW_COLOR_TOKENS
        .iter()
        .map(|(_, css_name, _)| *css_name)
        .collect::<Vec<_>>();
    color_names.sort_by_key(|name| std::cmp::Reverse(name.len()));
    for css_name in color_names {
        scoped_css = scoped_css.replace(
            &format!("@{css_name};"),
            &format!("@{css_name}-{color_suffix};"),
        );
    }
    css.push_str(&scoped_css);
    css
}

fn is_config_hex_color(value: &str) -> bool {
    let Some(hex) = value.strip_prefix('#') else {
        return false;
    };
    matches!(hex.len(), 3 | 6) && hex.chars().all(|ch| ch.is_ascii_hexdigit())
}

const VIEW_COLOR_TOKENS: &[(&str, &str, &str)] = &[
    ("background", "lc-bg", "#191919"),
    ("surface", "lc-surface", "#1e1e1e"),
    ("surface_raised", "lc-surface-raised", "#202020"),
    ("surface_muted", "lc-surface-muted", "#181818"),
    ("hover", "lc-hover", "#2a2a2a"),
    ("hover_soft", "lc-hover-soft", "#242424"),
    ("border", "lc-border", "#2a2a2a"),
    ("border_strong", "lc-border-strong", "#3a3a3a"),
    ("text", "lc-text", "#e4e4e4"),
    ("text_strong", "lc-text-strong", "#f8fafc"),
    ("text_muted", "lc-text-muted", "#8a8a8a"),
    ("accent", "lc-accent", "#8a8a8a"),
    ("accent_fg", "lc-accent-fg", "#f5f5f5"),
    ("success", "lc-success", "#d0d0d0"),
    ("warning", "lc-warning", "#f59e0b"),
    ("danger", "lc-danger", "#ff8a8a"),
];

const CUSTOM_COLOR_CSS: &str = r#"
window.lc-custom-colors,
.lc-custom-colors .dashboard,
.lc-custom-colors .page-shell,
.lc-custom-colors .history-view {
    background-color: @lc-bg;
    color: @lc-text;
}

.lc-custom-colors .page-header,
.lc-custom-colors .dashboard-header,
.lc-custom-colors .sidebar,
.lc-custom-colors .settings-toolbar,
.lc-custom-colors .settings-content-shell,
.lc-custom-colors .settings-group,
.lc-custom-colors .settings-rail {
    background-color: @lc-surface;
    border-color: @lc-border;
}

.lc-custom-colors .workspace-card,
.lc-custom-colors .command-panel,
.lc-custom-colors .metric-card,
.lc-custom-colors .detail-row,
.lc-custom-colors .settings-panel,
.lc-custom-colors .settings-content-panel,
.lc-custom-colors .project-template-card,
.lc-custom-colors .chat-composer-box {
    background-color: @lc-surface-raised;
    border-color: @lc-border;
    color: @lc-text;
}

.lc-custom-colors .card-meta,
.lc-custom-colors .workspace-meta,
.lc-custom-colors .detail-label,
.lc-custom-colors .project-tab,
.lc-custom-colors .column-count,
.lc-custom-colors .empty-label,
.lc-custom-colors .card-branch,
.lc-custom-colors .card-diff,
.lc-custom-colors .workspace-row-branch-icon {
    color: @lc-text-muted;
}

.lc-custom-colors .dashboard-title,
.lc-custom-colors .workspace-name,
.lc-custom-colors .card-title,
.lc-custom-colors .metric-value,
.lc-custom-colors .detail-value,
.lc-custom-colors .column-title {
    color: @lc-text;
}

.lc-custom-colors .nav-button-active,
.lc-custom-colors .nav-row-active,
.lc-custom-colors .nav-button:hover,
.lc-custom-colors .nav-row:hover,
.lc-custom-colors .workspace-list row:selected,
.lc-custom-colors .workspace-list row:hover {
    background-color: @lc-hover;
    color: @lc-text;
}

.lc-custom-colors .section-title,
.lc-custom-colors .project-tab-active,
.lc-custom-colors .card-activity,
.lc-custom-colors .workspace-title {
    color: @lc-text;
    border-color: @lc-border;
}

.lc-custom-colors .suggested-action,
.lc-custom-colors .suggested-action:hover {
    background-color: @lc-hover;
    color: @lc-text;
}

.lc-custom-colors .diff-added,
.lc-custom-colors .status-running,
.lc-custom-colors .workspace-status-running {
    color: @lc-success;
}

.lc-custom-colors .chat-context-usage-warning {
    border-color: @lc-warning;
    color: @lc-warning;
}

.lc-custom-colors .diff-removed,
.lc-custom-colors .status-error,
.lc-custom-colors .workspace-status-error,
.lc-custom-colors .ws-check-icon-fail {
    color: @lc-danger;
}
"#;

fn navigate_workspace_from_dashboard(
    app_state: &AppState,
    workspace_name: String,
    default_tab: Option<WorkspaceTab>,
    refresh_view_preferences: &dyn Fn(),
    refresh_workspace_detail: &dyn Fn(),
    show_workspace_stack: &dyn Fn(),
) {
    app_state.navigate_to_workspace_with_default_tab(Some(workspace_name), default_tab);
    refresh_view_preferences();
    refresh_workspace_detail();
    show_workspace_stack();
}

fn with_upgraded_navigation_target<T>(
    upgrade: impl FnOnce() -> Option<T>,
    navigate: impl FnOnce(&T),
) -> bool {
    let Some(target) = upgrade() else {
        return false;
    };
    navigate(&target);
    true
}

fn weak_navigation_callback<C: 'static>(
    coordinator: &Rc<C>,
    navigate: fn(&C, String),
) -> Rc<dyn Fn(String)> {
    let coordinator = Rc::downgrade(coordinator);
    Rc::new(move |workspace_name| {
        if let Some(coordinator) = coordinator.upgrade() {
            navigate(&coordinator, workspace_name);
        }
    })
}

struct WorkspaceNavigationCoordinator {
    app_state: AppState,
    main_stack: gtk::glib::WeakRef<Stack>,
    refresh_view_preferences: Rc<dyn Fn()>,
    refresh_workspace_detail: Rc<dyn Fn()>,
}

impl WorkspaceNavigationCoordinator {
    fn navigate(&self, workspace_name: String) {
        with_upgraded_navigation_target(
            || self.main_stack.upgrade(),
            |main_stack| {
                let default_tab = resolve_workspace_default_tab(
                    &self.app_state.paths.database_path,
                    &workspace_name,
                );
                navigate_workspace_from_dashboard(
                    &self.app_state,
                    workspace_name,
                    Some(default_tab),
                    self.refresh_view_preferences.as_ref(),
                    self.refresh_workspace_detail.as_ref(),
                    &|| main_stack.set_visible_child_name("workspace"),
                );
            },
        );
    }
}

fn build_ui(app: &Application, launch_target: LaunchTarget, debug_mode: bool) {
    let startup = Instant::now();
    let paths = AppPaths::from_env();
    if let Err(err) = logger::init_dev_logger(&paths) {
        eprintln!("failed to initialize dev logger: {err:#}");
    }
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        debug_mode,
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
                WorkspaceStore::open_app(paths.database_path.clone())
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
    let app_state_refresh_subscription = {
        let refresh_hub = refresh_hub.clone();
        app_state.subscribe(move |event, _snapshot| {
            if let AppStateEvent::RefreshRequested(refresh_event) = event {
                refresh_hub.refresh_event(refresh_event.clone());
            }
        })
    };
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Archductor")
        .default_width(1280)
        .default_height(800)
        .build();
    configure_window_chrome(&window);
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
    let view_colors_css = CssProvider::new();
    let color_scope_class = format!(
        "lc-custom-colors-{}",
        NEXT_COLOR_SCOPE_ID.fetch_add(1, Ordering::Relaxed)
    );
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().unwrap(),
        &view_colors_css,
        STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: styles loaded"
    );

    let split = adw::OverlaySplitView::new();
    split.set_min_sidebar_width(120.0);
    split.set_max_sidebar_width(360.0);
    split.set_sidebar_width_unit(adw::LengthUnit::Px);
    split.set_sidebar_width_fraction(APP_SIDEBAR_DEFAULT_WIDTH_PX);
    split.set_pin_sidebar(false);
    split.set_collapsed(false);
    let collapse_sidebar: Rc<dyn Fn()> = {
        let split = split.clone();
        Rc::new(move || split.set_collapsed(true))
    };

    let toast_overlay = adw::ToastOverlay::new();
    let toast_manager = ToastManager::new(&toast_overlay);
    let runtime_error_reporter = Rc::new(RefCell::new(RuntimeErrorReporter::default()));
    let main_stack = Stack::new();
    main_stack.set_hexpand(true);
    main_stack.set_vexpand(true);
    let main_stack_weak = main_stack.downgrade();

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
    let workspace_preference_scope = GBox::new(Orientation::Vertical, 0);
    workspace_preference_scope.set_hexpand(true);
    workspace_preference_scope.set_vexpand(true);
    workspace_preference_scope.append(&workspace_detail);
    apply_view_preferences(
        &workspace_preference_scope,
        &initial_view_preferences,
        &view_colors_css,
        &color_scope_class,
    );
    let refresh_view_preferences: Rc<dyn Fn()> = {
        let state_for_view = app_state.clone();
        let workspace_preference_scope = workspace_preference_scope.clone();
        let colors_css_for_view = view_colors_css.clone();
        let color_scope_class = color_scope_class.clone();
        let db_path_for_view = app_state.workspace_database_path();
        let keybindings_for_view = Rc::clone(&current_keybindings);
        Rc::new(move || {
            let workspace = state_for_view.selected_workspace();
            let preferences =
                resolve_view_preferences(db_path_for_view.clone(), workspace.as_deref());
            apply_view_preferences(
                &workspace_preference_scope,
                &preferences,
                &colors_css_for_view,
                &color_scope_class,
            );
            *keybindings_for_view.borrow_mut() =
                resolve_keybindings(db_path_for_view.clone(), workspace.as_deref());
        })
    };
    let navigation_coordinator = Rc::new(WorkspaceNavigationCoordinator {
        app_state: app_state.clone(),
        main_stack: main_stack_weak,
        refresh_view_preferences: refresh_view_preferences.clone(),
        refresh_workspace_detail: Rc::new(refresh_workspace_detail.clone()),
    });
    let navigate_workspace = weak_navigation_callback(
        &navigation_coordinator,
        WorkspaceNavigationCoordinator::navigate,
    );
    let navigation_coordinator_handle = Rc::new(RefCell::new(Some(navigation_coordinator)));
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: workspace center built"
    );
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: building dashboard"
    );
    let (dashboard, refresh_dashboard) =
        dashboard::build_dashboard_panel(&app_state.paths, navigate_workspace.clone());
    dashboard.set_hexpand(true);
    dashboard.set_vexpand(true);
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: dashboard built"
    );
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: building projects page"
    );
    let (projects_page, refresh_projects) = projects::build_projects_page(
        &app_state.paths,
        app_state.clone(),
        refresh_dashboard.clone(),
        {
            let refresh_workspace_detail = refresh_workspace_detail.clone();
            let refresh_hub = refresh_hub.clone();
            move || {
                refresh_workspace_detail();
                refresh_hub.refresh(RefreshScope::Sidebar);
            }
        },
        navigate_workspace.clone(),
        toast_manager.clone(),
    );
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: projects page built"
    );
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: building settings page"
    );
    let (settings_page, refresh_settings) =
        settings::build_settings_page(&app_state.paths, toast_manager.clone());
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: settings page built"
    );
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: building history page"
    );
    let (history_page, refresh_history) =
        history::build_history_page(&app_state.paths, navigate_workspace, toast_manager.clone());
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: history page built"
    );

    main_stack.add_named(&dashboard, Some("dashboard"));
    main_stack.add_named(&projects_page, Some("projects"));
    main_stack.add_named(&settings_page, Some("settings"));
    main_stack.add_named(&history_page, Some("history"));
    main_stack.add_named(&workspace_preference_scope, Some("workspace"));
    main_stack.set_visible_child_name(match app_state.snapshot().active_page {
        AppPage::Workspace => "workspace",
        AppPage::Projects => "projects",
        AppPage::Settings => "settings",
        AppPage::History => "history",
        _ => "dashboard",
    });

    let (sidebar, refresh_sidebar) = sidebar::build_app_sidebar(
        &app_state,
        refresh_hub.clone(),
        main_stack.clone(),
        window.clone(),
        split.clone(),
        refresh_workspace_detail.clone(),
        refresh_view_preferences.clone(),
        toast_manager.clone(),
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

    let content_overlay = Overlay::new();
    content_overlay.set_child(Some(&main_stack));

    toast_overlay.set_halign(Align::End);
    toast_overlay.set_valign(Align::End);
    toast_overlay.set_overflow(Overflow::Visible);
    toast_overlay.set_margin_end(16);
    toast_overlay.set_margin_bottom(16);
    content_overlay.add_overlay(&toast_overlay);

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

    split.set_sidebar(Some(&sidebar));
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
        let toast_for_palette = toast_manager.clone();
        let refresh_workspace_for_palette = refresh_workspace_detail.clone();
        let keybindings_for_palette = Rc::clone(&current_keybindings);
        Rc::new(move || {
            let keybindings = keybindings_for_palette.borrow().clone();
            let custom_commands = state_for_palette
                .selected_workspace()
                .and_then(|workspace| {
                    WorkspaceStore::open_app(state_for_palette.workspace_database_path())
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
            let toast_for_action = toast_for_palette.clone();
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
                        &toast_for_action,
                        &refresh_workspace_for_action,
                    );
                }),
            );
        })
    };
    let window_content = gtk::Overlay::new();
    window_content.set_hexpand(true);
    window_content.set_vexpand(true);
    window_content.set_child(Some(&split));
    let window_controls = gtk::WindowControls::new(gtk::PackType::Start);
    window_controls.set_decoration_layout(Some("close,minimize,maximize:"));
    window_controls.add_css_class("integrated-window-controls");
    window_controls.set_halign(gtk::Align::Start);
    window_controls.set_valign(gtk::Align::Start);
    window_controls.set_height_request(COLUMN_HEADER_HEIGHT);
    window_controls.set_overflow(gtk::Overflow::Hidden);
    window_content.add_overlay(&window_controls);
    let window_shell = GBox::new(Orientation::Vertical, 0);
    window_shell.set_hexpand(true);
    window_shell.set_vexpand(true);
    if let Some(banner_text) = dev_instance_banner_text() {
        let banner = build_dev_instance_banner(&banner_text);
        window_shell.prepend(&banner);
    }
    window_shell.append(&window_content);
    window.set_child(Some(&window_shell));
    window.present();
    setup::show_blocking_setup_if_needed(&window);
    tracing::info!(
        elapsed_ms = startup.elapsed().as_millis(),
        "gtk startup: window presented"
    );

    spawn_runtime_reconciliation(
        app_state.workspace_database_path(),
        refresh_hub.clone(),
        app_state.clone(),
        runtime_error_reporter.clone(),
        toast_manager.clone(),
        "startup",
        true,
    );
    spawn_workspace_lifecycle_recovery(
        app_state.workspace_database_path(),
        refresh_hub.clone(),
        runtime_error_reporter.clone(),
        toast_manager.clone(),
        "workspace lifecycle recovery",
    );

    let spotlight_event_tx = {
        let (tx, rx) = mpsc::channel();
        let hub_spotlight_events = refresh_hub.clone();
        let state_spotlight_events = app_state.clone();
        let db_path_spotlight_events = app_state.workspace_database_path();
        let runtime_reporter_spotlight_events = runtime_error_reporter.clone();
        let toast_spotlight_events = toast_manager.clone();
        // PER-190: Spotlight file events arrive on a notify thread via std::mpsc;
        // remove this timer when a GLib main-context channel replaces that bridge.
        glib::timeout_add_seconds_local(1, move || {
            if rx.try_iter().next().is_some() {
                spawn_runtime_reconciliation(
                    db_path_spotlight_events.clone(),
                    hub_spotlight_events.clone(),
                    state_spotlight_events.clone(),
                    runtime_reporter_spotlight_events.clone(),
                    toast_spotlight_events.clone(),
                    "spotlight event",
                    false,
                );
            }
            glib::ControlFlow::Continue
        });
        tx
    };

    let spotlight_watcher = Rc::new(RefCell::new(None));
    spawn_spotlight_file_watcher_refresh(
        app_state.workspace_database_path(),
        spotlight_event_tx.clone(),
        spotlight_watcher.clone(),
        runtime_error_reporter.clone(),
        toast_manager.clone(),
        "spotlight watcher",
    );

    {
        let db_path_on_close = app_state.workspace_database_path();
        let hub_on_close = refresh_hub.clone();
        let state_on_close = app_state.clone();
        let spotlight_watcher_on_close = spotlight_watcher.clone();
        let runtime_reporter_on_close = runtime_error_reporter.clone();
        let toast_on_close = toast_manager.clone();
        let navigation_coordinator_on_close = navigation_coordinator_handle.clone();
        let app_state_refresh_subscription = app_state_refresh_subscription;
        window.connect_destroy(move |_| {
            let _keep_refresh_subscription_alive = &app_state_refresh_subscription;
            navigation_coordinator_on_close.borrow_mut().take();
            *spotlight_watcher_on_close.borrow_mut() = None;
            spawn_runtime_reconciliation(
                db_path_on_close.clone(),
                hub_on_close.clone(),
                state_on_close.clone(),
                runtime_reporter_on_close.clone(),
                toast_on_close.clone(),
                "close",
                false,
            );
        });
    }

    {
        let db_path_on_focus = app_state.workspace_database_path();
        let hub_on_focus = refresh_hub.clone();
        let state_on_focus = app_state.clone();
        let runtime_reporter_on_focus = runtime_error_reporter.clone();
        let toast_on_focus = toast_manager.clone();
        window.connect_is_active_notify(move |window| {
            if !window.is_active() {
                return;
            }
            spawn_runtime_reconciliation(
                db_path_on_focus.clone(),
                hub_on_focus.clone(),
                state_on_focus.clone(),
                runtime_reporter_on_focus.clone(),
                toast_on_focus.clone(),
                "focus",
                false,
            );
            spawn_workspace_lifecycle_recovery(
                db_path_on_focus.clone(),
                hub_on_focus.clone(),
                runtime_reporter_on_focus.clone(),
                toast_on_focus.clone(),
                "workspace lifecycle recovery",
            );
        });
    }

    {
        let db_path_background = app_state.workspace_database_path();
        let state_background = app_state.clone();
        let previous_background = Rc::new(RefCell::new(
            background_sync::BackgroundSyncSnapshot::default(),
        ));
        let background_sync_in_flight = Rc::new(Cell::new(false));
        // PER-190: Background sync samples persisted running chat markers while
        // off-focus event routing is still being split from active timelines.
        // Remove when archcar/runtime producers emit typed RefreshEvents for
        // every running workspace regardless of the selected GTK page.
        // PER-190: running chats can update while their workspace is not
        // focused; keep summaries current without loading hidden timelines.
        glib::timeout_add_seconds_local(2, move || {
            if background_sync_in_flight.get() {
                return glib::ControlFlow::Continue;
            }
            background_sync_in_flight.set(true);
            let db_path = db_path_background.clone();
            let state = state_background.clone();
            let previous_background = Rc::clone(&previous_background);
            let in_flight = Rc::clone(&background_sync_in_flight);
            archcar_async::spawn_background_job(
                move || background_sync::load_background_sync_snapshot(&db_path),
                move |result| {
                    in_flight.set(false);
                    let next = match result {
                        Ok(next) => next,
                        Err(err) => {
                            tracing::warn!(error = %err, "gtk background sync snapshot failed");
                            return;
                        }
                    };
                    let previous = previous_background.borrow().clone();
                    if previous.running_threads.is_empty() && next.running_threads.is_empty() {
                        return;
                    }
                    let events = background_sync::coalesce_refresh_events(
                        background_sync::diff_background_sync(&previous, &next),
                    );
                    *previous_background.borrow_mut() = next;
                    for event in events {
                        state.request_refresh(event);
                    }
                },
            );
            glib::ControlFlow::Continue
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
    let toast_kb = toast_manager.clone();
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
                    &toast_kb,
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
                        &toast_kb,
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
    // State tracks the previously sampled workspace notification status.
    let notification_prev: Rc<RefCell<Option<NotificationPrevious>>> = Rc::new(RefCell::new(None));
    {
        let db_path_notif = app_state.workspace_database_path();
        let state_notif = app_state.clone();
        let toast_notif = toast_manager.clone();
        let prev_notif = notification_prev.clone();
        let notification_in_flight = Rc::new(Cell::new(false));
        // PER-190: Notification rules intentionally sample persisted workspace
        // status every five seconds; remove when process-exit events drive rules.
        glib::timeout_add_seconds_local(5, move || {
            if notification_in_flight.get() {
                return glib::ControlFlow::Continue;
            }
            let Some(workspace) = state_notif.selected_workspace() else {
                *prev_notif.borrow_mut() = None;
                return glib::ControlFlow::Continue;
            };
            notification_in_flight.set(true);
            let db_path = db_path_notif.clone();
            let toast_notif = toast_notif.clone();
            let prev_notif = prev_notif.clone();
            let in_flight = Rc::clone(&notification_in_flight);
            archcar_async::spawn_background_job(
                move || load_notification_snapshot(db_path, workspace),
                move |result| {
                    in_flight.set(false);
                    let Ok(Some(snapshot)) = result else {
                        return;
                    };
                    let mut prev = prev_notif.borrow_mut();
                    if let Some(previous) = prev.as_ref() {
                        let prev_check_fail = previous.checks_failed;
                        if previous.workspace == snapshot.workspace
                            && snapshot.notify_session_stop
                            && previous.session_running
                            && !snapshot.session_running
                        {
                            toast_notif.show(ToastMessage::success("Agent session stopped."));
                        }
                        if previous.workspace == snapshot.workspace
                            && snapshot.notify_check_fail
                            && !prev_check_fail
                            && snapshot.checks_failed
                        {
                            toast_notif.show(ToastMessage::warning("Checks failed."));
                        }
                    }
                    *prev = Some(NotificationPrevious {
                        workspace: snapshot.workspace,
                        active_sessions: snapshot.active_sessions,
                        session_running: snapshot.session_running,
                        checks_failed: snapshot.checks_failed,
                    });
                },
            );
            glib::ControlFlow::Continue
        });
    }

    let db_path_runtime_auto = app_state.workspace_database_path();
    let hub_runtime_auto = refresh_hub.clone();
    let state_runtime_auto = app_state.clone();
    let spotlight_watcher_auto = spotlight_watcher.clone();
    let spotlight_event_tx_auto = spotlight_event_tx.clone();
    let runtime_reporter_auto = runtime_error_reporter.clone();
    let toast_auto = toast_manager.clone();
    // PER-190: This is the fallback runtime reconciler for missed focus/file
    // events; remove when all runtime producers emit reliable RefreshHub events.
    glib::timeout_add_seconds_local(5, move || {
        spawn_runtime_reconciliation(
            db_path_runtime_auto.clone(),
            hub_runtime_auto.clone(),
            state_runtime_auto.clone(),
            runtime_reporter_auto.clone(),
            toast_auto.clone(),
            "timer",
            false,
        );
        spawn_spotlight_file_watcher_refresh(
            db_path_runtime_auto.clone(),
            spotlight_event_tx_auto.clone(),
            spotlight_watcher_auto.clone(),
            runtime_reporter_auto.clone(),
            toast_auto.clone(),
            "spotlight watcher",
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
    toast_manager: &ToastManager,
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
                let db_path = state.workspace_database_path();
                let refresh_hub = refresh_hub.clone();
                let toast_manager = toast_manager.clone();
                archcar_async::spawn_background_job(
                    move || {
                        WorkspaceStore::open_app(db_path)
                            .and_then(|store| store.terminal_command(&workspace, &cmd))
                            .map_err(|err| format!("{err:#}"))
                    },
                    move |result| {
                        if let Err(err) = result {
                            toast_manager.show(ToastMessage::warning(format!(
                                "Terminal command failed: {err}"
                            )));
                        }
                        refresh_hub.refresh(RefreshScope::Workspace);
                    },
                );
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

fn refresh_runtime_reconciliation_event(refresh_hub: &RefreshHub, state: &AppState) {
    if let Some(workspace) = state.selected_workspace() {
        refresh_hub.refresh_event(RefreshEvent::WorkspaceRuntimeChanged { workspace });
    } else {
        refresh_hub.refresh_event(RefreshEvent::WorkspaceInventoryChanged);
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

#[derive(Default)]
struct RuntimeErrorReporter {
    last_key: Option<String>,
    suppressed_count: usize,
}

impl RuntimeErrorReporter {
    fn record(&mut self, trigger: &str, message: &str) -> Option<String> {
        let key = format!("{trigger}\n{message}");
        if self.last_key.as_deref() == Some(key.as_str()) {
            self.suppressed_count += 1;
            return None;
        }
        let visible = match self.suppressed_count {
            0 => format!("{trigger} runtime refresh failed: {message}"),
            count => format!(
                "{trigger} runtime refresh failed: {message} ({count} repeated runtime errors suppressed)"
            ),
        };
        self.last_key = Some(key);
        self.suppressed_count = 0;
        Some(visible)
    }
}

fn report_runtime_error(
    reporter: &Rc<RefCell<RuntimeErrorReporter>>,
    toast_manager: &ToastManager,
    trigger: &str,
    err: anyhow::Error,
) {
    tracing::error!(trigger, error = %err, "runtime refresh failed");
    report_runtime_error_message(reporter, toast_manager, trigger, &format!("{err:#}"));
}

fn report_runtime_error_message(
    reporter: &Rc<RefCell<RuntimeErrorReporter>>,
    toast_manager: &ToastManager,
    trigger: &str,
    message: &str,
) {
    if let Some(message) = reporter.borrow_mut().record(trigger, message) {
        toast_manager.show(ToastMessage::warning(message));
    }
}

struct NotificationSnapshot {
    workspace: String,
    notify_session_stop: bool,
    notify_check_fail: bool,
    active_sessions: usize,
    session_running: bool,
    checks_failed: bool,
}

struct NotificationPrevious {
    workspace: String,
    active_sessions: usize,
    session_running: bool,
    checks_failed: bool,
}

fn notification_rule_enabled(rules: &[String], aliases: &[&str]) -> bool {
    rules.iter().any(|rule| {
        let normalized = rule.to_ascii_lowercase().replace('_', "");
        aliases.iter().any(|alias| normalized == *alias)
    })
}

fn load_notification_snapshot(
    db_path: PathBuf,
    workspace: String,
) -> Result<Option<NotificationSnapshot>, String> {
    let store = WorkspaceStore::open_app(db_path).map_err(|err| format!("{err:#}"))?;
    let rules = store
        .workspace_view_defaults(&workspace)
        .map(|defaults| defaults.notification_rules)
        .unwrap_or_default();
    let notify_session_stop = notification_rule_enabled(
        &rules,
        &["sessionstopped", "sessionstop", "onsessionstop", "session"],
    );
    let notify_check_fail = notification_rule_enabled(
        &rules,
        &["checksfailed", "checkfail", "oncheckfail", "checks"],
    );
    if !notify_session_stop && !notify_check_fail {
        return Ok(None);
    }
    let summary = store
        .checks_summary(&workspace)
        .map_err(|err| format!("{err:#}"))?;
    let checks_failed = store
        .list_checks(&workspace)
        .map_err(|err| format!("{err:#}"))?
        .into_iter()
        .next()
        .is_some_and(|check| {
            check.status == ProcessStatus::Exited && check.exit_code.is_some_and(|code| code != 0)
        });
    Ok(Some(NotificationSnapshot {
        workspace,
        notify_session_stop,
        notify_check_fail,
        active_sessions: summary.active_sessions,
        session_running: summary.session_status == Some(ProcessStatus::Running),
        checks_failed,
    }))
}

fn spawn_runtime_reconciliation(
    db_path: PathBuf,
    refresh_hub: RefreshHub,
    state: AppState,
    reporter: Rc<RefCell<RuntimeErrorReporter>>,
    toast_manager: ToastManager,
    trigger: &'static str,
    refresh_all: bool,
) {
    archcar_async::spawn_background_job(
        move || {
            reconcile_runtime_state(&db_path)
                .map(|report| report.changed())
                .map_err(|err| format!("{err:#}"))
        },
        move |result| match result {
            Ok(true) if refresh_all => refresh_hub.refresh(RefreshScope::All),
            Ok(true) => refresh_runtime_reconciliation_event(&refresh_hub, &state),
            Ok(false) => {}
            Err(message) => {
                tracing::error!(trigger, error = %message, "runtime refresh failed");
                report_runtime_error_message(&reporter, &toast_manager, trigger, &message);
            }
        },
    );
}

fn spawn_workspace_lifecycle_recovery(
    db_path: PathBuf,
    refresh_hub: RefreshHub,
    reporter: Rc<RefCell<RuntimeErrorReporter>>,
    toast_manager: ToastManager,
    trigger: &'static str,
) {
    archcar_async::spawn_background_job(
        move || {
            WorkspaceStore::open_app(db_path)
                .and_then(|store| store.recover_workspace_lifecycle_jobs())
                .map_err(|err| format!("{err:#}"))
        },
        move |result| match result {
            Ok(recovered) if recovered > 0 => {
                refresh_hub.refresh_event(RefreshEvent::WorkspaceInventoryChanged);
            }
            Ok(_) => {}
            Err(message) => {
                tracing::error!(trigger, error = %message, "runtime refresh failed");
                report_runtime_error_message(&reporter, &toast_manager, trigger, &message);
            }
        },
    );
}

fn reconcile_runtime_state_for_ui(
    db_path: &Path,
    reporter: &Rc<RefCell<RuntimeErrorReporter>>,
    toast_manager: &ToastManager,
    trigger: &str,
) -> bool {
    match reconcile_runtime_state(db_path) {
        Ok(report) => report.changed(),
        Err(err) => {
            report_runtime_error(reporter, toast_manager, trigger, err);
            false
        }
    }
}

fn reconcile_runtime_state(db_path: &Path) -> anyhow::Result<RuntimeReconciliationReport> {
    let store = WorkspaceStore::open_app(db_path)?;
    let synced = store.spotlight_sync_active_sessions()?;
    let reconciled = store.reconcile_terminal_processes()?;
    Ok(RuntimeReconciliationReport {
        spotlight_sessions_synced: synced.len(),
        terminal_processes_reconciled: reconciled.len(),
    })
}

fn interrupt_running_managed_chats_on_shutdown(paths: &AppPaths) {
    let session_ids = match shutdown_managed_chat_session_ids_from_store(&paths.database_path) {
        Ok(session_ids) => session_ids,
        Err(err) => {
            tracing::warn!(error = %err, "failed to list managed chat sessions for gtk shutdown");
            return;
        }
    };
    if session_ids.is_empty() {
        return;
    }

    std::thread::scope(|scope| {
        let requests = session_ids
            .into_iter()
            .map(|session_id| {
                scope.spawn(move || {
                    let client = ArchcarClient::from_paths(paths);
                    (
                        session_id,
                        client.send_without_spawning(ArchcarRequest::InterruptTurn { session_id }),
                    )
                })
            })
            .collect::<Vec<_>>();

        for request in requests {
            let Ok((session_id, result)) = request.join() else {
                tracing::warn!("gtk shutdown interrupt worker panicked");
                continue;
            };
            match result {
                Ok(response) => {
                    tracing::info!(
                        session_id,
                        ?response,
                        "gtk shutdown interrupted managed chat session"
                    );
                }
                Err(err) => {
                    tracing::warn!(
                        session_id,
                        error = %err,
                        "gtk shutdown could not interrupt managed chat session"
                    );
                }
            }
        }
    });
}

fn shutdown_managed_chat_session_ids_from_store(db_path: &Path) -> anyhow::Result<Vec<i64>> {
    let records = WorkspaceStore::open_app(db_path)?.list_running_sessions()?;
    Ok(shutdown_managed_chat_session_ids(&records))
}

fn shutdown_managed_chat_session_ids(records: &[ProcessRecord]) -> Vec<i64> {
    records
        .iter()
        .filter(|record| shutdown_managed_chat_session(record))
        .map(|record| record.id)
        .collect()
}

fn shutdown_managed_chat_session(record: &ProcessRecord) -> bool {
    record.kind == ProcessKind::Session
        && record.status == ProcessStatus::Running
        && record.chat_thread_id.is_some()
        && (shutdown_session_metadata_is_managed(record.session_harness_metadata.as_deref())
            || matches!(
                shutdown_session_executable_name(&record.command).as_deref(),
                Some("codex" | "claude")
            ))
}

fn shutdown_session_executable_name(command: &str) -> Option<String> {
    let executable = command.split_whitespace().next()?.trim();
    PathBuf::from(executable)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
}

fn shutdown_session_metadata_is_managed(metadata: Option<&str>) -> bool {
    metadata.is_some_and(|metadata| {
        metadata.contains("harness=codex-app-server")
            || metadata.contains("harness=claude-stream-json")
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

enum SpotlightWatchRefresh {
    Unchanged,
    Clear,
    Active(SpotlightFileWatcher),
}

fn load_spotlight_file_watcher(
    db_path: PathBuf,
    event_tx: Sender<()>,
    current_targets: Option<Vec<SpotlightWatchTargetKey>>,
) -> Result<SpotlightWatchRefresh, String> {
    let store = WorkspaceStore::open_app(db_path).map_err(|err| format!("{err:#}"))?;
    let targets = store
        .spotlight_watch_targets()
        .map_err(|err| format!("{err:#}"))?;
    let target_keys = targets
        .iter()
        .map(|target| SpotlightWatchTargetKey {
            session_id: target.session_id,
            workspace_path: target.workspace_path.clone(),
        })
        .collect::<Vec<_>>();

    if current_targets.as_ref() == Some(&target_keys) {
        return Ok(SpotlightWatchRefresh::Unchanged);
    }

    if targets.is_empty() {
        return Ok(SpotlightWatchRefresh::Clear);
    }

    let mut watcher = RecommendedWatcher::new(
        move |event: notify::Result<notify::Event>| {
            if event.is_ok() {
                let _ = event_tx.send(());
            }
        },
        Config::default(),
    )
    .map_err(|err| format!("{err:#}"))?;
    for target in &targets {
        watcher
            .watch(&target.workspace_path, RecursiveMode::Recursive)
            .map_err(|err| format!("{err:#}"))?;
    }

    Ok(SpotlightWatchRefresh::Active(SpotlightFileWatcher {
        targets: target_keys,
        _watcher: watcher,
    }))
}

fn spawn_spotlight_file_watcher_refresh(
    db_path: PathBuf,
    event_tx: Sender<()>,
    current: Rc<RefCell<Option<SpotlightFileWatcher>>>,
    reporter: Rc<RefCell<RuntimeErrorReporter>>,
    toast_manager: ToastManager,
    trigger: &'static str,
) {
    let current_targets = current
        .borrow()
        .as_ref()
        .map(|watcher| watcher.targets.clone());
    archcar_async::spawn_background_job(
        move || load_spotlight_file_watcher(db_path, event_tx, current_targets),
        move |result| match result {
            Ok(SpotlightWatchRefresh::Unchanged) => {}
            Ok(SpotlightWatchRefresh::Clear) => {
                *current.borrow_mut() = None;
            }
            Ok(SpotlightWatchRefresh::Active(watcher)) => {
                *current.borrow_mut() = Some(watcher);
            }
            Err(message) => {
                tracing::error!(trigger, error = %message, "runtime refresh failed");
                report_runtime_error_message(&reporter, &toast_manager, trigger, &message);
            }
        },
    );
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
    let label_widget = text::detail_label(label);
    label_widget.set_xalign(0.0);
    label_widget.set_width_chars(18);
    let value_widget = text::detail_value(value);
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
        .and_then(|path| path.parent().map(|parent| parent.join("archductor")))
        .filter(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from("archductor"))
}

pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(crate) fn default_clone_parent() -> PathBuf {
    archductor_core::platform::home_dir()
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
    #[cfg(windows)]
    let full_cmd = format!("{cmd} & echo. & pause");
    #[cfg(not(windows))]
    let full_cmd = format!("{cmd}; echo; echo '--- Press Enter to close ---'; read");

    #[cfg(windows)]
    {
        if std::process::Command::new("wt.exe")
            .args(["new-tab", "cmd.exe", "/D", "/S", "/C", &full_cmd])
            .spawn()
            .is_ok()
        {
            return;
        }
        if std::process::Command::new("cmd.exe")
            .args(windows_cmd_terminal_fallback_args(&full_cmd))
            .spawn()
            .is_ok()
        {
            return;
        }
        eprintln!("No Windows terminal launcher was found. Run manually:\n  {cmd}");
    }

    #[cfg(not(windows))]
    {
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
}

#[cfg(any(test, windows))]
fn windows_cmd_terminal_fallback_args(full_cmd: &str) -> Vec<String> {
    vec![
        "/D".to_owned(),
        "/S".to_owned(),
        "/C".to_owned(),
        "start".to_owned(),
        "cmd.exe".to_owned(),
        "/K".to_owned(),
        full_cmd.to_owned(),
    ]
}

// ── STYLES ────────────────────────────────────────────────────────────────

fn configure_window_chrome(window: &ApplicationWindow) {
    window.set_titlebar(None::<&gtk::Widget>);
    window.set_decorated(false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use archductor_core::repository::{AddRepository, RepositoryStore};
    use archductor_core::workspace::{CreateWorkspace, ProcessKind, ProcessRecord, ProcessStatus};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::{Mutex, OnceLock};

    #[test]
    fn app_window_chrome_is_platform_appropriate() {
        let source = include_str!("main.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("production source exists");

        assert!(
            production.contains("configure_window_chrome(&window);"),
            "the main window should explicitly configure platform chrome"
        );
        assert!(
            production.contains("use adw::Application;")
                && production.contains("ApplicationWindow,")
                && !production.contains("use adw::{Application, ApplicationWindow};"),
            "custom title bars require GTK ApplicationWindow rather than AdwApplicationWindow"
        );
        assert!(
            production.contains("window.set_titlebar(None::<&gtk::Widget>)")
                && production.contains("window.set_decorated(false)"),
            "the integrated column headers should replace duplicate native chrome"
        );
        assert!(!production.contains("gtk::HeaderBar::builder()"));
    }

    #[test]
    fn workspace_column_headers_share_fixed_draggable_chrome() {
        assert_eq!(COLUMN_HEADER_HEIGHT, 52);
        let sidebar = include_str!("sidebar.rs");
        let session = include_str!("session_surface.rs");
        let workspace = include_str!("workspace_command_center.rs");
        for source in [sidebar, session, workspace] {
            assert!(source.contains("configure_column_header("));
        }
        let drag_source = include_str!("window_chrome.rs");
        assert!(drag_source.contains("compute_point"));
        let production = include_str!("main.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        assert!(production.contains("gtk::WindowControls::new(gtk::PackType::Start)"));
        assert!(production
            .contains("window_controls.set_decoration_layout(Some(\"close,minimize,maximize:\"))"));
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn process_record_for_shutdown(
        id: i64,
        command: &str,
        status: ProcessStatus,
        chat_thread_id: Option<i64>,
    ) -> ProcessRecord {
        ProcessRecord {
            id,
            workspace_id: 1,
            chat_thread_id,
            kind: ProcessKind::Session,
            command: command.to_owned(),
            pid: 1234,
            log_path: PathBuf::from("/tmp/session.log"),
            status,
            started_at: "2026-07-20T00:00:00Z".to_owned(),
            exit_code: None,
            ended_at: None,
            session_harness_metadata: None,
            session_resume_id: None,
        }
    }

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
    fn shutdown_interrupt_targets_only_running_managed_chat_sessions() {
        let records = vec![
            process_record_for_shutdown(1, "claude", ProcessStatus::Running, Some(10)),
            process_record_for_shutdown(2, "codex", ProcessStatus::Running, Some(11)),
            process_record_for_shutdown(3, "bash", ProcessStatus::Running, Some(12)),
            process_record_for_shutdown(4, "claude", ProcessStatus::Stopped, Some(13)),
            process_record_for_shutdown(5, "codex", ProcessStatus::Running, None),
        ];

        assert_eq!(shutdown_managed_chat_session_ids(&records), vec![1, 2]);
    }

    #[test]
    fn shutdown_interrupt_recognizes_managed_harness_metadata() {
        let mut record =
            process_record_for_shutdown(7, "archcar", ProcessStatus::Running, Some(20));
        record.session_harness_metadata = Some("harness=claude-stream-json".to_owned());

        assert_eq!(shutdown_managed_chat_session_ids(&[record]), vec![7]);
    }

    #[test]
    fn gtk_view_preferences_use_app_shared_settings() {
        let _guard = env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        let db_path = temp.path().join("state.db");
        RepositoryStore::open(&db_path)
            .unwrap()
            .add(AddRepository {
                name: Some("demo".to_owned()),
                root_path: repo_path.clone(),
                default_branch: Some("main".to_owned()),
                remote_name: "origin".to_owned(),
                workspace_parent_path: Some(temp.path().join("workspaces/demo")),
            })
            .unwrap();
        WorkspaceStore::open(&db_path)
            .unwrap()
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        fs::remove_file(repo_path.join(".archductor/settings.toml")).unwrap();

        let config_home = temp.path().join("app-config");
        #[cfg(windows)]
        let (config_env, settings_path) = ("APPDATA", config_home.join("Archductor/settings.toml"));
        #[cfg(not(windows))]
        let (config_env, settings_path) = (
            "XDG_CONFIG_HOME",
            config_home.join("archductor/settings.toml"),
        );
        fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        fs::write(settings_path, "[customization.view]\ntheme = \"light\"\n").unwrap();
        let previous_config_home = std::env::var_os(config_env);
        std::env::set_var(config_env, &config_home);

        let preferences = resolve_view_preferences(db_path, Some("berlin"));

        if let Some(previous) = previous_config_home {
            std::env::set_var(config_env, previous);
        } else {
            std::env::remove_var(config_env);
        }
        assert_eq!(preferences.theme, Some(ViewTheme::Light));
    }

    #[test]
    fn windows_terminal_fallback_launches_cmd_with_cmd_syntax() {
        let args = windows_cmd_terminal_fallback_args("cargo test & echo. & pause");

        assert_eq!(
            args,
            vec![
                "/D".to_owned(),
                "/S".to_owned(),
                "/C".to_owned(),
                "start".to_owned(),
                "cmd.exe".to_owned(),
                "/K".to_owned(),
                "cargo test & echo. & pause".to_owned(),
            ]
        );
    }

    #[test]
    fn launch_target_parses_workspace_and_tab_args() {
        let target =
            parse_launch_target(["archductor-gtk", "--workspace", "berlin", "--tab", "checks"])
                .unwrap();

        assert_eq!(target.workspace.as_deref(), Some("berlin"));
        assert_eq!(target.workspace_tab, WorkspaceTab::Checks);
        assert_eq!(target.page, AppPage::Workspace);
    }

    #[test]
    fn dev_application_id_uses_sanitized_instance_suffix() {
        assert_eq!(
            dev_application_id_from_suffix(Some("feature/Dogfood Archductor!!")),
            "io.github.pranavkannepalli.archductor.dev.instance-feature-dogfood-archductor"
        );
        assert_eq!(dev_application_id_from_suffix(Some("...")), APP_ID);
        assert_eq!(dev_application_id_from_suffix(None), APP_ID);
    }

    #[test]
    fn dev_instance_banner_is_inserted_above_app_content() {
        let source = include_str!("main.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("production source exists");

        assert!(production.contains("dev_instance_banner_text()"));
        assert!(production.contains("build_dev_instance_banner(&banner_text)"));
        assert!(production.contains("window_shell.prepend(&banner)"));
        assert!(production.contains("window.set_child(Some(&window_shell))"));
    }

    #[test]
    fn dashboard_navigation_refreshes_selected_workspace_before_showing_it() {
        let state = AppState::new(
            AppPaths::from_env(),
            None,
            WorkspaceTab::Checks,
            AppPage::Dashboard,
        );
        let events = Rc::new(RefCell::new(Vec::new()));

        navigate_workspace_from_dashboard(
            &state,
            "berlin".to_owned(),
            Some(WorkspaceTab::Checks),
            &{
                let state = state.clone();
                let events = events.clone();
                move || {
                    assert_eq!(state.selected_workspace().as_deref(), Some("berlin"));
                    events.borrow_mut().push("preferences");
                }
            },
            &{
                let state = state.clone();
                let events = events.clone();
                move || {
                    assert_eq!(state.selected_workspace().as_deref(), Some("berlin"));
                    events.borrow_mut().push("workspace-detail");
                }
            },
            &{
                let events = events.clone();
                move || events.borrow_mut().push("workspace-stack")
            },
        );

        let snapshot = state.snapshot();
        assert_eq!(snapshot.selected_workspace.as_deref(), Some("berlin"));
        assert_eq!(snapshot.active_page, AppPage::Workspace);
        assert_eq!(snapshot.active_workspace_tab, WorkspaceTab::Checks);
        assert_eq!(
            events.borrow().as_slice(),
            ["preferences", "workspace-detail", "workspace-stack"]
        );
    }

    #[test]
    fn navigation_callback_does_not_keep_its_target_alive() {
        let target = Rc::new(RefCell::new(0));
        let weak_target = Rc::downgrade(&target);
        let navigate = {
            let weak_target = weak_target.clone();
            move || {
                with_upgraded_navigation_target(
                    || weak_target.upgrade(),
                    |target| *target.borrow_mut() += 1,
                )
            }
        };

        assert_eq!(Rc::strong_count(&target), 1);
        assert!(navigate());
        assert_eq!(*target.borrow(), 1);
        drop(target);
        assert!(!navigate());
    }

    #[test]
    fn descendant_navigation_callback_stops_before_state_and_refresh_after_root_drop() {
        struct TestNavigationCoordinator {
            root: std::rc::Weak<()>,
            selected: Rc<RefCell<Option<String>>>,
            events: Rc<RefCell<Vec<&'static str>>>,
        }

        impl TestNavigationCoordinator {
            fn navigate(&self, workspace: String) {
                with_upgraded_navigation_target(
                    || self.root.upgrade(),
                    |_| {
                        *self.selected.borrow_mut() = Some(workspace);
                        self.events.borrow_mut().extend([
                            "preferences",
                            "workspace-detail",
                            "workspace-stack",
                        ]);
                    },
                );
            }
        }

        let root = Rc::new(());
        let selected = Rc::new(RefCell::new(None));
        let events = Rc::new(RefCell::new(Vec::new()));
        let coordinator = Rc::new(TestNavigationCoordinator {
            root: Rc::downgrade(&root),
            selected: selected.clone(),
            events: events.clone(),
        });
        let weak_coordinator = Rc::downgrade(&coordinator);
        let navigate = weak_navigation_callback(&coordinator, TestNavigationCoordinator::navigate);

        assert_eq!(Rc::strong_count(&coordinator), 1);
        navigate("berlin".to_owned());
        assert_eq!(selected.borrow().as_deref(), Some("berlin"));
        assert_eq!(
            events.borrow().as_slice(),
            ["preferences", "workspace-detail", "workspace-stack"]
        );

        events.borrow_mut().clear();
        drop(root);
        navigate("london".to_owned());
        assert_eq!(selected.borrow().as_deref(), Some("berlin"));
        assert!(events.borrow().is_empty());

        drop(coordinator);
        assert!(weak_coordinator.upgrade().is_none());
        navigate("paris".to_owned());
        assert_eq!(selected.borrow().as_deref(), Some("berlin"));
        assert!(events.borrow().is_empty());
    }

    #[test]
    fn launch_target_parses_workspace_deep_link() {
        let target =
            parse_launch_target(["archductor-gtk", "archductor://workspace/berlin?tab=review"])
                .unwrap();

        assert_eq!(target.workspace.as_deref(), Some("berlin"));
        assert_eq!(target.workspace_tab, WorkspaceTab::Review);
        assert_eq!(target.page, AppPage::Workspace);
    }

    #[test]
    fn launch_target_parses_page_deep_links_and_tab_aliases() {
        let history = parse_launch_target(["archductor-gtk", "archductor://history"]).unwrap();
        let terminal = parse_workspace_tab("big-terminal").unwrap();

        assert_eq!(history.page, AppPage::History);
        assert_eq!(history.workspace, None);
        assert_eq!(terminal, WorkspaceTab::Terminal);
    }

    #[test]
    fn debug_mode_accepts_only_explicit_truthy_env_values() {
        assert!(!debug_mode_enabled_from_env(None));
        assert!(!debug_mode_enabled_from_env(Some("")));
        assert!(!debug_mode_enabled_from_env(Some("0")));
        assert!(!debug_mode_enabled_from_env(Some("false")));
        assert!(debug_mode_enabled_from_env(Some("1")));
        assert!(debug_mode_enabled_from_env(Some("true")));
        assert!(debug_mode_enabled_from_env(Some("yes")));
    }

    #[test]
    fn session_logs_route_is_removed() {
        let err = parse_launch_target_with_debug_mode(
            ["archductor-gtk", "--page", "session-logs"],
            false,
        )
        .unwrap_err();
        assert_eq!(err, "unknown page: sessionlogs");

        let debug_err =
            parse_launch_target_with_debug_mode(["archductor-gtk", "--page", "session-logs"], true)
                .unwrap_err();
        assert_eq!(debug_err, "unknown page: sessionlogs");

        let legacy_err = parse_launch_target_with_debug_mode(
            ["archductor-gtk", "--page", "pty-inspector"],
            true,
        )
        .unwrap_err();
        assert_eq!(legacy_err, "unknown page: ptyinspector");
    }

    #[test]
    fn view_preferences_parse_known_theme_accent_and_density() {
        let preferences = ViewPreferences::from_defaults(WorkspaceViewDefaults {
            default_visible_tab: None,
            theme: Some("light".to_owned()),
            accent_color: Some("green".to_owned()),
            colors: BTreeMap::new(),
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
            colors: BTreeMap::new(),
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

    #[test]
    fn view_preferences_apply_custom_color_tokens() {
        let preferences = ViewPreferences::from_defaults(WorkspaceViewDefaults {
            default_visible_tab: None,
            theme: None,
            accent_color: None,
            colors: BTreeMap::from([
                ("accent".to_owned(), "#0ea5e9".to_owned()),
                ("accent_fg".to_owned(), "#001018".to_owned()),
                ("background".to_owned(), "#101820".to_owned()),
            ]),
            density: None,
            keybindings: None,
            terminal_font: None,
            terminal_scrollback: None,
            command_palette_presets: Vec::new(),
            agent_profile_names: Vec::new(),
            notification_rules: Vec::new(),
        });

        assert_eq!(preferences.css_classes(), vec!["lc-custom-colors"]);
        let css = view_colors_css("lc-custom-colors-test", &preferences.colors);
        assert!(css.contains("@define-color lc-accent-lc_custom_colors_test #0ea5e9;"));
        assert!(css.contains("@define-color lc-accent-fg-lc_custom_colors_test #001018;"));
        assert!(css.contains("@define-color lc-bg-lc_custom_colors_test #101820;"));
        assert!(!css.contains(".lc-custom-colors-test .chat-mode-selected"));
        assert!(!css.contains(".lc-custom-colors-test .chat-send-btn-active"));
        assert!(!css.contains(".lc-custom-colors-test .chat-user-bubble"));
        assert!(css.contains(".lc-custom-colors-test .suggested-action"));
        assert!(!css.contains(".lc-custom-colors .chat-mode-selected"));
    }

    #[test]
    fn runtime_error_reporter_dedupes_repeated_reconciliation_failures() {
        let mut reporter = RuntimeErrorReporter::default();

        assert_eq!(
            reporter.record("startup", "database is locked"),
            Some("startup runtime refresh failed: database is locked".to_owned())
        );
        assert_eq!(reporter.record("startup", "database is locked"), None);
        assert_eq!(
            reporter.record("focus", "database is locked"),
            Some(
                "focus runtime refresh failed: database is locked (1 repeated runtime errors suppressed)"
                    .to_owned()
            )
        );
        assert_eq!(
            reporter.record("spotlight watcher", "watch permission denied"),
            Some("spotlight watcher runtime refresh failed: watch permission denied".to_owned())
        );
    }

    #[test]
    fn runtime_reconciliation_sources_spawn_background_jobs() {
        let source = include_str!("main.rs");
        for (name, start_needle, end_needle) in [
            (
                "startup",
                "\"gtk startup: window presented\"",
                "let spotlight_event_tx = {",
            ),
            (
                "close",
                "window.connect_destroy(move |_| {",
                "window.connect_is_active_notify(move |window| {",
            ),
            (
                "focus",
                "window.connect_is_active_notify(move |window| {",
                "let db_path_background = app_state.workspace_database_path();",
            ),
            (
                "timer",
                concat!("glib::timeout", "_add_seconds_local(5, move || {"),
                "glib::ControlFlow::Continue\n    });\n}",
            ),
        ] {
            let start = source.find(start_needle).expect(name);
            let end = source[start..]
                .find(end_needle)
                .map(|offset| start + offset)
                .expect(name);
            let region = &source[start..end];

            assert!(
                region.contains("spawn_runtime_reconciliation("),
                "{name} runtime reconciliation must be scheduled off the GTK thread"
            );
            assert!(
                !region.contains("reconcile_runtime_state_for_ui("),
                "{name} must not reconcile runtime state on the GTK thread"
            );
        }
    }

    #[test]
    fn notification_timer_loads_snapshot_in_background() {
        let source = include_str!("main.rs");
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("main source should contain production code");
        let start = production_source
            .find("Notification tracker:")
            .expect("notification tracker exists");
        let end = production_source[start..]
            .find("let db_path_runtime_auto")
            .map(|offset| start + offset)
            .expect("runtime auto timer follows notification tracker");
        let region = &production_source[start..end];

        assert!(
            region.contains("spawn_background_job("),
            "notification timer must load persisted state off the GTK thread"
        );
        assert!(
            !region.contains("WorkspaceStore::open_app("),
            "notification timer must not open workspace storage on the GTK thread"
        );
    }

    #[test]
    fn notification_timer_coalesces_background_loads() {
        let source = include_str!("main.rs");
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("main source should contain production code");
        let start = production_source
            .find("Notification tracker:")
            .expect("notification tracker exists");
        let end = production_source[start..]
            .find("let db_path_runtime_auto")
            .map(|offset| start + offset)
            .expect("runtime auto timer follows notification tracker");
        let region = &production_source[start..end];

        assert!(region.contains("notification_in_flight.get()"));
        assert!(region.contains("notification_in_flight.set(true);"));
        assert!(region.contains("in_flight.set(false);"));
    }

    #[test]
    fn notification_timer_toasts_on_check_failure_transition() {
        let source = include_str!("main.rs");
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("main source should contain production code");
        let start = production_source
            .find("Notification tracker:")
            .expect("notification tracker exists");
        let end = production_source[start..]
            .find("let db_path_runtime_auto")
            .map(|offset| start + offset)
            .expect("runtime auto timer follows notification tracker");
        let region = &production_source[start..end];

        assert!(region.contains("prev_check_fail"));
        assert!(region.contains("snapshot.notify_check_fail"));
        assert!(region.contains("!prev_check_fail"));
        assert!(region.contains("snapshot.checks_failed"));
        assert!(region.contains("Checks failed."));
        assert!(!region.contains("let _ = snapshot.notify_check_fail"));
    }

    #[test]
    fn spotlight_sources_spawn_background_jobs() {
        let source = include_str!("main.rs");
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("main source should contain production code");

        let event_start = production_source
            .find("let spotlight_event_tx = {")
            .expect("spotlight event bridge exists");
        let event_end = production_source[event_start..]
            .find("let spotlight_watcher =")
            .map(|offset| event_start + offset)
            .expect("spotlight watcher follows event bridge");
        let event_region = &production_source[event_start..event_end];
        assert!(
            event_region.contains("spawn_runtime_reconciliation("),
            "spotlight event bridge must reconcile runtime state off the GTK thread"
        );
        assert!(
            !event_region.contains("reconcile_runtime_state_for_ui("),
            "spotlight event bridge must not reconcile runtime state on the GTK thread"
        );

        let startup_start = production_source
            .find("let spotlight_watcher =")
            .expect("spotlight watcher setup exists");
        let startup_end = production_source[startup_start..]
            .find("let db_path_on_close")
            .map(|offset| startup_start + offset)
            .expect("window close setup follows watcher setup");
        let startup_region = &production_source[startup_start..startup_end];
        assert!(
            startup_region.contains("spawn_spotlight_file_watcher_refresh("),
            "startup spotlight watcher setup must run off the GTK thread"
        );
        assert!(
            !startup_region.contains("refresh_spotlight_file_watcher("),
            "startup must not refresh spotlight watchers on the GTK thread"
        );

        let timer_start = production_source
            .find("let spotlight_watcher_auto = spotlight_watcher.clone();")
            .expect("spotlight timer captures watcher");
        let timer_end = production_source[timer_start..]
            .find("glib::ControlFlow::Continue\n    });\n}")
            .map(|offset| timer_start + offset)
            .expect("spotlight timer ends");
        let timer_region = &production_source[timer_start..timer_end];
        assert!(
            timer_region.contains("spawn_spotlight_file_watcher_refresh("),
            "periodic spotlight watcher refresh must run off the GTK thread"
        );
        assert!(
            !timer_region.contains("refresh_spotlight_file_watcher("),
            "periodic timer must not refresh spotlight watchers on the GTK thread"
        );
    }

    #[test]
    fn palette_run_command_spawns_background_job() {
        let source = include_str!("main.rs");
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("main source should contain production code");
        let start = production_source
            .find("PaletteTarget::RunCommand(cmd) => {")
            .expect("palette run command handler exists");
        let end = production_source[start..]
            .find("fn page_stack_name")
            .map(|offset| start + offset)
            .expect("page_stack_name follows palette handling");
        let region = &production_source[start..end];

        assert!(
            region.contains("spawn_background_job("),
            "palette run commands must execute storage/process work off the GTK thread"
        );
        assert!(
            !region.contains("WorkspaceStore::open_app(state.workspace_database_path())"),
            "palette run commands must not open workspace storage on the GTK thread"
        );
    }

    #[test]
    fn palette_run_command_surfaces_terminal_command_errors() {
        let source = include_str!("main.rs");
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("main source should contain production code");
        let start = production_source
            .find("PaletteTarget::RunCommand(cmd) => {")
            .expect("palette run command handler exists");
        let end = production_source[start..]
            .find("fn page_stack_name")
            .map(|offset| start + offset)
            .expect("page_stack_name follows palette handling");
        let region = &production_source[start..end];

        assert!(region.contains("toast_manager.clone()"));
        assert!(region.contains("if let Err(err) = result"));
        assert!(region.contains("ToastMessage::warning"));
        assert!(region.contains("Terminal command failed"));
        assert!(region.contains("refresh_hub.refresh(RefreshScope::Workspace)"));
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
                "user.name=Archductor",
                "-c",
                "user.email=archductor@example.test",
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
