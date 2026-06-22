use gtk::prelude::*;
use gtk::{
    Box as GBox, Button, CheckButton, ComboBoxText, Entry, Label, Orientation, PolicyType,
    ScrolledWindow, TextView,
};
use linux_conductor_core::paths::AppPaths;
use linux_conductor_core::repository::{AddRepository, RepositoryStore};
use linux_conductor_core::settings::{
    customization_settings_from_toml, customization_settings_to_toml, inspect_repository_settings,
    load_repository_settings, save_repository_settings, FilePatternSource, GitSettings,
    PromptSettings, ProviderSettings, RepositorySettings, ScriptSettings, SettingsLayer,
};
use linux_conductor_core::workspace::{CreateWorkspace, WorkspaceSourcePreflight, WorkspaceStore};
use std::path::PathBuf;

use crate::{default_clone_parent, detail_row, repo_name_from_url};

pub(crate) fn build_projects_page(
    paths: &AppPaths,
    refresh_dashboard: impl Fn() + Clone + 'static,
    refresh_workspace: impl Fn() + Clone + 'static,
) -> (GBox, impl Fn() + Clone + 'static) {
    let root = GBox::new(Orientation::Vertical, 0);
    root.add_css_class("dashboard");
    root.add_css_class("page-shell");
    let header = GBox::new(Orientation::Vertical, 8);
    header.add_css_class("dashboard-header");
    header.add_css_class("page-header");
    let title = Label::new(Some("Projects"));
    title.add_css_class("dashboard-title");
    title.set_xalign(0.0);
    let subtitle = Label::new(Some("Create workspaces and inspect imported repositories."));
    subtitle.add_css_class("card-meta");
    subtitle.set_xalign(0.0);
    header.append(&title);
    header.append(&subtitle);
    root.append(&header);

    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    scroll.set_vexpand(true);
    let body = GBox::new(Orientation::Vertical, 14);
    body.add_css_class("detail-body");
    body.add_css_class("page-body");
    scroll.set_child(Some(&body));
    root.append(&scroll);

    let repo_title = Label::new(Some("Add Repository"));
    repo_title.add_css_class("section-title");
    repo_title.set_xalign(0.0);
    body.append(&repo_title);

    let repo_box = GBox::new(Orientation::Horizontal, 8);
    let repo_path_entry = Entry::new();
    repo_path_entry.set_placeholder_text(Some("local path or git URL"));
    repo_path_entry.set_hexpand(true);
    let repo_name_entry = Entry::new();
    repo_name_entry.set_placeholder_text(Some("project name, optional"));
    let add_repo_btn = Button::with_label("Add Local");
    let clone_repo_btn = Button::with_label("Clone");
    let repo_result = Label::new(Some(&format!(
        "Clones go to {}. Local repos infer the project name from the folder when blank.",
        default_clone_parent().display()
    )));
    repo_result.add_css_class("card-meta");
    repo_result.set_xalign(0.0);
    repo_box.append(&repo_path_entry);
    repo_box.append(&repo_name_entry);
    repo_box.append(&add_repo_btn);
    repo_box.append(&clone_repo_btn);
    body.append(&repo_box);
    body.append(&repo_result);

    let workspace_title = Label::new(Some("New Workspace"));
    workspace_title.add_css_class("section-title");
    workspace_title.set_xalign(0.0);
    workspace_title.set_margin_top(10);
    body.append(&workspace_title);

    let create_box = GBox::new(Orientation::Horizontal, 8);
    let repo_entry = Entry::new();
    repo_entry.set_placeholder_text(Some("repository name"));
    repo_entry.set_width_chars(18);
    let name_entry = Entry::new();
    name_entry.set_placeholder_text(Some("workspace name"));
    let branch_entry = Entry::new();
    branch_entry.set_placeholder_text(Some("branch name"));
    let base_entry = Entry::new();
    base_entry.set_placeholder_text(Some("base ref, optional"));
    let source_select = ComboBoxText::new();
    source_select.append(Some("branch"), "Branch");
    source_select.append(Some("github_issue"), "GitHub Issue");
    source_select.append(Some("github_pr"), "GitHub PR");
    source_select.append(Some("linear_issue"), "Linear Issue");
    source_select.append(Some("prompt"), "Prompt");
    source_select.set_active_id(Some("branch"));
    let source_entry = Entry::new();
    source_entry.set_placeholder_text(Some("issue/PR id or prompt"));
    let check_sources_btn = Button::with_label("Check Sources");
    let create_btn = Button::with_label("Create Workspace");
    let result = Label::new(None);
    result.add_css_class("card-meta");
    result.set_xalign(0.0);
    create_box.append(&repo_entry);
    create_box.append(&name_entry);
    create_box.append(&branch_entry);
    create_box.append(&base_entry);
    create_box.append(&source_select);
    create_box.append(&source_entry);
    create_box.append(&check_sources_btn);
    create_box.append(&create_btn);
    body.append(&create_box);
    body.append(&result);

    let repo_list = GBox::new(Orientation::Vertical, 8);
    body.append(&repo_list);

    let settings_title = Label::new(Some("Project Settings"));
    settings_title.add_css_class("section-title");
    settings_title.set_xalign(0.0);
    settings_title.set_margin_top(10);
    body.append(&settings_title);

    let settings_grid = GBox::new(Orientation::Vertical, 10);
    settings_grid.add_css_class("settings-panel");
    body.append(&settings_grid);

    let settings_top = GBox::new(Orientation::Horizontal, 8);
    let settings_repo_entry = Entry::new();
    settings_repo_entry.set_placeholder_text(Some("repository name"));
    let layer_select = ComboBoxText::new();
    layer_select.append(Some("shared"), "Shared");
    layer_select.append(Some("local"), "Local");
    layer_select.set_active_id(Some("shared"));
    let load_settings_btn = Button::with_label("Load Settings");
    let save_settings_btn = Button::with_label("Save Settings");
    settings_top.append(&settings_repo_entry);
    settings_top.append(&layer_select);
    settings_top.append(&load_settings_btn);
    settings_top.append(&save_settings_btn);
    settings_grid.append(&settings_top);

    let settings_result = Label::new(Some(
        "Shared settings are commit-safe. Use Local for machine secrets and overrides.",
    ));
    settings_result.add_css_class("card-meta");
    settings_result.set_xalign(0.0);
    settings_result.set_wrap(true);
    settings_grid.append(&settings_result);

    let scripts_section = Label::new(Some("Scripts"));
    scripts_section.add_css_class("section-title");
    scripts_section.set_xalign(0.0);
    scripts_section.set_margin_top(6);
    settings_grid.append(&scripts_section);

    let scripts_row = GBox::new(Orientation::Horizontal, 8);
    let setup_entry = Entry::new();
    setup_entry.set_placeholder_text(Some("setup script"));
    let run_entry = Entry::new();
    run_entry.set_placeholder_text(Some("run script"));
    let archive_entry = Entry::new();
    archive_entry.set_placeholder_text(Some("archive script"));
    let run_mode_entry = Entry::new();
    run_mode_entry.set_placeholder_text(Some("run mode: concurrent/nonconcurrent"));
    scripts_row.append(&setup_entry);
    scripts_row.append(&run_entry);
    scripts_row.append(&archive_entry);
    scripts_row.append(&run_mode_entry);
    settings_grid.append(&scripts_row);

    let booleans_row = GBox::new(Orientation::Horizontal, 10);
    let spotlight_check = CheckButton::with_label("Spotlight testing");
    let privacy_check = CheckButton::with_label("Enterprise data privacy");
    let archive_on_merge_check = CheckButton::with_label("Archive on merge");
    let delete_branch_check = CheckButton::with_label("Delete branch on archive");
    let auto_upstream_check = CheckButton::with_label("Auto setup upstream");
    booleans_row.append(&spotlight_check);
    booleans_row.append(&privacy_check);
    booleans_row.append(&archive_on_merge_check);
    booleans_row.append(&delete_branch_check);
    booleans_row.append(&auto_upstream_check);
    settings_grid.append(&booleans_row);

    let provider_row = GBox::new(Orientation::Horizontal, 8);
    let claude_path_entry = Entry::new();
    claude_path_entry.set_placeholder_text(Some("Claude executable"));
    let codex_path_entry = Entry::new();
    codex_path_entry.set_placeholder_text(Some("Codex executable"));
    let claude_provider_entry = Entry::new();
    claude_provider_entry.set_placeholder_text(Some("Claude provider"));
    let codex_provider_entry = Entry::new();
    codex_provider_entry.set_placeholder_text(Some("Codex provider"));
    provider_row.append(&claude_path_entry);
    provider_row.append(&codex_path_entry);
    provider_row.append(&claude_provider_entry);
    provider_row.append(&codex_provider_entry);
    settings_grid.append(&provider_row);

    let git_row = GBox::new(Orientation::Horizontal, 8);
    let branch_prefix_type_entry = Entry::new();
    branch_prefix_type_entry.set_placeholder_text(Some("branch prefix type"));
    let branch_prefix_entry = Entry::new();
    branch_prefix_entry.set_placeholder_text(Some("branch prefix"));
    let bedrock_region_entry = Entry::new();
    bedrock_region_entry.set_placeholder_text(Some("Bedrock region"));
    let vertex_project_entry = Entry::new();
    vertex_project_entry.set_placeholder_text(Some("Vertex project id"));
    git_row.append(&branch_prefix_type_entry);
    git_row.append(&branch_prefix_entry);
    git_row.append(&bedrock_region_entry);
    git_row.append(&vertex_project_entry);
    settings_grid.append(&git_row);

    let files_label = Label::new(Some("Files to copy"));
    files_label.add_css_class("detail-label");
    files_label.set_xalign(0.0);
    settings_grid.append(&files_label);
    let file_globs_view = settings_text_view(72);
    settings_grid.append(&file_globs_view.0);

    let env_label = Label::new(Some("Environment variables (KEY=value)"));
    env_label.add_css_class("detail-label");
    env_label.set_xalign(0.0);
    settings_grid.append(&env_label);
    let env_view = settings_text_view(72);
    settings_grid.append(&env_view.0);

    let prompts_section = Label::new(Some("Prompts"));
    prompts_section.add_css_class("section-title");
    prompts_section.set_xalign(0.0);
    prompts_section.set_margin_top(6);
    settings_grid.append(&prompts_section);

    let general_label = Label::new(Some("General agent instructions"));
    general_label.add_css_class("detail-label");
    general_label.set_xalign(0.0);
    settings_grid.append(&general_label);
    let general_prompt_view = settings_text_view(84);
    settings_grid.append(&general_prompt_view.0);

    let review_label = Label::new(Some("Code review"));
    review_label.add_css_class("detail-label");
    review_label.set_xalign(0.0);
    settings_grid.append(&review_label);
    let review_prompt_view = settings_text_view(84);
    settings_grid.append(&review_prompt_view.0);

    let create_pr_label = Label::new(Some("Create PR"));
    create_pr_label.add_css_class("detail-label");
    create_pr_label.set_xalign(0.0);
    settings_grid.append(&create_pr_label);
    let create_pr_prompt_view = settings_text_view(84);
    settings_grid.append(&create_pr_prompt_view.0);

    let fix_errors_label = Label::new(Some("Fix errors / failing checks"));
    fix_errors_label.add_css_class("detail-label");
    fix_errors_label.set_xalign(0.0);
    settings_grid.append(&fix_errors_label);
    let fix_errors_prompt_view = settings_text_view(84);
    settings_grid.append(&fix_errors_prompt_view.0);

    let conflicts_label = Label::new(Some("Resolve merge conflicts"));
    conflicts_label.add_css_class("detail-label");
    conflicts_label.set_xalign(0.0);
    settings_grid.append(&conflicts_label);
    let conflicts_prompt_view = settings_text_view(84);
    settings_grid.append(&conflicts_prompt_view.0);

    let rename_branch_label = Label::new(Some("Rename branch"));
    rename_branch_label.add_css_class("detail-label");
    rename_branch_label.set_xalign(0.0);
    settings_grid.append(&rename_branch_label);
    let rename_branch_prompt_view = settings_text_view(84);
    settings_grid.append(&rename_branch_prompt_view.0);

    let commit_label = Label::new(Some("Commit message generation"));
    commit_label.add_css_class("detail-label");
    commit_label.set_xalign(0.0);
    settings_grid.append(&commit_label);
    let commit_prompt_view = settings_text_view(84);
    settings_grid.append(&commit_prompt_view.0);

    let test_fixing_label = Label::new(Some("Test fixing"));
    test_fixing_label.add_css_class("detail-label");
    test_fixing_label.set_xalign(0.0);
    settings_grid.append(&test_fixing_label);
    let test_fixing_prompt_view = settings_text_view(84);
    settings_grid.append(&test_fixing_prompt_view.0);

    let refactor_label = Label::new(Some("Refactor style"));
    refactor_label.add_css_class("detail-label");
    refactor_label.set_xalign(0.0);
    settings_grid.append(&refactor_label);
    let refactor_prompt_view = settings_text_view(84);
    settings_grid.append(&refactor_prompt_view.0);

    let advanced_label = Label::new(Some(
        "Advanced customization TOML: naming, automation, agent profiles, merge rules, workspace defaults, and view settings",
    ));
    advanced_label.add_css_class("detail-label");
    advanced_label.set_xalign(0.0);
    advanced_label.set_wrap(true);
    settings_grid.append(&advanced_label);
    let customization_view = settings_text_view(180);
    settings_grid.append(&customization_view.0);

    let db_path = paths.database_path.clone();
    let refresh = {
        let repo_list = repo_list.clone();
        move || {
            while let Some(child) = repo_list.first_child() {
                repo_list.remove(&child);
            }
            if let Ok(store) = RepositoryStore::open(db_path.clone()) {
                if let Ok(repos) = store.list_with_workspace_counts() {
                    for (repo, active, total) in repos {
                        repo_list.append(&detail_row(
                            &repo.name,
                            &format!(
                                "{} active / {} total / {} / base {}",
                                active,
                                total,
                                repo.root_path.display(),
                                repo.default_branch
                            ),
                        ));
                    }
                }
            }
        }
    };

    let db_path_repo = paths.database_path.clone();
    let refresh_after_repo = refresh.clone();
    let repo_result_add = repo_result.clone();
    let repo_path_add = repo_path_entry.clone();
    let repo_name_add = repo_name_entry.clone();
    let repo_entry_after_add = repo_entry.clone();
    let settings_repo_after_add = settings_repo_entry.clone();
    add_repo_btn.connect_clicked(move |_| {
        let path = repo_path_add.text().trim().to_owned();
        let name = repo_name_add.text().trim().to_owned();
        if path.is_empty() {
            repo_result_add.set_text("Local repository path is required.");
            return;
        }
        match RepositoryStore::open(db_path_repo.clone()).and_then(|store| {
            store.add(AddRepository {
                name: (!name.is_empty()).then_some(name),
                root_path: PathBuf::from(path),
                default_branch: None,
                remote_name: "origin".to_owned(),
                workspace_parent_path: None,
            })
        }) {
            Ok(repo) => {
                repo_entry_after_add.set_text(&repo.name);
                settings_repo_after_add.set_text(&repo.name);
                repo_result_add.set_text(&format!(
                    "Added {}. Base branch: {}. Workspace parent: {}",
                    repo.name,
                    repo.default_branch,
                    repo.workspace_parent_path.display()
                ));
                refresh_after_repo();
            }
            Err(err) => repo_result_add.set_text(&format!("Add failed: {err:#}")),
        }
    });

    let db_path_clone = paths.database_path.clone();
    let refresh_after_clone = refresh.clone();
    let repo_entry_after_clone = repo_entry.clone();
    let settings_repo_after_clone = settings_repo_entry.clone();
    clone_repo_btn.connect_clicked(move |_| {
        let url = repo_path_entry.text().trim().to_owned();
        let explicit_name = repo_name_entry.text().trim().to_owned();
        if url.is_empty() {
            repo_result.set_text("Git URL is required.");
            return;
        }
        let name = if explicit_name.is_empty() {
            repo_name_from_url(&url)
        } else {
            explicit_name
        };
        let clone_path = default_clone_parent().join(&name);
        repo_result.set_text(&format!("Cloning {} into {}...", url, clone_path.display()));
        if let Some(parent) = clone_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let clone_result = if clone_path.exists() {
            Ok(())
        } else {
            std::process::Command::new("git")
                .args(["clone", &url])
                .arg(&clone_path)
                .status()
                .map(|status| {
                    if status.success() {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("git clone exited with {status}"))
                    }
                })
                .unwrap_or_else(|err| Err(err.into()))
        };
        match clone_result.and_then(|_| {
            RepositoryStore::open(db_path_clone.clone()).and_then(|store| {
                store.add(AddRepository {
                    name: Some(name),
                    root_path: clone_path,
                    default_branch: None,
                    remote_name: "origin".to_owned(),
                    workspace_parent_path: None,
                })
            })
        }) {
            Ok(repo) => {
                repo_entry_after_clone.set_text(&repo.name);
                settings_repo_after_clone.set_text(&repo.name);
                repo_result.set_text(&format!(
                    "Cloned and added {}. Base branch: {}. Workspace parent: {}",
                    repo.name,
                    repo.default_branch,
                    repo.workspace_parent_path.display()
                ));
                refresh_after_clone();
            }
            Err(err) => repo_result.set_text(&format!("Clone failed: {err:#}")),
        }
    });

    let db_path_check_sources = paths.database_path.clone();
    let result_for_check_sources = result.clone();
    check_sources_btn.connect_clicked(move |_| {
        match WorkspaceStore::open(db_path_check_sources.clone()) {
            Ok(store) => {
                result_for_check_sources.set_text(&source_preflight_text(&store.source_preflight()))
            }
            Err(err) => {
                result_for_check_sources.set_text(&format!("Source preflight failed: {err:#}"))
            }
        }
    });

    let db_path_create = paths.database_path.clone();
    let refresh_after_create = refresh.clone();
    let refresh_after_settings_save = refresh.clone();
    create_btn.connect_clicked(move |_| {
        let repo = repo_entry.text().trim().to_owned();
        let typed_name = name_entry.text().trim().to_owned();
        let typed_branch = branch_entry.text().trim().to_owned();
        let base = optional_entry_text(&base_entry);
        let source = source_select
            .active_id()
            .map(|id| id.to_string())
            .unwrap_or_else(|| "branch".to_owned());
        let source_value = source_entry.text().trim().to_owned();
        if repo.is_empty() {
            result.set_text("Repository is required.");
            return;
        }
        result.set_text("Creating workspace...");
        let request = workspace_source_request_from_form(
            &source,
            &source_value,
            base,
            (!typed_name.is_empty()).then_some(typed_name),
            (!typed_branch.is_empty()).then_some(typed_branch),
        );
        let create_result = request.and_then(|request| {
            WorkspaceStore::open(db_path_create.clone())
                .and_then(|store| request.create_workspace(&store, &repo))
        });
        let created = create_result.is_ok();
        result.set_text(&workspace_source_create_feedback(&source, create_result));
        if created {
            refresh_after_create();
            refresh_dashboard();
            refresh_workspace();
        }
    });

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
    let general_prompt_buffer_load = general_prompt_view.1.clone();
    let review_prompt_buffer_load = review_prompt_view.1.clone();
    let create_pr_prompt_buffer_load = create_pr_prompt_view.1.clone();
    let fix_errors_prompt_buffer_load = fix_errors_prompt_view.1.clone();
    let conflicts_prompt_buffer_load = conflicts_prompt_view.1.clone();
    let rename_branch_prompt_buffer_load = rename_branch_prompt_view.1.clone();
    let commit_prompt_buffer_load = commit_prompt_view.1.clone();
    let test_fixing_prompt_buffer_load = test_fixing_prompt_view.1.clone();
    let refactor_prompt_buffer_load = refactor_prompt_view.1.clone();
    let customization_buffer_load = customization_view.1.clone();
    load_settings_btn.connect_clicked(move |_| {
        let repo_name = settings_repo_entry_load.text().trim().to_owned();
        if repo_name.is_empty() {
            settings_result_load.set_text("Repository name is required.");
            return;
        }
        match repository_root(&db_path_load_settings, &repo_name)
            .and_then(|repo_path| load_repository_settings(&repo_path).map(|settings| (repo_path, settings)))
            .and_then(|(repo_path, settings)| {
                inspect_repository_settings(&repo_path).map(|inspection| (repo_path, settings, inspection))
            }) {
            Ok((repo_path, settings, inspection)) => {
                setup_entry_load.set_text(settings.scripts.setup.as_deref().unwrap_or(""));
                run_entry_load.set_text(settings.scripts.run.as_deref().unwrap_or(""));
                archive_entry_load.set_text(settings.scripts.archive.as_deref().unwrap_or(""));
                run_mode_entry_load.set_text(settings.scripts.run_mode.as_deref().unwrap_or("concurrent"));
                spotlight_check_load.set_active(settings.spotlight_testing.unwrap_or(false));
                privacy_check_load.set_active(settings.enterprise_data_privacy.unwrap_or(false));
                archive_on_merge_check_load.set_active(settings.git.archive_on_merge.unwrap_or(false));
                delete_branch_check_load.set_active(settings.git.delete_branch_on_archive.unwrap_or(false));
                auto_upstream_check_load.set_active(settings.git.worktree_push_auto_setup_remote.unwrap_or(false));
                claude_path_entry_load.set_text(settings.providers.claude_code_executable_path.as_deref().unwrap_or(""));
                codex_path_entry_load.set_text(settings.providers.codex_executable_path.as_deref().unwrap_or(""));
                claude_provider_entry_load.set_text(settings.providers.claude_provider.as_deref().unwrap_or(""));
                codex_provider_entry_load.set_text(settings.providers.codex_provider.as_deref().unwrap_or(""));
                bedrock_region_entry_load.set_text(settings.providers.bedrock_region.as_deref().unwrap_or(""));
                vertex_project_entry_load.set_text(settings.providers.vertex_project_id.as_deref().unwrap_or(""));
                branch_prefix_type_entry_load.set_text(settings.git.branch_prefix_type.as_deref().unwrap_or(""));
                branch_prefix_entry_load.set_text(settings.git.branch_prefix.as_deref().unwrap_or(""));
                if inspection.worktreeinclude_exists {
                    file_globs_text_load.set_editable(false);
                    file_globs_buffer_load.set_text(&inspection.active_file_patterns.join("\n"));
                } else {
                    file_globs_text_load.set_editable(true);
                    file_globs_buffer_load.set_text(&settings.file_include_globs.join("\n"));
                }
                env_buffer_load.set_text(&settings.environment_variables.iter().map(|(key, value)| format!("{key}={value}")).collect::<Vec<_>>().join("\n"));
                let prompts = settings.prompts.unwrap_or_default();
                general_prompt_buffer_load.set_text(prompts.general.as_deref().unwrap_or(""));
                review_prompt_buffer_load.set_text(prompts.code_review.as_deref().unwrap_or(""));
                create_pr_prompt_buffer_load.set_text(prompts.create_pr.as_deref().unwrap_or(""));
                fix_errors_prompt_buffer_load.set_text(prompts.fix_errors.as_deref().unwrap_or(""));
                conflicts_prompt_buffer_load.set_text(prompts.resolve_merge_conflicts.as_deref().unwrap_or(""));
                rename_branch_prompt_buffer_load.set_text(prompts.rename_branch.as_deref().unwrap_or(""));
                commit_prompt_buffer_load.set_text(prompts.commit_generation.as_deref().unwrap_or(""));
                test_fixing_prompt_buffer_load.set_text(prompts.test_fixing.as_deref().unwrap_or(""));
                refactor_prompt_buffer_load.set_text(prompts.refactor_style.as_deref().unwrap_or(""));
                customization_buffer_load.set_text(
                    &customization_settings_to_toml(&settings.customization).unwrap_or_default(),
                );
                let source = match inspection.active_file_patterns_source {
                    FilePatternSource::Worktreeinclude => ".worktreeinclude wins; Files to copy is read-only preview for new workspace copying.",
                    FilePatternSource::RepositorySettings => "repository settings provide Files to copy patterns.",
                    FilePatternSource::BuiltInDefault => "built-in default .env* pattern applies until settings are saved.",
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
                general: optional_buffer_text(&general_prompt_view.1),
                code_review: optional_buffer_text(&review_prompt_view.1),
                create_pr: optional_buffer_text(&create_pr_prompt_view.1),
                fix_errors: optional_buffer_text(&fix_errors_prompt_view.1),
                resolve_merge_conflicts: optional_buffer_text(&conflicts_prompt_view.1),
                rename_branch: optional_buffer_text(&rename_branch_prompt_view.1),
                commit_generation: optional_buffer_text(&commit_prompt_view.1),
                test_fixing: optional_buffer_text(&test_fixing_prompt_view.1),
                refactor_style: optional_buffer_text(&refactor_prompt_view.1),
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
                settings_result.set_text(&format!("Saved settings for {}", repo_path.display()));
                refresh_after_settings_save();
            }
            Err(err) => settings_result.set_text(&format!("Save failed: {err:#}")),
        }
    });

    refresh();
    (root, refresh)
}

