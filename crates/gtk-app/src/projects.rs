use gtk::prelude::*;
use gtk::{Box as GBox, Button, Entry, Label, Orientation};
use linux_conductor_core::paths::AppPaths;
use linux_conductor_core::repository::{AddRepository, RepositoryStore};
use linux_conductor_core::workspace::{CreateWorkspace, WorkspaceStore};
use std::path::PathBuf;

use crate::{default_clone_parent, detail_row, repo_name_from_url};

pub(crate) fn build_projects_page(
    paths: &AppPaths,
    refresh_dashboard: impl Fn() + Clone + 'static,
    refresh_workspace: impl Fn() + Clone + 'static,
) -> (GBox, impl Fn() + Clone + 'static) {
    let root = GBox::new(Orientation::Vertical, 0);
    root.add_css_class("dashboard");
    let header = GBox::new(Orientation::Vertical, 8);
    header.add_css_class("dashboard-header");
    let title = Label::new(Some("Projects"));
    title.add_css_class("dashboard-title");
    title.set_xalign(0.0);
    let subtitle = Label::new(Some("Create workspaces and inspect imported repositories."));
    subtitle.add_css_class("card-meta");
    subtitle.set_xalign(0.0);
    header.append(&title);
    header.append(&subtitle);
    root.append(&header);

    let body = GBox::new(Orientation::Vertical, 14);
    body.add_css_class("detail-body");
    root.append(&body);

    let repo_title = Label::new(Some("Add Repository"));
    repo_title.add_css_class("section-title");
    repo_title.set_xalign(0.0);
    body.append(&repo_title);

    let repo_box = GBox::new(Orientation::Horizontal, 8);
    let repo_path_entry = Entry::new();
    repo_path_entry.set_placeholder_text(Some("local path or git URL"));
    let repo_name_entry = Entry::new();
    repo_name_entry.set_placeholder_text(Some("project name"));
    let add_repo_btn = Button::with_label("Add Local");
    let clone_repo_btn = Button::with_label("Clone");
    let repo_result = Label::new(None);
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
    let name_entry = Entry::new();
    name_entry.set_placeholder_text(Some("workspace name"));
    let branch_entry = Entry::new();
    branch_entry.set_placeholder_text(Some("branch name"));
    let create_btn = Button::with_label("Create Workspace");
    let result = Label::new(None);
    result.add_css_class("card-meta");
    result.set_xalign(0.0);
    create_box.append(&repo_entry);
    create_box.append(&name_entry);
    create_box.append(&branch_entry);
    create_box.append(&create_btn);
    body.append(&create_box);
    body.append(&result);

    let repo_list = GBox::new(Orientation::Vertical, 8);
    body.append(&repo_list);

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
                                "{} active / {} total / {}",
                                active,
                                total,
                                repo.root_path.display()
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
                repo_result_add.set_text(&format!("Added {}", repo.name));
                refresh_after_repo();
            }
            Err(err) => repo_result_add.set_text(&format!("Add failed: {err:#}")),
        }
    });

    let db_path_clone = paths.database_path.clone();
    let refresh_after_clone = refresh.clone();
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
                repo_result.set_text(&format!("Cloned and added {}", repo.name));
                refresh_after_clone();
            }
            Err(err) => repo_result.set_text(&format!("Clone failed: {err:#}")),
        }
    });

    let db_path_create = paths.database_path.clone();
    let refresh_after_create = refresh.clone();
    create_btn.connect_clicked(move |_| {
        let repo = repo_entry.text().trim().to_owned();
        let name = name_entry.text().trim().to_owned();
        let branch = branch_entry.text().trim().to_owned();
        if repo.is_empty() || name.is_empty() || branch.is_empty() {
            result.set_text("Repository, workspace name, and branch are required.");
            return;
        }
        match WorkspaceStore::open(db_path_create.clone()).and_then(|store| {
            store.create(CreateWorkspace {
                repository_name: repo,
                name,
                branch,
                base_ref: None,
            })
        }) {
            Ok(workspace) => {
                result.set_text(&format!("Created {}", workspace.path.display()));
                refresh_after_create();
                refresh_dashboard();
                refresh_workspace();
            }
            Err(err) => result.set_text(&format!("Create failed: {err:#}")),
        }
    });

    refresh();
    (root, refresh)
}
