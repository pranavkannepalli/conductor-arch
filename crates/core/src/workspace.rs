use crate::settings::load_repository_settings;
use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use rusqlite::{params, Connection};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub id: i64,
    pub repository_id: i64,
    pub name: String,
    pub path: PathBuf,
    pub branch: String,
    pub base_ref: String,
    pub port_base: u16,
    pub status: String,
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateWorkspace {
    pub repository_name: String,
    pub name: String,
    pub branch: String,
    pub base_ref: Option<String>,
}

struct RepositoryRecord {
    id: i64,
    root_path: PathBuf,
    default_branch: String,
    remote_name: String,
    workspace_parent_path: PathBuf,
}

pub struct WorkspaceStore {
    conn: Connection,
}

impl WorkspaceStore {
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

    pub fn create(&self, input: CreateWorkspace) -> Result<Workspace> {
        let repository = self.load_repository(&input.repository_name)?;
        let base_ref = input.base_ref.unwrap_or_else(|| {
            if remote_exists(&repository.root_path, &repository.remote_name) {
                format!("{}/{}", repository.remote_name, repository.default_branch)
            } else {
                repository.default_branch.clone()
            }
        });
        if remote_exists(&repository.root_path, &repository.remote_name) {
            git(
                &repository.root_path,
                ["fetch", repository.remote_name.as_str(), "--prune"],
            )?;
        }

        let path = repository.workspace_parent_path.join(&input.name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create workspace parent {}", parent.display()))?;
        }

        git_dynamic(
            &repository.root_path,
            &[
                "worktree",
                "add",
                "-b",
                input.branch.as_str(),
                path.to_string_lossy().as_ref(),
                base_ref.as_str(),
            ],
        )?;
        std::fs::create_dir_all(path.join(".context"))
            .with_context(|| format!("create workspace context directory {}", path.display()))?;
        copy_included_ignored_files(&repository.root_path, &path)?;

        let port_base = self.next_port_base()?;
        let settings = load_repository_settings(&repository.root_path)?;
        run_setup_script(&settings, &repository, &path, &input.name, port_base)?;
        let now = timestamp();
        self.conn.execute(
            "INSERT INTO workspaces (
                repository_id, name, path, branch, base_ref, port_base, status, archived_at, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', NULL, ?7, ?8)",
            params![
                repository.id,
                input.name,
                path.to_string_lossy().to_string(),
                input.branch,
                base_ref,
                i64::from(port_base),
                now,
                now,
            ],
        )?;

        self.get_by_path(&path)
    }