fn repository_root(db_path: &PathBuf, name: &str) -> anyhow::Result<PathBuf> {
    RepositoryStore::open(db_path)?
        .list()?
        .into_iter()
        .find(|repo| repo.name == name)
        .map(|repo| repo.root_path)
        .ok_or_else(|| anyhow::anyhow!("repository {name} not found"))
}

fn source_preflight_text(preflight: &WorkspaceSourcePreflight) -> String {
    format!(
        "Source readiness: GitHub {}. Linear {}.",
        preflight.github_status(),
        preflight.linear_status()
    )
}

fn workspace_source_create_feedback(
    source: &str,
    result: anyhow::Result<linux_conductor_core::workspace::Workspace>,
) -> String {
    match result {
        Ok(workspace) => format!(
            "Created {} at {} from {}",
            workspace.name,
            workspace.path.display(),
            source
        ),
        Err(err) => format!("Create failed from {source}: {err:#}"),
    }
}

fn settings_text_view(height: i32) -> (ScrolledWindow, gtk::TextBuffer, TextView) {
    let view = TextView::new();
    view.set_monospace(true);
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_size_request(-1, height);
    let buffer = view.buffer();
    let scroll = ScrolledWindow::new();
    scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    scroll.set_child(Some(&view));
    (scroll, buffer, view)
}

