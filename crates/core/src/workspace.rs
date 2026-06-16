use crate::settings::load_repository_settings;
use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use rusqlite::{params, Connection};
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Shell,
    Codex,
    Claude,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLaunch {
    pub kind: SessionKind,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, OsString)>,
}

impl SessionLaunch {
    pub fn env_value(&self, key: &str) -> Option<&str> {
        self.env
            .iter()
            .find(|(name, _)| name == key)
            .and_then(|(_, value)| value.to_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessKind {
    Run,
    Session,
}

impl ProcessKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::Session => "session",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStatus {
    Running,
    Stopped,
    Exited,
}

impl ProcessStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Stopped => "stopped",
            Self::Exited => "exited",
        }
    }

    fn from_str(value: &str) -> rusqlite::Result<Self> {
        match value {
            "running" => Ok(Self::Running),
            "stopped" => Ok(Self::Stopped),
            "exited" => Ok(Self::Exited),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessRecord {
    pub id: i64,
    pub workspace_id: i64,
    pub kind: ProcessKind,
    pub command: String,
    pub pid: u32,
    pub log_path: PathBuf,
    pub status: ProcessStatus,
    pub started_at: String,
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    pub id: i64,
    pub workspace_id: i64,
    pub provider: String,
    pub number: i64,
    pub url: String,
    pub state: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Todo {
    pub id: i64,
    pub workspace_id: i64,
    pub text: String,
    pub status: String,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewComment {
    pub id: i64,
    pub workspace_id: i64,
    pub file_path: String,
    pub line_number: Option<i64>,
    pub body: String,
    pub status: String,
    pub github_thread_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchPushState {
    pub ahead: usize,
    pub behind: usize,
    pub has_upstream: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecksSummary {
    pub workspace: Workspace,
    pub changed_files: usize,
    pub run_status: Option<ProcessStatus>,
    pub session_status: Option<ProcessStatus>,
    pub active_sessions: usize,
    pub pull_request: Option<PullRequest>,
    pub open_todos: usize,
    pub total_todos: usize,
    pub branch_push_state: Option<BranchPushState>,
    pub open_review_comments: usize,
    pub conflicting_workspaces: Vec<(String, Vec<String>)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceStatusLine {
    pub workspace: Workspace,
    pub open_todos: usize,
    pub pull_request: Option<PullRequest>,
    pub run_running: bool,
    pub active_sessions: usize,
    pub branch_push_state: Option<BranchPushState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    pub id: i64,
    pub workspace_id: i64,
    pub session_id: Option<i64>,
    pub git_ref: String,
    pub message: String,
    pub created_at: String,
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
    logs_dir: PathBuf,
}

impl WorkspaceStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let logs_dir = path
            .as_ref()
            .parent()
            .map(|parent| parent.join("logs"))
            .unwrap_or_else(|| PathBuf::from("logs"));
        Self::open_with_logs(path, logs_dir)
    }

    pub fn open_with_logs(path: impl AsRef<Path>, logs_dir: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create data directory {}", parent.display()))?;
        }

        let conn = Connection::open(path.as_ref())
            .with_context(|| format!("open database {}", path.as_ref().display()))?;
        let store = Self {
            conn,
            logs_dir: logs_dir.as_ref().to_path_buf(),
        };
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
        let settings = load_repository_settings(&repository.root_path)?;
        initialize_context_files(&path, &settings)?;
        copy_included_ignored_files(&repository.root_path, &path)?;

        let port_base = self.next_port_base()?;
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

    pub fn list_status(&self) -> Result<Vec<WorkspaceStatusLine>> {
        let workspaces = self.list()?;
        let mut lines = Vec::with_capacity(workspaces.len());
        for workspace in workspaces {
            let open_todos: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM todos WHERE workspace_id = ?1 AND status = 'open'",
                [workspace.id],
                |row| row.get(0),
            )?;
            let pull_request = self.pull_request_by_workspace_id(workspace.id)?;
            let run_running: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM processes WHERE workspace_id = ?1 AND kind = 'run' AND status = 'running'",
                [workspace.id],
                |row| row.get(0),
            )?;
            let active_sessions =
                self.count_running_processes(workspace.id, ProcessKind::Session)?;
            let branch_push_state = if workspace.status == "active" {
                self.branch_push_state(&workspace.name).ok()
            } else {
                None
            };
            lines.push(WorkspaceStatusLine {
                workspace,
                open_todos: open_todos as usize,
                pull_request,
                run_running: run_running > 0,
                active_sessions,
                branch_push_state,
            });
        }
        Ok(lines)
    }

    pub fn rename(&self, name: &str, new_name: &str) -> Result<Workspace> {
        anyhow::ensure!(!new_name.is_empty(), "new workspace name must not be empty");
        let workspace = self.get_by_name(name)?;
        let new_path = workspace
            .path
            .parent()
            .map(|parent| parent.join(new_name))
            .with_context(|| {
                format!("workspace path has no parent: {}", workspace.path.display())
            })?;

        if workspace.path.exists() {
            fs::rename(&workspace.path, &new_path).with_context(|| {
                format!(
                    "rename workspace directory {} to {}",
                    workspace.path.display(),
                    new_path.display()
                )
            })?;
        }

        let now = timestamp();
        let changed = self.conn.execute(
            "UPDATE workspaces SET name = ?1, path = ?2, updated_at = ?3 WHERE id = ?4",
            params![
                new_name,
                new_path.to_string_lossy().to_string(),
                now,
                workspace.id
            ],
        )?;
        anyhow::ensure!(changed > 0, "workspace {name} not found");
        self.get_by_name(new_name)
    }

    pub fn discard(&self, name: &str) -> Result<Workspace> {
        let workspace = self.archive(name, true)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        // Delete the local branch (ignore errors if already gone or not fully merged)
        let _ = git_dynamic(
            &repository.root_path,
            &["branch", "-D", workspace.branch.as_str()],
        );
        Ok(workspace)
    }

    pub fn archive(&self, name: &str, remove_worktree: bool) -> Result<Workspace> {
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let settings = load_repository_settings(&repository.root_path)?;

        for kind in [ProcessKind::Run, ProcessKind::Session] {
            if let Some(process) = self.find_latest_running_process(workspace.id, kind)? {
                stop_process(process.pid)?;
                let now = timestamp();
                self.conn.execute(
                    "UPDATE processes SET status = ?1, ended_at = ?2 WHERE id = ?3",
                    params![ProcessStatus::Stopped.as_str(), now, process.id],
                )?;
            }
        }

        if let Some(archive_script) = &settings.scripts.archive {
            if workspace.path.exists() {
                run_shell_script(archive_script, &settings, &repository, &workspace)?;
            }
        }

        if remove_worktree {
            git_dynamic(
                &repository.root_path,
                &[
                    "worktree",
                    "remove",
                    "--force",
                    workspace.path.to_string_lossy().as_ref(),
                ],
            )?;
        }

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

    pub fn restore(&self, name: &str) -> Result<Workspace> {
        let workspace = self.get_by_name(name)?;
        anyhow::ensure!(
            workspace.status == "archived",
            "workspace {name} is not archived (status: {})",
            workspace.status
        );

        if !workspace.path.exists() {
            let repository = self.load_repository_by_id(workspace.repository_id)?;
            git_dynamic(
                &repository.root_path,
                &[
                    "worktree",
                    "add",
                    workspace.path.to_string_lossy().as_ref(),
                    workspace.branch.as_str(),
                ],
            )?;
            std::fs::create_dir_all(workspace.path.join(".context")).with_context(|| {
                format!(
                    "create workspace context directory {}",
                    workspace.path.display()
                )
            })?;
            let settings = load_repository_settings(&repository.root_path)?;
            copy_included_ignored_files(&repository.root_path, &workspace.path)?;
            initialize_context_files(&workspace.path, &settings)?;
        }

        let now = timestamp();
        self.conn.execute(
            "UPDATE workspaces SET status = 'active', archived_at = NULL, updated_at = ?1 WHERE name = ?2",
            params![now, name],
        )?;
        self.get_by_name(name)
    }

    pub fn run_workspace(&self, name: &str) -> Result<ProcessRecord> {
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let settings = load_repository_settings(&repository.root_path)?;
        let Some(run) = &settings.scripts.run else {
            anyhow::bail!("workspace {name} has no scripts.run configured");
        };

        let run_mode = settings.scripts.run_mode.as_deref().unwrap_or("concurrent");
        if run_mode == "nonconcurrent" {
            if let Some(conflicting) =
                self.find_running_workspace_in_repo(repository.id, workspace.id)?
            {
                anyhow::bail!(
                    "workspace {} is already running in this repository (run_mode = nonconcurrent); stop it first",
                    conflicting
                );
            }
        }

        self.start_process(ProcessKind::Run, run, &settings, &repository, &workspace)
    }

    pub fn stop_workspace(&self, name: &str) -> Result<ProcessRecord> {
        let workspace = self.get_by_name(name)?;
        let process = self.latest_running_process(workspace.id, ProcessKind::Run)?;
        stop_process(process.pid)?;
        let now = timestamp();
        self.conn.execute(
            "UPDATE processes SET status = ?1, ended_at = ?2 WHERE id = ?3",
            params![ProcessStatus::Stopped.as_str(), now, process.id],
        )?;
        self.get_process(process.id)
    }

    pub fn read_latest_run_log(&self, name: &str) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        let process = self.latest_process(workspace.id, ProcessKind::Run)?;
        fs::read_to_string(&process.log_path)
            .with_context(|| format!("read log {}", process.log_path.display()))
    }

    pub fn stop_session(&self, name: &str) -> Result<ProcessRecord> {
        let workspace = self.get_by_name(name)?;
        let process = self.latest_running_process(workspace.id, ProcessKind::Session)?;
        stop_process(process.pid)?;
        let now = timestamp();
        self.conn.execute(
            "UPDATE processes SET status = ?1, ended_at = ?2 WHERE id = ?3",
            params![ProcessStatus::Stopped.as_str(), now, process.id],
        )?;
        self.get_process(process.id)
    }

    pub fn read_latest_session_log(&self, name: &str) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        let process = self.latest_process(workspace.id, ProcessKind::Session)?;
        fs::read_to_string(&process.log_path)
            .with_context(|| format!("read log {}", process.log_path.display()))
    }

    pub fn changed_files(&self, name: &str) -> Result<Vec<String>> {
        let workspace = self.get_by_name(name)?;
        let output = git_output(&workspace.path, ["status", "--short"])?;
        Ok(output
            .lines()
            .filter_map(|line| line.get(3..))
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
            .collect())
    }

    pub fn unified_diff(&self, name: &str, path: Option<&Path>) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        if let Some(path) = path {
            let path_value = path.to_string_lossy().to_string();
            return git_output_dynamic(&workspace.path, &["diff", "--", path_value.as_str()]);
        }
        git_output(&workspace.path, ["diff", "--"])
    }

    pub fn push_branch(&self, name: &str) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        git_output_dynamic(
            &workspace.path,
            &["push", "-u", "origin", workspace.branch.as_str()],
        )
    }

    pub fn create_pull_request(
        &self,
        name: &str,
        title: Option<&str>,
        body: Option<&str>,
        draft: bool,
    ) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        let changed = self.changed_files(name)?;
        if changed.is_empty() {
            anyhow::bail!(
                "workspace {name} has no changed files; commit changes before creating a PR"
            );
        }
        let mut args = vec!["pr", "create"];
        if let Some(title) = title {
            args.extend(["--title", title]);
        } else {
            args.push("--fill");
        }
        if let Some(body) = body {
            args.extend(["--body", body]);
        }
        if draft {
            args.push("--draft");
        }
        let output = command_output(&workspace.path, "gh", &args)?;
        if let Some(url) = extract_pull_request_url(&output) {
            self.record_pull_request(workspace.id, &url)?;
        }
        Ok(output)
    }

