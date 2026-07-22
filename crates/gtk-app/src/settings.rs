use crate::buttons::text_button;
use archductor_core::paths::AppPaths;
use archductor_core::repository::RepositoryStore;
use archductor_core::settings::{
    app_shared_customization_settings_toml, customization_settings_from_toml,
    default_repository_settings_toml, ensure_repository_config,
    explicit_empty_collection_fields_from_toml, inspect_repository_settings,
    load_app_shared_settings, load_effective_app_shared_settings,
    load_effective_repository_settings, load_repository_settings_for_layer,
    present_collection_fields_from_toml, repository_customization_settings_toml,
    repository_settings_from_toml, save_app_shared_settings_with_collection_intent,
    save_repository_settings_replacing, save_repository_settings_with_collection_intent,
    AgentProfileSettings, GitSettings, PromptKind, PromptSettings, ProviderSettings,
    RepositorySettings, ScriptSettings, SettingsCollectionField, SettingsLayer,
};
use archductor_core::workspace::WorkspaceStore;
use gtk::prelude::*;
use gtk::{
    Align, Box as GBox, Button, CheckButton, ComboBoxText, Entry, Label, Orientation, PolicyType,
    ScrolledWindow, Stack, TextView,
};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use crate::tabs::{set_standard_tab_active, standard_tab, standard_tab_strip};
use crate::toast::{surface_label_error, ToastManager};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SettingsSection {
    id: &'static str,
    title: &'static str,
    description: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsUiScope {
    Shared,
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum GeneralSettingField {
    EnterprisePrivacy,
    ClaudeExecutable,
    CodexExecutable,
    DefaultAgent,
    DefaultModel,
    ClaudeProvider,
    CodexProvider,
    BedrockRegion,
    VertexProject,
    SpotlightTesting,
    EnvironmentVariables,
}

impl GeneralSettingField {
    fn id(self) -> &'static str {
        match self {
            Self::EnterprisePrivacy => "enterprise_privacy",
            Self::ClaudeExecutable => "claude_executable",
            Self::CodexExecutable => "codex_executable",
            Self::DefaultAgent => "default_agent",
            Self::DefaultModel => "default_model",
            Self::ClaudeProvider => "claude_provider",
            Self::CodexProvider => "codex_provider",
            Self::BedrockRegion => "bedrock_region",
            Self::VertexProject => "vertex_project",
            Self::SpotlightTesting => "spotlight_testing",
            Self::EnvironmentVariables => "environment_variables",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GeneralGroupSpec {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    scope: SettingsUiScope,
    rows: Vec<Vec<GeneralSettingField>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsContentPresentation {
    Editor,
    SelectProject,
}

struct GeneralFieldWidgets<'a> {
    privacy_check: &'a CheckButton,
    spotlight_check: &'a CheckButton,
    claude_path_entry: &'a Entry,
    codex_path_entry: &'a Entry,
    default_agent_entry: &'a Entry,
    default_model_entry: &'a Entry,
    claude_provider_entry: &'a Entry,
    codex_provider_entry: &'a Entry,
    bedrock_region_entry: &'a Entry,
    vertex_project_entry: &'a Entry,
    environment_view: &'a ScrolledWindow,
}

impl GeneralFieldWidgets<'_> {
    fn render(&self, field: GeneralSettingField) -> GBox {
        match field {
            GeneralSettingField::EnterprisePrivacy => settings_toggle_row(
                self.privacy_check,
                "Uses privacy-safe behavior for repository and agent operations.",
            ),
            GeneralSettingField::ClaudeExecutable => settings_field(
                "Claude executable",
                "Absolute path or command name used to start Claude Code.",
                self.claude_path_entry,
            ),
            GeneralSettingField::CodexExecutable => settings_field(
                "Codex executable",
                "Absolute path or command name used to start Codex.",
                self.codex_path_entry,
            ),
            GeneralSettingField::DefaultAgent => settings_field(
                "Default agent",
                "Saved as `customization.automation.auto_start_agent`.",
                self.default_agent_entry,
            ),
            GeneralSettingField::DefaultModel => settings_field(
                "Default model",
                "Saved as `customization.agent_profiles.default.model`.",
                self.default_model_entry,
            ),
            GeneralSettingField::ClaudeProvider => settings_field(
                "Claude provider",
                "Provider override used when Claude sessions need a specific backend.",
                self.claude_provider_entry,
            ),
            GeneralSettingField::CodexProvider => settings_field(
                "Codex provider",
                "Provider override used when Codex sessions need a specific backend.",
                self.codex_provider_entry,
            ),
            GeneralSettingField::BedrockRegion => settings_field(
                "Bedrock region",
                "AWS region used for Bedrock requests when that provider is active.",
                self.bedrock_region_entry,
            ),
            GeneralSettingField::VertexProject => settings_field(
                "Vertex project id",
                "Google Cloud project id used for Vertex provider calls.",
                self.vertex_project_entry,
            ),
            GeneralSettingField::SpotlightTesting => settings_toggle_row(
                self.spotlight_check,
                "Turns on spotlight state tracking for workspace sync flows.",
            ),
            GeneralSettingField::EnvironmentVariables => settings_editor_field(
                "Environment variables",
                "One `KEY=value` per line. Leave blank when the project does not need extra environment.",
                self.environment_view,
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SettingsSaveTarget {
    Shared,
    Local(String),
}

fn apply_settings_navigation<T: PartialEq>(
    current: &mut T,
    requested: T,
    flush_pending_autosave: impl FnOnce() -> bool,
) -> bool {
    if *current == requested {
        return true;
    }
    if !flush_pending_autosave() {
        return false;
    }
    *current = requested;
    true
}

fn flush_autosave_target<T: Clone>(
    pending_target: &mut Option<T>,
    save: impl FnOnce(T) -> bool,
) -> bool {
    let Some(target) = pending_target.clone() else {
        return true;
    };
    if !save(target) {
        return false;
    }
    *pending_target = None;
    true
}

#[derive(Clone)]
struct PromptEditor {
    kind: PromptKind,
    buffer: gtk::TextBuffer,
    inherited_label: Label,
}

pub(crate) fn build_settings_page(
    paths: &AppPaths,
    toast_manager: ToastManager,
) -> (GBox, impl Fn() + Clone + 'static) {
    let root = GBox::new(Orientation::Vertical, 0);
    root.add_css_class("dashboard");
    root.add_css_class("page-shell");
    root.add_css_class("settings-page");

    let header = GBox::new(Orientation::Vertical, 8);
    header.add_css_class("dashboard-header");
    header.add_css_class("page-header");
    let title = Label::new(Some("Settings"));
    title.add_css_class("dashboard-title");
    title.set_xalign(0.0);
    let subtitle = Label::new(Some(
        "Choose defaults for every project or customize one project.",
    ));
    subtitle.add_css_class("card-meta");
    subtitle.set_xalign(0.0);
    header.append(&title);
    header.append(&subtitle);
    root.append(&header);

    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    scroll.set_vexpand(true);
    let body = GBox::new(Orientation::Vertical, 16);
    body.add_css_class("detail-body");
    body.add_css_class("page-body");
    scroll.set_child(Some(&body));
    root.append(&scroll);

    let (settings_tabs_scroll, settings_tabs_row) = standard_tab_strip();
    settings_tabs_row.add_css_class("settings-tabs-row");
    let shared_tab = standard_tab("Shared");
    let local_tab = standard_tab("Local");
    set_standard_tab_active(&shared_tab, true);
    let settings_scope = Rc::new(RefCell::new(SettingsUiScope::Shared));
    let scope_description = Label::new(Some(scope_copy(SettingsUiScope::Shared)));
    scope_description.add_css_class("card-meta");
    scope_description.set_xalign(0.0);
    scope_description.set_wrap(true);
    let settings_result = Label::new(Some(""));
    settings_result.add_css_class("settings-status");
    settings_result.add_css_class("card-meta");
    settings_result.set_xalign(0.0);
    settings_result.set_hexpand(true);
    settings_result.set_wrap(true);
    let field_edits: Rc<RefCell<HashSet<&'static str>>> = Rc::new(RefCell::new(HashSet::new()));
    let loaded_settings_target: Rc<RefCell<Option<SettingsSaveTarget>>> =
        Rc::new(RefCell::new(None));
    let loaded_customization_source = Rc::new(RefCell::new(String::new()));
    let forced_save_target: Rc<RefCell<Option<SettingsSaveTarget>>> = Rc::new(RefCell::new(None));
    settings_tabs_row.append(&shared_tab);
    settings_tabs_row.append(&local_tab);
    body.append(&settings_tabs_scroll);
    body.append(&scope_description);

    let settings_toolbar = GBox::new(Orientation::Vertical, 10);
    settings_toolbar.add_css_class("settings-toolbar");
    let settings_top = GBox::new(Orientation::Horizontal, 8);
    settings_top.add_css_class("settings-toolbar-row");
    let project_controls = GBox::new(Orientation::Horizontal, 8);
    project_controls.set_hexpand(true);
    let project_label = Label::new(Some("Project"));
    project_label.add_css_class("settings-field-title");
    let settings_repo_select = ComboBoxText::new();
    settings_repo_select.set_hexpand(true);
    let init_settings_btn = text_button("Initialize");
    let recover_defaults_btn = text_button("Recover Defaults");
    let save_settings_btn = text_button("Save");
    save_settings_btn.add_css_class("suggested-action");
    refresh_repository_select(&settings_repo_select, &paths.database_path, None);
    project_controls.append(&project_label);
    project_controls.append(&settings_repo_select);
    project_controls.append(&init_settings_btn);
    project_controls.append(&recover_defaults_btn);
    project_controls.set_visible(false);
    settings_top.append(&project_controls);
    settings_top.append(&save_settings_btn);
    settings_toolbar.append(&settings_top);
    settings_toolbar.append(&settings_result);
    body.append(&settings_toolbar);

    let inspector = GBox::new(Orientation::Horizontal, 16);
    inspector.add_css_class("settings-inspector");
    let settings_content_area = Stack::new();
    settings_content_area.set_vexpand(true);
    settings_content_area.add_named(&inspector, Some("editor"));
    let select_project_state = GBox::new(Orientation::Vertical, 8);
    select_project_state.set_valign(Align::Center);
    select_project_state.set_halign(Align::Center);
    let select_project_title = Label::new(Some("Select a project"));
    select_project_title.add_css_class("settings-group-title");
    let select_project_copy = Label::new(Some(
        "Choose a project above to view and edit its Local overrides.",
    ));
    select_project_copy.add_css_class("settings-field-copy");
    select_project_copy.set_wrap(true);
    select_project_state.append(&select_project_title);
    select_project_state.append(&select_project_copy);
    settings_content_area.add_named(&select_project_state, Some("select-project"));
    settings_content_area.set_visible_child_name("editor");
    body.append(&settings_content_area);

    let settings_rail = GBox::new(Orientation::Vertical, 6);
    settings_rail.add_css_class("settings-rail");
    settings_rail.set_width_request(230);
    inspector.append(&settings_rail);

    let content_shell = GBox::new(Orientation::Vertical, 0);
    content_shell.add_css_class("settings-content-shell");
    content_shell.set_hexpand(true);
    inspector.append(&content_shell);

    let content_scroll = ScrolledWindow::new();
    content_scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    content_scroll.set_vexpand(true);
    let content_stack = Stack::new();
    content_stack.add_css_class("settings-content-stack");
    content_scroll.set_child(Some(&content_stack));
    content_shell.append(&content_scroll);

    let general_panel = settings_content_panel();
    let prompts_panel = settings_content_panel();
    let scripts_panel = settings_content_panel();
    let git_panel = settings_content_panel();
    let terminal_panel = settings_content_panel();
    let shortcuts_panel = settings_content_panel();
    let notifications_panel = settings_content_panel();
    let advanced_panel = settings_content_panel();
    content_stack.add_named(&general_panel, Some("general"));
    content_stack.add_named(&prompts_panel, Some("prompts"));
    content_stack.add_named(&scripts_panel, Some("scripts"));
    content_stack.add_named(&git_panel, Some("git"));
    content_stack.add_named(&terminal_panel, Some("terminal"));
    content_stack.add_named(&shortcuts_panel, Some("shortcuts"));
    content_stack.add_named(&notifications_panel, Some("notifications"));
    content_stack.add_named(&advanced_panel, Some("advanced"));

    let rail_buttons: Rc<RefCell<Vec<(String, Button)>>> = Rc::new(RefCell::new(Vec::new()));
    let active_section = Rc::new(RefCell::new(String::from("general")));
    let flush_pending_autosave_handle = Rc::new(RefCell::new(None::<Rc<dyn Fn() -> bool>>));
    let sync_rail_state: Rc<dyn Fn()> = {
        let rail_buttons = rail_buttons.clone();
        let active_section = active_section.clone();
        Rc::new(move || {
            let active = active_section.borrow().clone();
            for (id, button) in rail_buttons.borrow().iter() {
                button.remove_css_class("settings-rail-button-active");
                button.add_css_class("settings-rail-button");
                if *id == active {
                    button.remove_css_class("settings-rail-button");
                    button.add_css_class("settings-rail-button-active");
                }
            }
        })
    };

    let rebuild_settings_rail: Rc<dyn Fn(SettingsUiScope)> = {
        let settings_rail = settings_rail.clone();
        let content_stack = content_stack.clone();
        let rail_buttons = rail_buttons.clone();
        let active_section = active_section.clone();
        let sync_rail_state = sync_rail_state.clone();
        let flush_pending_autosave_handle = flush_pending_autosave_handle.clone();
        Rc::new(move |scope| {
            while let Some(child) = settings_rail.first_child() {
                settings_rail.remove(&child);
            }
            rail_buttons.borrow_mut().clear();
            let sections = settings_sections_for_scope(scope);
            if !sections
                .iter()
                .any(|section| section.id == active_section.borrow().as_str())
            {
                *active_section.borrow_mut() = "general".to_owned();
            }
            for section in sections {
                let button = settings_rail_button(section);
                let stack_for_button = content_stack.clone();
                let active_for_button = active_section.clone();
                let sync_for_button = sync_rail_state.clone();
                let flush_for_button = flush_pending_autosave_handle.clone();
                let id = section.id.to_owned();
                button.connect_clicked(move |_| {
                    let mut active = active_for_button.borrow().clone();
                    if !apply_settings_navigation(&mut active, id.clone(), || {
                        flush_for_button
                            .borrow()
                            .as_ref()
                            .is_none_or(|flush| flush())
                    }) {
                        return;
                    }
                    *active_for_button.borrow_mut() = active;
                    stack_for_button.set_visible_child_name(&id);
                    sync_for_button();
                });
                settings_rail.append(&button);
                rail_buttons
                    .borrow_mut()
                    .push((section.id.to_owned(), button));
            }
            content_stack.set_visible_child_name(active_section.borrow().as_str());
            sync_rail_state();
        })
    };
    rebuild_settings_rail(SettingsUiScope::Shared);

    let setup_entry = machine_entry("setup script");
    let run_entry = machine_entry("run script");
    let archive_entry = machine_entry("archive script");
    let test_entry = machine_entry("test command");
    let lint_entry = machine_entry("lint command");
    let typecheck_entry = machine_entry("typecheck command");
    let build_entry = machine_entry("build command");
    let run_mode_entry = machine_entry("run mode: concurrent/nonconcurrent");
    let spotlight_check = CheckButton::with_label("Enable spotlight testing");
    let privacy_check = CheckButton::with_label("Use enterprise privacy mode");
    let archive_on_merge_check = CheckButton::with_label("Archive workspace on merge");
    let delete_branch_check = CheckButton::with_label("Delete branch when archiving");
    let auto_upstream_check = CheckButton::with_label("Auto setup upstream remote");

    let env_view = settings_editor_view(120);
    let claude_path_entry = machine_entry("Claude executable");
    let codex_path_entry = machine_entry("Codex executable");
    let claude_provider_entry = machine_entry("Claude provider");
    let codex_provider_entry = machine_entry("Codex provider");
    let bedrock_region_entry = machine_entry("Bedrock region");
    let vertex_project_entry = machine_entry("Vertex project id");
    let default_agent_entry = machine_entry("codex/claude/opencode");
    let default_model_entry = machine_entry("default model label");

    let general_field_widgets = GeneralFieldWidgets {
        privacy_check: &privacy_check,
        spotlight_check: &spotlight_check,
        claude_path_entry: &claude_path_entry,
        codex_path_entry: &codex_path_entry,
        default_agent_entry: &default_agent_entry,
        default_model_entry: &default_model_entry,
        claude_provider_entry: &claude_provider_entry,
        codex_provider_entry: &codex_provider_entry,
        bedrock_region_entry: &bedrock_region_entry,
        vertex_project_entry: &vertex_project_entry,
        environment_view: &env_view.0,
    };
    let mut general_group_widgets = Vec::new();
    for spec in general_group_specs() {
        let scope = spec.scope;
        let group = settings_group(spec.title, spec.description);
        for row in spec.rows {
            let mut fields = row
                .into_iter()
                .map(|field| general_field_widgets.render(field))
                .collect::<Vec<_>>();
            let row = match fields.len() {
                1 => fields.remove(0),
                2 => {
                    let right = fields.remove(1);
                    settings_field_pair(fields.remove(0), right)
                }
                _ => unreachable!("General setting rows contain one or two fields"),
            };
            group.1.append(&row);
        }
        general_panel.append(&group.0);
        general_group_widgets.push((scope, group.0));
    }

    let script_group = settings_group(
        "Workspace scripts",
        "Commands Archductor can run from the workspace context.",
    );
    scripts_panel.append(&script_group.0);
    script_group.1.append(&settings_field_pair(
        settings_field(
            "Setup script",
            "Runs after a workspace is created or prepared.",
            &setup_entry,
        ),
        settings_field(
            "Run script",
            "Starts the repository runtime from the workspace.",
            &run_entry,
        ),
    ));
    script_group.1.append(&settings_field_pair(
        settings_field(
            "Archive script",
            "Runs before a workspace is archived if you use custom cleanup.",
            &archive_entry,
        ),
        settings_field(
            "Run mode",
            "Usually `concurrent` unless this repository must serialize runs.",
            &run_mode_entry,
        ),
    ));

    let checks_group = settings_group(
        "Checks",
        "Commands used by terminal presets now. TODO: connect these to a first-class check runner.",
    );
    scripts_panel.append(&checks_group.0);
    checks_group.1.append(&settings_field_pair(
        settings_field("Test", "Runs the repository test suite.", &test_entry),
        settings_field("Lint", "Runs repository lint checks.", &lint_entry),
    ));
    checks_group.1.append(&settings_field_pair(
        settings_field(
            "Typecheck",
            "Runs static type checks when available.",
            &typecheck_entry,
        ),
        settings_field("Build", "Runs a production or release build.", &build_entry),
    ));

    let branch_prefix_type_entry = machine_entry("branch prefix type");
    let branch_prefix_entry = machine_entry("branch prefix");
    let git_behavior = settings_group(
        "Git behavior",
        "Naming and branch defaults that shape generated workspaces.",
    );
    git_panel.append(&git_behavior.0);
    git_behavior.1.append(&settings_toggle_row(
        &archive_on_merge_check,
        "Archives the workspace automatically after a successful merge.",
    ));
    git_behavior.1.append(&settings_toggle_row(
        &delete_branch_check,
        "Deletes the local branch during archive cleanup.",
    ));
    git_behavior.1.append(&settings_toggle_row(
        &auto_upstream_check,
        "Automatically configures upstream remotes for new worktree branches.",
    ));
    git_behavior.1.append(&settings_field_pair(
        settings_field(
            "Branch prefix type",
            "Optional naming mode for branch prefixes if your repository uses one.",
            &branch_prefix_type_entry,
        ),
        settings_field(
            "Branch prefix",
            "Short prefix used when Archductor generates branch names.",
            &branch_prefix_entry,
        ),
    ));

    let file_globs_view = settings_editor_view(120);
    let copy_rules = settings_group(
        "Files to copy",
        "Ignored local files copied into new workspaces unless `.worktreeinclude` takes over.",
    );
    git_panel.append(&copy_rules.0);
    copy_rules.1.append(&settings_editor_field(
        "File copy rules",
        "One glob per line. This becomes read-only when `.worktreeinclude` is present.",
        &file_globs_view.0,
    ));

    let terminal_font_entry = machine_entry("terminal font");
    let terminal_scrollback_entry = machine_entry("terminal scrollback lines");
    let terminal_group = settings_group(
        "Terminal",
        "Terminal and transcript display defaults used by workspace surfaces.",
    );
    terminal_panel.append(&terminal_group.0);
    terminal_group.1.append(&settings_field_pair(
        settings_field(
            "Terminal font",
            "Font family and size, for example `JetBrains Mono 12`.",
            &terminal_font_entry,
        ),
        settings_field(
            "Scrollback",
            "Maximum terminal transcript lines retained in the UI.",
            &terminal_scrollback_entry,
        ),
    ));

    let keybindings_entry = machine_entry("vim or action=shortcut list");
    let command_presets_view = settings_editor_view(120);
    let shortcuts_group = settings_group(
        "Shortcuts and commands",
        "Keyboard bindings and terminal command palette presets.",
    );
    shortcuts_panel.append(&shortcuts_group.0);
    shortcuts_group.1.append(&settings_field(
        "Keybindings",
        "Use `vim` or comma-separated `action=shortcut` mappings.",
        &keybindings_entry,
    ));
    shortcuts_group.1.append(&settings_editor_field(
        "Command palette presets",
        "One preset per line. Aliases include test, lint, typecheck, build, ci, status, diff, env, and files.",
        &command_presets_view.0,
    ));

    let notifications_view = settings_editor_view(120);
    let notifications_group = settings_group(
        "Notifications",
        "Notification rule labels. TODO: richer notification routing when the notification engine lands.",
    );
    notifications_panel.append(&notifications_group.0);
    notifications_group.1.append(&settings_editor_field(
        "Notification rules",
        "One rule per line, such as `checks_failed`, `review_requested`, or `agent_stopped`.",
        &notifications_view.0,
    ));

    let prompt_specs = [
        (
            PromptKind::NewWorkspace,
            "New workspace",
            "Saved as a default for compatible agent workflows.",
            110,
        ),
        (
            PromptKind::General,
            "General agent instructions",
            "Included with the first message in each new agent chat.",
            120,
        ),
        (
            PromptKind::ContinueWork,
            "Continue work",
            "Prompt guidance for resuming from the current workspace state.",
            110,
        ),
        (
            PromptKind::SummarizeSession,
            "Summarize session",
            "Saved as a default for compatible agent workflows.",
            110,
        ),
        (
            PromptKind::Handoff,
            "Handoff",
            "Saved as a default for compatible agent workflows.",
            110,
        ),
        (
            PromptKind::CodeReview,
            "Code review",
            "Prompt guidance used when asking an agent to review code.",
            110,
        ),
        (
            PromptKind::CreatePr,
            "Create PR",
            "Prompt template used when generating PR content.",
            110,
        ),
        (
            PromptKind::FixErrors,
            "Fix errors / failing checks",
            "Prompt guidance for CI failures and broken builds.",
            110,
        ),
        (
            PromptKind::ResolveMergeConflicts,
            "Resolve merge conflicts",
            "Prompt guidance for conflict-resolution flows.",
            110,
        ),
        (
            PromptKind::RenameBranch,
            "Rename branch",
            "Saved as a default for compatible agent workflows.",
            110,
        ),
        (
            PromptKind::CommitGeneration,
            "Commit message generation",
            "Prompt guidance for generating repository-specific commit messages.",
            110,
        ),
        (
            PromptKind::TestFixing,
            "Test fixing",
            "Prompt guidance for agents fixing failing tests.",
            110,
        ),
        (
            PromptKind::RefactorStyle,
            "Refactor style",
            "Saved as a default for compatible agent workflows.",
            110,
        ),
        (
            PromptKind::SetupScript,
            "Setup script",
            "Prompt guidance for inferring or updating setup scripts.",
            110,
        ),
        (
            PromptKind::RunScript,
            "Run script",
            "Prompt guidance for inferring or updating run scripts.",
            110,
        ),
    ];
    let prompt_group = settings_group(
        "Prompt editors",
        "Machine-owned prompt bodies. Keep these concise, explicit, and repository-specific.",
    );
    prompts_panel.append(&prompt_group.0);
    let mut prompt_views = Vec::new();
    for (kind, label, help, height) in prompt_specs {
        let view = settings_editor_view(height);
        let field = settings_editor_field(label, help, &view.0);
        let inherited_label = Label::new(Some("Inherited from broader defaults."));
        inherited_label.add_css_class("settings-inherited-label");
        inherited_label.set_xalign(0.0);
        inherited_label.set_visible(false);
        field.append(&inherited_label);
        prompt_group.1.append(&field);
        prompt_views.push(PromptEditor {
            kind,
            buffer: view.1,
            inherited_label,
        });
    }

    let customization_view = settings_editor_view(240);
    let advanced_group = settings_group(
        "Advanced customization",
        "Raw TOML for naming, automation, agent profiles, merge rules, workspace defaults, and view settings.",
    );
    advanced_panel.append(&advanced_group.0);
    advanced_group.1.append(&settings_editor_field(
        "Customization TOML",
        "Use this when the higher-level fields are not enough. Keep it valid TOML.",
        &customization_view.0,
    ));

    let db_path_load_settings = paths.database_path.clone();
    let db_path_init_settings = paths.database_path.clone();
    let settings_repo_select_init = settings_repo_select.clone();
    let settings_result_init = settings_result.clone();
    let toast_init = toast_manager.clone();
    init_settings_btn.connect_clicked(move |_| {
        let repo_name = selected_repository_name(&settings_repo_select_init);
        if repo_name.is_empty() {
            surface_label_error(
                &settings_result_init,
                &toast_init,
                "Repository name is required.",
            );
            return;
        }
        match repository_root(&db_path_init_settings, &repo_name).and_then(|repo_path| {
            ensure_repository_config(&repo_path).map(|report| (repo_path, report))
        }) {
            Ok((repo_path, report)) => {
                let status = match (report.conductor_dir_created, report.shared_settings_created) {
                    (true, true) => "Created .archductor and shared settings.",
                    (false, true) => "Created shared settings.",
                    _ => "Config already exists and is valid.",
                };
                let prompt_pack_status =
                    if report.default_prompt_pack_created || report.active_prompt_pack_created {
                        " Seeded prompt pack defaults."
                    } else {
                        ""
                    };
                let gitignore_status = if report.context_gitignore_updated {
                    " Updated .gitignore for .context/."
                } else {
                    ""
                };
                settings_result_init.set_text(&format!(
                    "{status}{prompt_pack_status}{gitignore_status} {}",
                    repo_path.display()
                ));
                refresh_repository_select(
                    &settings_repo_select_init,
                    &db_path_init_settings,
                    Some(&repo_name),
                );
            }
            Err(err) => surface_label_error(
                &settings_result_init,
                &toast_init,
                format!("Initialize failed: {err:#}"),
            ),
        }
    });

    let settings_repo_select_load = settings_repo_select.clone();
    let settings_result_load = settings_result.clone();
    let loading_settings_load = Rc::new(RefCell::new(false));
    let loading_settings_for_load = loading_settings_load.clone();
    let field_edits_load = field_edits.clone();
    let loaded_settings_target_load = loaded_settings_target.clone();
    let loaded_customization_source_load = loaded_customization_source.clone();
    let setup_entry_load = setup_entry.clone();
    let run_entry_load = run_entry.clone();
    let archive_entry_load = archive_entry.clone();
    let test_entry_load = test_entry.clone();
    let lint_entry_load = lint_entry.clone();
    let typecheck_entry_load = typecheck_entry.clone();
    let build_entry_load = build_entry.clone();
    let run_mode_entry_load = run_mode_entry.clone();
    let spotlight_check_load = spotlight_check.clone();
    let privacy_check_load = privacy_check.clone();
    let archive_on_merge_check_load = archive_on_merge_check.clone();
    let delete_branch_check_load = delete_branch_check.clone();
    let auto_upstream_check_load = auto_upstream_check.clone();
    let claude_path_entry_load = claude_path_entry.clone();
    let codex_path_entry_load = codex_path_entry.clone();
    let claude_provider_entry_load = claude_provider_entry.clone();
    let codex_provider_entry_load = codex_provider_entry.clone();
    let bedrock_region_entry_load = bedrock_region_entry.clone();
    let vertex_project_entry_load = vertex_project_entry.clone();
    let default_agent_entry_load = default_agent_entry.clone();
    let default_model_entry_load = default_model_entry.clone();
    let branch_prefix_type_entry_load = branch_prefix_type_entry.clone();
    let branch_prefix_entry_load = branch_prefix_entry.clone();
    let terminal_font_entry_load = terminal_font_entry.clone();
    let terminal_scrollback_entry_load = terminal_scrollback_entry.clone();
    let keybindings_entry_load = keybindings_entry.clone();
    let command_presets_buffer_load = command_presets_view.1.clone();
    let notifications_buffer_load = notifications_view.1.clone();
    let file_globs_buffer_load = file_globs_view.1.clone();
    let file_globs_text_load = file_globs_view.2.clone();
    let env_buffer_load = env_view.1.clone();
    let customization_buffer_load = customization_view.1.clone();
    let prompt_editors_load = prompt_views.clone();
    let general_group_widgets_load = general_group_widgets.clone();
    let inspector_load = inspector.clone();
    let settings_content_area_load = settings_content_area.clone();
    let save_settings_btn_load = save_settings_btn.clone();
    let shared_settings_path_load = paths.shared_settings_path();
    let toast_load = toast_manager.clone();
    let load_selected_settings = {
        let settings_scope = settings_scope.clone();
        let shared_tab = shared_tab.clone();
        let local_tab = local_tab.clone();
        let project_controls = project_controls.clone();
        let scope_description = scope_description.clone();
        let loading_settings_for_load = loading_settings_for_load.clone();
        Rc::new(move || {
            let scope = *settings_scope.borrow();
            set_standard_tab_active(&shared_tab, scope == SettingsUiScope::Shared);
            set_standard_tab_active(&local_tab, scope == SettingsUiScope::Local);
            project_controls.set_visible(scope_uses_project_selector(scope));
            scope_description.set_text(scope_copy(scope));
            *loading_settings_for_load.borrow_mut() = true;
            field_edits_load.borrow_mut().clear();
            let repo_name = selected_repository_name(&settings_repo_select_load);
            for (group_scope, group) in &general_group_widgets_load {
                group.set_visible(*group_scope == scope);
            }
            let presentation = settings_content_presentation(scope, !repo_name.is_empty());
            settings_content_area_load.set_visible_child_name(match presentation {
                SettingsContentPresentation::Editor => "editor",
                SettingsContentPresentation::SelectProject => "select-project",
            });
            let content_enabled = presentation == SettingsContentPresentation::Editor;
            inspector_load.set_sensitive(content_enabled);
            save_settings_btn_load.set_sensitive(content_enabled);
            if !content_enabled {
                *loaded_settings_target_load.borrow_mut() = None;
                loaded_customization_source_load.borrow_mut().clear();
                settings_result_load.set_text("Select a project to edit Local overrides.");
                *loading_settings_for_load.borrow_mut() = false;
                return;
            }
            let loaded: anyhow::Result<(
                SettingsSaveTarget,
                RepositorySettings,
                RepositorySettings,
                Option<archductor_core::settings::RepositorySettingsInspection>,
                String,
            )> = match scope {
                SettingsUiScope::Shared => load_app_shared_settings(&shared_settings_path_load)
                    .and_then(|raw| {
                        let effective =
                            load_effective_app_shared_settings(&shared_settings_path_load)?;
                        app_shared_customization_settings_toml(&shared_settings_path_load).map(
                            |source| (SettingsSaveTarget::Shared, effective, raw, None, source),
                        )
                    }),
                SettingsUiScope::Local => repository_root(&db_path_load_settings, &repo_name)
                    .and_then(|repo_path| {
                        let raw = load_repository_settings_for_layer(
                            &repo_path,
                            SettingsLayer::LocalOverride,
                        )?;
                        let effective = load_effective_repository_settings(
                            &repo_path,
                            &shared_settings_path_load,
                        )?;
                        let inspection = inspect_repository_settings(&repo_path)?;
                        let source = repository_customization_settings_toml(
                            &repo_path,
                            SettingsLayer::LocalOverride,
                        )?;
                        Ok((
                            SettingsSaveTarget::Local(repo_name.clone()),
                            effective,
                            raw,
                            Some(inspection),
                            source,
                        ))
                    }),
            };
            match loaded {
                Ok((target, settings, raw_settings, inspection, customization_source)) => {
                    *loaded_settings_target_load.borrow_mut() = Some(target.clone());
                    *loaded_customization_source_load.borrow_mut() = customization_source.clone();
                    setup_entry_load.set_text(settings.scripts.setup.as_deref().unwrap_or(""));
                    run_entry_load.set_text(settings.scripts.run.as_deref().unwrap_or(""));
                    archive_entry_load.set_text(settings.scripts.archive.as_deref().unwrap_or(""));
                    test_entry_load.set_text(
                        settings
                            .scripts
                            .test
                            .as_deref()
                            .or(settings.customization.automation.test_command.as_deref())
                            .unwrap_or(""),
                    );
                    lint_entry_load.set_text(
                        settings
                            .scripts
                            .lint
                            .as_deref()
                            .or(settings.customization.automation.lint_command.as_deref())
                            .unwrap_or(""),
                    );
                    typecheck_entry_load.set_text(
                        settings
                            .scripts
                            .typecheck
                            .as_deref()
                            .or(settings
                                .customization
                                .automation
                                .typecheck_command
                                .as_deref())
                            .unwrap_or(""),
                    );
                    build_entry_load.set_text(
                        settings
                            .scripts
                            .build
                            .as_deref()
                            .or(settings.customization.automation.build_command.as_deref())
                            .unwrap_or(""),
                    );
                    run_mode_entry_load
                        .set_text(settings.scripts.run_mode.as_deref().unwrap_or(""));
                    spotlight_check_load.set_active(settings.spotlight_testing.unwrap_or(false));
                    privacy_check_load
                        .set_active(settings.enterprise_data_privacy.unwrap_or(false));
                    archive_on_merge_check_load
                        .set_active(settings.git.archive_on_merge.unwrap_or(false));
                    delete_branch_check_load
                        .set_active(settings.git.delete_branch_on_archive.unwrap_or(false));
                    auto_upstream_check_load.set_active(
                        settings
                            .git
                            .worktree_push_auto_setup_remote
                            .unwrap_or(false),
                    );
                    claude_path_entry_load.set_text(
                        settings
                            .providers
                            .claude_code_executable_path
                            .as_deref()
                            .unwrap_or(""),
                    );
                    codex_path_entry_load.set_text(
                        settings
                            .providers
                            .codex_executable_path
                            .as_deref()
                            .unwrap_or(""),
                    );
                    claude_provider_entry_load
                        .set_text(settings.providers.claude_provider.as_deref().unwrap_or(""));
                    codex_provider_entry_load
                        .set_text(settings.providers.codex_provider.as_deref().unwrap_or(""));
                    bedrock_region_entry_load
                        .set_text(settings.providers.bedrock_region.as_deref().unwrap_or(""));
                    vertex_project_entry_load.set_text(
                        settings
                            .providers
                            .vertex_project_id
                            .as_deref()
                            .unwrap_or(""),
                    );
                    default_agent_entry_load.set_text(
                        settings
                            .customization
                            .automation
                            .auto_start_agent
                            .as_deref()
                            .unwrap_or(""),
                    );
                    default_model_entry_load.set_text(
                        settings
                            .customization
                            .agent_profiles
                            .get("default")
                            .and_then(|profile| profile.model.as_deref())
                            .unwrap_or(""),
                    );
                    branch_prefix_type_entry_load
                        .set_text(settings.git.branch_prefix_type.as_deref().unwrap_or(""));
                    branch_prefix_entry_load
                        .set_text(settings.git.branch_prefix.as_deref().unwrap_or(""));
                    terminal_font_entry_load.set_text(
                        settings
                            .customization
                            .view
                            .terminal_font
                            .as_deref()
                            .unwrap_or(""),
                    );
                    terminal_scrollback_entry_load.set_text(
                        &settings
                            .customization
                            .view
                            .terminal_scrollback
                            .map(|value| value.to_string())
                            .unwrap_or_default(),
                    );
                    keybindings_entry_load.set_text(
                        settings
                            .customization
                            .view
                            .keybindings
                            .as_deref()
                            .unwrap_or(""),
                    );
                    command_presets_buffer_load.set_text(
                        &settings
                            .customization
                            .view
                            .command_palette_presets
                            .join("\n"),
                    );
                    notifications_buffer_load
                        .set_text(&settings.customization.view.notification_rules.join("\n"));
                    if inspection
                        .as_ref()
                        .is_some_and(|inspection| inspection.worktreeinclude_exists)
                    {
                        file_globs_text_load.set_editable(false);
                        file_globs_buffer_load.set_text(
                            &inspection
                                .as_ref()
                                .map(|inspection| inspection.active_file_patterns.join("\n"))
                                .unwrap_or_default(),
                        );
                    } else {
                        file_globs_text_load.set_editable(true);
                        file_globs_buffer_load.set_text(&settings.file_include_globs.join("\n"));
                    }
                    env_buffer_load.set_text(
                        &settings
                            .environment_variables
                            .iter()
                            .map(|(key, value)| format!("{key}={value}"))
                            .collect::<Vec<_>>()
                            .join("\n"),
                    );
                    for editor in &prompt_editors_load {
                        editor
                            .buffer
                            .set_text(prompt_editor_display_text(&settings, editor.kind));
                        editor
                            .inherited_label
                            .set_visible(prompt_editor_shows_inherited_label(
                                scope,
                                &settings,
                                &raw_settings,
                                editor.kind,
                            ));
                    }
                    customization_buffer_load.set_text(&customization_source);
                    settings_result_load.set_text(match target {
                        SettingsSaveTarget::Shared => "Shared defaults loaded.",
                        SettingsSaveTarget::Local(_) => "Local overrides loaded.",
                    });
                }
                Err(err) => {
                    *loaded_settings_target_load.borrow_mut() = None;
                    loaded_customization_source_load.borrow_mut().clear();
                    inspector_load.set_sensitive(false);
                    save_settings_btn_load.set_sensitive(false);
                    surface_label_error(
                        &settings_result_load,
                        &toast_load,
                        format!("Could not load settings: {err:#}"),
                    );
                }
            }
            *loading_settings_for_load.borrow_mut() = false;
        })
    };
    let load_selected_settings_for_repo = load_selected_settings.clone();
    let pending_autosave: Rc<RefCell<Option<gtk::glib::SourceId>>> = Rc::new(RefCell::new(None));
    let pending_autosave_target: Rc<RefCell<Option<SettingsSaveTarget>>> =
        Rc::new(RefCell::new(None));
    let last_save_succeeded = Rc::new(Cell::new(false));
    let db_path_recover_settings = paths.database_path.clone();
    let settings_repo_select_recover = settings_repo_select.clone();
    let settings_result_recover = settings_result.clone();
    let toast_recover = toast_manager.clone();
    let load_selected_settings_for_recover = load_selected_settings.clone();
    let pending_autosave_recover = pending_autosave.clone();
    let pending_autosave_target_recover = pending_autosave_target.clone();
    let recover_confirmation_for_click: Rc<RefCell<Option<(String, SettingsLayer)>>> =
        Rc::new(RefCell::new(None));
    recover_defaults_btn.connect_clicked(move |_| {
        let repo_name = selected_repository_name(&settings_repo_select_recover);
        if repo_name.is_empty() {
            surface_label_error(
                &settings_result_recover,
                &toast_recover,
                "Repository name is required.",
            );
            return;
        }
        let layer = SettingsLayer::LocalOverride;
        let target = (repo_name.clone(), layer);
        if recover_confirmation_for_click.borrow().as_ref() != Some(&target) {
            *recover_confirmation_for_click.borrow_mut() = Some(target);
            settings_result_recover.set_text(&format!(
                "Click Recover Defaults again to replace {:?} settings for {}.",
                layer, repo_name
            ));
            return;
        }
        let settings = match recovered_settings_for_layer(layer) {
            Ok(settings) => settings,
            Err(err) => {
                surface_label_error(
                    &settings_result_recover,
                    &toast_recover,
                    format!("Recovery failed: {err:#}"),
                );
                return;
            }
        };
        if let Some(source_id) = pending_autosave_recover.borrow_mut().take() {
            source_id.remove();
        }
        *pending_autosave_target_recover.borrow_mut() = None;
        *recover_confirmation_for_click.borrow_mut() = None;
        match repository_root(&db_path_recover_settings, &repo_name).and_then(|repo_path| {
            save_recovered_settings_for_layer(&repo_path, layer, &settings)?;
            let refreshed =
                refresh_repository_prompt_snapshots(&db_path_recover_settings, &repo_name)?;
            Ok((repo_path, refreshed))
        }) {
            Ok((repo_path, refreshed)) => {
                load_selected_settings_for_recover();
                settings_result_recover.set_text(&format!(
                    "Local overrides reset for {}. Refreshed {refreshed} prompt snapshot(s).",
                    repo_path.display(),
                ));
            }
            Err(err) => surface_label_error(
                &settings_result_recover,
                &toast_recover,
                format!("Recovery failed: {err:#}"),
            ),
        }
    });
    let loading_settings_for_repo = loading_settings_for_load.clone();
    let settings_scope_for_repo = settings_scope.clone();
    let flush_pending_autosave = {
        let pending_autosave = pending_autosave.clone();
        let pending_autosave_target = pending_autosave_target.clone();
        let forced_save_target = forced_save_target.clone();
        let save_settings_btn = save_settings_btn.clone();
        let last_save_succeeded = last_save_succeeded.clone();
        Rc::new(move || -> bool {
            if let Some(source_id) = pending_autosave.borrow_mut().take() {
                source_id.remove();
            }
            flush_autosave_target(&mut pending_autosave_target.borrow_mut(), |target| {
                *forced_save_target.borrow_mut() = Some(target);
                last_save_succeeded.set(false);
                save_settings_btn.emit_clicked();
                last_save_succeeded.get()
            })
        })
    };
    *flush_pending_autosave_handle.borrow_mut() = Some(flush_pending_autosave.clone());
    let flush_pending_autosave_for_repo = flush_pending_autosave.clone();
    let settings_repo_select_for_change = settings_repo_select.clone();
    let loaded_settings_target_for_repo = loaded_settings_target.clone();
    settings_repo_select.connect_changed(move |_| {
        if !*loading_settings_for_repo.borrow()
            && *settings_scope_for_repo.borrow() == SettingsUiScope::Local
        {
            let requested = selected_repository_name(&settings_repo_select_for_change);
            let mut current = loaded_settings_target_for_repo
                .borrow()
                .as_ref()
                .and_then(|target| match target {
                    SettingsSaveTarget::Local(project) => Some(project.clone()),
                    SettingsSaveTarget::Shared => None,
                })
                .unwrap_or_else(|| requested.clone());
            if !apply_settings_navigation(&mut current, requested, || {
                flush_pending_autosave_for_repo()
            }) {
                *loading_settings_for_repo.borrow_mut() = true;
                settings_repo_select_for_change.set_active_id(Some(&current));
                *loading_settings_for_repo.borrow_mut() = false;
                return;
            }
            load_selected_settings_for_repo();
        }
    });
    let load_selected_settings_for_shared = load_selected_settings.clone();
    let settings_scope_for_shared = settings_scope.clone();
    let rebuild_settings_rail_for_shared = rebuild_settings_rail.clone();
    let flush_pending_autosave_for_shared = flush_pending_autosave.clone();
    shared_tab.connect_clicked(move |_| {
        if *settings_scope_for_shared.borrow() != SettingsUiScope::Shared {
            let mut scope = *settings_scope_for_shared.borrow();
            if !apply_settings_navigation(&mut scope, SettingsUiScope::Shared, || {
                flush_pending_autosave_for_shared()
            }) {
                return;
            }
            *settings_scope_for_shared.borrow_mut() = scope;
            rebuild_settings_rail_for_shared(SettingsUiScope::Shared);
            load_selected_settings_for_shared();
        }
    });
    let load_selected_settings_for_local = load_selected_settings.clone();
    let settings_scope_for_local = settings_scope.clone();
    let rebuild_settings_rail_for_local = rebuild_settings_rail.clone();
    let flush_pending_autosave_for_local = flush_pending_autosave.clone();
    local_tab.connect_clicked(move |_| {
        if *settings_scope_for_local.borrow() != SettingsUiScope::Local {
            let mut scope = *settings_scope_for_local.borrow();
            if !apply_settings_navigation(&mut scope, SettingsUiScope::Local, || {
                flush_pending_autosave_for_local()
            }) {
                return;
            }
            *settings_scope_for_local.borrow_mut() = scope;
            rebuild_settings_rail_for_local(SettingsUiScope::Local);
            load_selected_settings_for_local();
        }
    });

    let autosave = {
        let save_settings_btn = save_settings_btn.clone();
        let loading_settings = loading_settings_for_load.clone();
        let pending_autosave = pending_autosave.clone();
        let pending_autosave_target = pending_autosave_target.clone();
        let loaded_settings_target = loaded_settings_target.clone();
        let forced_save_target = forced_save_target.clone();
        let last_save_succeeded = last_save_succeeded.clone();
        Rc::new(move || {
            if !*loading_settings.borrow() {
                if let Some(source_id) = pending_autosave.borrow_mut().take() {
                    source_id.remove();
                }
                *pending_autosave_target.borrow_mut() = loaded_settings_target.borrow().clone();
                let save_settings_btn = save_settings_btn.clone();
                let pending_autosave_for_timeout = pending_autosave.clone();
                let pending_autosave_target_for_timeout = pending_autosave_target.clone();
                let forced_save_target_for_timeout = forced_save_target.clone();
                let last_save_succeeded_for_timeout = last_save_succeeded.clone();
                let source_id =
                    gtk::glib::timeout_add_local_once(Duration::from_millis(600), move || {
                        *pending_autosave_for_timeout.borrow_mut() = None;
                        flush_autosave_target(
                            &mut pending_autosave_target_for_timeout.borrow_mut(),
                            |target| {
                                *forced_save_target_for_timeout.borrow_mut() = Some(target);
                                last_save_succeeded_for_timeout.set(false);
                                save_settings_btn.emit_clicked();
                                last_save_succeeded_for_timeout.get()
                            },
                        );
                    });
                *pending_autosave.borrow_mut() = Some(source_id);
            }
        })
    };
    for (entry, field) in [
        (&setup_entry, "script_setup"),
        (&run_entry, "script_run"),
        (&archive_entry, "script_archive"),
        (&test_entry, "script_test"),
        (&lint_entry, "script_lint"),
        (&typecheck_entry, "script_typecheck"),
        (&build_entry, "script_build"),
        (&run_mode_entry, "script_run_mode"),
        (&claude_path_entry, "provider_claude_path"),
        (&codex_path_entry, "provider_codex_path"),
        (&claude_provider_entry, "provider_claude"),
        (&codex_provider_entry, "provider_codex"),
        (&bedrock_region_entry, "provider_bedrock_region"),
        (&vertex_project_entry, "provider_vertex_project"),
        (&default_agent_entry, "default_agent"),
        (&default_model_entry, "default_model"),
        (&branch_prefix_type_entry, "git_branch_prefix_type"),
        (&branch_prefix_entry, "git_branch_prefix"),
        (&terminal_font_entry, "terminal_font"),
        (&terminal_scrollback_entry, "terminal_scrollback"),
        (&keybindings_entry, "keybindings"),
    ] {
        connect_entry_autosave(
            entry,
            field,
            field_edits.clone(),
            loading_settings_for_load.clone(),
            autosave.clone(),
        );
    }
    connect_bool_autosave(
        &spotlight_check,
        "spotlight_testing",
        field_edits.clone(),
        loading_settings_for_load.clone(),
        autosave.clone(),
    );
    connect_bool_autosave(
        &privacy_check,
        "enterprise_data_privacy",
        field_edits.clone(),
        loading_settings_for_load.clone(),
        autosave.clone(),
    );
    connect_bool_autosave(
        &archive_on_merge_check,
        "archive_on_merge",
        field_edits.clone(),
        loading_settings_for_load.clone(),
        autosave.clone(),
    );
    connect_bool_autosave(
        &delete_branch_check,
        "delete_branch_on_archive",
        field_edits.clone(),
        loading_settings_for_load.clone(),
        autosave.clone(),
    );
    connect_bool_autosave(
        &auto_upstream_check,
        "worktree_push_auto_setup_remote",
        field_edits.clone(),
        loading_settings_for_load.clone(),
        autosave.clone(),
    );
    for (buffer, field) in [
        (&command_presets_view.1, "command_presets"),
        (&notifications_view.1, "notifications"),
        (&file_globs_view.1, "file_globs"),
        (&env_view.1, "environment"),
        (&customization_view.1, "customization"),
    ] {
        connect_buffer_autosave(
            buffer,
            field,
            field_edits.clone(),
            loading_settings_for_load.clone(),
            autosave.clone(),
        );
    }
    for editor in &prompt_views {
        let inherited_label = editor.inherited_label.clone();
        let settings_scope = settings_scope.clone();
        connect_buffer_autosave_with_edit(
            &editor.buffer,
            editor.kind.as_str(),
            field_edits.clone(),
            loading_settings_for_load.clone(),
            autosave.clone(),
            move || {
                if *settings_scope.borrow() == SettingsUiScope::Local {
                    inherited_label.set_visible(false);
                }
            },
        );
    }

    load_selected_settings();

    let db_path_save_settings = paths.database_path.clone();
    let shared_settings_path_save = paths.shared_settings_path();
    let forced_save_target_for_save = forced_save_target.clone();
    let loaded_settings_target_for_save = loaded_settings_target.clone();
    let loaded_customization_source_for_save = loaded_customization_source.clone();
    let field_edits_for_save = field_edits.clone();
    let toast_save = toast_manager.clone();
    let load_selected_settings_after_save = load_selected_settings.clone();
    let last_save_succeeded_for_save = last_save_succeeded.clone();
    save_settings_btn.connect_clicked(move |_| {
        last_save_succeeded_for_save.set(false);
        let save_target = forced_save_target_for_save
            .borrow_mut()
            .take()
            .or_else(|| loaded_settings_target_for_save.borrow().clone());
        let Some(save_target) = save_target else {
            surface_label_error(&settings_result, &toast_save, "Settings are not loaded.");
            return;
        };
        let (repo_name, repo_path, current_settings) = match &save_target {
            SettingsSaveTarget::Shared => {
                match load_app_shared_settings(&shared_settings_path_save) {
                    Ok(settings) => (None, None, settings),
                    Err(err) => {
                        surface_label_error(
                            &settings_result,
                            &toast_save,
                            format!("Save failed: {err:#}"),
                        );
                        return;
                    }
                }
            }
            SettingsSaveTarget::Local(repo_name) => {
                let repo_path = match repository_root(&db_path_save_settings, repo_name) {
                    Ok(path) => path,
                    Err(err) => {
                        surface_label_error(
                            &settings_result,
                            &toast_save,
                            format!("Save failed: {err:#}"),
                        );
                        return;
                    }
                };
                match load_repository_settings_for_layer(&repo_path, SettingsLayer::LocalOverride) {
                    Ok(settings) => (Some(repo_name.clone()), Some(repo_path), settings),
                    Err(err) => {
                        surface_label_error(
                            &settings_result,
                            &toast_save,
                            format!("Save failed: {err:#}"),
                        );
                        return;
                    }
                }
            }
        };
        let edits = field_edits_for_save.borrow().clone();
        let mut customization = if edits.contains("customization") {
            match customization_settings_from_toml(&text_buffer_text(&customization_view.1)) {
                Ok(customization) => customization,
                Err(err) => {
                    surface_label_error(
                        &settings_result,
                        &toast_save,
                        format!("Save failed: customization TOML invalid: {err:#}"),
                    );
                    return;
                }
            }
        } else {
            current_settings.customization.clone()
        };
        if edits.contains("default_agent") {
            customization.automation.auto_start_agent = optional_entry_text(&default_agent_entry);
        }
        if edits.contains("script_test") {
            customization.automation.test_command = optional_entry_text(&test_entry);
        }
        if edits.contains("script_lint") {
            customization.automation.lint_command = optional_entry_text(&lint_entry);
        }
        if edits.contains("script_typecheck") {
            customization.automation.typecheck_command = optional_entry_text(&typecheck_entry);
        }
        if edits.contains("script_build") {
            customization.automation.build_command = optional_entry_text(&build_entry);
        }
        if edits.contains("default_model") {
            match optional_entry_text(&default_model_entry) {
                Some(model) => {
                    customization
                        .agent_profiles
                        .entry("default".to_owned())
                        .or_insert_with(AgentProfileSettings::default)
                        .model = Some(model);
                }
                None => {
                    if let Some(profile) = customization.agent_profiles.get_mut("default") {
                        profile.model = None;
                    }
                }
            }
        }
        if edits.contains("terminal_font") {
            customization.view.terminal_font = optional_entry_text(&terminal_font_entry);
        }
        if edits.contains("terminal_scrollback") {
            customization.view.terminal_scrollback =
                match optional_entry_text(&terminal_scrollback_entry) {
                    Some(value) => match value.parse::<u32>() {
                        Ok(parsed) => Some(parsed),
                        Err(err) => {
                            surface_label_error(
                                &settings_result,
                                &toast_save,
                                format!("Save failed: terminal scrollback must be a number: {err}"),
                            );
                            return;
                        }
                    },
                    None => None,
                };
        }
        if edits.contains("keybindings") {
            customization.view.keybindings = optional_entry_text(&keybindings_entry);
        }
        if edits.contains("command_presets") {
            customization.view.command_palette_presets =
                parse_text_lines(&text_buffer_text(&command_presets_view.1));
        }
        if edits.contains("notifications") {
            customization.view.notification_rules =
                parse_text_lines(&text_buffer_text(&notifications_view.1));
        }

        let environment_variables = if edits.contains("environment") {
            match parse_environment_lines(&text_buffer_text(&env_view.1)) {
                Ok(environment_variables) => environment_variables,
                Err(err) => {
                    surface_label_error(
                        &settings_result,
                        &toast_save,
                        format!("Save failed: {err:#}"),
                    );
                    return;
                }
            }
        } else {
            current_settings.environment_variables.clone()
        };
        let mut prompt_settings = current_settings.prompts.clone().unwrap_or_default();
        for editor in &prompt_views {
            if edits.contains(editor.kind.as_str()) {
                set_prompt_value(
                    &mut prompt_settings,
                    editor.kind,
                    optional_buffer_text(&editor.buffer),
                );
            }
        }
        let settings = RepositorySettings {
            file_include_globs: setting_for_save(
                edits.contains("file_globs") && file_globs_view.2.is_editable(),
                current_settings.file_include_globs.clone(),
                parse_text_lines(&text_buffer_text(&file_globs_view.1)),
            ),
            env_file_refs: current_settings.env_file_refs.clone(),
            spotlight_testing: setting_for_save(
                edits.contains("spotlight_testing"),
                current_settings.spotlight_testing,
                Some(spotlight_check.is_active()),
            ),
            enterprise_data_privacy: setting_for_save(
                edits.contains("enterprise_data_privacy"),
                current_settings.enterprise_data_privacy,
                Some(privacy_check.is_active()),
            ),
            scripts: ScriptSettings {
                setup: setting_for_save(
                    edits.contains("script_setup"),
                    current_settings.scripts.setup.clone(),
                    optional_entry_text(&setup_entry),
                ),
                run: setting_for_save(
                    edits.contains("script_run"),
                    current_settings.scripts.run.clone(),
                    optional_entry_text(&run_entry),
                ),
                archive: setting_for_save(
                    edits.contains("script_archive"),
                    current_settings.scripts.archive.clone(),
                    optional_entry_text(&archive_entry),
                ),
                test: setting_for_save(
                    edits.contains("script_test"),
                    current_settings.scripts.test.clone(),
                    optional_entry_text(&test_entry),
                ),
                lint: setting_for_save(
                    edits.contains("script_lint"),
                    current_settings.scripts.lint.clone(),
                    optional_entry_text(&lint_entry),
                ),
                typecheck: setting_for_save(
                    edits.contains("script_typecheck"),
                    current_settings.scripts.typecheck.clone(),
                    optional_entry_text(&typecheck_entry),
                ),
                build: setting_for_save(
                    edits.contains("script_build"),
                    current_settings.scripts.build.clone(),
                    optional_entry_text(&build_entry),
                ),
                run_mode: setting_for_save(
                    edits.contains("script_run_mode"),
                    current_settings.scripts.run_mode.clone(),
                    optional_entry_text(&run_mode_entry),
                ),
            },
            environment_variables,
            prompt_pack: current_settings.prompt_pack.clone(),
            prompts: (!prompt_settings_is_empty(&prompt_settings)).then_some(prompt_settings),
            providers: ProviderSettings {
                claude_code_executable_path: setting_for_save(
                    edits.contains("provider_claude_path"),
                    current_settings
                        .providers
                        .claude_code_executable_path
                        .clone(),
                    optional_entry_text(&claude_path_entry),
                ),
                codex_executable_path: setting_for_save(
                    edits.contains("provider_codex_path"),
                    current_settings.providers.codex_executable_path.clone(),
                    optional_entry_text(&codex_path_entry),
                ),
                claude_provider: setting_for_save(
                    edits.contains("provider_claude"),
                    current_settings.providers.claude_provider.clone(),
                    optional_entry_text(&claude_provider_entry),
                ),
                codex_provider: setting_for_save(
                    edits.contains("provider_codex"),
                    current_settings.providers.codex_provider.clone(),
                    optional_entry_text(&codex_provider_entry),
                ),
                bedrock_region: setting_for_save(
                    edits.contains("provider_bedrock_region"),
                    current_settings.providers.bedrock_region.clone(),
                    optional_entry_text(&bedrock_region_entry),
                ),
                vertex_project_id: setting_for_save(
                    edits.contains("provider_vertex_project"),
                    current_settings.providers.vertex_project_id.clone(),
                    optional_entry_text(&vertex_project_entry),
                ),
            },
            git: GitSettings {
                delete_branch_on_archive: setting_for_save(
                    edits.contains("delete_branch_on_archive"),
                    current_settings.git.delete_branch_on_archive,
                    Some(delete_branch_check.is_active()),
                ),
                archive_on_merge: setting_for_save(
                    edits.contains("archive_on_merge"),
                    current_settings.git.archive_on_merge,
                    Some(archive_on_merge_check.is_active()),
                ),
                worktree_push_auto_setup_remote: setting_for_save(
                    edits.contains("worktree_push_auto_setup_remote"),
                    current_settings.git.worktree_push_auto_setup_remote,
                    Some(auto_upstream_check.is_active()),
                ),
                branch_prefix_type: setting_for_save(
                    edits.contains("git_branch_prefix_type"),
                    current_settings.git.branch_prefix_type.clone(),
                    optional_entry_text(&branch_prefix_type_entry),
                ),
                branch_prefix: setting_for_save(
                    edits.contains("git_branch_prefix"),
                    current_settings.git.branch_prefix.clone(),
                    optional_entry_text(&branch_prefix_entry),
                ),
            },
            customization,
        };
        let collection_intent = match collection_intent_for_settings_save(
            &edits,
            &settings,
            &text_buffer_text(&customization_view.1),
            &loaded_customization_source_for_save.borrow(),
        ) {
            Ok(fields) => fields,
            Err(err) => {
                surface_label_error(
                    &settings_result,
                    &toast_save,
                    format!("Save failed: customization TOML invalid: {err:#}"),
                );
                return;
            }
        };
        let save_result = match (&save_target, repo_name.as_deref(), repo_path.as_ref()) {
            (SettingsSaveTarget::Shared, _, _) => save_app_shared_settings_with_collection_intent(
                &shared_settings_path_save,
                &settings,
                &collection_intent.explicit_empty,
                &collection_intent.unset,
            )
            .and_then(|()| refresh_all_prompt_snapshots(&db_path_save_settings)),
            (SettingsSaveTarget::Local(_), Some(repo_name), Some(repo_path)) => {
                save_repository_settings_with_collection_intent(
                    repo_path,
                    SettingsLayer::LocalOverride,
                    &settings,
                    &collection_intent.explicit_empty,
                    &collection_intent.unset,
                )
                .and_then(|()| {
                    refresh_repository_prompt_snapshots(&db_path_save_settings, repo_name)
                })
            }
            _ => Err(anyhow::anyhow!("invalid settings save target")),
        };
        match save_result {
            Ok(_) => {
                last_save_succeeded_for_save.set(true);
                field_edits_for_save.borrow_mut().clear();
                load_selected_settings_after_save();
                let status = match save_target {
                    SettingsSaveTarget::Shared => "Shared defaults saved.".to_owned(),
                    SettingsSaveTarget::Local(project) => {
                        format!("Local overrides saved for {project}.")
                    }
                };
                settings_result.set_text(&status);
            }
            Err(err) => {
                surface_label_error(
                    &settings_result,
                    &toast_save,
                    format!("Save failed: {err:#}"),
                );
            }
        }
    });

    (root, || {})
}

fn settings_sections() -> Vec<SettingsSection> {
    vec![
        SettingsSection {
            id: "general",
            title: "General",
            description: "Repository defaults, environment, agents, and providers.",
        },
        SettingsSection {
            id: "prompts",
            title: "Prompts",
            description: "Prompt bodies for agent tasks and workflows.",
        },
        SettingsSection {
            id: "scripts",
            title: "Scripts",
            description: "Setup, run, archive, and local check commands.",
        },
        SettingsSection {
            id: "git",
            title: "Git & Workspaces",
            description: "Branch behavior and file copy rules.",
        },
        SettingsSection {
            id: "terminal",
            title: "Terminal",
            description: "Terminal and transcript display defaults.",
        },
        SettingsSection {
            id: "shortcuts",
            title: "Shortcuts",
            description: "Keybindings and command palette presets.",
        },
        SettingsSection {
            id: "notifications",
            title: "Notifications",
            description: "Notification routing labels.",
        },
        SettingsSection {
            id: "advanced",
            title: "Advanced",
            description: "Raw TOML for deeper repository customization.",
        },
    ]
}

fn scope_uses_project_selector(scope: SettingsUiScope) -> bool {
    matches!(scope, SettingsUiScope::Local)
}

fn scope_copy(scope: SettingsUiScope) -> &'static str {
    match scope {
        SettingsUiScope::Shared => {
            "Shared: Defaults used by every Archductor project on this machine."
        }
        SettingsUiScope::Local => "Local: Overrides for the selected project and its workspaces.",
    }
}

fn setting_for_save<T>(edited: bool, current: T, displayed: T) -> T {
    if edited {
        displayed
    } else {
        current
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct SettingsCollectionSaveIntent {
    explicit_empty: Vec<SettingsCollectionField>,
    unset: Vec<SettingsCollectionField>,
}

fn collection_intent_for_settings_save(
    edits: &HashSet<&'static str>,
    settings: &RepositorySettings,
    customization_toml: &str,
    original_customization_toml: &str,
) -> anyhow::Result<SettingsCollectionSaveIntent> {
    let customization_edited = edits.contains("customization");
    let mut fields = if customization_edited {
        explicit_empty_collection_fields_from_toml(customization_toml)?
    } else {
        Vec::new()
    };
    let mut unset = Vec::new();
    if customization_edited {
        let present = present_collection_fields_from_toml(customization_toml)?;
        let original_present = present_collection_fields_from_toml(original_customization_toml)?;
        for field in original_present {
            if is_customization_collection_field(&field) && !present.contains(&field) {
                unset.push(field);
            }
        }
    }
    let mut add = |field| {
        if !fields.contains(&field) {
            fields.push(field);
        }
    };
    if edits.contains("file_globs") && settings.file_include_globs.is_empty() {
        add(SettingsCollectionField::FileIncludeGlobs);
    }
    if edits.contains("environment") && settings.environment_variables.is_empty() {
        add(SettingsCollectionField::EnvironmentVariables);
    }
    if edits.contains("command_presets")
        && settings
            .customization
            .view
            .command_palette_presets
            .is_empty()
    {
        add(SettingsCollectionField::CommandPalettePresets);
    }
    if edits.contains("notifications") && settings.customization.view.notification_rules.is_empty()
    {
        add(SettingsCollectionField::NotificationRules);
    }
    Ok(SettingsCollectionSaveIntent {
        explicit_empty: fields,
        unset,
    })
}

fn is_customization_collection_field(field: &SettingsCollectionField) -> bool {
    matches!(
        field,
        SettingsCollectionField::AgentProfiles
            | SettingsCollectionField::AgentProfileMcpServers(_)
            | SettingsCollectionField::PrBodySections
            | SettingsCollectionField::RequiredLocalFiles
            | SettingsCollectionField::ViewColors
            | SettingsCollectionField::DashboardColumns
            | SettingsCollectionField::NotificationRules
            | SettingsCollectionField::CommandPalettePresets
    )
}

fn settings_sections_for_scope(scope: SettingsUiScope) -> Vec<SettingsSection> {
    let visible_ids: &[&str] = match scope {
        SettingsUiScope::Shared => &[
            "general",
            "prompts",
            "terminal",
            "shortcuts",
            "notifications",
        ],
        SettingsUiScope::Local => &["general", "prompts", "scripts", "git", "advanced"],
    };
    settings_sections()
        .into_iter()
        .filter(|section| visible_ids.contains(&section.id))
        .collect()
}

fn general_group_specs() -> Vec<GeneralGroupSpec> {
    use GeneralSettingField::*;

    vec![
        GeneralGroupSpec {
            id: "privacy",
            title: "Privacy",
            description: "App-wide privacy defaults used across Archductor projects.",
            scope: SettingsUiScope::Shared,
            rows: vec![vec![EnterprisePrivacy]],
        },
        GeneralGroupSpec {
            id: "agents",
            title: "Agents and providers",
            description:
                "Executable paths, default agent, and provider routing for local agent launches.",
            scope: SettingsUiScope::Shared,
            rows: vec![
                vec![ClaudeExecutable, CodexExecutable],
                vec![DefaultAgent, DefaultModel],
                vec![ClaudeProvider, CodexProvider],
            ],
        },
        GeneralGroupSpec {
            id: "provider_platforms",
            title: "Platform settings",
            description: "Provider-specific machine values for Bedrock and Vertex setups.",
            scope: SettingsUiScope::Shared,
            rows: vec![vec![BedrockRegion, VertexProject]],
        },
        GeneralGroupSpec {
            id: "project_behavior",
            title: "Project behavior",
            description: "High-level behavior for the selected project and its workspaces.",
            scope: SettingsUiScope::Local,
            rows: vec![vec![SpotlightTesting]],
        },
        GeneralGroupSpec {
            id: "environment",
            title: "Environment",
            description:
                "Environment values passed into scripts and sessions for the selected project.",
            scope: SettingsUiScope::Local,
            rows: vec![vec![EnvironmentVariables]],
        },
    ]
}

fn general_group_ids_for_scope(scope: SettingsUiScope) -> Vec<&'static str> {
    general_group_specs()
        .into_iter()
        .filter(|spec| spec.scope == scope)
        .map(|spec| spec.id)
        .collect()
}

fn general_field_ids_for_scope(scope: SettingsUiScope) -> Vec<&'static str> {
    general_group_specs()
        .into_iter()
        .filter(|spec| spec.scope == scope)
        .flat_map(|spec| spec.rows.into_iter().flatten())
        .map(GeneralSettingField::id)
        .collect()
}

fn settings_content_enabled(scope: SettingsUiScope, has_project: bool) -> bool {
    settings_content_presentation(scope, has_project) == SettingsContentPresentation::Editor
}

fn settings_content_presentation(
    scope: SettingsUiScope,
    has_project: bool,
) -> SettingsContentPresentation {
    if scope == SettingsUiScope::Local && !has_project {
        SettingsContentPresentation::SelectProject
    } else {
        SettingsContentPresentation::Editor
    }
}

fn settings_rail_button(section: SettingsSection) -> Button {
    let button = text_button(section.title);
    button.add_css_class("settings-rail-button");
    button.set_halign(Align::Fill);
    button.set_hexpand(true);
    button.set_tooltip_text(Some(section.description));
    button
}

fn settings_content_panel() -> GBox {
    let panel = GBox::new(Orientation::Vertical, 12);
    panel.add_css_class("settings-content-panel");
    panel.set_margin_top(4);
    panel.set_margin_bottom(4);
    panel.set_margin_start(4);
    panel.set_margin_end(4);
    panel
}

fn settings_group(title: &str, description: &str) -> (GBox, GBox) {
    let shell = GBox::new(Orientation::Vertical, 10);
    shell.add_css_class("settings-group");

    let header = GBox::new(Orientation::Vertical, 4);
    let title_label = Label::new(Some(title));
    title_label.add_css_class("settings-group-title");
    title_label.set_xalign(0.0);
    let copy_label = Label::new(Some(description));
    copy_label.add_css_class("settings-group-copy");
    copy_label.set_wrap(true);
    copy_label.set_xalign(0.0);
    header.append(&title_label);
    header.append(&copy_label);
    shell.append(&header);

    let body = GBox::new(Orientation::Vertical, 10);
    body.add_css_class("settings-group-body");
    shell.append(&body);
    (shell, body)
}

fn settings_field(label: &str, help: &str, widget: &impl IsA<gtk::Widget>) -> GBox {
    let field = GBox::new(Orientation::Vertical, 4);
    field.add_css_class("settings-field");
    field.set_hexpand(true);

    let title = Label::new(Some(label));
    title.add_css_class("settings-field-title");
    title.set_xalign(0.0);
    let copy = Label::new(Some(help));
    copy.add_css_class("settings-field-copy");
    copy.set_wrap(true);
    copy.set_xalign(0.0);
    field.append(&title);
    field.append(&copy);
    field.append(widget);
    field
}

fn settings_editor_field(label: &str, help: &str, widget: &impl IsA<gtk::Widget>) -> GBox {
    let field = settings_field(label, help, widget);
    field.add_css_class("settings-editor-field");
    field
}

fn settings_field_pair(left: GBox, right: GBox) -> GBox {
    let row = GBox::new(Orientation::Horizontal, 12);
    row.add_css_class("settings-field-row");
    row.append(&left);
    row.append(&right);
    row
}

fn settings_toggle_row(check: &CheckButton, help: &str) -> GBox {
    let row = GBox::new(Orientation::Vertical, 4);
    row.add_css_class("settings-toggle-row");
    check.set_halign(Align::Start);
    let copy = Label::new(Some(help));
    copy.add_css_class("settings-field-copy");
    copy.set_wrap(true);
    copy.set_xalign(0.0);
    row.append(check);
    row.append(&copy);
    row
}

fn machine_entry(placeholder: &str) -> Entry {
    let entry = Entry::new();
    entry.set_placeholder_text(Some(placeholder));
    entry.add_css_class("settings-machine-entry");
    entry
}

fn settings_editor_view(height: i32) -> (ScrolledWindow, gtk::TextBuffer, TextView) {
    let view = TextView::new();
    view.set_monospace(true);
    view.add_css_class("settings-editor");
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_size_request(-1, height);
    let buffer = view.buffer();
    let scroll = ScrolledWindow::new();
    scroll.add_css_class("settings-editor-shell");
    scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    scroll.set_child(Some(&view));
    (scroll, buffer, view)
}

fn repository_root(db_path: &PathBuf, name: &str) -> anyhow::Result<PathBuf> {
    RepositoryStore::open(db_path)?
        .list()?
        .into_iter()
        .find(|repo| repo.id.to_string() == name || repo.name == name)
        .map(|repo| repo.root_path)
        .ok_or_else(|| anyhow::anyhow!("repository {name} not found"))
}

fn refresh_repository_prompt_snapshots(db_path: &PathBuf, name: &str) -> anyhow::Result<usize> {
    let repository = RepositoryStore::open(db_path)?.get_by_name(name)?;
    WorkspaceStore::open_app(db_path)?.refresh_repository_prompt_snapshots(repository.id)
}

fn refresh_all_prompt_snapshots(db_path: &PathBuf) -> anyhow::Result<usize> {
    let repositories = RepositoryStore::open(db_path)?.list()?;
    let store = WorkspaceStore::open_app(db_path)?;
    repositories.into_iter().try_fold(0, |total, repository| {
        store
            .refresh_repository_prompt_snapshots(repository.id)
            .map(|refreshed| total + refreshed)
    })
}

fn selected_repository_name(select: &ComboBoxText) -> String {
    select
        .active_id()
        .or_else(|| select.active_text())
        .map(|value| value.to_string())
        .unwrap_or_default()
}

fn bool_setting_for_layer(
    layer: SettingsLayer,
    current: Option<bool>,
    active: bool,
    edited: bool,
) -> Option<bool> {
    match layer {
        SettingsLayer::RepositoryShared => Some(active),
        SettingsLayer::LocalOverride => {
            if edited {
                Some(active)
            } else {
                current
            }
        }
    }
}

fn run_mode_setting_for_layer(layer: SettingsLayer, value: Option<String>) -> Option<String> {
    match layer {
        SettingsLayer::RepositoryShared => value.or_else(|| Some("concurrent".to_owned())),
        SettingsLayer::LocalOverride => value,
    }
}

fn recovered_settings_for_layer(layer: SettingsLayer) -> anyhow::Result<RepositorySettings> {
    match layer {
        SettingsLayer::RepositoryShared => {
            repository_settings_from_toml(&default_repository_settings_toml()?)
        }
        SettingsLayer::LocalOverride => Ok(RepositorySettings::default()),
    }
}

fn save_recovered_settings_for_layer(
    repo_path: &std::path::Path,
    layer: SettingsLayer,
    settings: &RepositorySettings,
) -> anyhow::Result<()> {
    save_repository_settings_replacing(repo_path, layer, settings)
}

fn prompt_settings_is_empty(settings: &PromptSettings) -> bool {
    settings == &PromptSettings::default()
}

fn prompt_editor_display_text(settings: &RepositorySettings, kind: PromptKind) -> &str {
    settings
        .prompts
        .as_ref()
        .and_then(|prompts| prompts.get(kind))
        .unwrap_or("")
}

fn prompt_editor_shows_inherited_label(
    scope: SettingsUiScope,
    settings: &RepositorySettings,
    raw_settings: &RepositorySettings,
    kind: PromptKind,
) -> bool {
    scope == SettingsUiScope::Local
        && raw_settings
            .prompts
            .as_ref()
            .and_then(|prompts| prompts.get(kind))
            .is_none()
        && settings
            .prompts
            .as_ref()
            .and_then(|prompts| prompts.get(kind))
            .is_some()
}

fn set_prompt_value(settings: &mut PromptSettings, kind: PromptKind, value: Option<String>) {
    match kind {
        PromptKind::NewWorkspace => settings.new_workspace = value,
        PromptKind::General => settings.general = value,
        PromptKind::ContinueWork => settings.continue_work = value,
        PromptKind::SummarizeSession => settings.summarize_session = value,
        PromptKind::Handoff => settings.handoff = value,
        PromptKind::CodeReview => settings.code_review = value,
        PromptKind::CreatePr => settings.create_pr = value,
        PromptKind::FixErrors => settings.fix_errors = value,
        PromptKind::ResolveMergeConflicts => settings.resolve_merge_conflicts = value,
        PromptKind::RenameBranch => settings.rename_branch = value,
        PromptKind::CommitGeneration => settings.commit_generation = value,
        PromptKind::TestFixing => settings.test_fixing = value,
        PromptKind::RefactorStyle => settings.refactor_style = value,
        PromptKind::SetupScript => settings.setup_script = value,
        PromptKind::RunScript => settings.run_script = value,
    }
}

fn refresh_repository_select(
    select: &ComboBoxText,
    db_path: &PathBuf,
    selected_name: Option<&str>,
) {
    select.remove_all();
    if let Ok(store) = RepositoryStore::open(db_path) {
        if let Ok(repositories) = store.list() {
            for repository in repositories {
                select.append(Some(&repository.id.to_string()), &repository.name);
            }
        }
    }
    if let Some(name) = selected_name {
        if select.set_active_id(Some(name)) {
            return;
        }
    }
    if select.active_id().is_none() {
        select.set_active(Some(0));
    }
}

fn connect_entry_autosave(
    entry: &Entry,
    field: &'static str,
    field_edits: Rc<RefCell<HashSet<&'static str>>>,
    loading_settings: Rc<RefCell<bool>>,
    autosave: Rc<dyn Fn()>,
) {
    entry.connect_changed(move |_| {
        if !*loading_settings.borrow() {
            field_edits.borrow_mut().insert(field);
        }
        autosave();
    });
}

fn connect_bool_autosave(
    check: &CheckButton,
    field: &'static str,
    field_edits: Rc<RefCell<HashSet<&'static str>>>,
    loading_settings: Rc<RefCell<bool>>,
    autosave: Rc<dyn Fn()>,
) {
    check.connect_toggled(move |_| {
        if !*loading_settings.borrow() {
            field_edits.borrow_mut().insert(field);
        }
        autosave();
    });
}

fn connect_buffer_autosave(
    buffer: &gtk::TextBuffer,
    field: &'static str,
    field_edits: Rc<RefCell<HashSet<&'static str>>>,
    loading_settings: Rc<RefCell<bool>>,
    autosave: Rc<dyn Fn()>,
) {
    connect_buffer_autosave_with_edit(
        buffer,
        field,
        field_edits,
        loading_settings,
        autosave,
        || {},
    );
}

fn connect_buffer_autosave_with_edit(
    buffer: &gtk::TextBuffer,
    field: &'static str,
    field_edits: Rc<RefCell<HashSet<&'static str>>>,
    loading_settings: Rc<RefCell<bool>>,
    autosave: Rc<dyn Fn()>,
    edited: impl Fn() + 'static,
) {
    buffer.connect_changed(move |_| {
        if !*loading_settings.borrow() {
            field_edits.borrow_mut().insert(field);
            edited();
        }
        autosave();
    });
}

fn optional_entry_text(entry: &Entry) -> Option<String> {
    let value = entry.text().trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn optional_buffer_text(buffer: &gtk::TextBuffer) -> Option<String> {
    let value = text_buffer_text(buffer);
    (!value.is_empty()).then_some(value)
}

fn text_buffer_text(buffer: &gtk::TextBuffer) -> String {
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .trim()
        .to_owned()
}

fn parse_environment_lines(text: &str) -> anyhow::Result<Vec<(String, String)>> {
    let mut environment = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("environment line {} must be KEY=value", index + 1))?;
        let key = key.trim();
        anyhow::ensure!(
            is_valid_environment_key(key),
            "environment line {} has invalid key {:?}",
            index + 1,
            key
        );
        environment.push((key.to_owned(), value.trim().to_owned()));
    }
    Ok(environment)
}

fn is_valid_environment_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(ch) if ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn parse_text_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_scope_hides_project_selector_and_project_sections() {
        assert!(!scope_uses_project_selector(SettingsUiScope::Shared));
        let ids = settings_sections_for_scope(SettingsUiScope::Shared)
            .into_iter()
            .map(|section| section.id)
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                "general",
                "prompts",
                "terminal",
                "shortcuts",
                "notifications"
            ]
        );
    }

    #[test]
    fn local_scope_shows_only_project_dependent_sections() {
        assert!(scope_uses_project_selector(SettingsUiScope::Local));
        let ids = settings_sections_for_scope(SettingsUiScope::Local)
            .into_iter()
            .map(|section| section.id)
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec!["general", "prompts", "scripts", "git", "advanced"]
        );
    }

    #[test]
    fn shared_scope_general_groups_are_cross_project_defaults_only() {
        assert_eq!(
            general_group_ids_for_scope(SettingsUiScope::Shared),
            vec!["privacy", "agents", "provider_platforms"]
        );
    }

    #[test]
    fn local_scope_general_groups_are_project_dependent_only() {
        assert_eq!(
            general_group_ids_for_scope(SettingsUiScope::Local),
            vec!["project_behavior", "environment"]
        );
    }

    #[test]
    fn shared_scope_general_fields_exclude_project_values() {
        assert_eq!(
            general_field_ids_for_scope(SettingsUiScope::Shared),
            vec![
                "enterprise_privacy",
                "claude_executable",
                "codex_executable",
                "default_agent",
                "default_model",
                "claude_provider",
                "codex_provider",
                "bedrock_region",
                "vertex_project",
            ]
        );
    }

    #[test]
    fn local_scope_general_fields_exclude_app_wide_defaults() {
        assert_eq!(
            general_field_ids_for_scope(SettingsUiScope::Local),
            vec!["spotlight_testing", "environment_variables"]
        );
    }

    #[test]
    fn local_settings_content_requires_a_selected_project() {
        assert!(settings_content_enabled(SettingsUiScope::Shared, false));
        assert!(!settings_content_enabled(SettingsUiScope::Local, false));
        assert!(settings_content_enabled(SettingsUiScope::Local, true));
    }

    #[test]
    fn local_without_project_replaces_editor_with_selection_state() {
        assert_eq!(
            settings_content_presentation(SettingsUiScope::Local, false),
            SettingsContentPresentation::SelectProject
        );
        assert_eq!(
            settings_content_presentation(SettingsUiScope::Local, true),
            SettingsContentPresentation::Editor
        );
        assert_eq!(
            settings_content_presentation(SettingsUiScope::Shared, false),
            SettingsContentPresentation::Editor
        );
    }

    #[test]
    fn general_layout_specs_own_every_rendered_field_once() {
        let specs = general_group_specs();
        let fields = specs
            .iter()
            .flat_map(|spec| spec.rows.iter().flatten().copied())
            .collect::<Vec<_>>();
        let unique = fields.iter().copied().collect::<HashSet<_>>();

        assert_eq!(fields.len(), 11);
        assert_eq!(unique.len(), fields.len());
        assert_eq!(
            fields
                .into_iter()
                .map(GeneralSettingField::id)
                .collect::<Vec<_>>(),
            vec![
                "enterprise_privacy",
                "claude_executable",
                "codex_executable",
                "default_agent",
                "default_model",
                "claude_provider",
                "codex_provider",
                "bedrock_region",
                "vertex_project",
                "spotlight_testing",
                "environment_variables",
            ]
        );
    }

    #[test]
    fn scope_copy_describes_the_active_target() {
        assert_eq!(
            scope_copy(SettingsUiScope::Shared),
            "Shared: Defaults used by every Archductor project on this machine."
        );
        assert_eq!(
            scope_copy(SettingsUiScope::Local),
            "Local: Overrides for the selected project and its workspaces."
        );
    }

    #[test]
    fn local_inherited_prompt_is_not_copied_into_overrides_until_edited() {
        assert_eq!(
            setting_for_save(false, None::<String>, Some("Inherited".to_owned())),
            None
        );
        assert_eq!(
            setting_for_save(true, None::<String>, Some("Override".to_owned())),
            Some("Override".to_owned())
        );
        assert_eq!(
            setting_for_save(true, Some("Override".to_owned()), None::<String>),
            None
        );
    }

    #[test]
    fn local_unedited_effective_values_are_not_copied_into_overrides() {
        assert_eq!(
            setting_for_save(false, None::<String>, Some("pnpm dev".to_owned())),
            None,
            "unedited inherited script values must stay absent from Local overrides"
        );
        assert_eq!(
            setting_for_save(false, None::<u32>, Some(5000)),
            None,
            "unedited inherited view values must stay absent from Local overrides"
        );
        assert_eq!(
            setting_for_save(true, None::<String>, Some("codex".to_owned())),
            Some("codex".to_owned()),
            "edited Local-only values should still write overrides"
        );
    }

    #[test]
    fn dirty_empty_collection_fields_are_forwarded_to_settings_save() {
        let edits = HashSet::from([
            "file_globs",
            "environment",
            "command_presets",
            "notifications",
            "customization",
        ]);
        let intent = collection_intent_for_settings_save(
            &edits,
            &RepositorySettings::default(),
            r#"
[customization.naming]
pr_body_sections = []

[customization.view]
colors = {}
"#,
            r#"
[customization.naming]
pr_body_sections = []

[customization.view]
colors = {}
"#,
        )
        .unwrap();

        assert!(intent
            .explicit_empty
            .contains(&SettingsCollectionField::FileIncludeGlobs));
        assert!(intent
            .explicit_empty
            .contains(&SettingsCollectionField::EnvironmentVariables));
        assert!(intent
            .explicit_empty
            .contains(&SettingsCollectionField::CommandPalettePresets));
        assert!(intent
            .explicit_empty
            .contains(&SettingsCollectionField::NotificationRules));
        assert!(intent
            .explicit_empty
            .contains(&SettingsCollectionField::PrBodySections));
        assert!(intent
            .explicit_empty
            .contains(&SettingsCollectionField::ViewColors));
        assert!(collection_intent_for_settings_save(
            &HashSet::new(),
            &RepositorySettings::default(),
            "",
            "",
        )
        .unwrap()
        .explicit_empty
        .is_empty());
    }

    #[test]
    fn unrelated_advanced_scalar_edit_preserves_existing_empty_marker() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join(".archductor")).unwrap();
        std::fs::write(
            temp.path().join(".archductor/settings.toml"),
            r#"
[customization.view]
notification_rules = ["checks_failed"]
"#,
        )
        .unwrap();
        archductor_core::settings::save_repository_settings_from_toml(
            temp.path(),
            SettingsLayer::LocalOverride,
            r#"
[customization.view]
theme = "dark"
notification_rules = []
"#,
        )
        .unwrap();
        let original = archductor_core::settings::repository_customization_settings_toml(
            temp.path(),
            SettingsLayer::LocalOverride,
        )
        .unwrap();
        assert!(original.contains("notification_rules = []"));
        let edited = original.replace("theme = \"dark\"", "theme = \"light\"");
        let mut settings = archductor_core::settings::load_repository_settings_for_layer(
            temp.path(),
            SettingsLayer::LocalOverride,
        )
        .unwrap();
        settings.customization = customization_settings_from_toml(&edited).unwrap();
        let intent = collection_intent_for_settings_save(
            &HashSet::from(["customization"]),
            &settings,
            &edited,
            &original,
        )
        .unwrap();

        save_repository_settings_with_collection_intent(
            temp.path(),
            SettingsLayer::LocalOverride,
            &settings,
            &intent.explicit_empty,
            &intent.unset,
        )
        .unwrap();

        let saved =
            std::fs::read_to_string(temp.path().join(".archductor/settings.local.toml")).unwrap();
        assert!(saved.contains("notification_rules = []"));
        let effective = archductor_core::settings::load_repository_settings(temp.path()).unwrap();
        assert!(effective.customization.view.notification_rules.is_empty());
        assert_eq!(effective.customization.view.theme.as_deref(), Some("light"));
    }

    #[test]
    fn advanced_toml_removal_unsets_prior_collection_markers() {
        let settings = RepositorySettings::default();
        let original = r#"
[customization]
agent_profiles = {}

[customization.naming]
pr_body_sections = []

[customization.automation]
required_local_files = []

[customization.view]
colors = {}
dashboard_columns = []
notification_rules = []
command_palette_presets = []
"#;
        let intent = collection_intent_for_settings_save(
            &HashSet::from(["customization"]),
            &settings,
            "",
            original,
        )
        .unwrap();

        assert!(intent.explicit_empty.is_empty());
        assert!(intent
            .unset
            .contains(&SettingsCollectionField::AgentProfiles));
        assert!(intent
            .unset
            .contains(&SettingsCollectionField::PrBodySections));
        assert!(intent
            .unset
            .contains(&SettingsCollectionField::RequiredLocalFiles));
        assert!(intent.unset.contains(&SettingsCollectionField::ViewColors));
        assert!(intent
            .unset
            .contains(&SettingsCollectionField::DashboardColumns));
        assert!(intent
            .unset
            .contains(&SettingsCollectionField::NotificationRules));
        assert!(intent
            .unset
            .contains(&SettingsCollectionField::CommandPalettePresets));

        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join(".archductor")).unwrap();
        std::fs::write(
            temp.path().join(".archductor/settings.toml"),
            r##"
[customization]
agent_profiles = { default = { mcp_servers = ["github"] } }

[customization.naming]
pr_body_sections = ["Summary"]

[customization.automation]
required_local_files = [".env"]

[customization.view]
colors = { accent = "#5b9dff" }
dashboard_columns = ["ready"]
notification_rules = ["checks_failed"]
command_palette_presets = ["test"]
"##,
        )
        .unwrap();
        archductor_core::settings::save_repository_settings_from_toml(
            temp.path(),
            SettingsLayer::LocalOverride,
            original,
        )
        .unwrap();
        save_repository_settings_with_collection_intent(
            temp.path(),
            SettingsLayer::LocalOverride,
            &settings,
            &intent.explicit_empty,
            &intent.unset,
        )
        .unwrap();

        let saved =
            std::fs::read_to_string(temp.path().join(".archductor/settings.local.toml")).unwrap();
        for removed in [
            "agent_profiles",
            "pr_body_sections",
            "required_local_files",
            "colors =",
            "dashboard_columns",
            "notification_rules",
            "command_palette_presets",
        ] {
            assert!(!saved.contains(removed), "{removed} remained in:\n{saved}");
        }
        let effective = archductor_core::settings::load_repository_settings(temp.path()).unwrap();
        assert_eq!(
            effective.customization.view.notification_rules,
            ["checks_failed"]
        );
        assert_eq!(effective.customization.view.colors["accent"], "#5b9dff");
        assert_eq!(effective.customization.naming.pr_body_sections, ["Summary"]);
    }

    #[test]
    fn invalid_advanced_toml_blocks_scope_change_and_preserves_dirty_edit() {
        let dirty_toml = String::from("[view\ntheme = \"dark\"");
        let mut scope = SettingsUiScope::Shared;

        let changed = apply_settings_navigation(&mut scope, SettingsUiScope::Local, || {
            customization_settings_from_toml(&dirty_toml).is_ok()
        });

        assert!(!changed);
        assert_eq!(scope, SettingsUiScope::Shared);
        assert_eq!(dirty_toml, "[view\ntheme = \"dark\"");
    }

    #[test]
    fn invalid_environment_blocks_section_change_and_preserves_dirty_edit() {
        let dirty_environment = String::from("NOT KEY=value");
        let mut section = String::from("general");

        let changed = apply_settings_navigation(&mut section, "advanced".to_owned(), || {
            parse_environment_lines(&dirty_environment).is_ok()
        });

        assert!(!changed);
        assert_eq!(section, "general");
        assert_eq!(dirty_environment, "NOT KEY=value");
    }

    #[test]
    fn persistence_failure_blocks_project_change() {
        let temp = tempfile::tempdir().unwrap();
        let blocked_parent = temp.path().join("not-a-directory");
        std::fs::write(&blocked_parent, "file").unwrap();
        let settings_path = blocked_parent.join("settings.toml");
        let mut project = String::from("alpha");

        let changed = apply_settings_navigation(&mut project, "bravo".to_owned(), || {
            archductor_core::settings::save_app_shared_settings(
                &settings_path,
                &RepositorySettings::default(),
            )
            .is_ok()
        });

        assert!(!changed);
        assert_eq!(project, "alpha");
    }

    #[test]
    fn recover_defaults_removes_prior_empty_collection_markers() {
        let temp = tempfile::tempdir().unwrap();
        archductor_core::settings::save_repository_settings_from_toml(
            temp.path(),
            SettingsLayer::LocalOverride,
            r#"
file_include_globs = ""
environment_variables = {}

[customization.view]
colors = {}
notification_rules = []
"#,
        )
        .unwrap();

        save_recovered_settings_for_layer(
            temp.path(),
            SettingsLayer::LocalOverride,
            &RepositorySettings::default(),
        )
        .unwrap();

        let saved =
            std::fs::read_to_string(temp.path().join(".archductor/settings.local.toml")).unwrap();
        assert!(!saved.contains("file_include_globs"));
        assert!(!saved.contains("environment_variables"));
        assert!(!saved.contains("colors ="));
        assert!(!saved.contains("notification_rules"));
    }

    #[test]
    fn failed_forced_autosave_keeps_its_original_target_for_retry() {
        let mut pending = Some(SettingsSaveTarget::Local("alpha".to_owned()));

        assert!(!flush_autosave_target(&mut pending, |_| false));
        assert_eq!(pending, Some(SettingsSaveTarget::Local("alpha".to_owned())));
        assert!(flush_autosave_target(&mut pending, |target| {
            target == SettingsSaveTarget::Local("alpha".to_owned())
        }));
        assert_eq!(pending, None);
    }

    #[test]
    fn shared_prompt_editor_displays_effective_default_values() {
        let settings = RepositorySettings {
            prompts: Some(PromptSettings {
                general: Some("Default general prompt".to_owned()),
                create_pr: Some("Shared PR prompt".to_owned()),
                ..PromptSettings::default()
            }),
            ..RepositorySettings::default()
        };
        let raw = RepositorySettings {
            prompts: Some(PromptSettings {
                create_pr: Some("Shared PR prompt".to_owned()),
                ..PromptSettings::default()
            }),
            ..RepositorySettings::default()
        };

        assert_eq!(
            prompt_editor_display_text(&settings, PromptKind::General),
            "Default general prompt"
        );
        assert_eq!(
            prompt_editor_display_text(&settings, PromptKind::CreatePr),
            "Shared PR prompt"
        );
        assert!(!prompt_editor_shows_inherited_label(
            SettingsUiScope::Shared,
            &settings,
            &raw,
            PromptKind::General
        ));
    }

    #[test]
    fn local_prompt_editor_marks_inherited_effective_values() {
        let settings = RepositorySettings {
            prompts: Some(PromptSettings {
                general: Some("Inherited general prompt".to_owned()),
                create_pr: Some("Local PR prompt".to_owned()),
                ..PromptSettings::default()
            }),
            ..RepositorySettings::default()
        };
        let raw = RepositorySettings {
            prompts: Some(PromptSettings {
                create_pr: Some("Local PR prompt".to_owned()),
                ..PromptSettings::default()
            }),
            ..RepositorySettings::default()
        };

        assert_eq!(
            prompt_editor_display_text(&settings, PromptKind::General),
            "Inherited general prompt"
        );
        assert!(prompt_editor_shows_inherited_label(
            SettingsUiScope::Local,
            &settings,
            &raw,
            PromptKind::General
        ));
        assert!(!prompt_editor_shows_inherited_label(
            SettingsUiScope::Local,
            &settings,
            &raw,
            PromptKind::CreatePr
        ));
    }

    #[test]
    fn settings_sections_keep_expected_order() {
        let ids = settings_sections()
            .iter()
            .map(|section| section.id)
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                "general",
                "prompts",
                "scripts",
                "git",
                "terminal",
                "shortcuts",
                "notifications",
                "advanced"
            ]
        );
    }

    #[test]
    fn local_bool_settings_preserve_unset_inherited_values() {
        assert_eq!(
            bool_setting_for_layer(SettingsLayer::LocalOverride, None, false, false),
            None
        );
        assert_eq!(
            bool_setting_for_layer(SettingsLayer::LocalOverride, None, true, true),
            Some(true)
        );
        assert_eq!(
            bool_setting_for_layer(SettingsLayer::LocalOverride, None, false, true),
            Some(false)
        );
        assert_eq!(
            bool_setting_for_layer(SettingsLayer::LocalOverride, Some(true), false, false),
            Some(true)
        );
        assert_eq!(
            bool_setting_for_layer(SettingsLayer::LocalOverride, Some(true), false, true),
            Some(false)
        );
        assert_eq!(
            bool_setting_for_layer(SettingsLayer::RepositoryShared, None, false, false),
            Some(false)
        );
    }

    #[test]
    fn local_run_mode_settings_preserve_unset_values() {
        assert_eq!(
            run_mode_setting_for_layer(SettingsLayer::LocalOverride, None),
            None
        );
        assert_eq!(
            run_mode_setting_for_layer(SettingsLayer::RepositoryShared, None),
            Some("concurrent".to_owned())
        );
    }

    #[test]
    fn recovery_defaults_match_selected_layer() {
        let shared = recovered_settings_for_layer(SettingsLayer::RepositoryShared).unwrap();
        assert_eq!(shared.file_include_globs, [".env*"]);
        assert_eq!(shared.scripts.run_mode.as_deref(), Some("concurrent"));

        let local = recovered_settings_for_layer(SettingsLayer::LocalOverride).unwrap();
        assert_eq!(local, RepositorySettings::default());
    }

    #[test]
    fn prompt_section_uses_editor_style_fields() {
        let prompts = settings_sections()
            .into_iter()
            .find(|section| section.id == "prompts")
            .unwrap();
        assert!(prompts.description.contains("Prompt") || prompts.description.contains("prompt"));
    }
}
