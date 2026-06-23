use crate::buttons::text_button;
use gtk::prelude::*;
use gtk::{
    Align, Box as GBox, Button, CheckButton, ComboBoxText, Entry, Label, Orientation, PolicyType,
    ScrolledWindow, Stack, TextView,
};
use linux_conductor_core::paths::AppPaths;
use linux_conductor_core::repository::RepositoryStore;
use linux_conductor_core::settings::{
    customization_settings_from_toml, customization_settings_to_toml, inspect_repository_settings,
    load_repository_settings, save_repository_settings, FilePatternSource, GitSettings,
    PromptSettings, ProviderSettings, RepositorySettings, ScriptSettings, SettingsLayer,
};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SettingsSection {
    id: &'static str,
    title: &'static str,
    description: &'static str,
}

pub(crate) fn build_settings_page(paths: &AppPaths) -> (GBox, impl Fn() + Clone + 'static) {
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

    let settings_shell = GBox::new(Orientation::Vertical, 14);
    settings_shell.add_css_class("settings-shell");
    body.append(&settings_shell);

    let settings_toolbar = GBox::new(Orientation::Vertical, 10);
    settings_toolbar.add_css_class("settings-toolbar");
    let settings_top = GBox::new(Orientation::Horizontal, 8);
    settings_top.add_css_class("settings-toolbar-row");
    let settings_repo_entry = Entry::new();
    settings_repo_entry.set_placeholder_text(Some("repository name"));
    settings_repo_entry.set_hexpand(true);
    let layer_select = ComboBoxText::new();
    layer_select.append(Some("shared"), "Shared");
    layer_select.append(Some("local"), "Local");
    layer_select.set_active_id(Some("shared"));
    let load_settings_btn = text_button("Load");
    let save_settings_btn = text_button("Save");
    save_settings_btn.add_css_class("suggested-action");
    settings_top.append(&settings_repo_entry);
    settings_top.append(&layer_select);
    settings_top.append(&load_settings_btn);
    settings_top.append(&save_settings_btn);
    settings_toolbar.append(&settings_top);

    let settings_result = Label::new(Some(
        "Shared settings are commit-safe. Use Local for machine secrets and per-machine overrides.",
    ));
    settings_result.add_css_class("settings-status");
    settings_result.add_css_class("card-meta");
    settings_result.set_xalign(0.0);
    settings_result.set_wrap(true);
    settings_toolbar.append(&settings_result);
    settings_shell.append(&settings_toolbar);

    let inspector = GBox::new(Orientation::Horizontal, 16);
    inspector.add_css_class("settings-inspector");
    settings_shell.append(&inspector);

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
    let providers_panel = settings_content_panel();
    let git_panel = settings_content_panel();
    let advanced_panel = settings_content_panel();
    content_stack.add_named(&general_panel, Some("general"));
    content_stack.add_named(&prompts_panel, Some("prompts"));
    content_stack.add_named(&providers_panel, Some("providers"));
    content_stack.add_named(&git_panel, Some("git"));
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
    let run_mode_entry = machine_entry("run mode: concurrent/nonconcurrent");
    let spotlight_check = CheckButton::with_label("Enable spotlight testing");
    let privacy_check = CheckButton::with_label("Use enterprise privacy mode");
    let archive_on_merge_check = CheckButton::with_label("Archive workspace on merge");
    let delete_branch_check = CheckButton::with_label("Delete branch when archiving");
    let auto_upstream_check = CheckButton::with_label("Auto setup upstream remote");

    let general_intro = settings_group(
        "Repository runtime",
        "Commands and runtime flags Linux Conductor uses when preparing and running this repository.",
    );
    general_panel.append(&general_intro.0);
    general_intro.1.append(&settings_field_pair(
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
    general_intro.1.append(&settings_field_pair(
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

    let runtime_flags = settings_group(
        "Runtime flags",
        "Toggle repository-level behavior that affects merges, upstream setup, and privacy defaults.",
    );
    general_panel.append(&runtime_flags.0);
    runtime_flags.1.append(&settings_toggle_row(
        &spotlight_check,
        "Turns on spotlight state tracking for workspace sync flows.",
    ));
    runtime_flags.1.append(&settings_toggle_row(
        &privacy_check,
        "Uses privacy-safe behavior for repository and agent operations.",
    ));
    runtime_flags.1.append(&settings_toggle_row(
        &archive_on_merge_check,
        "Archives the workspace automatically after a successful merge.",
    ));
    runtime_flags.1.append(&settings_toggle_row(
        &delete_branch_check,
        "Deletes the local branch during archive cleanup.",
    ));
    runtime_flags.1.append(&settings_toggle_row(
        &auto_upstream_check,
        "Automatically configures upstream remotes for new worktree branches.",
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

    let provider_paths = settings_group(
        "Provider paths",
        "Executable paths and provider routing for local agent launches.",
    );
    providers_panel.append(&provider_paths.0);
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
        "Provider-specific machine values for Bedrock and Vertex-backed setups.",
    );
    providers_panel.append(&provider_platforms.0);
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

    let branch_prefix_type_entry = machine_entry("branch prefix type");
    let branch_prefix_entry = machine_entry("branch prefix");
    let git_behavior = settings_group(
        "Git behavior",
        "Naming and branch defaults that shape generated workspaces.",
    );
    git_panel.append(&git_behavior.0);
    git_behavior.1.append(&settings_field_pair(
        settings_field(
            "Branch prefix type",
            "Optional naming mode for branch prefixes if your repository uses one.",
            &branch_prefix_type_entry,
        ),
        settings_field(
            "Branch prefix",
            "Short prefix used when Linux Conductor generates branch names.",
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

    let prompt_specs = [
        (
            "General agent instructions",
            "The default prompt context used for agent work in this repository.",
            120,
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
    let settings_repo_entry_load = settings_repo_entry.clone();
    let settings_result_load = settings_result.clone();
    let setup_entry_load = setup_entry.clone();
    let run_entry_load = run_entry.clone();
    let archive_entry_load = archive_entry.clone();
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
    let branch_prefix_type_entry_load = branch_prefix_type_entry.clone();
    let branch_prefix_entry_load = branch_prefix_entry.clone();
    let file_globs_buffer_load = file_globs_view.1.clone();
    let file_globs_text_load = file_globs_view.2.clone();
    let env_buffer_load = env_view.1.clone();
    let customization_buffer_load = customization_view.1.clone();
    let prompt_buffers_load = prompt_views
        .iter()
        .map(|(_, buffer, _)| buffer.clone())
        .collect::<Vec<_>>();
    load_settings_btn.connect_clicked(move |_| {
        let repo_name = settings_repo_entry_load.text().trim().to_owned();
        if repo_name.is_empty() {
            settings_result_load.set_text("Repository name is required.");
            return;
        }
        match repository_root(&db_path_load_settings, &repo_name)
            .and_then(|repo_path| {
                load_repository_settings(&repo_path).map(|settings| (repo_path, settings))
            })
            .and_then(|(repo_path, settings)| {
                inspect_repository_settings(&repo_path)
                    .map(|inspection| (repo_path, settings, inspection))
            }) {
            Ok((repo_path, settings, inspection)) => {
                setup_entry_load.set_text(settings.scripts.setup.as_deref().unwrap_or(""));
                run_entry_load.set_text(settings.scripts.run.as_deref().unwrap_or(""));
                archive_entry_load.set_text(settings.scripts.archive.as_deref().unwrap_or(""));
                run_mode_entry_load
                    .set_text(settings.scripts.run_mode.as_deref().unwrap_or("concurrent"));
                spotlight_check_load.set_active(settings.spotlight_testing.unwrap_or(false));
                privacy_check_load.set_active(settings.enterprise_data_privacy.unwrap_or(false));
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
                branch_prefix_type_entry_load
                    .set_text(settings.git.branch_prefix_type.as_deref().unwrap_or(""));
                branch_prefix_entry_load
                    .set_text(settings.git.branch_prefix.as_deref().unwrap_or(""));
                if inspection.worktreeinclude_exists {
                    file_globs_text_load.set_editable(false);
                    file_globs_buffer_load.set_text(&inspection.active_file_patterns.join("\n"));
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
                    prompts.general,
                    prompts.code_review,
                    prompts.create_pr,
                    prompts.fix_errors,
                    prompts.resolve_merge_conflicts,
                    prompts.rename_branch,
                    prompts.commit_generation,
                    prompts.test_fixing,
                    prompts.refactor_style,
                ];
                for (buffer, value) in prompt_buffers_load.iter().zip(prompt_values.iter()) {
                    buffer.set_text(value.as_deref().unwrap_or(""));
                }
                customization_buffer_load.set_text(
                    &customization_settings_to_toml(&settings.customization).unwrap_or_default(),
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
                    "Loaded {}. Shared={} Local={} Worktreeinclude={} Active files: {} ({})",
                    repo_path.display(),
                    inspection.shared_settings_exists,
                    inspection.local_settings_exists,
                    inspection.worktreeinclude_exists,
                    inspection.active_file_patterns.join(", "),
                    source
                ));
            }
            Err(err) => settings_result_load.set_text(&format!("Load failed: {err:#}")),
        }
    });

    let db_path_save_settings = paths.database_path.clone();
    save_settings_btn.connect_clicked(move |_| {
        let repo_name = settings_repo_entry.text().trim().to_owned();
        if repo_name.is_empty() {
            settings_result.set_text("Repository name is required.");
            return;
        }
        let layer = match layer_select.active_id().as_deref() {
            Some("local") => SettingsLayer::LocalOverride,
            _ => SettingsLayer::RepositoryShared,
        };
        let repo_path = match repository_root(&db_path_save_settings, &repo_name) {
            Ok(path) => path,
            Err(err) => {
                settings_result.set_text(&format!("Save failed: {err:#}"));
                return;
            }
        };
        let current_file_globs = load_repository_settings(&repo_path)
            .map(|settings| settings.file_include_globs)
            .unwrap_or_default();
        let customization =
            match customization_settings_from_toml(&text_buffer_text(&customization_view.1)) {
                Ok(customization) => customization,
                Err(err) => {
                    settings_result
                        .set_text(&format!("Save failed: customization TOML invalid: {err:#}"));
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
            spotlight_testing: Some(spotlight_check.is_active()),
            enterprise_data_privacy: Some(privacy_check.is_active()),
            scripts: ScriptSettings {
                setup: optional_entry_text(&setup_entry),
                run: optional_entry_text(&run_entry),
                archive: optional_entry_text(&archive_entry),
                run_mode: optional_entry_text(&run_mode_entry)
                    .or_else(|| Some("concurrent".to_owned())),
            },
            environment_variables: parse_environment_lines(&text_buffer_text(&env_view.1)),
            prompts: Some(PromptSettings {
                general: optional_buffer_text(&prompt_views[0].1),
                code_review: optional_buffer_text(&prompt_views[1].1),
                create_pr: optional_buffer_text(&prompt_views[2].1),
                fix_errors: optional_buffer_text(&prompt_views[3].1),
                resolve_merge_conflicts: optional_buffer_text(&prompt_views[4].1),
                rename_branch: optional_buffer_text(&prompt_views[5].1),
                commit_generation: optional_buffer_text(&prompt_views[6].1),
                test_fixing: optional_buffer_text(&prompt_views[7].1),
                refactor_style: optional_buffer_text(&prompt_views[8].1),
            }),
            providers: ProviderSettings {
                claude_code_executable_path: optional_entry_text(&claude_path_entry),
                codex_executable_path: optional_entry_text(&codex_path_entry),
                claude_provider: optional_entry_text(&claude_provider_entry),
                codex_provider: optional_entry_text(&codex_provider_entry),
                bedrock_region: optional_entry_text(&bedrock_region_entry),
                vertex_project_id: optional_entry_text(&vertex_project_entry),
                ssh_key_path: None,
            },
            git: GitSettings {
                delete_branch_on_archive: Some(delete_branch_check.is_active()),
                archive_on_merge: Some(archive_on_merge_check.is_active()),
                worktree_push_auto_setup_remote: Some(auto_upstream_check.is_active()),
                branch_prefix_type: optional_entry_text(&branch_prefix_type_entry),
                branch_prefix: optional_entry_text(&branch_prefix_entry),
            },
            customization,
        };
        match save_repository_settings(&repo_path, layer, &settings) {
            Ok(()) => {
                settings_result.set_text(&format!("Saved settings for {}", repo_path.display()))
            }
            Err(err) => settings_result.set_text(&format!("Save failed: {err:#}")),
        }
    });

    (root, || {})
}

fn settings_sections() -> Vec<SettingsSection> {
    vec![
        SettingsSection {
            id: "general",
            title: "General",
            description: "Scripts, runtime flags, and environment.",
        },
        SettingsSection {
            id: "prompts",
            title: "Prompts",
            description: "Prompt bodies for agent tasks and workflows.",
        },
        SettingsSection {
            id: "providers",
            title: "Providers",
            description: "Executable paths and provider platform values.",
        },
        SettingsSection {
            id: "git",
            title: "Git & Workspaces",
            description: "Branch behavior and file copy rules.",
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
        .find(|repo| repo.name == name)
        .map(|repo| repo.root_path)
        .ok_or_else(|| anyhow::anyhow!("repository {name} not found"))
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

fn parse_environment_lines(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            let key = key.trim();
            (!key.is_empty()).then(|| (key.to_owned(), value.trim().to_owned()))
        })
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
            vec!["general", "prompts", "providers", "git", "advanced"]
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