    pub fn create_from_issue(
        &self,
        repository_name: &str,
        issue_number: u64,
        branch_prefix: Option<&str>,
    ) -> Result<Workspace> {
        let repository = self.load_repository(repository_name)?;
        // Fetch issue title from gh
        let output = command_output(
            &repository.root_path,
            "gh",
            &[
                "issue",
                "view",
                &issue_number.to_string(),
                "--json",
                "title,number",
            ],
        )?;
        let title = extract_json_string_field(&output, "title")
            .unwrap_or_else(|| format!("issue-{issue_number}"));

        // Slugify title for branch name
        let slug = slugify(&title);
        let prefix = branch_prefix.unwrap_or("lc");
        let branch = format!("{prefix}/{issue_number}/{slug}");
        let workspace_name = format!("issue-{issue_number}");

        self.create(CreateWorkspace {
            repository_name: repository_name.to_owned(),
            name: workspace_name,
            branch,
            base_ref: None,
        })
    }

    pub fn read_context_brief(&self, name: &str) -> Result<Option<String>> {
        let workspace = self.get_by_name(name)?;
        let path = workspace.path.join(".context/brief.md");
        if !path.exists() {
            return Ok(None);
        }
        let contents =
            fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        // Strip leading H1 heading line ("# Brief") and blank lines
        let body = contents
            .lines()
            .skip_while(|line| line.starts_with('#') || line.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if body.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(body))
    }