    pub fn list(&self) -> Result<Vec<Workspace>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, repository_id, name, path, branch, base_ref, port_base, status, archived_at, created_at, updated_at
             FROM workspaces ORDER BY name",
        )?;
        let workspaces = stmt
            .query_map([], row_to_workspace)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(workspaces)
    }

    pub fn archive(&self, name: &str) -> Result<Workspace> {
        let now = timestamp();
        let changed = self.conn.execute(
            "UPDATE workspaces
             SET status = 'archived', archived_at = ?1, updated_at = ?2
             WHERE name = ?3",
            params![now, now, name],
        )?;
        anyhow::ensure!(changed > 0, "workspace {name} not found");
        self.get_by_name(name)
    }

    fn get_by_path(&self, path: &Path) -> Result<Workspace> {
        self.conn
            .query_row(
                "SELECT id, repository_id, name, path, branch, base_ref, port_base, status, archived_at, created_at, updated_at
                 FROM workspaces WHERE path = ?1",
                [path.to_string_lossy().to_string()],
                row_to_workspace,
            )
            .context("load workspace")
    }

    fn get_by_name(&self, name: &str) -> Result<Workspace> {
        self.conn
            .query_row(
                "SELECT id, repository_id, name, path, branch, base_ref, port_base, status, archived_at, created_at, updated_at
                 FROM workspaces WHERE name = ?1",
                [name],
                row_to_workspace,
            )
            .with_context(|| format!("load workspace {name}"))
    }

    fn load_repository(&self, name: &str) -> Result<RepositoryRecord> {
        self.conn
            .query_row(
                "SELECT id, root_path, default_branch, remote_name, workspace_parent_path
                 FROM repositories WHERE name = ?1",
                [name],
                |row| {
                    Ok(RepositoryRecord {
                        id: row.get(0)?,
                        root_path: PathBuf::from(row.get::<_, String>(1)?),
                        default_branch: row.get(2)?,
                        remote_name: row.get(3)?,
                        workspace_parent_path: PathBuf::from(row.get::<_, String>(4)?),
                    })
                },
            )
            .with_context(|| format!("load repository {name}"))
    }

    fn next_port_base(&self) -> Result<u16> {
        let next = self
            .conn
            .query_row("SELECT MAX(port_base) FROM workspaces", [], |row| {
                row.get::<_, Option<i64>>(0)
            })?
            .map(|port| port + 10)
            .unwrap_or(3000);
        u16::try_from(next).context("workspace port base exceeded u16 range")
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

            CREATE TABLE IF NOT EXISTS workspaces (
              id INTEGER PRIMARY KEY,
              repository_id INTEGER NOT NULL REFERENCES repositories(id),
              name TEXT NOT NULL,
              path TEXT NOT NULL UNIQUE,
              branch TEXT NOT NULL,
              base_ref TEXT NOT NULL,
              port_base INTEGER NOT NULL,
              status TEXT NOT NULL,
              archived_at TEXT,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
            ",
        )?;
        Ok(())
    }
}

fn row_to_workspace(row: &rusqlite::Row<'_>) -> rusqlite::Result<Workspace> {
    let port_base = row.get::<_, i64>(6)?;
    Ok(Workspace {
        id: row.get(0)?,
        repository_id: row.get(1)?,
        name: row.get(2)?,
        path: PathBuf::from(row.get::<_, String>(3)?),
        branch: row.get(4)?,
        base_ref: row.get(5)?,
        port_base: u16::try_from(port_base).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                6,
                rusqlite::types::Type::Integer,
                Box::new(err),
            )
        })?,
        status: row.get(7)?,
        archived_at: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

fn remote_exists(root_path: &Path, remote_name: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(root_path)
        .args(["remote", "get-url", remote_name])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn git<const N: usize>(cwd: &Path, args: [&str; N]) -> Result<()> {
    git_dynamic(cwd, &args)
}

fn git_dynamic(cwd: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .with_context(|| format!("run git in {}", cwd.display()))?;
    anyhow::ensure!(
        output.status.success(),
        "git command failed in {}: {}\n{}",
        cwd.display(),
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    Ok(())
}

fn timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_owned())
}

fn copy_included_ignored_files(repo_path: &Path, workspace_path: &Path) -> Result<()> {
    let patterns = included_file_patterns(repo_path)?;
    if patterns.is_empty() {
        return Ok(());
    }

    let matcher = build_glob_set(&patterns)?;
    for entry in WalkDir::new(repo_path)
        .into_iter()
        .filter_entry(|entry| should_descend(entry.path()))
    {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }

        let source_path = entry.path();
        let relative_path = source_path
            .strip_prefix(repo_path)
            .with_context(|| format!("strip repository path {}", source_path.display()))?;
        if !matcher.is_match(relative_path) || !git_ignored(repo_path, relative_path) {
            continue;
        }

        let destination = workspace_path.join(relative_path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create copy destination {}", parent.display()))?;
        }
        fs::copy(source_path, &destination).with_context(|| {
            format!(
                "copy ignored included file {} to {}",
                source_path.display(),
                destination.display()
            )
        })?;
    }

    Ok(())
}

