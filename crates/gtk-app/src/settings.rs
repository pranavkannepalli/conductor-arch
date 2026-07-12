use crate::buttons::text_button;
use gtk::prelude::*;
use gtk::{
    Align, Box as GBox, Button, CheckButton, ComboBoxText, Entry, Label, Orientation, PolicyType,
    ScrolledWindow, Stack, TextView, ToggleButton,
};
use linux_archductor_core::paths::AppPaths;
use linux_archductor_core::repository::RepositoryStore;
use linux_archductor_core::settings::{
    customization_settings_from_toml, customization_settings_to_toml, ensure_repository_config,
    inspect_repository_settings, load_repository_settings_for_layer, save_repository_settings,
    AgentProfileSettings, FilePatternSource, GitSettings, PromptSettings, ProviderSettings,
    RepositorySettings, ScriptSettings, SettingsLayer,
};
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use crate::toast::{surface_label_error, ToastManager};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SettingsSection {
    id: &'static str,
    title: &'static str,
    description: &'static str,
}

pub(crate) fn build_settings_page(
    paths: &AppPaths,
    toast_manager: ToastManager,
) -> (GBox, impl Fn() + Clone + 'static) {
    let root = GBox::new(Orientation::Vertical, 0);
    root.add_css_class("dashboard");
    root.add_css_class("page-shell");

    let header = GBox::new(Orientation::Vertical, 8);
    header.add_css_class("dashboard-header");
    header.add_css_class("page-header");
    let title = Label::new(Some("Settings"));
    title.add_css_class("dashboard-title");
    title.set_xalign(0.0);
    let subtitle = Label::new(Some(
        "Repository settings live in a panel-style editor, not a loose form page.",
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

    let settings_tabs_row = GBox::new(Orientation::Horizontal, 10);
    settings_tabs_row.add_css_class("settings-tabs-row");
    let shared_tab = ToggleButton::with_label("Shared");
    shared_tab.add_css_class("settings-tab");
    shared_tab.set_active(true);
    let local_tab = ToggleButton::with_label("Local");
    local_tab.add_css_class("settings-tab");
    let settings_result = Label::new(Some(
        "Shared settings are commit-safe. Use Local for machine secrets and per-machine overrides.",
    ));
    settings_result.add_css_class("settings-status");
    settings_result.add_css_class("card-meta");
    settings_result.set_xalign(0.0);
    settings_result.set_hexpand(true);
    settings_result.set_wrap(true);
    let bool_edits: Rc<RefCell<HashSet<&'static str>>> = Rc::new(RefCell::new(HashSet::new()));
    let loaded_settings_target: Rc<RefCell<Option<(String, SettingsLayer)>>> =
        Rc::new(RefCell::new(None));
    let forced_save_target: Rc<RefCell<Option<(String, SettingsLayer)>>> =
        Rc::new(RefCell::new(None));
    settings_tabs_row.append(&shared_tab);
    settings_tabs_row.append(&local_tab);
    settings_tabs_row.append(&settings_result);
    body.append(&settings_tabs_row);

    let settings_toolbar = GBox::new(Orientation::Vertical, 10);
    settings_toolbar.add_css_class("settings-toolbar");
    let settings_top = GBox::new(Orientation::Horizontal, 8);
    settings_top.add_css_class("settings-toolbar-row");
    let settings_repo_select = ComboBoxText::new();
    settings_repo_select.set_hexpand(true);
    let init_settings_btn = text_button("Initialize");
    let save_settings_btn = text_button("Save");
    save_settings_btn.add_css_class("suggested-action");
    refresh_repository_select(&settings_repo_select, &paths.database_path, None);
    settings_top.append(&settings_repo_select);
    settings_top.append(&init_settings_btn);
    settings_top.append(&save_settings_btn);
    settings_toolbar.append(&settings_top);
    body.append(&settings_toolbar);

    let inspector = GBox::new(Orientation::Horizontal, 16);
    inspector.add_css_class("settings-inspector");
    body.append(&inspector);

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

    for section in settings_sections() {
        let button = settings_rail_button(section);
        let stack_for_button = content_stack.clone();
        let active_for_button = active_section.clone();
        let sync_for_button = sync_rail_state.clone();
        let id = section.id.to_owned();
        button.connect_clicked(move |_| {
            *active_for_button.borrow_mut() = id.clone();
            stack_for_button.set_visible_child_name(&id);
            sync_for_button();
        });
        settings_rail.append(&button);
        rail_buttons
            .borrow_mut()
            .push((section.id.to_owned(), button.clone()));
    }
    content_stack.set_visible_child_name("general");
    sync_rail_state();

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

    let general_intro = settings_group(
        "Repository defaults",
        "High-level behavior for this repository. Scripts, checks, and Git defaults live in their own sections.",
    );
    general_panel.append(&general_intro.0);
    general_intro.1.append(&settings_toggle_row(
        &spotlight_check,
        "Turns on spotlight state tracking for workspace sync flows.",
    ));
    general_intro.1.append(&settings_toggle_row(
        &privacy_check,
        "Uses privacy-safe behavior for repository and agent operations.",
    ));

    let env_view = settings_editor_view(120);
    let environment_group = settings_group(
        "Environment",
        "Machine-style environment values passed into scripts and sessions as `KEY=value` lines.",
    );
    general_panel.append(&environment_group.0);
    environment_group.1.append(&settings_editor_field(
        "Environment variables",
        "One `KEY=value` per line. Leave blank when the repo does not need extra environment.",
        &env_view.0,
    ));

    let claude_path_entry = machine_entry("Claude executable");
    let codex_path_entry = machine_entry("Codex executable");
    let claude_provider_entry = machine_entry("Claude provider");
    let codex_provider_entry = machine_entry("Codex provider");
    let bedrock_region_entry = machine_entry("Bedrock region");
    let vertex_project_entry = machine_entry("Vertex project id");
    let default_agent_entry = machine_entry("codex/claude/opencode");
    let default_model_entry = machine_entry("default model label");

    let provider_paths = settings_group(
        "Agents and providers",
        "Executable paths, default agent, and provider routing for local agent launches.",
    );
    general_panel.append(&provider_paths.0);
    provider_paths.1.append(&settings_field_pair(
        settings_field(
            "Claude executable",
            "Absolute path or command name used to start Claude Code.",
            &claude_path_entry,
        ),
        settings_field(
            "Codex executable",
            "Absolute path or command name used to start Codex.",
            &codex_path_entry,
        ),
    ));
    provider_paths.1.append(&settings_field_pair(
        settings_field(
            "Default agent",
            "Saved as `customization.automation.auto_start_agent`.",
            &default_agent_entry,
        ),
        settings_field(
            "Default model",
            "Saved as `customization.agent_profiles.default.model`. TODO: pass through once provider launch args land.",
            &default_model_entry,
        ),
    ));
    provider_paths.1.append(&settings_field_pair(
        settings_field(
            "Claude provider",
            "Provider override used when Claude sessions need a specific backend.",
            &claude_provider_entry,
        ),
        settings_field(
            "Codex provider",
            "Provider override used when Codex sessions need a specific backend.",
            &codex_provider_entry,
        ),
    ));

    let provider_platforms = settings_group(
        "Platform settings",
        "Provider-specific machine values for Bedrock, Vertex, and SSH-backed setups.",
    );
    general_panel.append(&provider_platforms.0);
    provider_platforms.1.append(&settings_field_pair(
        settings_field(
            "Bedrock region",
            "AWS region used for Bedrock requests when that provider is active.",
            &bedrock_region_entry,
        ),
        settings_field(
            "Vertex project id",
            "Google Cloud project id used for Vertex provider calls.",
            &vertex_project_entry,
        ),
    ));

    let script_group = settings_group(
        "Workspace scripts",
        "Commands Linux Archductor can run from the workspace context.",
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
            "Short prefix used when Linux Archductor generates branch names.",
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
            "New workspace",
            "Prompt guidance for planning a newly created workspace.",
            110,
        ),
        (
            "General agent instructions",
            "The default prompt context used for agent work in this repository.",
            120,
        ),
        (
            "Continue work",
            "Prompt guidance for resuming from the current workspace state.",
            110,
        ),
        (
            "Summarize session",
            "Prompt guidance for end-of-session summaries.",
            110,
        ),
        (
            "Handoff",
            "Prompt guidance for handoff notes when work stops before completion.",
            110,
        ),
        (
            "Code review",
            "Prompt guidance used when asking an agent to review code.",
            110,
        ),
        (
            "Create PR",
            "Prompt template used when generating PR content.",
            110,
        ),
        (
            "Fix errors / failing checks",
            "Prompt guidance for CI failures and broken builds.",
            110,
        ),
        (
            "Resolve merge conflicts",
            "Prompt guidance for conflict-resolution flows.",
            110,
        ),
        (
            "Rename branch",
            "Prompt used when branch naming needs refinement later in the workflow.",
            110,
        ),
        (
            "Commit message generation",
            "Prompt guidance for generating repository-specific commit messages.",
            110,
        ),
        (
            "Test fixing",
            "Prompt guidance for agents fixing failing tests.",
            110,
        ),
        (
            "Refactor style",
            "Prompt guidance for safe structural cleanup and refactors.",
            110,
        ),
        (
            "Setup script",
            "Prompt guidance for inferring or updating setup scripts.",
            110,
        ),
        (
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
    for (label, help, height) in prompt_specs {
        let view = settings_editor_view(height);
        prompt_group
            .1
            .append(&settings_editor_field(label, help, &view.0));
        prompt_views.push(view);
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
    let shared_tab_load = shared_tab.clone();
    let local_tab_load = local_tab.clone();
    let settings_result_load = settings_result.clone();
    let loading_settings_load = Rc::new(RefCell::new(false));
    let loading_settings_for_load = loading_settings_load.clone();
    let bool_edits_load = bool_edits.clone();
    let loaded_settings_target_load = loaded_settings_target.clone();
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
    let prompt_buffers_load = prompt_views
        .iter()
        .map(|(_, buffer, _)| buffer.clone())
        .collect::<Vec<_>>();
    let toast_load = toast_manager.clone();
    let load_selected_settings = {
        let db_path_load_settings = db_path_load_settings.clone();
        let settings_repo_select_load = settings_repo_select_load.clone();
        let settings_result_load = settings_result_load.clone();
        let shared_tab_load = shared_tab_load.clone();
        let local_tab_load = local_tab_load.clone();
        let loading_settings_for_load = loading_settings_for_load.clone();
        let bool_edits_load = bool_edits_load.clone();
        let loaded_settings_target_load = loaded_settings_target_load.clone();
        let setup_entry_load = setup_entry_load.clone();
        let run_entry_load = run_entry_load.clone();
        let archive_entry_load = archive_entry_load.clone();
        let test_entry_load = test_entry_load.clone();
        let lint_entry_load = lint_entry_load.clone();
        let typecheck_entry_load = typecheck_entry_load.clone();
        let build_entry_load = build_entry_load.clone();
        let run_mode_entry_load = run_mode_entry_load.clone();
        let spotlight_check_load = spotlight_check_load.clone();
        let privacy_check_load = privacy_check_load.clone();
        let archive_on_merge_check_load = archive_on_merge_check_load.clone();
        let delete_branch_check_load = delete_branch_check_load.clone();
        let auto_upstream_check_load = auto_upstream_check_load.clone();
        let claude_path_entry_load = claude_path_entry_load.clone();
        let codex_path_entry_load = codex_path_entry_load.clone();
        let claude_provider_entry_load = claude_provider_entry_load.clone();
        let codex_provider_entry_load = codex_provider_entry_load.clone();
        let bedrock_region_entry_load = bedrock_region_entry_load.clone();
        let vertex_project_entry_load = vertex_project_entry_load.clone();
        let default_agent_entry_load = default_agent_entry_load.clone();
        let default_model_entry_load = default_model_entry_load.clone();
        let branch_prefix_type_entry_load = branch_prefix_type_entry_load.clone();
        let branch_prefix_entry_load = branch_prefix_entry_load.clone();
        let terminal_font_entry_load = terminal_font_entry_load.clone();
        let terminal_scrollback_entry_load = terminal_scrollback_entry_load.clone();
        let keybindings_entry_load = keybindings_entry_load.clone();
        let command_presets_buffer_load = command_presets_buffer_load.clone();
        let notifications_buffer_load = notifications_buffer_load.clone();
        let file_globs_buffer_load = file_globs_buffer_load.clone();
        let file_globs_text_load = file_globs_text_load.clone();
        let env_buffer_load = env_buffer_load.clone();
        let customization_buffer_load = customization_buffer_load.clone();
        let prompt_buffers_load = prompt_buffers_load.clone();
        let toast_load = toast_load.clone();
        Rc::new(move || {
            let repo_name = selected_repository_name(&settings_repo_select_load);
            if repo_name.is_empty() {
                surface_label_error(&settings_result_load, &toast_load, "Select a repository.");
                *loaded_settings_target_load.borrow_mut() = None;
                return;
            }
            *loading_settings_for_load.borrow_mut() = true;
            bool_edits_load.borrow_mut().clear();
            let layer = if local_tab_load.is_active() {
                SettingsLayer::LocalOverride
            } else {
                SettingsLayer::RepositoryShared
            };
            shared_tab_load.set_active(matches!(layer, SettingsLayer::RepositoryShared));
            local_tab_load.set_active(matches!(layer, SettingsLayer::LocalOverride));
            match repository_root(&db_path_load_settings, &repo_name)
                .and_then(|repo_path| {
                    load_repository_settings_for_layer(&repo_path, layer)
                        .map(|settings| (repo_path, settings))
                })
                .and_then(|(repo_path, settings)| {
                    inspect_repository_settings(&repo_path)
                        .map(|inspection| (repo_path, settings, inspection))
                }) {
                Ok((repo_path, settings, inspection)) => {
                    *loaded_settings_target_load.borrow_mut() = Some((repo_name.clone(), layer));
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
                    run_mode_entry_load.set_text(
                        settings
                            .scripts
                            .run_mode
                            .as_deref()
                            .or_else(|| {
                                matches!(layer, SettingsLayer::RepositoryShared)
                                    .then_some("concurrent")
                            })
                            .unwrap_or(""),
                    );
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
                    if inspection.worktreeinclude_exists {
                        file_globs_text_load.set_editable(false);
                        file_globs_buffer_load
                            .set_text(&inspection.active_file_patterns.join("\n"));
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
                    let prompts = settings.prompts.unwrap_or_default();
                    let prompt_values = [
                        prompts.new_workspace,
                        prompts.general,
                        prompts.continue_work,
                        prompts.summarize_session,
                        prompts.handoff,
                        prompts.code_review,
                        prompts.create_pr,
                        prompts.fix_errors,
                        prompts.resolve_merge_conflicts,
                        prompts.rename_branch,
                        prompts.commit_generation,
                        prompts.test_fixing,
                        prompts.refactor_style,
                        prompts.setup_script,
                        prompts.run_script,
                    ];
                    for (buffer, value) in prompt_buffers_load.iter().zip(prompt_values.iter()) {
                        buffer.set_text(value.as_deref().unwrap_or(""));
                    }
                    customization_buffer_load.set_text(
                        &customization_settings_to_toml(&settings.customization)
                            .unwrap_or_default(),
                    );
                    let source = match inspection.active_file_patterns_source {
                        FilePatternSource::Worktreeinclude => {
                            ".worktreeinclude wins; file rules are read-only here."
                        }
                        FilePatternSource::RepositorySettings => {
                            "Repository settings define file copy rules."
                        }
                        FilePatternSource::BuiltInDefault => {
                            "Built-in `.env*` defaults apply until custom rules are saved."
                        }
                    };
                    settings_result_load.set_text(&format!(
                            "Loaded {} ({:?}). Shared={} Local={} Worktreeinclude={} Active files: {} ({})",
                            repo_path.display(),
                            layer,
                            inspection.shared_settings_exists,
                            inspection.local_settings_exists,
                            inspection.worktreeinclude_exists,
                            inspection.active_file_patterns.join(", "),
                            source
                        ));
                }
                Err(err) => {
                    *loaded_settings_target_load.borrow_mut() = None;
                    surface_label_error(
                        &settings_result_load,
                        &toast_load,
                        format!("Load failed: {err:#}"),
                    );
                }
            }
            *loading_settings_for_load.borrow_mut() = false;
        })
    };
    let load_selected_settings_for_repo = load_selected_settings.clone();
    let loading_settings_for_repo = loading_settings_for_load.clone();
    let pending_autosave: Rc<RefCell<Option<gtk::glib::SourceId>>> = Rc::new(RefCell::new(None));
    let pending_autosave_target: Rc<RefCell<Option<(String, SettingsLayer)>>> =
        Rc::new(RefCell::new(None));
    let flush_pending_autosave = {
        let pending_autosave = pending_autosave.clone();
        let pending_autosave_target = pending_autosave_target.clone();
        let forced_save_target = forced_save_target.clone();
        let save_settings_btn = save_settings_btn.clone();
        Rc::new(move || {
            if let Some(source_id) = pending_autosave.borrow_mut().take() {
                source_id.remove();
                if let Some(target) = pending_autosave_target.borrow_mut().take() {
                    *forced_save_target.borrow_mut() = Some(target);
                    save_settings_btn.emit_clicked();
                }
            }
        })
    };
    let flush_pending_autosave_for_repo = flush_pending_autosave.clone();
    settings_repo_select.connect_changed(move |_| {
        if !*loading_settings_for_repo.borrow() {
            flush_pending_autosave_for_repo();
            load_selected_settings_for_repo();
        }
    });
    let load_selected_settings_for_shared = load_selected_settings.clone();
    let local_tab_for_shared = local_tab.clone();
    let loading_settings_for_shared = loading_settings_for_load.clone();
    let flush_pending_autosave_for_shared = flush_pending_autosave.clone();
    shared_tab.connect_toggled(move |button| {
        if *loading_settings_for_shared.borrow() {
            return;
        }
        if button.is_active() {
            flush_pending_autosave_for_shared();
            local_tab_for_shared.set_active(false);
            load_selected_settings_for_shared();
        } else if !local_tab_for_shared.is_active() {
            button.set_active(true);
        }
    });
    let load_selected_settings_for_local = load_selected_settings.clone();
    let shared_tab_for_local = shared_tab.clone();
    let loading_settings_for_local = loading_settings_for_load.clone();
    let flush_pending_autosave_for_local = flush_pending_autosave.clone();
    local_tab.connect_toggled(move |button| {
        if *loading_settings_for_local.borrow() {
            return;
        }
        if button.is_active() {
            flush_pending_autosave_for_local();
            shared_tab_for_local.set_active(false);
            load_selected_settings_for_local();
        } else if !shared_tab_for_local.is_active() {
            button.set_active(true);
        }
    });

    let autosave = {
        let save_settings_btn = save_settings_btn.clone();
        let loading_settings = loading_settings_for_load.clone();
        let pending_autosave = pending_autosave.clone();
        let pending_autosave_target = pending_autosave_target.clone();
        let loaded_settings_target = loaded_settings_target.clone();
        Rc::new(move || {
            if !*loading_settings.borrow() {
                if let Some(source_id) = pending_autosave.borrow_mut().take() {
                    source_id.remove();
                }
                *pending_autosave_target.borrow_mut() = loaded_settings_target.borrow().clone();
                let save_settings_btn = save_settings_btn.clone();
                let pending_autosave_for_timeout = pending_autosave.clone();
                let pending_autosave_target_for_timeout = pending_autosave_target.clone();
                let source_id =
                    gtk::glib::timeout_add_local_once(Duration::from_millis(600), move || {
                        *pending_autosave_for_timeout.borrow_mut() = None;
                        *pending_autosave_target_for_timeout.borrow_mut() = None;
                        save_settings_btn.emit_clicked();
                    });
                *pending_autosave.borrow_mut() = Some(source_id);
            }
        })
    };
    connect_entry_autosave(&setup_entry, autosave.clone());
    connect_entry_autosave(&run_entry, autosave.clone());
    connect_entry_autosave(&archive_entry, autosave.clone());
    connect_entry_autosave(&test_entry, autosave.clone());
    connect_entry_autosave(&lint_entry, autosave.clone());
    connect_entry_autosave(&typecheck_entry, autosave.clone());
    connect_entry_autosave(&build_entry, autosave.clone());
    connect_entry_autosave(&run_mode_entry, autosave.clone());
    connect_bool_autosave(
        &spotlight_check,
        "spotlight_testing",
        bool_edits.clone(),
        loading_settings_for_load.clone(),
        autosave.clone(),
    );
    connect_bool_autosave(
        &privacy_check,
        "enterprise_data_privacy",
        bool_edits.clone(),
        loading_settings_for_load.clone(),
        autosave.clone(),
    );
    connect_bool_autosave(
        &archive_on_merge_check,
        "archive_on_merge",
        bool_edits.clone(),
        loading_settings_for_load.clone(),
        autosave.clone(),
    );
    connect_bool_autosave(
        &delete_branch_check,
        "delete_branch_on_archive",
        bool_edits.clone(),
        loading_settings_for_load.clone(),
        autosave.clone(),
    );
    connect_bool_autosave(
        &auto_upstream_check,
        "worktree_push_auto_setup_remote",
        bool_edits.clone(),
        loading_settings_for_load.clone(),
        autosave.clone(),
    );
    connect_entry_autosave(&claude_path_entry, autosave.clone());
    connect_entry_autosave(&codex_path_entry, autosave.clone());
    connect_entry_autosave(&claude_provider_entry, autosave.clone());
    connect_entry_autosave(&codex_provider_entry, autosave.clone());
    connect_entry_autosave(&bedrock_region_entry, autosave.clone());
    connect_entry_autosave(&vertex_project_entry, autosave.clone());
    connect_entry_autosave(&default_agent_entry, autosave.clone());
    connect_entry_autosave(&default_model_entry, autosave.clone());
    connect_entry_autosave(&branch_prefix_type_entry, autosave.clone());
    connect_entry_autosave(&branch_prefix_entry, autosave.clone());
    connect_entry_autosave(&terminal_font_entry, autosave.clone());
    connect_entry_autosave(&terminal_scrollback_entry, autosave.clone());
    connect_entry_autosave(&keybindings_entry, autosave.clone());
    connect_buffer_autosave(&command_presets_view.1, autosave.clone());
    connect_buffer_autosave(&notifications_view.1, autosave.clone());
    connect_buffer_autosave(&file_globs_view.1, autosave.clone());
    connect_buffer_autosave(&env_view.1, autosave.clone());
    connect_buffer_autosave(&customization_view.1, autosave.clone());
    for (_, buffer, _) in &prompt_views {
        connect_buffer_autosave(buffer, autosave.clone());
    }

    if !selected_repository_name(&settings_repo_select).is_empty() {
        load_selected_settings();
    }

    let db_path_save_settings = paths.database_path.clone();
    let forced_save_target_for_save = forced_save_target.clone();
    let loaded_settings_target_for_save = loaded_settings_target.clone();
    let bool_edits_for_save = bool_edits.clone();
    let toast_save = toast_manager.clone();
    save_settings_btn.connect_clicked(move |_| {
        let save_target = forced_save_target_for_save
            .borrow_mut()
            .take()
            .or_else(|| loaded_settings_target_for_save.borrow().clone());
        let (repo_name, layer) = save_target.unwrap_or_else(|| {
            (
                selected_repository_name(&settings_repo_select),
                selected_settings_layer(&local_tab),
            )
        });
        if repo_name.is_empty() {
            surface_label_error(
                &settings_result,
                &toast_save,
                "Repository name is required.",
            );
            return;
        }
        let repo_path = match repository_root(&db_path_save_settings, &repo_name) {
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
        let current_settings = match load_repository_settings_for_layer(&repo_path, layer) {
            Ok(settings) => settings,
            Err(err) => {
                surface_label_error(
                    &settings_result,
                    &toast_save,
                    format!("Save failed: {err:#}"),
                );
                return;
            }
        };
        let current_file_globs = current_settings.file_include_globs.clone();
        let mut customization =
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
            };
        let terminal_scrollback = match optional_entry_text(&terminal_scrollback_entry) {
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
        let test_command = optional_entry_text(&test_entry);
        let lint_command = optional_entry_text(&lint_entry);
        let typecheck_command = optional_entry_text(&typecheck_entry);
        let build_command = optional_entry_text(&build_entry);
        customization.automation.auto_start_agent = optional_entry_text(&default_agent_entry);
        customization.automation.test_command = test_command.clone();
        customization.automation.lint_command = lint_command.clone();
        customization.automation.typecheck_command = typecheck_command.clone();
        customization.automation.build_command = build_command.clone();
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
        customization.view.terminal_font = optional_entry_text(&terminal_font_entry);
        customization.view.terminal_scrollback = terminal_scrollback;
        customization.view.keybindings = optional_entry_text(&keybindings_entry);
        customization.view.command_palette_presets =
            parse_text_lines(&text_buffer_text(&command_presets_view.1));
        customization.view.notification_rules =
            parse_text_lines(&text_buffer_text(&notifications_view.1));
        let prompt_settings = PromptSettings {
            new_workspace: optional_buffer_text(&prompt_views[0].1),
            general: optional_buffer_text(&prompt_views[1].1),
            continue_work: optional_buffer_text(&prompt_views[2].1),
            summarize_session: optional_buffer_text(&prompt_views[3].1),
            handoff: optional_buffer_text(&prompt_views[4].1),
            code_review: optional_buffer_text(&prompt_views[5].1),
            create_pr: optional_buffer_text(&prompt_views[6].1),
            fix_errors: optional_buffer_text(&prompt_views[7].1),
            resolve_merge_conflicts: optional_buffer_text(&prompt_views[8].1),
            rename_branch: optional_buffer_text(&prompt_views[9].1),
            commit_generation: optional_buffer_text(&prompt_views[10].1),
            test_fixing: optional_buffer_text(&prompt_views[11].1),
            refactor_style: optional_buffer_text(&prompt_views[12].1),
            setup_script: optional_buffer_text(&prompt_views[13].1),
            run_script: optional_buffer_text(&prompt_views[14].1),
        };
        let environment_variables = match parse_environment_lines(&text_buffer_text(&env_view.1)) {
            Ok(environment_variables) => environment_variables,
            Err(err) => {
                surface_label_error(
                    &settings_result,
                    &toast_save,
                    format!("Save failed: {err:#}"),
                );
                return;
            }
        };
        let settings = RepositorySettings {
            file_include_globs: if file_globs_view.2.is_editable() {
                text_buffer_text(&file_globs_view.1)
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_owned)
                    .collect()
            } else {
                current_file_globs
            },
            spotlight_testing: bool_setting_for_layer(
                layer,
                current_settings.spotlight_testing,
                spotlight_check.is_active(),
                bool_edits_for_save.borrow().contains("spotlight_testing"),
            ),
            enterprise_data_privacy: bool_setting_for_layer(
                layer,
                current_settings.enterprise_data_privacy,
                privacy_check.is_active(),
                bool_edits_for_save
                    .borrow()
                    .contains("enterprise_data_privacy"),
            ),
            scripts: ScriptSettings {
                setup: optional_entry_text(&setup_entry),
                run: optional_entry_text(&run_entry),
                archive: optional_entry_text(&archive_entry),
                test: test_command,
                lint: lint_command,
                typecheck: typecheck_command,
                build: build_command,
                run_mode: run_mode_setting_for_layer(layer, optional_entry_text(&run_mode_entry)),
            },
            environment_variables,
            prompt_pack: current_settings.prompt_pack,
            prompts: (!prompt_settings_is_empty(&prompt_settings)).then_some(prompt_settings),
            providers: ProviderSettings {
                claude_code_executable_path: optional_entry_text(&claude_path_entry),
                codex_executable_path: optional_entry_text(&codex_path_entry),
                claude_provider: optional_entry_text(&claude_provider_entry),
                codex_provider: optional_entry_text(&codex_provider_entry),
                bedrock_region: optional_entry_text(&bedrock_region_entry),
                vertex_project_id: optional_entry_text(&vertex_project_entry),
            },
            git: GitSettings {
                delete_branch_on_archive: bool_setting_for_layer(
                    layer,
                    current_settings.git.delete_branch_on_archive,
                    delete_branch_check.is_active(),
                    bool_edits_for_save
                        .borrow()
                        .contains("delete_branch_on_archive"),
                ),
                archive_on_merge: bool_setting_for_layer(
                    layer,
                    current_settings.git.archive_on_merge,
                    archive_on_merge_check.is_active(),
                    bool_edits_for_save.borrow().contains("archive_on_merge"),
                ),
                worktree_push_auto_setup_remote: bool_setting_for_layer(
                    layer,
                    current_settings.git.worktree_push_auto_setup_remote,
                    auto_upstream_check.is_active(),
                    bool_edits_for_save
                        .borrow()
                        .contains("worktree_push_auto_setup_remote"),
                ),
                branch_prefix_type: optional_entry_text(&branch_prefix_type_entry),
                branch_prefix: optional_entry_text(&branch_prefix_entry),
            },
            customization,
        };
        match save_repository_settings(&repo_path, layer, &settings) {
            Ok(()) => {
                bool_edits_for_save.borrow_mut().clear();
                settings_result.set_text(&format!("Saved settings for {}", repo_path.display()))
            }
            Err(err) => surface_label_error(
                &settings_result,
                &toast_save,
                format!("Save failed: {err:#}"),
            ),
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

fn selected_repository_name(select: &ComboBoxText) -> String {
    select
        .active_id()
        .or_else(|| select.active_text())
        .map(|value| value.to_string())
        .unwrap_or_default()
}

fn selected_settings_layer(local: &ToggleButton) -> SettingsLayer {
    if local.is_active() {
        SettingsLayer::LocalOverride
    } else {
        SettingsLayer::RepositoryShared
    }
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

fn prompt_settings_is_empty(settings: &PromptSettings) -> bool {
    settings == &PromptSettings::default()
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

fn connect_entry_autosave(entry: &Entry, autosave: Rc<dyn Fn()>) {
    entry.connect_changed(move |_| autosave());
}

fn connect_bool_autosave(
    check: &CheckButton,
    field: &'static str,
    bool_edits: Rc<RefCell<HashSet<&'static str>>>,
    loading_settings: Rc<RefCell<bool>>,
    autosave: Rc<dyn Fn()>,
) {
    check.connect_toggled(move |_| {
        if !*loading_settings.borrow() {
            bool_edits.borrow_mut().insert(field);
        }
        autosave();
    });
}

fn connect_buffer_autosave(buffer: &gtk::TextBuffer, autosave: Rc<dyn Fn()>) {
    buffer.connect_changed(move |_| autosave());
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
    fn prompt_section_uses_editor_style_fields() {
        let prompts = settings_sections()
            .into_iter()
            .find(|section| section.id == "prompts")
            .unwrap();
        assert!(prompts.description.contains("Prompt") || prompts.description.contains("prompt"));
    }
}