    pub fn pull_request_checks(&self, name: &str) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        command_output(&workspace.path, "gh", &["pr", "checks"])
    }

    pub fn pull_request(&self, name: &str) -> Result<Option<PullRequest>> {
        let workspace = self.get_by_name(name)?;
        self.pull_request_by_workspace_id(workspace.id)
    }

    pub fn refresh_pull_request_state(&self, name: &str) -> Result<Option<PullRequest>> {
        let workspace = self.get_by_name(name)?;
        if self.pull_request_by_workspace_id(workspace.id)?.is_none() {
            return Ok(None);
        }
        let state = command_output(&workspace.path, "gh", &["pr", "view", "--json", "state"])?;
        let state = extract_json_string_field(&state, "state").unwrap_or_else(|| "open".to_owned());
        let now = timestamp();
        self.conn.execute(
            "UPDATE pull_requests SET state = ?1, updated_at = ?2 WHERE workspace_id = ?3",
            params![state, now, workspace.id],
        )?;
        self.pull_request_by_workspace_id(workspace.id)
    }

    fn record_pull_request(&self, workspace_id: i64, url: &str) -> Result<PullRequest> {
        let number = parse_pull_request_number(url)
            .with_context(|| format!("parse pull request number from {url}"))?;
        let now = timestamp();
        self.conn.execute(
            "INSERT INTO pull_requests (
                workspace_id, provider, number, url, state, created_at, updated_at
            ) VALUES (?1, 'github', ?2, ?3, 'open', ?4, ?5)
            ON CONFLICT(workspace_id) DO UPDATE SET
                number = excluded.number,
                url = excluded.url,
                state = 'open',
                updated_at = excluded.updated_at",
            params![workspace_id, number, url, now, now],
        )?;
        self.pull_request_by_workspace_id(workspace_id)?
            .context("load recorded pull request")
    }

    fn pull_request_by_workspace_id(&self, workspace_id: i64) -> Result<Option<PullRequest>> {
        let result = self.conn.query_row(
            "SELECT id, workspace_id, provider, number, url, state, created_at, updated_at
             FROM pull_requests WHERE workspace_id = ?1",
            [workspace_id],
            row_to_pull_request,
        );
        match result {
            Ok(pull_request) => Ok(Some(pull_request)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    pub fn sync_todos_from_context(&self, name: &str) -> Result<usize> {
        let workspace = self.get_by_name(name)?;
        let todos_path = workspace.path.join(".context/todos.md");
        if !todos_path.exists() {
            return Ok(0);
        }
        let contents = fs::read_to_string(&todos_path)
            .with_context(|| format!("read {}", todos_path.display()))?;

        let mut imported = 0usize;
        for line in contents.lines() {
            let trimmed = line.trim();
            let (done, text) = if let Some(rest) = trimmed
                .strip_prefix("- [x] ")
                .or_else(|| trimmed.strip_prefix("- [X] "))
            {
                (true, rest)
            } else if let Some(rest) = trimmed.strip_prefix("- [ ] ") {
                (false, rest)
            } else {
                continue;
            };
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            let already_exists: bool = self.conn.query_row(
                "SELECT COUNT(*) FROM todos
                 WHERE workspace_id = ?1 AND text = ?2 AND source = 'context'",
                params![workspace.id, text],
                |row| row.get::<_, i64>(0),
            )? > 0;
            if !already_exists {
                let now = timestamp();
                let status = if done { "done" } else { "open" };
                self.conn.execute(
                    "INSERT INTO todos (workspace_id, text, status, source, created_at, updated_at)
                     VALUES (?1, ?2, ?3, 'context', ?4, ?5)",
                    params![workspace.id, text, status, now, now],
                )?;
                imported += 1;
            }
        }
        Ok(imported)
    }

    pub fn add_review_comment(
        &self,
        name: &str,
        file_path: &str,
        line_number: Option<i64>,
        body: &str,
    ) -> Result<ReviewComment> {
        let workspace = self.get_by_name(name)?;
        let now = timestamp();
        self.conn.execute(
            "INSERT INTO review_comments
                (workspace_id, file_path, line_number, body, status, github_thread_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'open', NULL, ?5, ?6)",
            params![workspace.id, file_path, line_number, body, now, now],
        )?;
        self.get_review_comment(self.conn.last_insert_rowid())
    }

    pub fn list_review_comments(&self, name: &str) -> Result<Vec<ReviewComment>> {
        let workspace = self.get_by_name(name)?;
        let mut stmt = self.conn.prepare(
            "SELECT id, workspace_id, file_path, line_number, body, status, github_thread_id, created_at, updated_at
             FROM review_comments WHERE workspace_id = ?1 ORDER BY file_path, line_number",
        )?;
        let comments = stmt
            .query_map([workspace.id], row_to_review_comment)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(comments)
    }

    pub fn resolve_review_comment(&self, id: i64) -> Result<ReviewComment> {
        let now = timestamp();
        let changed = self.conn.execute(
            "UPDATE review_comments SET status = 'resolved', updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        anyhow::ensure!(changed > 0, "review comment {id} not found");
        self.get_review_comment(id)
    }

    fn get_review_comment(&self, id: i64) -> Result<ReviewComment> {
        self.conn
            .query_row(
                "SELECT id, workspace_id, file_path, line_number, body, status, github_thread_id, created_at, updated_at
                 FROM review_comments WHERE id = ?1",
                [id],
                row_to_review_comment,
            )
            .with_context(|| format!("load review comment {id}"))
    }

    pub fn checkpoint_create(
        &self,
        name: &str,
        message: &str,
        session_id: Option<i64>,
    ) -> Result<Checkpoint> {
        let workspace = self.get_by_name(name)?;
        let now = timestamp();
        let git_ref = format!("refs/linux-conductor/checkpoints/{}/{now}", workspace.id);
        // Create the ref pointing at the current HEAD of the workspace branch
        let head = git_output_dynamic(&workspace.path, &["rev-parse", "HEAD"])?;
        let head = head.trim();
        git_dynamic(&workspace.path, &["update-ref", &git_ref, head])?;

        self.conn.execute(
            "INSERT INTO checkpoints (workspace_id, session_id, git_ref, message, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![workspace.id, session_id, git_ref, message, now],
        )?;
        self.get_checkpoint(self.conn.last_insert_rowid())
    }

    pub fn checkpoint_list(&self, name: &str) -> Result<Vec<Checkpoint>> {
        let workspace = self.get_by_name(name)?;
        let mut stmt = self.conn.prepare(
            "SELECT id, workspace_id, session_id, git_ref, message, created_at
             FROM checkpoints WHERE workspace_id = ?1 ORDER BY id DESC",
        )?;
        let checkpoints = stmt
            .query_map([workspace.id], row_to_checkpoint)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(checkpoints)
    }

    pub fn checkpoint_restore(&self, name: &str, checkpoint_id: i64) -> Result<Checkpoint> {
        let workspace = self.get_by_name(name)?;
        let checkpoint = self.get_checkpoint(checkpoint_id)?;
        anyhow::ensure!(
            checkpoint.workspace_id == workspace.id,
            "checkpoint {checkpoint_id} does not belong to workspace {name}"
        );

        // Resolve the checkpoint ref to a commit hash
        let commit = git_output_dynamic(&workspace.path, &["rev-parse", &checkpoint.git_ref])?;
        let commit = commit.trim();

        // Hard-reset the workspace to the checkpoint commit
        git_dynamic(&workspace.path, &["reset", "--hard", commit])?;
        // Remove untracked files that weren't part of the checkpoint
        git_dynamic(&workspace.path, &["clean", "-fd"])?;

        Ok(checkpoint)
    }

    fn get_checkpoint(&self, id: i64) -> Result<Checkpoint> {
        self.conn
            .query_row(
                "SELECT id, workspace_id, session_id, git_ref, message, created_at
                 FROM checkpoints WHERE id = ?1",
                [id],
                row_to_checkpoint,
            )
            .with_context(|| format!("load checkpoint {id}"))
    }

    pub fn branch_push_state(&self, name: &str) -> Result<BranchPushState> {
        let workspace = self.get_by_name(name)?;
        let upstream_exists = Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args(["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);

        if !upstream_exists {
            return Ok(BranchPushState {
                ahead: 0,
                behind: 0,
                has_upstream: false,
            });
        }

        let ahead = count_git_rev_list(&workspace.path, "@{u}..HEAD");
        let behind = count_git_rev_list(&workspace.path, "HEAD..@{u}");
        Ok(BranchPushState {
            ahead,
            behind,
            has_upstream: true,
        })
    }

    pub fn add_todo(&self, name: &str, text: &str) -> Result<Todo> {
        let workspace = self.get_by_name(name)?;
        let now = timestamp();
        self.conn.execute(
            "INSERT INTO todos (workspace_id, text, status, source, created_at, updated_at)
             VALUES (?1, ?2, 'open', 'manual', ?3, ?4)",
            params![workspace.id, text, now, now],
        )?;
        self.get_todo(self.conn.last_insert_rowid())
    }

    pub fn list_todos(&self, name: &str) -> Result<Vec<Todo>> {
        let workspace = self.get_by_name(name)?;
        let mut stmt = self.conn.prepare(
            "SELECT id, workspace_id, text, status, source, created_at, updated_at
             FROM todos WHERE workspace_id = ?1 ORDER BY id",
        )?;
        let todos = stmt
            .query_map([workspace.id], row_to_todo)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(todos)
    }

    pub fn complete_todo(&self, id: i64) -> Result<Todo> {
        let now = timestamp();
        let changed = self.conn.execute(
            "UPDATE todos SET status = 'done', updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        anyhow::ensure!(changed > 0, "todo {id} not found");
        self.get_todo(id)
    }

    fn get_todo(&self, id: i64) -> Result<Todo> {
        self.conn
            .query_row(
                "SELECT id, workspace_id, text, status, source, created_at, updated_at
                 FROM todos WHERE id = ?1",
                [id],
                row_to_todo,
            )
            .with_context(|| format!("load todo {id}"))
    }

    pub fn mcp_status(&self, name: &str) -> Result<crate::mcp::McpStatus> {
        let workspace = self.get_by_name(name)?;
        Ok(crate::mcp::workspace_mcp_status(&workspace.path))
    }

    /// Returns other active workspaces in the same repository that have overlapping changed files.
    pub fn find_conflicting_workspaces(&self, name: &str) -> Result<Vec<(String, Vec<String>)>> {
        let workspace = self.get_by_name(name)?;
        let my_files: std::collections::HashSet<String> =
            self.changed_files(name)?.into_iter().collect();
        if my_files.is_empty() {
            return Ok(Vec::new());
        }

        let siblings: Vec<Workspace> = {
            let mut stmt = self.conn.prepare(
                "SELECT id, repository_id, name, path, branch, base_ref, port_base, status, archived_at, created_at, updated_at
                 FROM workspaces WHERE repository_id = ?1 AND id != ?2 AND status = 'active'",
            )?;
            let rows = stmt
                .query_map(
                    params![workspace.repository_id, workspace.id],
                    row_to_workspace,
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };

        let mut conflicts = Vec::new();
        for sibling in siblings {
            let sibling_files: std::collections::HashSet<String> =
                self.changed_files(&sibling.name)?.into_iter().collect();
            let overlap: Vec<String> = my_files.intersection(&sibling_files).cloned().collect();
            if !overlap.is_empty() {
                let mut sorted = overlap;
                sorted.sort();
                conflicts.push((sibling.name, sorted));
            }
        }
        Ok(conflicts)
    }

    pub fn checks_summary(&self, name: &str) -> Result<ChecksSummary> {
        let workspace = self.get_by_name(name)?;
        let changed_files = self.changed_files(name)?.len();
        let run_status = self.latest_process_status(workspace.id, ProcessKind::Run)?;
        let session_status = self.latest_process_status(workspace.id, ProcessKind::Session)?;
        let active_sessions = self.count_running_processes(workspace.id, ProcessKind::Session)?;
        let pull_request = self.pull_request_by_workspace_id(workspace.id)?;
        let todos = self.list_todos(name)?;
        let open_todos = todos.iter().filter(|todo| todo.status == "open").count();
        let branch_push_state = self.branch_push_state(name).ok();
        let comments = self.list_review_comments(name)?;
        let open_review_comments = comments.iter().filter(|c| c.status == "open").count();
        let conflicting_workspaces = self.find_conflicting_workspaces(name).unwrap_or_default();
        Ok(ChecksSummary {
            workspace,
            changed_files,
            run_status,
            session_status,
            active_sessions,
            pull_request,
            open_todos,
            total_todos: todos.len(),
            branch_push_state,
            open_review_comments,
            conflicting_workspaces,
        })
    }

    fn count_running_processes(&self, workspace_id: i64, kind: ProcessKind) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM processes
             WHERE workspace_id = ?1 AND kind = ?2 AND status = 'running'",
            params![workspace_id, kind.as_str()],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    fn latest_process_status(
        &self,
        workspace_id: i64,
        kind: ProcessKind,
    ) -> Result<Option<ProcessStatus>> {
        let result = self.conn.query_row(
            "SELECT status FROM processes
             WHERE workspace_id = ?1 AND kind = ?2
             ORDER BY id DESC LIMIT 1",
            params![workspace_id, kind.as_str()],
            |row| row.get::<_, String>(0),
        );
        match result {
            Ok(status) => Ok(Some(ProcessStatus::from_str(&status)?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    fn find_latest_running_process(
        &self,
        workspace_id: i64,
        kind: ProcessKind,
    ) -> Result<Option<ProcessRecord>> {
        let result = self.conn.query_row(
            "SELECT id, workspace_id, kind, command, pid, log_path, status, started_at, ended_at
             FROM processes
             WHERE workspace_id = ?1 AND kind = ?2 AND status = 'running'
             ORDER BY id DESC LIMIT 1",
            params![workspace_id, kind.as_str()],
            row_to_process,
        );
        match result {
            Ok(process) => Ok(Some(process)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    fn find_running_workspace_in_repo(
        &self,
        repository_id: i64,
        exclude_workspace_id: i64,
    ) -> Result<Option<String>> {
        let result = self.conn.query_row(
            "SELECT w.name FROM workspaces w
             INNER JOIN processes p ON p.workspace_id = w.id
             WHERE w.repository_id = ?1
               AND w.id != ?2
               AND p.kind = 'run'
               AND p.status = 'running'
             ORDER BY p.id DESC LIMIT 1",
            params![repository_id, exclude_workspace_id],
            |row| row.get::<_, String>(0),
        );
        match result {
            Ok(name) => Ok(Some(name)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    pub fn merge_pull_request(&self, name: &str, method: &str) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        let pr = self
            .pull_request_by_workspace_id(workspace.id)?
            .with_context(|| format!("no pull request recorded for workspace {name}"))?;

        let todos = self.list_todos(name)?;
        let open_todos = todos.iter().filter(|t| t.status == "open").count();
        if open_todos > 0 {
            anyhow::bail!(
                "{open_todos} open todo(s) remain in workspace {name}; complete them before merging"
            );
        }

        let output = command_output(
            &workspace.path,
            "gh",
            &[
                "pr",
                "merge",
                pr.number.to_string().as_str(),
                &format!("--{method}"),
            ],
        )?;

        let now = timestamp();
        self.conn.execute(
            "UPDATE pull_requests SET state = 'merged', updated_at = ?1 WHERE workspace_id = ?2",
            params![now, workspace.id],
        )?;

        Ok(output)
    }

    pub fn editor_launch(&self, name: &str, editor: &str) -> Result<SessionLaunch> {
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let settings = load_repository_settings(&repository.root_path)?;
        Ok(SessionLaunch {
            kind: SessionKind::Shell,
            program: PathBuf::from(editor),
            args: vec![workspace.path.to_string_lossy().to_string()],
            cwd: workspace.path.clone(),
            env: conductor_environment(&settings, &repository, &workspace),
        })
    }

    pub fn session_launch(&self, name: &str, kind: SessionKind) -> Result<SessionLaunch> {
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let settings = load_repository_settings(&repository.root_path)?;
        let (program, args) = match kind {
            SessionKind::Shell => (
                std::env::var_os("SHELL")
                    .filter(|shell| !shell.is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("/bin/sh")),
                Vec::new(),
            ),
            SessionKind::Codex => (PathBuf::from("codex"), Vec::new()),
            SessionKind::Claude => (PathBuf::from("claude"), Vec::new()),
        };

        Ok(SessionLaunch {
            kind,
            program,
            args,
            cwd: workspace.path.clone(),
            env: conductor_environment(&settings, &repository, &workspace),
        })
    }

    pub fn list_sessions(&self, name: &str) -> Result<Vec<ProcessRecord>> {
        self.list_processes(name, ProcessKind::Session)
    }

    pub fn list_runs(&self, name: &str) -> Result<Vec<ProcessRecord>> {
        self.list_processes(name, ProcessKind::Run)
    }

    fn list_processes(&self, name: &str, kind: ProcessKind) -> Result<Vec<ProcessRecord>> {
        let workspace = self.get_by_name(name)?;
        let mut stmt = self.conn.prepare(
            "SELECT id, workspace_id, kind, command, pid, log_path, status, started_at, ended_at
             FROM processes WHERE workspace_id = ?1 AND kind = ?2
             ORDER BY id DESC",
        )?;
        let records = stmt
            .query_map(params![workspace.id, kind.as_str()], row_to_process)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(records)
    }

    pub fn start_session(&self, name: &str, kind: SessionKind) -> Result<ProcessRecord> {
        let launch = self.session_launch(name, kind)?;
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let settings = load_repository_settings(&repository.root_path)?;
        let command = shell_words(&launch.program, &launch.args);
        self.start_process(
            ProcessKind::Session,
            &command,
            &settings,
            &repository,
            &workspace,
        )
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

    fn load_repository_by_id(&self, id: i64) -> Result<RepositoryRecord> {
        self.conn
            .query_row(
                "SELECT id, root_path, default_branch, remote_name, workspace_parent_path
                 FROM repositories WHERE id = ?1",
                [id],
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
            .with_context(|| format!("load repository {id}"))
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

    fn start_process(
        &self,
        kind: ProcessKind,
        script: &str,
        settings: &crate::settings::RepositorySettings,
        repository: &RepositoryRecord,
        workspace: &Workspace,
    ) -> Result<ProcessRecord> {
        let now = timestamp();
        let log_path = self
            .logs_dir
            .join(&workspace.name)
            .join(format!("{}-{now}.log", kind.as_str()));
        if let Some(parent) = log_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create log directory {}", parent.display()))?;
        }
        let log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .with_context(|| format!("open log {}", log_path.display()))?;
        let stderr = log_file
            .try_clone()
            .with_context(|| format!("clone log {}", log_path.display()))?;

        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg(script)
            .current_dir(&workspace.path)
            .envs(conductor_environment(settings, repository, workspace))
            .stdin(Stdio::null())
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(stderr));
        #[cfg(unix)]
        {
            command.process_group(0);
        }
        let child = command
            .spawn()
            .with_context(|| format!("start script in {}", workspace.path.display()))?;

        self.conn.execute(
            "INSERT INTO processes (
                workspace_id, kind, command, pid, log_path, status, started_at, ended_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
            params![
                workspace.id,
                kind.as_str(),
                script,
                i64::from(child.id()),
                log_path.to_string_lossy().to_string(),
                ProcessStatus::Running.as_str(),
                now,
            ],
        )?;

        self.latest_process(workspace.id, kind)
    }

    fn latest_running_process(
        &self,
        workspace_id: i64,
        kind: ProcessKind,
    ) -> Result<ProcessRecord> {
        self.find_latest_running_process(workspace_id, kind)?
            .context("load latest running process")
    }

    fn latest_process(&self, workspace_id: i64, kind: ProcessKind) -> Result<ProcessRecord> {
        self.conn
            .query_row(
                "SELECT id, workspace_id, kind, command, pid, log_path, status, started_at, ended_at
                 FROM processes
                 WHERE workspace_id = ?1 AND kind = ?2
                 ORDER BY id DESC LIMIT 1",
                params![workspace_id, kind.as_str()],
                row_to_process,
            )
            .context("load latest process")
    }

    fn get_process(&self, id: i64) -> Result<ProcessRecord> {
        self.conn
            .query_row(
                "SELECT id, workspace_id, kind, command, pid, log_path, status, started_at, ended_at
                 FROM processes WHERE id = ?1",
                [id],
                row_to_process,
            )
            .with_context(|| format!("load process {id}"))
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

            CREATE TABLE IF NOT EXISTS processes (
              id INTEGER PRIMARY KEY,
              workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
              kind TEXT NOT NULL,
              command TEXT NOT NULL,
              pid INTEGER NOT NULL,
              log_path TEXT NOT NULL,
              status TEXT NOT NULL,
              started_at TEXT NOT NULL,
              ended_at TEXT
            );

            CREATE TABLE IF NOT EXISTS pull_requests (
              id INTEGER PRIMARY KEY,
              workspace_id INTEGER NOT NULL UNIQUE REFERENCES workspaces(id),
              provider TEXT NOT NULL,
              number INTEGER NOT NULL,
              url TEXT NOT NULL,
              state TEXT NOT NULL,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS todos (
              id INTEGER PRIMARY KEY,
              workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
              text TEXT NOT NULL,
              status TEXT NOT NULL,
              source TEXT NOT NULL,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS review_comments (
              id INTEGER PRIMARY KEY,
              workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
              file_path TEXT NOT NULL,
              line_number INTEGER,
              body TEXT NOT NULL,
              status TEXT NOT NULL,
              github_thread_id TEXT,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS checkpoints (
              id INTEGER PRIMARY KEY,
              workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
              session_id INTEGER REFERENCES processes(id),
              git_ref TEXT NOT NULL,
              message TEXT NOT NULL,
              created_at TEXT NOT NULL
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

fn row_to_process(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProcessRecord> {
    let kind = match row.get::<_, String>(2)?.as_str() {
        "run" => ProcessKind::Run,
        "session" => ProcessKind::Session,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let pid = row.get::<_, i64>(4)?;
    Ok(ProcessRecord {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        kind,
        command: row.get(3)?,
        pid: u32::try_from(pid).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Integer,
                Box::new(err),
            )
        })?,
        log_path: PathBuf::from(row.get::<_, String>(5)?),
        status: ProcessStatus::from_str(&row.get::<_, String>(6)?)?,
        started_at: row.get(7)?,
        ended_at: row.get(8)?,
    })
}

fn row_to_pull_request(row: &rusqlite::Row<'_>) -> rusqlite::Result<PullRequest> {
    Ok(PullRequest {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        provider: row.get(2)?,
        number: row.get(3)?,
        url: row.get(4)?,
        state: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn row_to_todo(row: &rusqlite::Row<'_>) -> rusqlite::Result<Todo> {
    Ok(Todo {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        text: row.get(2)?,
        status: row.get(3)?,
        source: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

fn row_to_review_comment(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReviewComment> {
    Ok(ReviewComment {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        file_path: row.get(2)?,
        line_number: row.get(3)?,
        body: row.get(4)?,
        status: row.get(5)?,
        github_thread_id: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn row_to_checkpoint(row: &rusqlite::Row<'_>) -> rusqlite::Result<Checkpoint> {
    Ok(Checkpoint {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        session_id: row.get(2)?,
        git_ref: row.get(3)?,
        message: row.get(4)?,
        created_at: row.get(5)?,
    })
}

fn count_git_rev_list(cwd: &Path, range: &str) -> usize {
    Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-list", "--count", range])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(0)
}

fn extract_pull_request_url(output: &str) -> Option<String> {
    output
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| line.starts_with("https://"))
        .map(str::to_owned)
}

fn parse_pull_request_number(url: &str) -> Option<i64> {
    url.trim_end_matches('/')
        .rsplit('/')
        .next()
        .and_then(|segment| segment.parse::<i64>().ok())
}

fn extract_json_string_field(json: &str, field: &str) -> Option<String> {
    let needle = format!("\"{field}\"");
    let field_start = json.find(&needle)? + needle.len();
    let after_colon = json[field_start..].trim_start();
    let after_colon = after_colon.strip_prefix(':')?.trim_start();
    let after_quote = after_colon.strip_prefix('"')?;
    let end = after_quote.find('"')?;
    Some(after_quote[..end].to_owned())
}

fn slugify(text: &str) -> String {
    let slug: String = text
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.len() > 40 {
        slug[..40].trim_end_matches('-').to_owned()
    } else {
        slug
    }
}

fn initialize_context_files(
    workspace_path: &Path,
    settings: &crate::settings::RepositorySettings,
) -> Result<()> {
    let context_dir = workspace_path.join(".context");

    let brief = "# Brief\n\nDescribe the task for this workspace.\n";
    let agent_notes =
        "# Agent Notes\n\nHandoff notes and context for agents working in this workspace.\n";
    let todos = "# Todos\n\n- [ ] Define task scope\n";

    fs::write(context_dir.join("brief.md"), brief).context("write .context/brief.md")?;
    fs::write(context_dir.join("agent-notes.md"), agent_notes)
        .context("write .context/agent-notes.md")?;
    fs::write(context_dir.join("todos.md"), todos).context("write .context/todos.md")?;

    let mut prompts_lines = Vec::new();
    if let Some(general) = settings.prompts.as_ref().and_then(|p| p.general.as_deref()) {
        prompts_lines.push(format!("## General\n\n{general}\n"));
    }
    if let Some(code_review) = settings
        .prompts
        .as_ref()
        .and_then(|p| p.code_review.as_deref())
    {
        prompts_lines.push(format!("## Code Review\n\n{code_review}\n"));
    }
    if let Some(create_pr) = settings
        .prompts
        .as_ref()
        .and_then(|p| p.create_pr.as_deref())
    {
        prompts_lines.push(format!("## Create PR\n\n{create_pr}\n"));
    }
    if !prompts_lines.is_empty() {
        let content = format!("# Prompts\n\n{}", prompts_lines.join("\n"));
        fs::write(context_dir.join("PROMPTS.md"), content).context("write .context/PROMPTS.md")?;
    }

    Ok(())
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

fn git_output<const N: usize>(cwd: &Path, args: [&str; N]) -> Result<String> {
    git_output_dynamic(cwd, &args)
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

fn git_output_dynamic(cwd: &Path, args: &[&str]) -> Result<String> {
    command_output(cwd, "git", args)
}

fn command_output(cwd: &Path, program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("run {program} in {}", cwd.display()))?;
    anyhow::ensure!(
        output.status.success(),
        "{program} command failed in {}: {}\n{}",
        cwd.display(),
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
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

fn stop_process(pid: u32) -> Result<()> {
    let target = format!("-{pid}");
    let status = Command::new("kill")
        .arg("-TERM")
        .arg(&target)
        .status()
        .or_else(|_| {
            Command::new("kill")
                .arg("-TERM")
                .arg(pid.to_string())
                .status()
        })
        .context("run kill")?;
    anyhow::ensure!(status.success(), "failed to stop process {pid}");
    Ok(())
}

fn shell_words(program: &Path, args: &[String]) -> String {
    let mut words = vec![quote_shell_word(&program.to_string_lossy())];
    words.extend(args.iter().map(|arg| quote_shell_word(arg)));
    words.join(" ")
}

fn quote_shell_word(value: &str) -> String {
    if value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-'))
    {
        return value.to_owned();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
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

    let workspace = Workspace {
        id: 0,
        repository_id: repository.id,
        name: workspace_name.to_owned(),
        path: workspace_path.to_path_buf(),
        branch: String::new(),
        base_ref: repository.default_branch.clone(),
        port_base,
        status: "active".to_owned(),
        archived_at: None,
        created_at: String::new(),
        updated_at: String::new(),
    };
    run_shell_script(setup, settings, repository, &workspace)
}

fn run_shell_script(
    script: &str,
    settings: &crate::settings::RepositorySettings,
    repository: &RepositoryRecord,
    workspace: &Workspace,
) -> Result<()> {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(script)
        .current_dir(&workspace.path)
        .envs(conductor_environment(settings, repository, workspace));

    let output = command
        .output()
        .with_context(|| format!("run script in {}", workspace.path.display()))?;
    anyhow::ensure!(
        output.status.success(),
        "script failed in {}: {}\n{}",
        workspace.path.display(),
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );

    Ok(())
}

fn conductor_environment(
    settings: &crate::settings::RepositorySettings,
    repository: &RepositoryRecord,
    workspace: &Workspace,
) -> Vec<(String, OsString)> {
    let mut env = vec![
        (
            "CONDUCTOR_WORKSPACE_NAME".to_owned(),
            OsString::from(&workspace.name),
        ),
        (
            "CONDUCTOR_WORKSPACE_PATH".to_owned(),
            workspace.path.as_os_str().to_owned(),
        ),
        (
            "CONDUCTOR_ROOT_PATH".to_owned(),
            repository.root_path.as_os_str().to_owned(),
        ),
        (
            "CONDUCTOR_DEFAULT_BRANCH".to_owned(),
            OsString::from(&repository.default_branch),
        ),
        (
            "CONDUCTOR_PORT".to_owned(),
            OsString::from(workspace.port_base.to_string()),
        ),
        ("CONDUCTOR_IS_LOCAL".to_owned(), OsString::from("1")),
    ];
    env.extend(
        settings
            .environment_variables
            .iter()
            .map(|(key, value)| (key.clone(), OsString::from(value))),
    );
    env
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
                "-c",
                "commit.gpgsign=false",
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
                "-c",
                "commit.gpgsign=false",
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

        let archived = store.archive("berlin", false).unwrap();

        assert_eq!(archived.status, "archived");
        assert!(archived.archived_at.is_some());
        assert_eq!(store.list().unwrap()[0], archived);
    }

    #[test]
    fn archive_stops_running_processes_and_removes_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".conductor")).unwrap();
        fs::write(
            repo_path.join(".conductor/settings.toml"),
            r#"
[scripts]
run = "printf 'started\n'; while true; do sleep 1; done"
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
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "add run script",
            ])
            .status()
            .unwrap();

        let db_path = temp.path().join("state.db");
        let store = WorkspaceStore::open_with_logs(&db_path, temp.path().join("logs")).unwrap();
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

        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        let run = store.run_workspace("berlin").unwrap();
        wait_for_log(&run.log_path, "started");

        let archived = store.archive("berlin", true).unwrap();

        assert_eq!(archived.status, "archived");
        assert!(!workspace.path.exists());
        let summary = store.checks_summary("berlin").unwrap();
        assert_eq!(summary.run_status, Some(ProcessStatus::Stopped));
    }

    #[test]
    fn run_workspace_executes_run_script_with_conductor_environment() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".conductor")).unwrap();
        fs::write(
            repo_path.join(".conductor/settings.toml"),
            r#"
[scripts]
run = "printf '%s\n' \"$CONDUCTOR_WORKSPACE_NAME\" \"$CONDUCTOR_WORKSPACE_PATH\" \"$CONDUCTOR_ROOT_PATH\" \"$CONDUCTOR_DEFAULT_BRANCH\" \"$CONDUCTOR_PORT\" \"$CONDUCTOR_IS_LOCAL\" \"$CUSTOM_VALUE\" > .context/run-env"

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
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "add run script",
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

        let store = WorkspaceStore::open(&db_path).unwrap();
        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        let run = store.run_workspace("berlin").unwrap();
        wait_for_path(&workspace.path.join(".context/run-env"));

        let run_env = fs::read_to_string(workspace.path.join(".context/run-env")).unwrap();
        let lines = run_env.lines().collect::<Vec<_>>();
        assert_eq!(run.workspace_id, workspace.id);
        assert_eq!(run.kind, ProcessKind::Run);
        assert_eq!(run.status, ProcessStatus::Running);
        assert!(run.log_path.exists());
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
    fn run_workspace_captures_logs_and_stop_marks_process_stopped() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".conductor")).unwrap();
        fs::write(
            repo_path.join(".conductor/settings.toml"),
            r#"
[scripts]
run = "printf 'started\n'; while true; do sleep 1; done"
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
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "add run script",
            ])
            .status()
            .unwrap();

        let db_path = temp.path().join("state.db");
        let store = WorkspaceStore::open_with_logs(&db_path, temp.path().join("logs")).unwrap();
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

        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        let run = store.run_workspace("berlin").unwrap();
        wait_for_log(&run.log_path, "started");

        assert!(store
            .read_latest_run_log("berlin")
            .unwrap()
            .contains("started"));
        let stopped = store.stop_workspace("berlin").unwrap();

        assert_eq!(stopped.id, run.id);
        assert_eq!(stopped.status, ProcessStatus::Stopped);
        assert!(stopped.ended_at.is_some());
    }

    #[test]
    fn session_launch_for_shell_uses_workspace_directory_and_environment() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
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

        let store = WorkspaceStore::open(&db_path).unwrap();
        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        let launch = store.session_launch("berlin", SessionKind::Shell).unwrap();

        assert_eq!(launch.cwd, workspace.path);
        assert!(!launch.program.as_os_str().is_empty());
        assert_eq!(launch.args, Vec::<String>::new());
        assert_eq!(launch.env_value("CONDUCTOR_WORKSPACE_NAME"), Some("berlin"));
        assert_eq!(launch.env_value("CONDUCTOR_PORT"), Some("3000"));
        assert_eq!(
            launch.env_value("CONDUCTOR_ROOT_PATH"),
            repo_path.canonicalize().unwrap().to_str()
        );
    }

    #[test]
    fn start_session_persists_session_process_metadata() {
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

        let store = WorkspaceStore::open_with_logs(&db_path, temp.path().join("logs")).unwrap();
        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        let session = store.start_session("berlin", SessionKind::Shell).unwrap();

        assert_eq!(session.workspace_id, workspace.id);
        assert_eq!(session.kind, ProcessKind::Session);
        assert_eq!(session.status, ProcessStatus::Running);
        assert!(session.log_path.exists());
        assert!(!session.command.is_empty());
    }

    #[test]
    fn session_logs_and_stop_use_latest_session_process() {
        let temp = tempfile::tempdir().unwrap();
        let fake_shell = temp.path().join("fake-shell");
        fs::write(
            &fake_shell,
            "#!/bin/sh\nprintf 'session:%s:%s\\n' \"$CONDUCTOR_WORKSPACE_NAME\" \"$CONDUCTOR_PORT\"\nwhile true; do sleep 1; done\n",
        )
        .unwrap();
        Command::new("chmod")
            .arg("+x")
            .arg(&fake_shell)
            .status()
            .unwrap();

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
        let store = WorkspaceStore::open_with_logs(&db_path, temp.path().join("logs")).unwrap();
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        temp_env_var("SHELL", &fake_shell, || {
            let session = store.start_session("berlin", SessionKind::Shell).unwrap();
            wait_for_log(&session.log_path, "session:berlin:3000");

            assert!(store
                .read_latest_session_log("berlin")
                .unwrap()
                .contains("session:berlin:3000"));
            let stopped = store.stop_session("berlin").unwrap();
            assert_eq!(stopped.id, session.id);
            assert_eq!(stopped.status, ProcessStatus::Stopped);
            assert!(stopped.ended_at.is_some());
        });
    }

    #[test]
    fn changed_files_and_unified_diff_read_workspace_git_state() {
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
        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        fs::write(workspace.path.join("README.md"), "demo\nchanged\n").unwrap();
        fs::write(workspace.path.join("notes.txt"), "new\n").unwrap();

        let changed = store.changed_files("berlin").unwrap();
        let diff = store.unified_diff("berlin", None).unwrap();

        assert!(changed.contains(&"README.md".to_owned()));
        assert!(changed.contains(&"notes.txt".to_owned()));
        assert!(diff.contains("diff --git a/README.md b/README.md"));
        assert!(diff.contains("+changed"));
    }

    #[test]
    fn extract_pull_request_url_finds_last_url_line() {
        let output = "Creating pull request for lc/berlin into main\nhttps://github.com/example/demo/pull/42\n";
        assert_eq!(
            extract_pull_request_url(output),
            Some("https://github.com/example/demo/pull/42".to_owned())
        );
    }

    #[test]
    fn parse_pull_request_number_reads_trailing_segment() {
        assert_eq!(
            parse_pull_request_number("https://github.com/example/demo/pull/42"),
            Some(42)
        );
        assert_eq!(parse_pull_request_number("not-a-url"), None);
    }

    #[test]
    fn record_pull_request_persists_number_and_is_visible_in_checks_summary() {
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
        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        let recorded = store
            .record_pull_request(workspace.id, "https://github.com/example/demo/pull/42")
            .unwrap();

        assert_eq!(recorded.number, 42);
        assert_eq!(recorded.state, "open");

        let fetched = store.pull_request("berlin").unwrap().unwrap();
        assert_eq!(fetched, recorded);

        let summary = store.checks_summary("berlin").unwrap();
        assert_eq!(summary.pull_request, Some(recorded));
    }

    #[test]
    fn todos_can_be_added_listed_and_completed() {
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

        let todo = store.add_todo("berlin", "write tests").unwrap();
        assert_eq!(todo.status, "open");

        let todos = store.list_todos("berlin").unwrap();
        assert_eq!(todos, vec![todo.clone()]);

        let done = store.complete_todo(todo.id).unwrap();
        assert_eq!(done.status, "done");

        let summary = store.checks_summary("berlin").unwrap();
        assert_eq!(summary.total_todos, 1);
        assert_eq!(summary.open_todos, 0);
    }

    #[test]
    fn rename_updates_name_path_and_moves_directory() {
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
        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        let old_path = workspace.path.clone();
        let renamed = store.rename("berlin", "oslo").unwrap();

        assert_eq!(renamed.name, "oslo");
        assert!(!old_path.exists());
        assert!(renamed.path.exists());
        assert!(renamed.path.join(".context").is_dir());

        // Should appear under new name in list
        let list = store.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "oslo");
    }

    #[test]
    fn checkpoint_create_makes_git_ref_and_list_returns_it() {
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

        let cp = store
            .checkpoint_create("berlin", "before refactor", None)
            .unwrap();
        assert_eq!(cp.message, "before refactor");
        assert!(cp.git_ref.starts_with("refs/linux-conductor/checkpoints/"));
        assert!(cp.session_id.is_none());

        let list = store.checkpoint_list("berlin").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, cp.id);
    }

    #[test]
    fn checkpoint_restore_resets_workspace_to_checkpoint_commit() {
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
        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        // Take checkpoint at initial state
        let cp = store
            .checkpoint_create("berlin", "clean state", None)
            .unwrap();

        // Make a change and commit it
        fs::write(workspace.path.join("added.txt"), "new content\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args(["add", "added.txt"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args([
                "-c",
                "user.name=Linux Conductor",
                "-c",
                "user.email=linux-conductor@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "added file",
            ])
            .status()
            .unwrap();
        assert!(workspace.path.join("added.txt").exists());

        // Restore to checkpoint
        store.checkpoint_restore("berlin", cp.id).unwrap();

        // The added file should be gone
        assert!(!workspace.path.join("added.txt").exists());
    }

    #[test]
    fn find_conflicting_workspaces_detects_same_changed_file() {
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
        let berlin = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let tokyo = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "tokyo".to_owned(),
                branch: "lc/tokyo".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        // Both workspaces modify the same file
        fs::write(berlin.path.join("README.md"), "berlin changes\n").unwrap();
        fs::write(tokyo.path.join("README.md"), "tokyo changes\n").unwrap();
        // Berlin also modifies a unique file
        fs::write(berlin.path.join("berlin-only.txt"), "unique\n").unwrap();

        let conflicts = store.find_conflicting_workspaces("berlin").unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].0, "tokyo");
        assert!(conflicts[0].1.contains(&"README.md".to_owned()));
        assert!(!conflicts[0].1.contains(&"berlin-only.txt".to_owned()));

        // Tokyo sees the same conflict
        let conflicts_from_tokyo = store.find_conflicting_workspaces("tokyo").unwrap();
        assert_eq!(conflicts_from_tokyo.len(), 1);
        assert_eq!(conflicts_from_tokyo[0].0, "berlin");
    }

    #[test]
    fn slugify_converts_to_kebab_case() {
        assert_eq!(slugify("Add search feature"), "add-search-feature");
        assert_eq!(slugify("Fix: weird  spaces"), "fix-weird-spaces");
        assert_eq!(slugify("feat/cool-thing"), "feat-cool-thing");
        let long = "a".repeat(50);
        assert!(slugify(&long).len() <= 40);
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
                "-c",
                "commit.gpgsign=false",
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

    fn wait_for_path(path: &Path) {
        for _ in 0..50 {
            if path.exists() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("timed out waiting for {}", path.display());
    }

    fn wait_for_log(path: &Path, needle: &str) {
        for _ in 0..50 {
            if fs::read_to_string(path)
                .map(|contents| contents.contains(needle))
                .unwrap_or(false)
            {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!(
            "timed out waiting for log {} to contain {needle}",
            path.display()
        );
    }

    fn temp_env_var(key: &str, value: &Path, run: impl FnOnce()) {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        run();
        if let Some(previous) = previous {
            std::env::set_var(key, previous);
        } else {
            std::env::remove_var(key);
        }
    }
}