fn included_file_patterns(repo_path: &Path) -> Result<Vec<String>> {
    let mut patterns = Vec::new();
    let worktreeinclude_path = repo_path.join(".worktreeinclude");
    if worktreeinclude_path.exists() {
        patterns.extend(parse_pattern_lines(
            &fs::read_to_string(&worktreeinclude_path)
                .with_context(|| format!("read {}", worktreeinclude_path.display()))?,
        ));
    }
    patterns.extend(load_repository_settings(repo_path)?.file_include_globs);
    Ok(patterns)
}

fn parse_pattern_lines(input: &str) -> Vec<String> {
    input
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with('!'))
        .map(str::to_owned)
        .collect()
}

fn build_glob_set(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern).with_context(|| format!("parse include glob {pattern}"))?);
    }
    builder.build().context("build include glob set")
}

fn should_descend(path: &Path) -> bool {
    !path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| matches!(name, ".git" | "node_modules" | "target"))
        .unwrap_or(false)
}

fn git_ignored(repo_path: &Path, relative_path: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("check-ignore")
        .arg(relative_path)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn run_setup_script(
    settings: &crate::settings::RepositorySettings,
    repository: &RepositoryRecord,
    workspace_path: &Path,
    workspace_name: &str,
    port_base: u16,
) -> Result<()> {
    let Some(setup) = &settings.scripts.setup else {
        return Ok(());
    };

    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(setup)
        .current_dir(workspace_path)
        .env("CONDUCTOR_WORKSPACE_NAME", workspace_name)
        .env("CONDUCTOR_WORKSPACE_PATH", workspace_path)
        .env("CONDUCTOR_ROOT_PATH", &repository.root_path)
        .env("CONDUCTOR_DEFAULT_BRANCH", &repository.default_branch)
        .env("CONDUCTOR_PORT", port_base.to_string())
        .env("CONDUCTOR_IS_LOCAL", "1");

    for (key, value) in &settings.environment_variables {
        command.env(key, value);
    }

    let output = command
        .output()
        .with_context(|| format!("run setup script in {}", workspace_path.display()))?;
    anyhow::ensure!(
        output.status.success(),
        "setup script failed in {}: {}\n{}",
        workspace_path.display(),
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::{AddRepository, RepositoryStore};
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    #[test]
    fn create_workspace_adds_git_worktree_context_dir_and_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        let db_path = temp.path().join("state.db");
        let workspace_parent = temp.path().join("workspaces/demo");

        RepositoryStore::open(&db_path)
            .unwrap()
            .add(AddRepository {
                name: Some("demo".to_owned()),
                root_path: repo_path.clone(),
                default_branch: Some("main".to_owned()),
                remote_name: "origin".to_owned(),
                workspace_parent_path: Some(workspace_parent.clone()),
            })
            .unwrap();

        let store = WorkspaceStore::open(&db_path).unwrap();
        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        assert_eq!(workspace.name, "berlin");
        assert_eq!(workspace.branch, "lc/berlin");
        assert_eq!(workspace.base_ref, "main");
        assert_eq!(workspace.port_base, 3000);
        assert_eq!(workspace.status, "active");
        assert_eq!(workspace.path, workspace_parent.join("berlin"));
        assert!(workspace.path.join(".context").is_dir());

        let branch = git_output(&workspace.path, ["branch", "--show-current"]);
        assert_eq!(branch.trim(), "lc/berlin");

        let workspaces = store.list().unwrap();
        assert_eq!(workspaces, vec![workspace]);
    }

    #[test]
    fn create_workspace_allocates_next_port_block() {
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

        let store = WorkspaceStore::open(&db_path).unwrap();
        let first = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let second = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "tokyo".to_owned(),
                branch: "lc/tokyo".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        assert_eq!(first.port_base, 3000);
        assert_eq!(second.port_base, 3010);
    }

    #[test]
    fn create_workspace_copies_only_included_ignored_files() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::write(repo_path.join(".gitignore"), ".env*\nconfig/*.local.json\n").unwrap();
        fs::write(
            repo_path.join(".worktreeinclude"),
            ".env.local\nREADME.md\n",
        )
        .unwrap();
        fs::create_dir(repo_path.join(".conductor")).unwrap();
        fs::write(
            repo_path.join(".conductor/settings.toml"),
            r#"
file_include_globs = """
config/*.local.json
notes.local
"""
"#,
        )
        .unwrap();
        fs::create_dir(repo_path.join("config")).unwrap();
        fs::write(repo_path.join(".env.local"), "TOKEN=secret\n").unwrap();
        fs::write(repo_path.join("config/app.local.json"), "{}\n").unwrap();
        fs::write(repo_path.join("notes.local"), "not ignored\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "add",
                ".gitignore",
                ".worktreeinclude",
                ".conductor/settings.toml",
            ])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Conductor",
                "-c",
                "user.email=linux-conductor@example.test",
                "commit",
                "-m",
                "add conductor settings",
            ])
            .status()
            .unwrap();

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

        let workspace = WorkspaceStore::open(&db_path)
            .unwrap()
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        assert_eq!(
            fs::read_to_string(workspace.path.join(".env.local")).unwrap(),
            "TOKEN=secret\n"
        );
        assert_eq!(
            fs::read_to_string(workspace.path.join("config/app.local.json")).unwrap(),
            "{}\n"
        );
        assert!(!workspace.path.join("notes.local").exists());
    }

    #[test]
    fn create_workspace_runs_setup_script_with_conductor_environment() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".conductor")).unwrap();
        fs::write(
            repo_path.join(".conductor/settings.toml"),
            r#"
[scripts]
setup = "printf '%s\n' \"$CONDUCTOR_WORKSPACE_NAME\" \"$CONDUCTOR_WORKSPACE_PATH\" \"$CONDUCTOR_ROOT_PATH\" \"$CONDUCTOR_DEFAULT_BRANCH\" \"$CONDUCTOR_PORT\" \"$CONDUCTOR_IS_LOCAL\" \"$CUSTOM_VALUE\" > .context/setup-env"

[environment_variables]
CUSTOM_VALUE = "from-settings"
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".conductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Conductor",
                "-c",
                "user.email=linux-conductor@example.test",
                "commit",
                "-m",
                "add setup script",
            ])
            .status()
            .unwrap();

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

        let workspace = WorkspaceStore::open(&db_path)
            .unwrap()
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        let setup_env = fs::read_to_string(workspace.path.join(".context/setup-env")).unwrap();
        let lines = setup_env.lines().collect::<Vec<_>>();
        assert_eq!(
            lines,
            [
                "berlin",
                workspace.path.to_str().unwrap(),
                repo_path.canonicalize().unwrap().to_str().unwrap(),
                "main",
                "3000",
                "1",
                "from-settings",
            ]
        );
    }

    #[test]
    fn archive_marks_workspace_archived() {
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

        let store = WorkspaceStore::open(&db_path).unwrap();
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        let archived = store.archive("berlin").unwrap();

        assert_eq!(archived.status, "archived");
        assert!(archived.archived_at.is_some());
        assert_eq!(store.list().unwrap()[0], archived);
    }

    fn init_repo(path: PathBuf) -> PathBuf {
        fs::create_dir(&path).unwrap();
        Command::new("git")
            .args(["init", "--initial-branch", "main"])
            .arg(&path)
            .status()
            .unwrap();
        fs::write(path.join("README.md"), "demo\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&path)
            .args(["add", "."])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&path)
            .args([
                "-c",
                "user.name=Linux Conductor",
                "-c",
                "user.email=linux-conductor@example.test",
                "commit",
                "-m",
                "initial",
            ])
            .status()
            .unwrap();
        path
    }

    fn git_output<const N: usize>(cwd: &Path, args: [&str; N]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "git command failed: {output:?}");
        String::from_utf8(output.stdout).unwrap()
    }
}
