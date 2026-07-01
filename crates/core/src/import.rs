use crate::workspace::WorkspaceStore;
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OpenFlags};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConductorImportSummary {
    pub repositories_imported: usize,
    pub workspaces_imported: usize,
    pub renamed_duplicate_workspaces: usize,
    pub skipped_workspaces: usize,
}

#[derive(Debug)]
struct SourceRepository {
    id: String,
    name: String,
    root_path: String,
    default_branch: String,
    remote_name: String,
}

#[derive(Debug)]
struct SourceWorkspace {
    repository_id: String,
    directory_name: Option<String>,
    workspace_name: Option<String>,
    workspace_path: Option<String>,
    branch: Option<String>,
    placeholder_branch_name: Option<String>,
    state: Option<String>,
    intended_target_branch: Option<String>,
    initialization_parent_branch: Option<String>,
    created_at: String,
    updated_at: String,
}

pub fn default_conductor_app_database() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Library/Application Support/com.conductor.app/conductor.db")
}

pub fn import_conductor_app_database(
    source_database: impl AsRef<Path>,
    target_database: impl AsRef<Path>,
) -> Result<ConductorImportSummary> {
    let source_database = source_database.as_ref();
    let target_database = target_database.as_ref();

    let _ = WorkspaceStore::open(target_database)?;
    let source = Connection::open_with_flags(source_database, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open Conductor database {}", source_database.display()))?;
    let target = Connection::open(target_database)
        .with_context(|| format!("open target database {}", target_database.display()))?;

    let repositories = read_source_repositories(&source)?;
    let workspace_parent_by_repo = infer_workspace_parents(&source)?;
    let mut repo_id_map = HashMap::new();
    let mut repositories_imported = 0;

    for repo in repositories {
        if repo.root_path.trim().is_empty() {
            continue;
        }
        let workspace_parent = workspace_parent_by_repo
            .get(&repo.id)
            .cloned()
            .unwrap_or_else(|| default_workspace_parent(&repo.name));
        let now = timestamp();
        target.execute(
            "INSERT INTO repositories (
                name, root_path, default_branch, remote_name, workspace_parent_path, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(root_path) DO UPDATE SET
                name = excluded.name,
                default_branch = excluded.default_branch,
                remote_name = excluded.remote_name,
                workspace_parent_path = excluded.workspace_parent_path,
                updated_at = excluded.updated_at",
            params![
                repo.name,
                repo.root_path,
                repo.default_branch,
                repo.remote_name,
                workspace_parent.to_string_lossy(),
                now,
                now,
            ],
        )?;
        let target_id: i64 = target.query_row(
            "SELECT id FROM repositories WHERE root_path = ?1",
            [repo.root_path],
            |row| row.get(0),
        )?;
        repo_id_map.insert(
            repo.id,
            (target_id, repo.name, repo.default_branch, workspace_parent),
        );
        repositories_imported += 1;
    }

    let existing_names = read_existing_workspace_names(&target)?;
    let mut used_names = existing_names;
    let mut workspaces_imported = 0;
    let mut renamed_duplicate_workspaces = 0;
    let mut skipped_workspaces = 0;
    let mut next_port_base = next_port_base(&target)?;

    for workspace in read_source_workspaces(&source)? {
        let Some((repository_id, repo_name, default_branch, workspace_parent)) =
            repo_id_map.get(&workspace.repository_id)
        else {
            skipped_workspaces += 1;
            continue;
        };

        let Some(raw_name) = preferred_workspace_name(&workspace) else {
            skipped_workspaces += 1;
            continue;
        };
        let (name, renamed) = unique_workspace_name(&raw_name, repo_name, &mut used_names);
        if renamed {
            renamed_duplicate_workspaces += 1;
        }

        let path = workspace
            .workspace_path
            .as_deref()
            .filter(|path| !path.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| workspace_parent.join(&raw_name));
        let branch = workspace
            .branch
            .as_deref()
            .or(workspace.placeholder_branch_name.as_deref())
            .filter(|branch| !branch.trim().is_empty())
            .unwrap_or(&raw_name)
            .to_owned();
        let base_ref = workspace
            .intended_target_branch
            .as_deref()
            .or(workspace.initialization_parent_branch.as_deref())
            .filter(|branch| !branch.trim().is_empty())
            .unwrap_or(default_branch)
            .to_owned();
        let status = match workspace.state.as_deref() {
            Some("archived") => "archived",
            _ => "active",
        };
        let archived_at = (status == "archived").then_some(workspace.updated_at.as_str());

        target.execute(
            "INSERT INTO workspaces (
                repository_id, name, path, branch, base_ref, port_base, status, archived_at, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ON CONFLICT(path) DO UPDATE SET
                repository_id = excluded.repository_id,
                name = excluded.name,
                branch = excluded.branch,
                base_ref = excluded.base_ref,
                status = excluded.status,
                archived_at = excluded.archived_at,
                updated_at = excluded.updated_at",
            params![
                repository_id,
                name,
                path.to_string_lossy(),
                branch,
                base_ref,
                i64::from(next_port_base),
                status,
                archived_at,
                workspace.created_at,
                workspace.updated_at,
            ],
        )?;
        next_port_base = next_port_base.saturating_add(10);
        workspaces_imported += 1;
    }

    Ok(ConductorImportSummary {
        repositories_imported,
        workspaces_imported,
        renamed_duplicate_workspaces,
        skipped_workspaces,
    })
}

fn read_source_repositories(source: &Connection) -> Result<Vec<SourceRepository>> {
    let mut stmt = source.prepare(
        "SELECT id, name, root_path, COALESCE(default_branch, 'main'), COALESCE(remote, 'origin')
         FROM repos
         WHERE COALESCE(hidden, 0) = 0
         ORDER BY display_order, name",
    )?;
    let repositories = stmt
        .query_map([], |row| {
            Ok(SourceRepository {
                id: row.get(0)?,
                name: row.get(1)?,
                root_path: row.get(2)?,
                default_branch: row.get(3)?,
                remote_name: row.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(repositories)
}

fn read_source_workspaces(source: &Connection) -> Result<Vec<SourceWorkspace>> {
    let mut stmt = source.prepare(
        "SELECT repository_id, directory_name, workspace_name, workspace_path, branch,
                placeholder_branch_name, state, intended_target_branch,
                initialization_parent_branch, created_at, updated_at
         FROM workspaces
         ORDER BY created_at",
    )?;
    let workspaces = stmt
        .query_map([], |row| {
            Ok(SourceWorkspace {
                repository_id: row.get(0)?,
                directory_name: row.get(1)?,
                workspace_name: row.get(2)?,
                workspace_path: row.get(3)?,
                branch: row.get(4)?,
                placeholder_branch_name: row.get(5)?,
                state: row.get(6)?,
                intended_target_branch: row.get(7)?,
                initialization_parent_branch: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(workspaces)
}

fn infer_workspace_parents(source: &Connection) -> Result<HashMap<String, PathBuf>> {
    let mut stmt = source.prepare(
        "SELECT repository_id, workspace_path
         FROM workspaces
         WHERE workspace_path IS NOT NULL AND workspace_path != ''
         ORDER BY created_at",
    )?;
    let mut parents = HashMap::new();
    for row in stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })? {
        let (repo_id, workspace_path) = row?;
        if parents.contains_key(&repo_id) {
            continue;
        }
        if let Some(parent) = Path::new(&workspace_path).parent() {
            parents.insert(repo_id, parent.to_path_buf());
        }
    }
    Ok(parents)
}

fn read_existing_workspace_names(target: &Connection) -> Result<HashSet<String>> {
    let mut stmt = target.prepare("SELECT name FROM workspaces")?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<HashSet<_>>>()?;
    Ok(names)
}

fn preferred_workspace_name(workspace: &SourceWorkspace) -> Option<String> {
    workspace
        .workspace_name
        .as_deref()
        .or(workspace.directory_name.as_deref())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

fn unique_workspace_name(
    raw_name: &str,
    repo_name: &str,
    used_names: &mut HashSet<String>,
) -> (String, bool) {
    if used_names.insert(raw_name.to_owned()) {
        return (raw_name.to_owned(), false);
    }

    let mut candidate = format!("{repo_name}-{raw_name}");
    let mut suffix = 2;
    while !used_names.insert(candidate.clone()) {
        candidate = format!("{repo_name}-{raw_name}-{suffix}");
        suffix += 1;
    }
    (candidate, true)
}

fn next_port_base(target: &Connection) -> Result<u16> {
    let max_port: Option<i64> =
        target.query_row("SELECT MAX(port_base) FROM workspaces", [], |row| {
            row.get(0)
        })?;
    Ok(max_port
        .and_then(|port| u16::try_from(port.saturating_add(10)).ok())
        .unwrap_or(3000))
}

fn default_workspace_parent(repo_name: &str) -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("archductor")
        .join("workspaces")
        .join(repo_name)
}

fn timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_repositories_and_workspaces_from_conductor_database() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("conductor.db");
        let target_path = temp.path().join("target.db");
        let source = Connection::open(&source_path).unwrap();
        source
            .execute_batch(
                "
                CREATE TABLE repos (
                    id TEXT PRIMARY KEY,
                    name TEXT,
                    root_path TEXT,
                    default_branch TEXT,
                    remote TEXT,
                    hidden INTEGER DEFAULT 0,
                    display_order INTEGER DEFAULT 0
                );
                CREATE TABLE workspaces (
                    id TEXT PRIMARY KEY,
                    repository_id TEXT,
                    directory_name TEXT,
                    workspace_name TEXT,
                    workspace_path TEXT,
                    branch TEXT,
                    placeholder_branch_name TEXT,
                    state TEXT,
                    intended_target_branch TEXT,
                    initialization_parent_branch TEXT,
                    created_at TEXT,
                    updated_at TEXT
                );
                ",
            )
            .unwrap();
        source
            .execute(
                "INSERT INTO repos (id, name, root_path, default_branch, remote) VALUES
                 ('repo-1', 'demo', '/tmp/demo', 'main', 'origin'),
                 ('repo-2', 'other', '/tmp/other', 'trunk', 'upstream')",
                [],
            )
            .unwrap();
        source
            .execute(
                "INSERT INTO workspaces (
                    id, repository_id, directory_name, workspace_path, branch, state,
                    intended_target_branch, created_at, updated_at
                 ) VALUES
                 ('ws-1', 'repo-1', 'berlin', '/tmp/workspaces/demo/berlin', 'feat/a', 'ready', 'main', '1', '2'),
                 ('ws-2', 'repo-2', 'berlin', '/tmp/workspaces/other/berlin', 'feat/b', 'archived', 'trunk', '3', '4')",
                [],
            )
            .unwrap();

        let summary = import_conductor_app_database(&source_path, &target_path).unwrap();
        assert_eq!(summary.repositories_imported, 2);
        assert_eq!(summary.workspaces_imported, 2);
        assert_eq!(summary.renamed_duplicate_workspaces, 1);

        let target = Connection::open(target_path).unwrap();
        let names = target
            .prepare("SELECT name FROM workspaces ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(names, vec!["berlin", "other-berlin"]);
    }
}
