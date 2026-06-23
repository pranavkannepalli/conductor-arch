use anyhow::{Context, Result};
use gtk::prelude::*;
use gtk::{
    Box as GBox, Button, CheckButton, ComboBoxText, Entry, Label, Orientation, PolicyType,
    ScrolledWindow, Stack, TextView,
};
use linux_conductor_core::paths::AppPaths;
use linux_conductor_core::repository::{AddRepository, RepositoryStore};
use linux_conductor_core::settings::{
    customization_settings_from_toml, customization_settings_to_toml, inspect_repository_settings,
    load_repository_settings, save_repository_settings, FilePatternSource, GitSettings,
    PromptSettings, ProviderSettings, RepositorySettings, ScriptSettings, SettingsLayer,
};
use linux_conductor_core::workspace::{CreateWorkspace, WorkspaceSourcePreflight, WorkspaceStore};
use serde_json::Value;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

use crate::buttons::text_button;
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

    let repo_actions = GBox::new(Orientation::Horizontal, 8);
    repo_actions.add_css_class("project-actions-row");
    let open_add_local_btn = text_button("Add Local Repository");
    open_add_local_btn.add_css_class("suggested-action");
    let open_clone_btn = text_button("Clone Repository");
    let open_new_project_btn = text_button("New Project");
    repo_actions.append(&open_add_local_btn);
    repo_actions.append(&open_clone_btn);
    repo_actions.append(&open_new_project_btn);
    body.append(&repo_actions);

    let repo_box = GBox::new(Orientation::Horizontal, 8);
    let repo_path_entry = Entry::new();
    repo_path_entry.set_placeholder_text(Some("local path or git URL"));
    repo_path_entry.set_hexpand(true);
    let repo_name_entry = Entry::new();
    repo_name_entry.set_placeholder_text(Some("project name, optional"));
    let add_repo_btn = text_button("Add Local");
    let clone_repo_btn = text_button("Clone");
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
    repo_box.set_visible(false);
    body.append(&repo_box);
    repo_result.set_visible(false);
    body.append(&repo_result);

    let workspace_title = Label::new(Some("New Workspace"));
    workspace_title.add_css_class("section-title");
    workspace_title.set_xalign(0.0);
    workspace_title.set_margin_top(10);
    body.append(&workspace_title);

    let workspace_actions = GBox::new(Orientation::Horizontal, 8);
    workspace_actions.add_css_class("project-actions-row");
    let open_workspace_modal_btn = text_button("Create Workspace");
    open_workspace_modal_btn.add_css_class("suggested-action");
    let open_source_check_btn = text_button("Check Sources");
    workspace_actions.append(&open_workspace_modal_btn);
    workspace_actions.append(&open_source_check_btn);
    body.append(&workspace_actions);

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
    let check_sources_btn = text_button("Check Sources");
    let create_btn = text_button("Create Workspace");
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
    create_box.set_visible(false);
    body.append(&create_box);
    result.set_visible(false);
    body.append(&result);

    let repo_list = GBox::new(Orientation::Vertical, 8);
    body.append(&repo_list);

    let settings_cta = GBox::new(Orientation::Horizontal, 10);
    settings_cta.add_css_class("settings-cta");
    let settings_title = Label::new(Some("Project settings moved"));
    settings_title.add_css_class("section-title");
    settings_title.set_xalign(0.0);
    settings_title.set_margin_top(10);
    settings_cta.append(&settings_title);
    let settings_hint = Label::new(Some(
        "Use the dedicated Settings page instead of editing everything inline here.",
    ));
    settings_hint.add_css_class("card-meta");
    settings_hint.set_hexpand(true);
    settings_hint.set_xalign(0.0);
    settings_cta.append(&settings_hint);
    body.append(&settings_cta);

    let settings_grid = GBox::new(Orientation::Vertical, 10);
    settings_grid.add_css_class("settings-panel");

    let settings_top = GBox::new(Orientation::Horizontal, 8);
    let settings_repo_entry = Entry::new();
    settings_repo_entry.set_placeholder_text(Some("repository name"));
    let layer_select = ComboBoxText::new();
    layer_select.append(Some("shared"), "Shared");
    layer_select.append(Some("local"), "Local");
    layer_select.set_active_id(Some("shared"));
    let load_settings_btn = text_button("Load Settings");
    let save_settings_btn = text_button("Save Settings");
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

    let quick_add_refresh: Rc<dyn Fn()> = Rc::new({
        let refresh_after_quick_add = refresh.clone();
        let refresh_dashboard_after_quick_add = refresh_dashboard.clone();
        let refresh_workspace_after_quick_add = refresh_workspace.clone();
        move || {
            refresh_after_quick_add();
            refresh_dashboard_after_quick_add();
            refresh_workspace_after_quick_add();
        }
    });
    let db_path_modal_add = paths.database_path.clone();
    let quick_add_refresh_for_folder = quick_add_refresh.clone();
    open_add_local_btn.connect_clicked(move |_| {
        show_repository_quick_add_dialog(
            db_path_modal_add.clone(),
            quick_add_refresh_for_folder.clone(),
            Some("folder"),
        );
    });

    let db_path_modal_clone = paths.database_path.clone();
    let quick_add_refresh_for_clone = quick_add_refresh.clone();
    open_clone_btn.connect_clicked(move |_| {
        show_repository_quick_add_dialog(
            db_path_modal_clone.clone(),
            quick_add_refresh_for_clone.clone(),
            Some("clone"),
        );
    });

    let db_path_modal_new = paths.database_path.clone();
    let quick_add_refresh_for_new = quick_add_refresh.clone();
    open_new_project_btn.connect_clicked(move |_| {
        show_repository_quick_add_dialog(
            db_path_modal_new.clone(),
            quick_add_refresh_for_new.clone(),
            Some("new"),
        );
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

    let db_path_modal_check_sources = paths.database_path.clone();
    open_source_check_btn.connect_clicked(move |_| {
        let message = match WorkspaceStore::open(db_path_modal_check_sources.clone()) {
            Ok(store) => source_preflight_text(&store.source_preflight()),
            Err(err) => format!("Source preflight failed: {err:#}"),
        };
        let dialog = gtk::Window::builder()
            .title("Source Readiness")
            .modal(true)
            .default_width(460)
            .default_height(140)
            .build();
        let body = dialog_body("Provider and source readiness.");
        let label = Label::new(Some(&message));
        label.add_css_class("card-meta");
        label.set_wrap(true);
        label.set_xalign(0.0);
        let actions = dialog_actions(&dialog, "Close");
        body.append(&label);
        body.append(&actions.0);
        dialog.set_child(Some(&body));
        dialog.present();
    });

    let db_path_create = paths.database_path.clone();
    let refresh_after_create = refresh.clone();
    let refresh_after_settings_save = refresh.clone();
    let refresh_dashboard_inline = refresh_dashboard.clone();
    let refresh_workspace_inline = refresh_workspace.clone();
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
            refresh_dashboard_inline();
            refresh_workspace_inline();
        }
    });

    let db_path_modal_workspace = paths.database_path.clone();
    let refresh_after_modal_workspace = refresh.clone();
    let refresh_dashboard_modal_workspace = refresh_dashboard.clone();
    let refresh_workspace_modal_workspace = refresh_workspace.clone();
    open_workspace_modal_btn.connect_clicked(move |_| {
        show_create_workspace_dialog(
            db_path_modal_workspace.clone(),
            Rc::new(refresh_after_modal_workspace.clone()),
            Rc::new(refresh_dashboard_modal_workspace.clone()),
            Rc::new(refresh_workspace_modal_workspace.clone()),
            None,
        );
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

pub(crate) fn show_create_workspace_dialog(
    database_path: PathBuf,
    refresh: Rc<dyn Fn()>,
    refresh_dashboard: Rc<dyn Fn()>,
    refresh_workspace: Rc<dyn Fn()>,
    preselected_repo: Option<String>,
) {
    let dialog = gtk::Window::builder()
        .title("Create Workspace")
        .modal(true)
        .default_width(720)
        .default_height(520)
        .build();
    let body = dialog_body("Create a branch, issue, PR, prompt, or Linear workspace.");
    body.add_css_class("workspace-modal");

    let repo_label = Label::new(Some("Repository"));
    repo_label.add_css_class("detail-label");
    repo_label.set_xalign(0.0);
    body.append(&repo_label);
    let repo_select = ComboBoxText::new();
    repo_select.add_css_class("workspace-modal-field");
    if let Ok(store) = RepositoryStore::open(database_path.clone()) {
        if let Ok(repos) = store.list() {
            for repo in repos {
                let summary = format!("{} · {}", repo.name, repo.root_path.display());
                repo_select.append(Some(&repo.name), &summary);
            }
        }
    }
    if let Some(repo) = preselected_repo {
        repo_select.set_active_id(Some(&repo));
    }
    if repo_select.active_id().is_none() {
        repo_select.set_active(Some(0));
    }
    body.append(&repo_select);

    let source_header = GBox::new(Orientation::Vertical, 10);
    source_header.add_css_class("workspace-modal-split");
    let start_column = GBox::new(Orientation::Vertical, 6);
    start_column.set_hexpand(true);
    let start_label = Label::new(Some("Start from"));
    start_label.add_css_class("detail-label");
    start_label.set_xalign(0.0);
    let name_entry = Entry::new();
    name_entry.add_css_class("workspace-modal-field");
    name_entry.set_placeholder_text(Some("workspace name"));
    let branch_entry = Entry::new();
    branch_entry.add_css_class("workspace-modal-field");
    branch_entry.set_placeholder_text(Some("branch name"));
    let base_entry = Entry::new();
    base_entry.add_css_class("workspace-modal-field");
    base_entry.set_placeholder_text(Some("base ref, optional"));
    let source_select = ComboBoxText::new();
    source_select.append(Some("branch"), "Branch");
    source_select.append(Some("github_issue"), "GitHub Issue");
    source_select.append(Some("github_pr"), "GitHub PR");
    source_select.append(Some("linear_issue"), "Linear Issue");
    source_select.append(Some("prompt"), "Prompt");
    source_select.set_active_id(Some("branch"));
    source_select.add_css_class("workspace-modal-field");
    let source_stack = Stack::new();
    source_stack.add_css_class("workspace-modal-field");

    let branch_select = ComboBoxText::new();
    branch_select.add_css_class("workspace-modal-field");
    branch_select.set_hexpand(true);
    let branch_refresh_btn = text_button("Load branches");
    let branch_row = GBox::new(Orientation::Vertical, 8);
    branch_row.append(&branch_select);
    branch_row.append(&branch_refresh_btn);
    let branch_hint = Label::new(Some("Select a branch from the repository."));
    branch_hint.add_css_class("surface-note");
    branch_hint.set_wrap(true);
    branch_hint.set_xalign(0.0);
    let branch_panel = GBox::new(Orientation::Vertical, 6);
    branch_panel.append(&branch_row);
    branch_panel.append(&branch_hint);
    source_stack.add_named(&branch_panel, Some("branch"));

    let github_issue_select = ComboBoxText::new();
    github_issue_select.add_css_class("workspace-modal-field");
    github_issue_select.set_hexpand(true);
    let github_issue_refresh_btn = text_button("Load issues");
    let github_issue_row = GBox::new(Orientation::Vertical, 8);
    github_issue_row.append(&github_issue_select);
    github_issue_row.append(&github_issue_refresh_btn);
    let github_issue_hint = Label::new(Some("Choose an open GitHub issue."));
    github_issue_hint.add_css_class("surface-note");
    github_issue_hint.set_wrap(true);
    github_issue_hint.set_xalign(0.0);
    let github_issue_panel = GBox::new(Orientation::Vertical, 6);
    github_issue_panel.append(&github_issue_row);
    github_issue_panel.append(&github_issue_hint);
    source_stack.add_named(&github_issue_panel, Some("github_issue"));

    let github_pr_select = ComboBoxText::new();
    github_pr_select.add_css_class("workspace-modal-field");
    github_pr_select.set_hexpand(true);
    let github_pr_refresh_btn = text_button("Load PRs");
    let github_pr_row = GBox::new(Orientation::Vertical, 8);
    github_pr_row.append(&github_pr_select);
    github_pr_row.append(&github_pr_refresh_btn);
    let github_pr_hint = Label::new(Some("Choose an open GitHub pull request."));
    github_pr_hint.add_css_class("surface-note");
    github_pr_hint.set_wrap(true);
    github_pr_hint.set_xalign(0.0);
    let github_pr_panel = GBox::new(Orientation::Vertical, 6);
    github_pr_panel.append(&github_pr_row);
    github_pr_panel.append(&github_pr_hint);
    source_stack.add_named(&github_pr_panel, Some("github_pr"));

    let linear_entry = Entry::new();
    linear_entry.add_css_class("workspace-modal-field");
    linear_entry.set_placeholder_text(Some("Linear issue id"));
    let linear_hint = Label::new(Some(
        "Type a Linear issue ID. We do not have a list endpoint yet.",
    ));
    linear_hint.add_css_class("surface-note");
    linear_hint.set_wrap(true);
    linear_hint.set_xalign(0.0);
    let linear_panel = GBox::new(Orientation::Vertical, 6);
    linear_panel.append(&linear_entry);
    linear_panel.append(&linear_hint);
    source_stack.add_named(&linear_panel, Some("linear_issue"));

    let prompt_entry = Entry::new();
    prompt_entry.add_css_class("workspace-modal-field");
    prompt_entry.set_placeholder_text(Some("Describe the task"));
    let prompt_hint = Label::new(Some("Type the prompt that should seed the workspace."));
    prompt_hint.add_css_class("surface-note");
    prompt_hint.set_wrap(true);
    prompt_hint.set_xalign(0.0);
    let prompt_panel = GBox::new(Orientation::Vertical, 6);
    prompt_panel.append(&prompt_entry);
    prompt_panel.append(&prompt_hint);
    source_stack.add_named(&prompt_panel, Some("prompt"));

    start_column.append(&start_label);
    start_column.append(&base_entry);
    start_column.append(&source_select);
    source_header.append(&start_column);

    let source_column = GBox::new(Orientation::Vertical, 6);
    source_column.set_hexpand(true);
    let source_value_label = Label::new(Some("Task source"));
    source_value_label.add_css_class("detail-label");
    source_value_label.set_xalign(0.0);
    source_column.append(&source_value_label);
    source_column.append(&source_stack);
    source_header.append(&source_column);
    body.append(&source_header);

    let workspace_header = GBox::new(Orientation::Vertical, 10);
    workspace_header.add_css_class("workspace-modal-split");
    let name_column = GBox::new(Orientation::Vertical, 6);
    name_column.set_hexpand(true);
    let workspace_label = Label::new(Some("Workspace"));
    workspace_label.add_css_class("detail-label");
    workspace_label.set_xalign(0.0);
    name_column.append(&workspace_label);
    name_column.append(&name_entry);
    workspace_header.append(&name_column);

    let branch_column = GBox::new(Orientation::Vertical, 6);
    branch_column.set_hexpand(true);
    let branch_label = Label::new(Some("Branch"));
    branch_label.add_css_class("detail-label");
    branch_label.set_xalign(0.0);
    branch_column.append(&branch_label);
    branch_column.append(&branch_entry);
    workspace_header.append(&branch_column);
    body.append(&workspace_header);

    let source_hint = Label::new(None);
    source_hint.add_css_class("surface-note");
    source_hint.add_css_class("workspace-modal-hint");
    source_hint.set_wrap(true);
    source_hint.set_xalign(0.0);
    body.append(&source_hint);

    let preview_panel = GBox::new(Orientation::Vertical, 6);
    preview_panel.add_css_class("workspace-modal-preview");
    let preview_title = Label::new(Some("Setup preview"));
    preview_title.add_css_class("detail-label");
    preview_title.set_xalign(0.0);
    let preview_body = Label::new(None);
    preview_body.add_css_class("workspace-meta");
    preview_body.add_css_class("workspace-modal-preview-copy");
    preview_body.set_wrap(true);
    preview_body.set_xalign(0.0);
    preview_panel.append(&preview_title);
    preview_panel.append(&preview_body);
    body.append(&preview_panel);

    let feedback = Label::new(None);
    feedback.add_css_class("card-meta");
    feedback.add_css_class("workspace-modal-feedback");
    feedback.set_xalign(0.0);
    feedback.set_wrap(true);
    let actions = dialog_actions(&dialog, "Create");
    let confirm = actions.1.clone();
    let dialog_for_create = dialog.clone();
    let repo_select_for_create = repo_select.clone();
    let name_entry_for_create = name_entry.clone();
    let base_entry_for_create = base_entry.clone();
    let source_select_for_create = source_select.clone();
    let feedback_for_create = feedback.clone();
    let db_path_for_create = database_path.clone();
    let refresh_for_create = refresh.clone();
    let refresh_dashboard_for_create = refresh_dashboard.clone();
    let refresh_workspace_for_create = refresh_workspace.clone();
    let loaded_source_choices = Rc::new(RefCell::new(None::<(String, String)>));
    let refresh_modal_copy: Rc<dyn Fn()> = {
        let source_select = source_select.clone();
        let source_stack = source_stack.clone();
        let branch_select = branch_select.clone();
        let github_issue_select = github_issue_select.clone();
        let github_pr_select = github_pr_select.clone();
        let linear_entry = linear_entry.clone();
        let prompt_entry = prompt_entry.clone();
        let source_value_label = source_value_label.clone();
        let base_entry = base_entry.clone();
        let preview_body = preview_body.clone();
        let repo_select = repo_select.clone();
        let name_entry = name_entry.clone();
        let branch_entry = branch_entry.clone();
        let database_path = database_path.clone();
        let source_hint = source_hint.clone();
        let loaded_source_choices = loaded_source_choices.clone();
        Rc::new(move || {
            let source = source_select
                .active_id()
                .map(|id| id.to_string())
                .unwrap_or_else(|| "branch".to_owned());
            let copy = workspace_source_form_copy(&source);
            source_value_label.set_text(copy.field_label);
            source_stack.set_visible_child_name(&source);
            source_hint.set_text(copy.help_text);
            base_entry.set_sensitive(copy.base_allowed);
            if !copy.base_allowed {
                base_entry.set_text("");
            }

            let repo_name = repo_select
                .active_id()
                .map(|id| id.to_string())
                .unwrap_or_default();
            if !repo_name.is_empty() {
                let desired_key = (repo_name.clone(), source.clone());
                if loaded_source_choices.borrow().as_ref() != Some(&desired_key) {
                    if let Ok(repo_root) = repository_root(&database_path, &repo_name) {
                        match source.as_str() {
                            "branch" => {
                                load_branch_choices(&repo_root, &branch_select);
                            }
                            "github_issue" => {
                                load_github_issue_choices(&repo_root, &github_issue_select);
                            }
                            "github_pr" => {
                                load_github_pr_choices(&repo_root, &github_pr_select);
                            }
                            _ => {}
                        }
                    }
                    *loaded_source_choices.borrow_mut() = Some(desired_key);
                }
            }

            let repo = repo_select
                .active_id()
                .map(|id| id.to_string())
                .unwrap_or_else(|| "pick a repository".to_owned());
            let base =
                optional_entry_text(&base_entry).unwrap_or_else(|| "repo default".to_owned());
            let source_value = match source.as_str() {
                "branch" => branch_select
                    .active_id()
                    .map(|id| id.to_string())
                    .unwrap_or_default(),
                "github_issue" => github_issue_select
                    .active_id()
                    .map(|id| id.to_string())
                    .unwrap_or_default(),
                "github_pr" => github_pr_select
                    .active_id()
                    .map(|id| id.to_string())
                    .unwrap_or_default(),
                "linear_issue" => linear_entry.text().trim().to_owned(),
                "prompt" => prompt_entry.text().trim().to_owned(),
                _ => String::new(),
            };
            let name = name_entry.text().trim().to_owned();
            let branch = branch_entry.text().trim().to_owned();
            let source_summary = if source_value.is_empty() {
                copy.preview_empty.to_owned()
            } else {
                format!("{} {}", copy.preview_prefix, source_value)
            };
            let workspace_summary = if name.is_empty() {
                "auto".to_owned()
            } else {
                name
            };
            let branch_summary = if branch.is_empty() {
                "auto".to_owned()
            } else {
                branch
            };
            preview_body.set_text(&format!(
                "Repository: {repo}\nBase: {base}\nSource: {source_summary}\nWorkspace: {workspace_summary}\nBranch: {branch_summary}"
            ));
        })
    };
    refresh_modal_copy();
    let refresh_modal_copy_select = refresh_modal_copy.clone();
    source_select.connect_changed(move |_| refresh_modal_copy_select());
    let refresh_modal_copy_repo = refresh_modal_copy.clone();
    repo_select.connect_changed(move |_| refresh_modal_copy_repo());
    let refresh_modal_copy_branch = refresh_modal_copy.clone();
    branch_select.connect_changed(move |_| refresh_modal_copy_branch());
    let refresh_modal_copy_issue = refresh_modal_copy.clone();
    github_issue_select.connect_changed(move |_| refresh_modal_copy_issue());
    let refresh_modal_copy_pr = refresh_modal_copy.clone();
    github_pr_select.connect_changed(move |_| refresh_modal_copy_pr());
    let refresh_modal_copy_base = refresh_modal_copy.clone();
    base_entry.connect_changed(move |_| refresh_modal_copy_base());
    let refresh_modal_copy_name = refresh_modal_copy.clone();
    name_entry.connect_changed(move |_| refresh_modal_copy_name());
    let refresh_modal_copy_branch = refresh_modal_copy.clone();
    branch_entry.connect_changed(move |_| refresh_modal_copy_branch());
    branch_refresh_btn.connect_clicked({
        let database_path = database_path.clone();
        let repo_select = repo_select.clone();
        let branch_select = branch_select.clone();
        let loaded_source_choices = loaded_source_choices.clone();
        move |_| {
            if let Some(repo_name) = repo_select.active_id().map(|id| id.to_string()) {
                if let Ok(repo_root) = repository_root(&database_path, &repo_name) {
                    *loaded_source_choices.borrow_mut() = None;
                    load_branch_choices(&repo_root, &branch_select);
                }
            }
        }
    });
    github_issue_refresh_btn.connect_clicked({
        let database_path = database_path.clone();
        let repo_select = repo_select.clone();
        let github_issue_select = github_issue_select.clone();
        let loaded_source_choices = loaded_source_choices.clone();
        move |_| {
            if let Some(repo_name) = repo_select.active_id().map(|id| id.to_string()) {
                if let Ok(repo_root) = repository_root(&database_path, &repo_name) {
                    *loaded_source_choices.borrow_mut() = None;
                    load_github_issue_choices(&repo_root, &github_issue_select);
                }
            }
        }
    });
    github_pr_refresh_btn.connect_clicked({
        let database_path = database_path.clone();
        let repo_select = repo_select.clone();
        let github_pr_select = github_pr_select.clone();
        let loaded_source_choices = loaded_source_choices.clone();
        move |_| {
            if let Some(repo_name) = repo_select.active_id().map(|id| id.to_string()) {
                if let Ok(repo_root) = repository_root(&database_path, &repo_name) {
                    *loaded_source_choices.borrow_mut() = None;
                    load_github_pr_choices(&repo_root, &github_pr_select);
                }
            }
        }
    });
    confirm.connect_clicked(move |_| {
        let Some(repo) = repo_select_for_create.active_id().map(|id| id.to_string()) else {
            feedback_for_create.set_text("Add a repository first.");
            return;
        };
        let typed_name = name_entry_for_create.text().trim().to_owned();
        let typed_branch = match source_select_for_create
            .active_id()
            .map(|id| id.to_string())
            .as_deref()
        {
            Some("branch") => branch_select
                .active_id()
                .map(|id| id.to_string())
                .unwrap_or_default(),
            Some("github_issue") => github_issue_select
                .active_id()
                .map(|id| id.to_string())
                .unwrap_or_default(),
            Some("github_pr") => github_pr_select
                .active_id()
                .map(|id| id.to_string())
                .unwrap_or_default(),
            Some("linear_issue") => linear_entry.text().trim().to_owned(),
            Some("prompt") => prompt_entry.text().trim().to_owned(),
            _ => String::new(),
        };
        let base = optional_entry_text(&base_entry_for_create);
        let source = source_select_for_create
            .active_id()
            .map(|id| id.to_string())
            .unwrap_or_else(|| "branch".to_owned());
        let source_value = match source.as_str() {
            "branch" => branch_select
                .active_id()
                .map(|id| id.to_string())
                .unwrap_or_default(),
            "github_issue" => github_issue_select
                .active_id()
                .map(|id| id.to_string())
                .unwrap_or_default(),
            "github_pr" => github_pr_select
                .active_id()
                .map(|id| id.to_string())
                .unwrap_or_default(),
            "linear_issue" => linear_entry.text().trim().to_owned(),
            "prompt" => prompt_entry.text().trim().to_owned(),
            _ => String::new(),
        };
        feedback_for_create.set_text("Creating workspace...");
        let request = workspace_source_request_from_form(
            &source,
            &source_value,
            base,
            (!typed_name.is_empty()).then_some(typed_name),
            (!typed_branch.is_empty()).then_some(typed_branch),
        );
        let create_result = request.and_then(|request| {
            WorkspaceStore::open(db_path_for_create.clone())
                .and_then(|store| request.create_workspace(&store, &repo))
        });
        match create_result {
            Ok(_) => {
                refresh_for_create();
                refresh_dashboard_for_create();
                refresh_workspace_for_create();
                dialog_for_create.close();
            }
            Err(err) => feedback_for_create.set_text(&format!("Create failed: {err:#}")),
        }
    });
    body.append(&feedback);
    body.append(&actions.0);
    dialog.set_child(Some(&body));
    dialog.present();
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

fn dialog_body(subtitle: &str) -> GBox {
    let body = GBox::new(Orientation::Vertical, 10);
    body.add_css_class("modal-body");
    body.set_margin_top(14);
    body.set_margin_bottom(14);
    body.set_margin_start(14);
    body.set_margin_end(14);
    let hint = Label::new(Some(subtitle));
    hint.add_css_class("card-meta");
    hint.set_wrap(true);
    hint.set_xalign(0.0);
    body.append(&hint);
    body
}

struct WorkspaceSourceFormCopy {
    field_label: &'static str,
    placeholder: &'static str,
    help_text: &'static str,
    preview_prefix: &'static str,
    preview_empty: &'static str,
    base_allowed: bool,
}

fn workspace_source_form_copy(source: &str) -> WorkspaceSourceFormCopy {
    match source {
        "github_issue" => WorkspaceSourceFormCopy {
            field_label: "GitHub issue",
            placeholder: "#123",
            help_text: "GitHub creates the branch and workspace name from the issue. Base ref comes from the repository default.",
            preview_prefix: "Issue",
            preview_empty: "Issue number missing",
            base_allowed: false,
        },
        "github_pr" => WorkspaceSourceFormCopy {
            field_label: "GitHub PR",
            placeholder: "#456",
            help_text: "PR workspaces pull branch and base from GitHub. Workspace and branch names are optional overrides.",
            preview_prefix: "PR",
            preview_empty: "PR number missing",
            base_allowed: false,
        },
        "linear_issue" => WorkspaceSourceFormCopy {
            field_label: "Linear issue",
            placeholder: "ENG-42",
            help_text: "Linear issue ID drives the task. Leave workspace or branch blank if the generated values are fine.",
            preview_prefix: "Linear",
            preview_empty: "Linear issue missing",
            base_allowed: true,
        },
        "prompt" => WorkspaceSourceFormCopy {
            field_label: "Prompt",
            placeholder: "Describe the task",
            help_text: "Prompt workspaces generate a workspace and branch from the task text. Base ref is optional.",
            preview_prefix: "Prompt",
            preview_empty: "Prompt missing",
            base_allowed: true,
        },
        _ => WorkspaceSourceFormCopy {
            field_label: "Task source",
            placeholder: "Optional note or ticket",
            help_text: "Manual branch mode can auto-generate the workspace and branch. Fill either field only if you want an override.",
            preview_prefix: "Manual",
            preview_empty: "Manual branch",
            base_allowed: true,
        },
    }
}

fn dialog_actions(dialog: &gtk::Window, primary_label: &str) -> (GBox, Button) {
    let row = GBox::new(Orientation::Horizontal, 8);
    row.set_halign(gtk::Align::End);
    let cancel = text_button("Cancel");
    let primary = text_button(primary_label);
    primary.add_css_class("suggested-action");
    let dialog_for_cancel = dialog.clone();
    cancel.connect_clicked(move |_| {
        dialog_for_cancel.close();
    });
    row.append(&cancel);
    row.append(&primary);
    (row, primary)
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
        _ => Ok(WorkspaceSourceRequest::Branch {
            name: typed_name.unwrap_or_default(),
            branch: typed_branch.unwrap_or_default(),
            base,
        }),
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct GithubRepoChoice {
    label: String,
    url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NewProjectTemplate {
    id: &'static str,
    label: &'static str,
    description: &'static str,
}

pub(crate) fn show_repository_quick_add_dialog(
    database_path: PathBuf,
    refresh: Rc<dyn Fn()>,
    initial_mode: Option<&str>,
) {
    let dialog = gtk::Window::builder()
        .title("Add Repository")
        .modal(true)
        .default_width(720)
        .default_height(420)
        .build();
    let body = dialog_body("Open a checked-out folder, clone from GitHub, or start a new project.");
    body.add_css_class("workspace-modal");

    let mode_label = Label::new(Some("Add mode"));
    mode_label.add_css_class("detail-label");
    mode_label.set_xalign(0.0);
    body.append(&mode_label);

    let mode_select = ComboBoxText::new();
    mode_select.add_css_class("workspace-modal-field");
    mode_select.append(Some("folder"), "Folder");
    mode_select.append(Some("clone"), "Clone");
    mode_select.append(Some("new"), "New");
    mode_select.set_active_id(initial_mode.or(Some("folder")));
    body.append(&mode_select);

    let stack = Stack::new();
    stack.set_vexpand(true);
    body.append(&stack);

    let folder_box = GBox::new(Orientation::Vertical, 8);
    let folder_path_row = GBox::new(Orientation::Horizontal, 8);
    let folder_path_entry = Entry::new();
    folder_path_entry.add_css_class("workspace-modal-field");
    folder_path_entry.set_hexpand(true);
    folder_path_entry.set_placeholder_text(Some("repository folder"));
    let folder_browse_btn = text_button("Browse");
    folder_path_row.append(&folder_path_entry);
    folder_path_row.append(&folder_browse_btn);
    let folder_name_entry = Entry::new();
    folder_name_entry.add_css_class("workspace-modal-field");
    folder_name_entry.set_placeholder_text(Some("project name, optional"));
    folder_box.append(&folder_path_row);
    folder_box.append(&folder_name_entry);
    stack.add_named(&folder_box, Some("folder"));

    let clone_box = GBox::new(Orientation::Vertical, 8);
    let github_row = GBox::new(Orientation::Horizontal, 8);
    let github_repo_select = ComboBoxText::new();
    github_repo_select.add_css_class("workspace-modal-field");
    github_repo_select.set_hexpand(true);
    let load_github_btn = text_button("Load GitHub Repos");
    github_row.append(&github_repo_select);
    github_row.append(&load_github_btn);
    let clone_url_entry = Entry::new();
    clone_url_entry.add_css_class("workspace-modal-field");
    clone_url_entry.set_placeholder_text(Some("git URL"));
    let clone_name_entry = Entry::new();
    clone_name_entry.add_css_class("workspace-modal-field");
    clone_name_entry.set_placeholder_text(Some("project name, optional"));
    clone_box.append(&github_row);
    clone_box.append(&clone_url_entry);
    clone_box.append(&clone_name_entry);
    stack.add_named(&clone_box, Some("clone"));

    let new_box = GBox::new(Orientation::Vertical, 8);
    let template_select = ComboBoxText::new();
    template_select.add_css_class("workspace-modal-field");
    for template in new_project_templates() {
        template_select.append(Some(template.id), template.label);
    }
    template_select.set_active(Some(0));
    let new_parent_row = GBox::new(Orientation::Horizontal, 8);
    let new_parent_entry = Entry::new();
    new_parent_entry.add_css_class("workspace-modal-field");
    new_parent_entry.set_hexpand(true);
    new_parent_entry.set_placeholder_text(Some("parent folder"));
    let new_parent_browse_btn = text_button("Browse");
    new_parent_row.append(&new_parent_entry);
    new_parent_row.append(&new_parent_browse_btn);
    let new_name_entry = Entry::new();
    new_name_entry.add_css_class("workspace-modal-field");
    new_name_entry.set_placeholder_text(Some("project name"));
    new_box.append(&template_select);
    new_box.append(&new_parent_row);
    new_box.append(&new_name_entry);
    stack.add_named(&new_box, Some("new"));

    let feedback = Label::new(None);
    feedback.add_css_class("card-meta");
    feedback.set_xalign(0.0);
    feedback.set_wrap(true);
    body.append(&feedback);

    let refresh_mode: Rc<dyn Fn()> = {
        let stack = stack.clone();
        let mode_select = mode_select.clone();
        Rc::new(move || {
            let mode = mode_select
                .active_id()
                .map(|id| id.to_string())
                .unwrap_or_else(|| "folder".to_owned());
            stack.set_visible_child_name(&mode);
        })
    };
    refresh_mode();
    let refresh_mode_on_change = refresh_mode.clone();
    mode_select.connect_changed(move |_| refresh_mode_on_change());

    let folder_path_entry_for_picker = folder_path_entry.clone();
    folder_browse_btn.connect_clicked(move |_| {
        choose_folder_for_entry(
            &folder_path_entry_for_picker,
            "Pick a checked-out repository",
        );
    });
    let new_parent_entry_for_picker = new_parent_entry.clone();
    new_parent_browse_btn.connect_clicked(move |_| {
        choose_folder_for_entry(&new_parent_entry_for_picker, "Pick a parent folder");
    });

    let clone_url_entry_for_select = clone_url_entry.clone();
    let clone_name_entry_for_select = clone_name_entry.clone();
    github_repo_select.connect_changed(move |select| {
        if let Some(url) = select.active_id().map(|id| id.to_string()) {
            clone_url_entry_for_select.set_text(&url);
            if clone_name_entry_for_select.text().trim().is_empty() {
                clone_name_entry_for_select.set_text(&repo_name_from_url(&url));
            }
        }
    });

    let github_repo_select_for_load = github_repo_select.clone();
    let feedback_for_load = feedback.clone();
    load_github_btn.connect_clicked(move |_| match load_github_repo_choices() {
        Ok(repos) if repos.is_empty() => {
            feedback_for_load.set_text("No GitHub repos returned by gh repo list.");
        }
        Ok(repos) => {
            github_repo_select_for_load.remove_all();
            for repo in repos {
                github_repo_select_for_load.append(Some(&repo.url), &repo.label);
            }
            github_repo_select_for_load.set_active(Some(0));
            feedback_for_load.set_text("Loaded GitHub repos.");
        }
        Err(err) => feedback_for_load.set_text(&format!("GitHub repo load failed: {err:#}")),
    });

    let actions = dialog_actions(&dialog, "Add");
    let confirm = actions.1.clone();
    let dialog_for_confirm = dialog.clone();
    confirm.connect_clicked(move |_| {
        let mode = mode_select
            .active_id()
            .map(|id| id.to_string())
            .unwrap_or_else(|| "folder".to_owned());
        let result = match mode.as_str() {
            "clone" => {
                let url = clone_url_entry.text().trim().to_owned();
                let explicit_name = optional_entry_text(&clone_name_entry);
                clone_repository_into_default_parent(&database_path, &url, explicit_name)
            }
            "new" => {
                let parent = new_parent_entry.text().trim().to_owned();
                let name = new_name_entry.text().trim().to_owned();
                let template = template_select
                    .active_id()
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "empty".to_owned());
                create_repository_from_template(&database_path, &parent, &name, &template)
            }
            _ => {
                let path = folder_path_entry.text().trim().to_owned();
                let explicit_name = optional_entry_text(&folder_name_entry);
                add_repository_from_path(&database_path, &path, explicit_name)
            }
        };

        match result {
            Ok(_) => {
                refresh();
                dialog_for_confirm.close();
            }
            Err(err) => feedback.set_text(&format!("Add failed: {err:#}")),
        }
    });
    body.append(&actions.0);
    dialog.set_child(Some(&body));
    dialog.present();
}

fn choose_folder_for_entry(entry: &Entry, title: &str) {
    let dialog = gtk::FileChooserNative::builder()
        .title(title)
        .action(gtk::FileChooserAction::SelectFolder)
        .accept_label("Select")
        .cancel_label("Cancel")
        .build();
    let entry = entry.clone();
    dialog.connect_response(move |dialog, response| {
        if response == gtk::ResponseType::Accept {
            if let Some(path) = dialog.file().and_then(|file| file.path()) {
                entry.set_text(&path.display().to_string());
            }
        }
        dialog.destroy();
    });
    dialog.show();
}

fn add_repository_from_path(
    database_path: &Path,
    path: &str,
    explicit_name: Option<String>,
) -> Result<linux_conductor_core::repository::Repository> {
    let trimmed = path.trim();
    anyhow::ensure!(!trimmed.is_empty(), "Local repository path is required.");
    RepositoryStore::open(database_path)?.add(AddRepository {
        name: explicit_name,
        root_path: PathBuf::from(trimmed),
        default_branch: None,
        remote_name: "origin".to_owned(),
        workspace_parent_path: None,
    })
}

fn clone_repository_into_default_parent(
    database_path: &Path,
    url: &str,
    explicit_name: Option<String>,
) -> Result<linux_conductor_core::repository::Repository> {
    let trimmed = url.trim();
    anyhow::ensure!(!trimmed.is_empty(), "Git URL is required.");
    let name = explicit_name.unwrap_or_else(|| repo_name_from_url(trimmed));
    let clone_path = default_clone_parent().join(&name);
    if !clone_path.exists() {
        if let Some(parent) = clone_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create clone parent {}", parent.display()))?;
        }
        let status = Command::new("git")
            .args(["clone", trimmed])
            .arg(&clone_path)
            .status()
            .with_context(|| format!("clone repository {trimmed}"))?;
        anyhow::ensure!(status.success(), "git clone exited with {status}");
    }
    RepositoryStore::open(database_path)?.add(AddRepository {
        name: Some(name),
        root_path: clone_path,
        default_branch: None,
        remote_name: "origin".to_owned(),
        workspace_parent_path: None,
    })
}

fn create_repository_from_template(
    database_path: &Path,
    parent_folder: &str,
    project_name: &str,
    template: &str,
) -> Result<linux_conductor_core::repository::Repository> {
    let parent = parent_folder.trim();
    anyhow::ensure!(!parent.is_empty(), "Parent folder is required.");
    let path = scaffold_new_repository(Path::new(parent), project_name, template)?;
    RepositoryStore::open(database_path)?.add(AddRepository {
        name: Some(project_name.trim().to_owned()),
        root_path: path,
        default_branch: None,
        remote_name: "origin".to_owned(),
        workspace_parent_path: None,
    })
}

fn parse_github_repo_choices(raw: &str) -> Vec<GithubRepoChoice> {
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|repo| {
            let label = repo.get("nameWithOwner")?.as_str()?.trim();
            let url = repo.get("url")?.as_str()?.trim();
            if label.is_empty() || url.is_empty() {
                None
            } else {
                Some(GithubRepoChoice {
                    label: label.to_owned(),
                    url: url.to_owned(),
                })
            }
        })
        .collect()
}

fn load_github_repo_choices() -> Result<Vec<GithubRepoChoice>> {
    let output = Command::new("gh")
        .args([
            "repo",
            "list",
            "--limit",
            "200",
            "--json",
            "nameWithOwner,url",
        ])
        .output()
        .context("run gh repo list")?;
    anyhow::ensure!(
        output.status.success(),
        "gh repo list exited with {}",
        output.status
    );
    Ok(parse_github_repo_choices(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

fn clear_combo_text(combo: &ComboBoxText) {
    combo.remove_all();
    combo.append(Some(""), "Select...");
}

fn load_branch_choices(repo_root: &Path, combo: &ComboBoxText) {
    clear_combo_text(combo);
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args([
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/heads",
            "refs/remotes/origin",
        ])
        .output();
    let Ok(output) = output else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let mut branches = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.ends_with("/HEAD"))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    branches.sort();
    branches.dedup();
    let has_items = !branches.is_empty();
    for branch in branches {
        combo.append(Some(&branch), &branch);
    }
    combo.set_active(Some(if has_items { 1 } else { 0 }));
}

fn parse_github_numbered_choices(raw: &str) -> Vec<(String, String)> {
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| {
            let number = item.get("number")?.as_u64()?.to_string();
            let title = item.get("title")?.as_str()?.trim();
            if title.is_empty() {
                None
            } else {
                Some((number.clone(), format!("#{number} {title}")))
            }
        })
        .collect()
}

fn load_github_issue_choices(repo_root: &Path, combo: &ComboBoxText) {
    clear_combo_text(combo);
    let output = Command::new("gh")
        .arg("-C")
        .arg(repo_root)
        .args([
            "issue",
            "list",
            "--limit",
            "100",
            "--state",
            "open",
            "--json",
            "number,title",
        ])
        .output();
    let Ok(output) = output else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let choices = parse_github_numbered_choices(&String::from_utf8_lossy(&output.stdout));
    let has_items = !choices.is_empty();
    for (value, label) in choices {
        combo.append(Some(&value), &label);
    }
    combo.set_active(Some(if has_items { 1 } else { 0 }));
}

fn load_github_pr_choices(repo_root: &Path, combo: &ComboBoxText) {
    clear_combo_text(combo);
    let output = Command::new("gh")
        .arg("-C")
        .arg(repo_root)
        .args([
            "pr",
            "list",
            "--limit",
            "100",
            "--state",
            "open",
            "--json",
            "number,title",
        ])
        .output();
    let Ok(output) = output else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let choices = parse_github_numbered_choices(&String::from_utf8_lossy(&output.stdout));
    let has_items = !choices.is_empty();
    for (value, label) in choices {
        combo.append(Some(&value), &label);
    }
    combo.set_active(Some(if has_items { 1 } else { 0 }));
}

fn new_project_templates() -> Vec<NewProjectTemplate> {
    vec![
        NewProjectTemplate {
            id: "empty",
            label: "Empty Git Repo",
            description: "Initialize an empty Git repository with a README.",
        },
        NewProjectTemplate {
            id: "rust-bin",
            label: "Rust CLI",
            description: "cargo new --bin",
        },
        NewProjectTemplate {
            id: "rust-lib",
            label: "Rust Library",
            description: "cargo new --lib",
        },
    ]
}

fn scaffold_new_repository(parent: &Path, project_name: &str, template: &str) -> Result<PathBuf> {
    let name = project_name.trim();
    anyhow::ensure!(!name.is_empty(), "Project name is required.");
    std::fs::create_dir_all(parent)
        .with_context(|| format!("create parent folder {}", parent.display()))?;
    let repo_path = parent.join(name);
    anyhow::ensure!(
        !repo_path.exists(),
        "Project folder already exists: {}",
        repo_path.display()
    );
    match template {
        "rust-bin" => {
            let status = Command::new("cargo")
                .args(["new", "--bin"])
                .arg(&repo_path)
                .status()
                .with_context(|| format!("create Rust CLI project {}", repo_path.display()))?;
            anyhow::ensure!(status.success(), "cargo new exited with {status}");
        }
        "rust-lib" => {
            let status = Command::new("cargo")
                .args(["new", "--lib"])
                .arg(&repo_path)
                .status()
                .with_context(|| format!("create Rust library project {}", repo_path.display()))?;
            anyhow::ensure!(status.success(), "cargo new exited with {status}");
        }
        _ => {
            std::fs::create_dir_all(&repo_path)
                .with_context(|| format!("create project folder {}", repo_path.display()))?;
            let status = Command::new("git")
                .args(["init", "--initial-branch", "main"])
                .arg(&repo_path)
                .status()
                .with_context(|| format!("initialize git repo {}", repo_path.display()))?;
            anyhow::ensure!(status.success(), "git init exited with {status}");
            std::fs::write(repo_path.join("README.md"), format!("# {name}\n"))
                .with_context(|| format!("write README for {}", repo_path.display()))?;
        }
    }
    Ok(repo_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

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
        let branch =
            workspace_source_request_from_form("branch", "", Some("main".to_owned()), None, None)
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

    #[test]
    fn workspace_source_form_copy_matches_source_requirements() {
        let branch = workspace_source_form_copy("branch");
        assert_eq!(branch.field_label, "Task source");
        assert!(branch.base_allowed);

        let pr = workspace_source_form_copy("github_pr");
        assert_eq!(pr.field_label, "GitHub PR");
        assert!(!pr.base_allowed);
        assert_eq!(pr.placeholder, "#456");
    }

    #[test]
    fn parse_github_repo_choices_reads_repo_names_and_urls() {
        let repos = parse_github_repo_choices(
            r#"[
  {"nameWithOwner":"acme/api","url":"https://github.com/acme/api"},
  {"nameWithOwner":"acme/web","url":"https://github.com/acme/web"}
]"#,
        );

        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].label, "acme/api");
        assert_eq!(repos[0].url, "https://github.com/acme/api");
        assert_eq!(repos[1].label, "acme/web");
    }

    #[test]
    fn new_project_templates_offer_three_choices() {
        let templates = new_project_templates();
        assert_eq!(templates.len(), 3);
        assert_eq!(templates[0].id, "empty");
        assert_eq!(templates[1].id, "rust-bin");
        assert_eq!(templates[2].id, "rust-lib");
    }

    #[test]
    fn scaffold_new_repository_creates_empty_git_repo() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = scaffold_new_repository(temp.path(), "demo", "empty").unwrap();

        assert!(repo_path.join(".git").is_dir());
        assert!(repo_path.join("README.md").is_file());
        let branch = Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["branch", "--show-current"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&branch.stdout).trim(), "main");
        assert_eq!(
            fs::read_to_string(repo_path.join("README.md")).unwrap(),
            "# demo\n"
        );
    }
}
