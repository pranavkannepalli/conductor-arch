use crate::settings::ensure_repository_config;
use anyhow::{Context, Result};
use rusqlite::{params, Connection};
#[cfg(unix)]
use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
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
        let input_path = input
            .root_path
            .canonicalize()
            .with_context(|| format!("resolve repository path {}", input.root_path.display()))?;
        let root_path = resolve_git_repository_root(&input_path)?;
        ensure_repository_config(&root_path)?;

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

    pub fn list_with_workspace_counts(&self) -> Result<Vec<(Repository, usize, usize)>> {
        let repos = self.list()?;
        let mut result = Vec::with_capacity(repos.len());
        for repo in repos {
            let active = self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM workspaces WHERE repository_id = ?1 AND status = 'active'",
                    [repo.id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0);
            let total = self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM workspaces WHERE repository_id = ?1",
                    [repo.id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0);
            result.push((repo, active as usize, total as usize));
        }
        Ok(result)
    }

    pub fn update(&self, name: &str) -> Result<Repository> {
        let repo = self.get_by_name(name)?;
        let remote_exists = Command::new("git")
            .arg("-C")
            .arg(&repo.root_path)
            .args(["remote", "get-url", &repo.remote_name])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if remote_exists {
            Command::new("git")
                .arg("-C")
                .arg(&repo.root_path)
                .args(["fetch", &repo.remote_name, "--prune"])
                .output()
                .with_context(|| format!("fetch {}", repo.remote_name))?;
        }

        let default_branch = detect_default_branch(&repo.root_path, &repo.remote_name);
        let now = timestamp();
        self.conn.execute(
            "UPDATE repositories SET default_branch = ?1, updated_at = ?2 WHERE name = ?3",
            rusqlite::params![default_branch, now, name],
        )?;
        self.get_by_name(name)
    }

    pub fn get_by_name(&self, name: &str) -> Result<Repository> {
        self.conn
            .query_row(
                "SELECT id, name, root_path, default_branch, remote_name, workspace_parent_path, created_at, updated_at
                 FROM repositories WHERE name = ?1",
                [name],
                row_to_repository,
            )
            .with_context(|| format!("load repository {name}"))
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

fn resolve_git_repository_root(path: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("run git rev-parse")?;
    anyhow::ensure!(
        output.status.success(),
        "{} is not a Git repository",
        path.display()
    );
    let root = path_from_git_stdout(output.stdout)?;
    root.canonicalize()
        .with_context(|| format!("resolve repository root {}", root.display()))
}

fn path_from_git_stdout(stdout: Vec<u8>) -> Result<PathBuf> {
    let stdout = stdout
        .strip_suffix(b"\r\n")
        .or_else(|| stdout.strip_suffix(b"\n"))
        .unwrap_or(&stdout);
    #[cfg(unix)]
    {
        Ok(PathBuf::from(OsString::from_vec(stdout.to_vec())))
    }
    #[cfg(not(unix))]
    {
        let root = String::from_utf8(stdout.to_vec()).context("parse git repository root")?;
        Ok(PathBuf::from(root.trim()))
    }
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
    crate::platform::home_dir()
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

    #[test]
    fn add_repository_bootstraps_repository_config() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = temp.path().join("demo");
        fs::create_dir(&repo_path).unwrap();
        Command::new("git")
            .args(["init", "--initial-branch", "main"])
            .arg(&repo_path)
            .status()
            .unwrap();

        RepositoryStore::open(temp.path().join("state.db"))
            .unwrap()
            .add(AddRepository {
                name: Some("demo-app".to_owned()),
                root_path: repo_path.clone(),
                default_branch: Some("main".to_owned()),
                remote_name: "origin".to_owned(),
                workspace_parent_path: Some(temp.path().join("workspaces/demo-app")),
            })
            .unwrap();

        assert!(repo_path.join(".archductor/settings.toml").exists());
    }

    #[test]
    fn add_repository_from_subdirectory_uses_git_root() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = temp.path().join("demo");
        let subdir = repo_path.join("apps/web");
        fs::create_dir_all(&subdir).unwrap();
        Command::new("git")
            .args(["init", "--initial-branch", "main"])
            .arg(&repo_path)
            .status()
            .unwrap();

        let store = RepositoryStore::open(temp.path().join("state.db")).unwrap();
        let saved = store
            .add(AddRepository {
                name: Some("demo-app".to_owned()),
                root_path: subdir.clone(),
                default_branch: Some("main".to_owned()),
                remote_name: "origin".to_owned(),
                workspace_parent_path: Some(temp.path().join("workspaces/demo-app")),
            })
            .unwrap();

        assert_eq!(saved.root_path, repo_path.canonicalize().unwrap());
        assert!(repo_path.join(".archductor/settings.toml").exists());
        assert!(!subdir.join(".archductor/settings.toml").exists());
    }

    #[test]
    fn path_from_git_stdout_strips_crlf_as_one_suffix() {
        let path = path_from_git_stdout(b"/tmp/demo\r\n".to_vec()).unwrap();

        assert_eq!(path, PathBuf::from("/tmp/demo"));
    }
}
