use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    pub id: i64,
    pub name: String,
    pub root_path: PathBuf,
    pub default_branch: String,
    pub remote_name: String,
    pub workspace_parent_path: PathBuf,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddRepository {
    pub name: Option<String>,
    pub root_path: PathBuf,
    pub default_branch: Option<String>,
    pub remote_name: String,
    pub workspace_parent_path: Option<PathBuf>,
}

pub struct RepositoryStore {
    conn: Connection,
}

impl RepositoryStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create data directory {}", parent.display()))?;
        }

        let conn = Connection::open(path.as_ref())
            .with_context(|| format!("open database {}", path.as_ref().display()))?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    pub fn add(&self, input: AddRepository) -> Result<Repository> {
        let root_path = input
            .root_path
            .canonicalize()
            .with_context(|| format!("resolve repository path {}", input.root_path.display()))?;
        ensure_git_repository(&root_path)?;

        let name = input.name.unwrap_or_else(|| {
            root_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("repository")
                .to_owned()
        });
        let default_branch = input
            .default_branch
            .unwrap_or_else(|| detect_default_branch(&root_path, &input.remote_name));
        let workspace_parent_path = input
            .workspace_parent_path
            .unwrap_or_else(|| default_workspace_parent(&name));
        let root_path_value = root_path.to_string_lossy().to_string();
        let workspace_parent_path_value = workspace_parent_path.to_string_lossy().to_string();
        let now = timestamp();

        self.conn.execute(
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
                name,
                root_path_value,
                default_branch,
                input.remote_name,
                workspace_parent_path_value,
                now,
                now,
            ],
        )?;

        self.get_by_path(&root_path)
    }

    pub fn list(&self) -> Result<Vec<Repository>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, root_path, default_branch, remote_name, workspace_parent_path, created_at, updated_at
             FROM repositories ORDER BY name",
        )?;
        let repositories = stmt
            .query_map([], row_to_repository)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(repositories)
    }

    fn get_by_path(&self, root_path: &Path) -> Result<Repository> {
        self.conn
            .query_row(
                "SELECT id, name, root_path, default_branch, remote_name, workspace_parent_path, created_at, updated_at
                 FROM repositories WHERE root_path = ?1",
                [root_path.to_string_lossy().to_string()],
                row_to_repository,
            )
            .context("load repository")
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS repositories (
              id INTEGER PRIMARY KEY,
              name TEXT NOT NULL,
              root_path TEXT NOT NULL UNIQUE,
              default_branch TEXT NOT NULL,
              remote_name TEXT NOT NULL DEFAULT 'origin',
              workspace_parent_path TEXT NOT NULL,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
            ",
        )?;
        Ok(())
    }
}

fn row_to_repository(row: &rusqlite::Row<'_>) -> rusqlite::Result<Repository> {
    Ok(Repository {
        id: row.get(0)?,
        name: row.get(1)?,
        root_path: PathBuf::from(row.get::<_, String>(2)?),
        default_branch: row.get(3)?,
        remote_name: row.get(4)?,
        workspace_parent_path: PathBuf::from(row.get::<_, String>(5)?),
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn ensure_git_repository(path: &Path) -> Result<()> {
    let status = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--show-toplevel"])
        .status()
        .context("run git rev-parse")?;
    anyhow::ensure!(
        status.success(),
        "{} is not a Git repository",
        path.display()
    );
    Ok(())
}

fn detect_default_branch(root_path: &Path, remote_name: &str) -> String {
    let remote_head = format!("refs/remotes/{remote_name}/HEAD");
    let output = Command::new("git")
        .arg("-C")
        .arg(root_path)
        .args(["symbolic-ref", "--short", &remote_head])
        .output();

    output
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|branch| {
            branch
                .trim()
                .strip_prefix(&format!("{remote_name}/"))
                .map(str::to_owned)
        })
        .filter(|branch| !branch.is_empty())
        .unwrap_or_else(|| "main".to_owned())
}

fn default_workspace_parent(repo_name: &str) -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("conductor")
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
    use std::fs;

    #[test]
    fn add_repository_persists_metadata_and_lists_by_name() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = temp.path().join("demo");
        fs::create_dir(&repo_path).unwrap();
        Command::new("git")
            .args(["init", "--initial-branch", "main"])
            .arg(&repo_path)
            .status()
            .unwrap();

        let store = RepositoryStore::open(temp.path().join("state.db")).unwrap();
        let saved = store
            .add(AddRepository {
                name: Some("demo-app".to_owned()),
                root_path: repo_path.clone(),
                default_branch: Some("main".to_owned()),
                remote_name: "origin".to_owned(),
                workspace_parent_path: Some(temp.path().join("workspaces/demo-app")),
            })
            .unwrap();

        assert_eq!(saved.name, "demo-app");
        assert_eq!(saved.default_branch, "main");
        assert_eq!(saved.remote_name, "origin");
        assert_eq!(saved.root_path, repo_path.canonicalize().unwrap());

        let repositories = store.list().unwrap();
        assert_eq!(repositories, vec![saved]);
    }
}