fn optional_entry_text(entry: &Entry) -> Option<String> {
    let value = entry.text().trim().to_owned();
    (!value.is_empty()).then_some(value)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WorkspaceSourceRequest {
    Branch {
        name: String,
        branch: String,
        base: Option<String>,
    },
    GitHubIssue {
        number: u64,
    },
    GitHubPullRequest {
        number: u64,
        name: Option<String>,
        branch: Option<String>,
    },
    LinearIssue {
        id: String,
        name: Option<String>,
        branch: Option<String>,
        base: Option<String>,
    },
    Prompt {
        prompt: String,
        name: Option<String>,
        branch: Option<String>,
        base: Option<String>,
    },
}

impl WorkspaceSourceRequest {
    fn source_label(&self) -> &'static str {
        match self {
            Self::Branch { .. } => "branch",
            Self::GitHubIssue { .. } => "github_issue",
            Self::GitHubPullRequest { .. } => "github_pr",
            Self::LinearIssue { .. } => "linear_issue",
            Self::Prompt { .. } => "prompt",
        }
    }

    fn create_workspace(
        &self,
        store: &WorkspaceStore,
        repository_name: &str,
    ) -> anyhow::Result<linux_conductor_core::workspace::Workspace> {
        match self {
            Self::Branch { name, branch, base } => store.create(CreateWorkspace {
                repository_name: repository_name.to_owned(),
                name: name.clone(),
                branch: branch.clone(),
                base_ref: base.clone(),
            }),
            Self::GitHubIssue { number } => store.create_from_issue(repository_name, *number, None),
            Self::GitHubPullRequest {
                number,
                name,
                branch,
            } => store.create_from_pull_request(
                repository_name,
                *number,
                name.as_deref(),
                branch.as_deref(),
            ),
            Self::LinearIssue {
                id,
                name,
                branch,
                base,
            } => store.create_from_linear_issue(
                repository_name,
                id,
                name.as_deref(),
                branch.as_deref(),
                base.as_deref(),
            ),
            Self::Prompt {
                prompt,
                name,
                branch,
                base,
            } => store.create_from_prompt(
                repository_name,
                prompt,
                name.as_deref(),
                branch.as_deref(),
                base.as_deref(),
            ),
        }
    }
}

fn workspace_source_request_from_form(
    source: &str,
    source_value: &str,
    base: Option<String>,
    typed_name: Option<String>,
    typed_branch: Option<String>,
) -> anyhow::Result<WorkspaceSourceRequest> {
    match source {
        "github_issue" => Ok(WorkspaceSourceRequest::GitHubIssue {
            number: source_value
                .trim()
                .trim_start_matches('#')
                .parse::<u64>()
                .map_err(|_| anyhow::anyhow!("GitHub issue number is required"))?,
        }),
        "github_pr" => {
            if base.is_some() {
                anyhow::bail!("Base ref is fetched from GitHub for PR workspaces.");
            }
            Ok(WorkspaceSourceRequest::GitHubPullRequest {
                number: source_value
                    .trim()
                    .trim_start_matches('#')
                    .parse::<u64>()
                    .map_err(|_| anyhow::anyhow!("GitHub PR number is required"))?,
                name: typed_name,
                branch: typed_branch,
            })
        }
        "linear_issue" => {
            let id = source_value.trim();
            anyhow::ensure!(!id.is_empty(), "Linear issue id is required");
            Ok(WorkspaceSourceRequest::LinearIssue {
                id: id.to_owned(),
                name: typed_name,
                branch: typed_branch,
                base,
            })
        }
        "prompt" => {
            let prompt = source_value.trim();
            anyhow::ensure!(!prompt.is_empty(), "Prompt is required");
            Ok(WorkspaceSourceRequest::Prompt {
                prompt: prompt.to_owned(),
                name: typed_name,
                branch: typed_branch,
                base,
            })
        }
        _ => {
            let name = typed_name.ok_or_else(|| anyhow::anyhow!("Workspace name is required"))?;
            let branch = typed_branch.ok_or_else(|| anyhow::anyhow!("Branch name is required"))?;
            Ok(WorkspaceSourceRequest::Branch { name, branch, base })
        }
    }
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
    fn source_preflight_text_summarizes_github_and_linear_readiness() {
        let text = source_preflight_text(&WorkspaceSourcePreflight {
            github_cli_installed: true,
            github_authenticated: false,
            linear_api_key_set: false,
        });

        assert_eq!(
            text,
            "Source readiness: GitHub gh auth required. Linear LINEAR_API_KEY missing."
        );
    }

    #[test]
    fn workspace_source_request_from_form_validates_source_specific_fields() {
        let branch = workspace_source_request_from_form(
            "branch",
            "",
            Some("main".to_owned()),
            Some("berlin".to_owned()),
            Some("lc/berlin".to_owned()),
        )
        .unwrap();
        assert!(matches!(branch, WorkspaceSourceRequest::Branch { .. }));

        let issue =
            workspace_source_request_from_form("github_issue", "#123", None, None, None).unwrap();
        assert_eq!(issue.source_label(), "github_issue");

        let pr_with_base = workspace_source_request_from_form(
            "github_pr",
            "10",
            Some("main".to_owned()),
            None,
            None,
        )
        .unwrap_err();
        assert_eq!(
            pr_with_base.to_string(),
            "Base ref is fetched from GitHub for PR workspaces."
        );

        let missing_prompt =
            workspace_source_request_from_form("prompt", "   ", None, None, None).unwrap_err();
        assert_eq!(missing_prompt.to_string(), "Prompt is required");
    }

    #[test]
    fn workspace_source_create_feedback_summarizes_success_and_failure() {
        let workspace = linux_conductor_core::workspace::Workspace {
            id: 1,
            repository_id: 2,
            name: "pr-10".to_owned(),
            path: PathBuf::from("/tmp/pr-10"),
            branch: "lc/pr-10".to_owned(),
            base_ref: "refs/linux-conductor/pull-requests/10".to_owned(),
            port_base: 3000,
            status: "active".to_owned(),
            archived_at: None,
            created_at: "now".to_owned(),
            updated_at: "now".to_owned(),
        };

        assert_eq!(
            workspace_source_create_feedback("github_pr", Ok(workspace)),
            "Created pr-10 at /tmp/pr-10 from github_pr"
        );
        assert_eq!(
            workspace_source_create_feedback(
                "linear_issue",
                Err(anyhow::anyhow!("LINEAR_API_KEY missing")),
            ),
            "Create failed from linear_issue: LINEAR_API_KEY missing"
        );
    }
}
