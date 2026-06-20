use crate::settings::load_repository_settings;
use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use rusqlite::{params, Connection};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

const SIGTERM_EXIT_CODE: i32 = 143;
const TERMINAL_SEARCH_CONTEXT_LINES: usize = 4;

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
    Cursor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLaunch {
    pub kind: SessionKind,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, OsString)>,
    pub harness_metadata: Option<String>,
}

impl SessionLaunch {
    pub fn env_value(&self, key: &str) -> Option<&str> {
        self.env
            .iter()
            .find(|(name, _)| name == key)
            .and_then(|(_, value)| value.to_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionHarnessOptions {
    pub plan_mode: bool,
    pub fast_mode: bool,
    pub approval_mode: Option<String>,
    pub reasoning_mode: Option<String>,
    pub effort_mode: Option<String>,
    pub codex_personality: Option<String>,
    pub codex_goals: Option<String>,
    pub codex_skills: Option<String>,
}

impl SessionHarnessOptions {
    pub fn is_empty(&self) -> bool {
        !self.plan_mode
            && !self.fast_mode
            && self.approval_mode.is_none()
            && self.reasoning_mode.is_none()
            && self.effort_mode.is_none()
            && self.codex_personality.is_none()
            && self.codex_goals.is_none()
            && self.codex_skills.is_none()
    }

    pub fn apply_to_env(&self, env: &mut Vec<(String, OsString)>) {
        if self.plan_mode {
            env.push((
                "CONDUCTOR_SESSION_PLAN_MODE".to_owned(),
                OsString::from("true"),
            ));
        }
        if self.fast_mode {
            env.push((
                "CONDUCTOR_SESSION_FAST_MODE".to_owned(),
                OsString::from("true"),
            ));
        }
        if let Some(value) = sanitize_empty_text(self.approval_mode.as_deref()) {
            env.push((
                "CONDUCTOR_SESSION_APPROVAL_MODE".to_owned(),
                OsString::from(value),
            ));
        }
        if let Some(value) = sanitize_empty_text(self.reasoning_mode.as_deref()) {
            env.push((
                "CONDUCTOR_SESSION_REASONING_MODE".to_owned(),
                OsString::from(value),
            ));
        }
        if let Some(value) = sanitize_empty_text(self.effort_mode.as_deref()) {
            env.push((
                "CONDUCTOR_SESSION_EFFORT_MODE".to_owned(),
                OsString::from(value),
            ));
        }
        if let Some(value) = sanitize_empty_text(self.codex_personality.as_deref()) {
            env.push((
                "CONDUCTOR_SESSION_CODEX_PERSONALITY".to_owned(),
                OsString::from(value),
            ));
        }
        if let Some(value) = sanitize_empty_text(self.codex_goals.as_deref()) {
            env.push((
                "CONDUCTOR_SESSION_CODEX_GOALS".to_owned(),
                OsString::from(value),
            ));
        }
        if let Some(value) = sanitize_empty_text(self.codex_skills.as_deref()) {
            env.push((
                "CONDUCTOR_SESSION_CODEX_SKILLS".to_owned(),
                OsString::from(value),
            ));
        }
    }

    pub fn metadata(&self) -> Option<String> {
        let mut entries = Vec::new();
        if self.plan_mode {
            entries.push("plan=true".to_owned());
        }
        if self.fast_mode {
            entries.push("fast=true".to_owned());
        }
        if let Some(value) = sanitize_empty_text(self.approval_mode.as_deref()) {
            entries.push(format!("approvals={value}"));
        }
        if let Some(value) = sanitize_empty_text(self.reasoning_mode.as_deref()) {
            entries.push(format!("reasoning={value}"));
        }
        if let Some(value) = sanitize_empty_text(self.effort_mode.as_deref()) {
            entries.push(format!("effort={value}"));
        }
        if let Some(value) = sanitize_empty_text(self.codex_personality.as_deref()) {
            entries.push(format!("personality={value}"));
        }
        if let Some(value) = sanitize_empty_text(self.codex_goals.as_deref()) {
            entries.push(format!("goals={}", sanitize_metadata_value(&value)));
        }
        if let Some(value) = sanitize_empty_text(self.codex_skills.as_deref()) {
            entries.push(format!("skills={}", sanitize_metadata_value(&value)));
        }
        if entries.is_empty() {
            None
        } else {
            Some(entries.join(";"))
        }
    }
}

fn sanitize_metadata_value(value: &str) -> String {
    value.replace(['\n', '\r', ';'], " ")
}

fn sanitize_empty_text(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessKind {
    Setup,
    Run,
    Session,
    Terminal,
}

impl ProcessKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Run => "run",
            Self::Session => "session",
            Self::Terminal => "terminal",
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
    pub exit_code: Option<i32>,
    pub ended_at: Option<String>,
    pub session_harness_metadata: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalCommandResult {
    pub command: String,
    pub cwd: PathBuf,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub started_at: String,
    pub ended_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalLogMatch {
    pub process_id: i64,
    pub command: String,
    pub log_path: PathBuf,
    pub line_number: usize,
    pub line: String,
    pub context_before: Vec<String>,
    pub context_after: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalSessionSummary {
    pub process: ProcessRecord,
    pub line_count: usize,
    pub byte_count: usize,
    pub preview: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffFileSummary {
    pub path: String,
    pub additions: Option<usize>,
    pub deletions: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotlightSession {
    pub id: i64,
    pub repository_id: i64,
    pub workspace_id: i64,
    pub workspace_name: String,
    pub patch_path: PathBuf,
    pub status: String,
    pub started_at: String,
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotlightWatchTarget {
    pub session_id: i64,
    pub workspace_name: String,
    pub workspace_path: PathBuf,
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
pub struct PullRequestCheckRun {
    pub name: String,
    pub status: String,
    pub detail: Option<String>,
}

impl PullRequestCheckRun {
    pub fn is_failure(&self) -> bool {
        matches!(
            self.status.to_ascii_lowercase().as_str(),
            "fail" | "failed" | "failure" | "error" | "cancelled" | "timed_out"
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergePullRequestResult {
    pub merge_output: String,
    pub archived_workspace: Option<Workspace>,
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
    pub repository_name: String,
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
    db_path: PathBuf,
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

        let db_path = path.as_ref().to_path_buf();
        let conn = Connection::open(&db_path)
            .with_context(|| format!("open database {}", db_path.display()))?;
        let store = Self {
            conn,
            db_path,
            logs_dir: logs_dir.as_ref().to_path_buf(),
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn create(&self, input: CreateWorkspace) -> Result<Workspace> {
        validate_workspace_name(&input.name)?;
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
            let repository_name: String = self
                .conn
                .query_row(
                    "SELECT name FROM repositories WHERE id = ?1",
                    [workspace.repository_id],
                    |row| row.get(0),
                )
                .unwrap_or_default();
            lines.push(WorkspaceStatusLine {
                workspace,
                repository_name,
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
        validate_workspace_name(new_name)?;
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
        self.conn.execute(
            "UPDATE spotlight_sessions SET workspace_name = ?1 WHERE workspace_id = ?2",
            params![new_name, workspace.id],
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

        for kind in [
            ProcessKind::Setup,
            ProcessKind::Run,
            ProcessKind::Session,
            ProcessKind::Terminal,
        ] {
            if let Some(process) = self.find_latest_running_process(workspace.id, kind)? {
                stop_process(process.pid)?;
                let now = timestamp();
                self.conn.execute(
                    "UPDATE processes SET status = ?1, ended_at = ?2, exit_code = ?3 WHERE id = ?4",
                    params![
                        ProcessStatus::Stopped.as_str(),
                        now,
                        SIGTERM_EXIT_CODE,
                        process.id
                    ],
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

        self.start_process(
            ProcessKind::Run,
            run,
            &settings,
            &repository,
            &workspace,
            None,
        )
    }

    pub fn setup_workspace(&self, name: &str) -> Result<ProcessRecord> {
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let settings = load_repository_settings(&repository.root_path)?;
        let Some(setup) = &settings.scripts.setup else {
            anyhow::bail!("workspace {name} has no scripts.setup configured");
        };

        self.start_process(
            ProcessKind::Setup,
            setup,
            &settings,
            &repository,
            &workspace,
            None,
        )
    }

    pub fn stop_workspace(&self, name: &str) -> Result<ProcessRecord> {
        let workspace = self.get_by_name(name)?;
        let process = self.latest_running_process(workspace.id, ProcessKind::Run)?;
        stop_process(process.pid)?;
        let now = timestamp();
        self.conn.execute(
            "UPDATE processes SET status = ?1, ended_at = ?2, exit_code = ?3 WHERE id = ?4",
            params![
                ProcessStatus::Stopped.as_str(),
                now,
                SIGTERM_EXIT_CODE,
                process.id
            ],
        )?;
        self.get_process(process.id)
    }

    pub fn read_latest_run_log(&self, name: &str) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        let process = self.latest_process(workspace.id, ProcessKind::Run)?;
        fs::read_to_string(&process.log_path)
            .with_context(|| format!("read log {}", process.log_path.display()))
    }

    pub fn read_latest_setup_log(&self, name: &str) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        let process = self.latest_process(workspace.id, ProcessKind::Setup)?;
        fs::read_to_string(&process.log_path)
            .with_context(|| format!("read log {}", process.log_path.display()))
    }

    pub fn read_latest_terminal_log(&self, name: &str) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        let process = self.latest_process(workspace.id, ProcessKind::Terminal)?;
        fs::read_to_string(&process.log_path)
            .with_context(|| format!("read log {}", process.log_path.display()))
    }

    pub fn read_terminal_log(&self, name: &str, process_id: i64) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        let process = self.get_process(process_id)?;
        anyhow::ensure!(
            process.workspace_id == workspace.id && process.kind == ProcessKind::Terminal,
            "terminal process {process_id} does not belong to workspace {name}"
        );
        fs::read_to_string(&process.log_path)
            .with_context(|| format!("read log {}", process.log_path.display()))
    }

    pub fn stop_session(&self, name: &str) -> Result<ProcessRecord> {
        let workspace = self.get_by_name(name)?;
        let process = self.latest_running_process(workspace.id, ProcessKind::Session)?;
        self.stop_session_process(name, process.id)
    }

    pub fn stop_session_process(&self, name: &str, process_id: i64) -> Result<ProcessRecord> {
        let process = self
            .list_sessions(name)?
            .into_iter()
            .find(|session| session.id == process_id)
            .with_context(|| {
                format!("session process {process_id} not found for workspace {name}")
            })?;
        if process.status != ProcessStatus::Running {
            return Ok(process);
        }
        anyhow::ensure!(
            process.pid > 0,
            "session process {process_id} has invalid pid"
        );
        stop_process(process.pid)?;
        self.mark_session_process_stopped(process.id, Some(SIGTERM_EXIT_CODE))
    }

    pub fn mark_session_process_stopped(
        &self,
        process_id: i64,
        exit_code: Option<i32>,
    ) -> Result<ProcessRecord> {
        let now = timestamp();
        self.conn.execute(
            "UPDATE processes SET status = ?1, ended_at = ?2, exit_code = ?3 WHERE id = ?4",
            params![ProcessStatus::Stopped.as_str(), now, exit_code, process_id,],
        )?;
        self.get_process(process_id)
    }

    pub fn mark_session_process_exited(
        &self,
        process_id: i64,
        exit_code: Option<i32>,
    ) -> Result<ProcessRecord> {
        let now = timestamp();
        self.conn.execute(
            "UPDATE processes SET status = ?1, ended_at = ?2, exit_code = ?3 WHERE id = ?4",
            params![ProcessStatus::Exited.as_str(), now, exit_code, process_id,],
        )?;
        self.get_process(process_id)
    }

    pub fn read_latest_session_log(&self, name: &str) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        let process = self.latest_process(workspace.id, ProcessKind::Session)?;
        fs::read_to_string(&process.log_path)
            .with_context(|| format!("read log {}", process.log_path.display()))
    }

    pub fn append_session_process_output(&self, process_id: i64, output: &str) -> Result<()> {
        if output.is_empty() {
            return Ok(());
        }
        let process = self.get_process(process_id)?;
        anyhow::ensure!(
            process.kind == ProcessKind::Session,
            "process {process_id} is not a session process"
        );
        if let Some(parent) = process.log_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create log directory {}", parent.display()))?;
        }
        use std::io::Write;
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&process.log_path)
            .with_context(|| format!("open log {}", process.log_path.display()))?
            .write_all(output.as_bytes())
            .with_context(|| format!("write log {}", process.log_path.display()))
    }

    pub fn reconcile_session_processes(&self) -> Result<Vec<ProcessRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, workspace_id, kind, command, pid, log_path, status, started_at, exit_code, ended_at, session_harness_metadata
             FROM processes
             WHERE kind = ?1 AND status = 'running'
             ORDER BY id",
        )?;
        let running = stmt
            .query_map([ProcessKind::Session.as_str()], row_to_process)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);

        let mut reconciled = Vec::new();
        for process in running {
            if process_alive(process.pid) {
                continue;
            }
            let now = timestamp();
            self.conn.execute(
                "UPDATE processes
                 SET status = ?1, ended_at = ?2, exit_code = NULL
                 WHERE id = ?3 AND status = 'running'",
                params![ProcessStatus::Exited.as_str(), now, process.id],
            )?;
            reconciled.push(self.get_process(process.id)?);
        }
        Ok(reconciled)
    }

    pub fn record_terminal_process(
        &self,
        name: &str,
        command: &str,
        pid: u32,
    ) -> Result<ProcessRecord> {
        let command = command.trim();
        anyhow::ensure!(!command.is_empty(), "terminal command is required");
        let workspace = self.get_by_name(name)?;
        self.record_process(
            ProcessKind::Terminal,
            &workspace,
            command,
            pid,
            "terminal",
            None,
        )
    }

    fn record_process(
        &self,
        kind: ProcessKind,
        workspace: &Workspace,
        command: &str,
        pid: u32,
        file_prefix: &str,
        session_harness_metadata: Option<&str>,
    ) -> Result<ProcessRecord> {
        let command = command.trim();
        anyhow::ensure!(!command.is_empty(), "process command is required");
        let now = timestamp();
        let log_path = self.logs_dir.join(&workspace.name).join(format!(
            "{file_prefix}-{}-{}.log",
            timestamp_nanos(),
            pid
        ));
        if let Some(parent) = log_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create log directory {}", parent.display()))?;
        }
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .with_context(|| format!("open log {}", log_path.display()))?;

        self.conn.execute(
            "INSERT INTO processes (
                workspace_id, kind, command, pid, log_path, status, started_at, exit_code, ended_at, session_harness_metadata
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, ?8)",
            params![
                workspace.id,
                kind.as_str(),
                command,
                i64::from(pid),
                log_path.to_string_lossy().to_string(),
                ProcessStatus::Running.as_str(),
                now,
                session_harness_metadata,
            ],
        )?;
        self.get_process(self.conn.last_insert_rowid())
    }

    pub fn mark_terminal_process_stopped(
        &self,
        process_id: i64,
        exit_code: Option<i32>,
    ) -> Result<ProcessRecord> {
        let now = timestamp();
        self.conn.execute(
            "UPDATE processes SET status = ?1, ended_at = ?2, exit_code = ?3 WHERE id = ?4",
            params![ProcessStatus::Stopped.as_str(), now, exit_code, process_id,],
        )?;
        self.get_process(process_id)
    }

    pub fn mark_terminal_process_exited(
        &self,
        process_id: i64,
        exit_code: Option<i32>,
    ) -> Result<ProcessRecord> {
        let now = timestamp();
        self.conn.execute(
            "UPDATE processes SET status = ?1, ended_at = ?2, exit_code = ?3 WHERE id = ?4",
            params![ProcessStatus::Exited.as_str(), now, exit_code, process_id,],
        )?;
        self.get_process(process_id)
    }

    pub fn stop_terminal_process(&self, name: &str, process_id: i64) -> Result<ProcessRecord> {
        let process = self
            .list_terminals(name)?
            .into_iter()
            .find(|terminal| terminal.id == process_id)
            .with_context(|| {
                format!("terminal session {process_id} not found for workspace {name}")
            })?;
        if process.status != ProcessStatus::Running {
            return Ok(process);
        }
        anyhow::ensure!(
            process.pid > 0,
            "terminal process {process_id} has invalid pid"
        );
        stop_process(process.pid as u32)?;
        self.mark_terminal_process_stopped(process.id, Some(SIGTERM_EXIT_CODE))
    }

    pub fn copy_conflict_file_from_workspace(
        &self,
        destination_workspace: &str,
        source_workspace: &str,
        relative_path: &str,
    ) -> Result<()> {
        let destination = self.get_by_name(destination_workspace)?;
        let source = self.get_by_name(source_workspace)?;
        anyhow::ensure!(
            destination.repository_id == source.repository_id,
            "workspace {source_workspace} is not in the same repository as {destination_workspace}",
        );

        let path = Path::new(relative_path);
        anyhow::ensure!(
            path.is_relative(),
            "conflict file path must be relative: {relative_path}",
        );
        for component in path.components() {
            anyhow::ensure!(
                !matches!(component, Component::ParentDir | Component::CurDir),
                "conflict file path may not use path traversal: {relative_path}",
            );
        }

        let source_path = source.path.join(path);
        let destination_path = destination.path.join(path);
        anyhow::ensure!(
            source_path.exists(),
            "source workspace {} does not contain {}",
            source.name,
            relative_path,
        );
        anyhow::ensure!(
            source_path.is_file(),
            "{} {} is not a regular file",
            source_workspace,
            relative_path,
        );

        if let Some(parent) = destination_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create directory {}", parent.display()))?;
        }
        fs::copy(&source_path, &destination_path).with_context(|| {
            format!(
                "copy {} to {}",
                source_path.display(),
                destination_path.display()
            )
        })?;
        Ok(())
    }

    pub fn append_terminal_process_output(&self, process_id: i64, output: &str) -> Result<()> {
        if output.is_empty() {
            return Ok(());
        }
        let process = self.get_process(process_id)?;
        anyhow::ensure!(
            process.kind == ProcessKind::Terminal,
            "process {process_id} is not a terminal process"
        );
        if let Some(parent) = process.log_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create log directory {}", parent.display()))?;
        }
        use std::io::Write;
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&process.log_path)
            .with_context(|| format!("open log {}", process.log_path.display()))?
            .write_all(output.as_bytes())
            .with_context(|| format!("write log {}", process.log_path.display()))
    }

    pub fn search_terminal_logs(&self, name: &str, query: &str) -> Result<Vec<TerminalLogMatch>> {
        let query = query.trim();
        anyhow::ensure!(!query.is_empty(), "terminal log search query is required");
        let needle = query.to_lowercase();
        let mut matches = Vec::new();
        for (process_index, process) in self.list_terminals(name)?.into_iter().enumerate() {
            let contents = fs::read_to_string(&process.log_path)
                .with_context(|| format!("read log {}", process.log_path.display()))?;
            let lines = contents.lines().collect::<Vec<_>>();
            for (index, line) in lines.iter().enumerate() {
                if line.to_lowercase().contains(&needle) {
                    let start = index.saturating_sub(TERMINAL_SEARCH_CONTEXT_LINES);
                    let end = (index + TERMINAL_SEARCH_CONTEXT_LINES + 1).min(lines.len());
                    let mut context_before = Vec::new();
                    let mut context_after = Vec::new();

                    for line in &lines[start..index] {
                        if process_index == 0 {
                            break;
                        }
                        context_before.push((*line).to_owned());
                    }
                    for line in &lines[index + 1..end] {
                        context_after.push((*line).to_owned());
                    }

                    matches.push(TerminalLogMatch {
                        process_id: process.id,
                        command: process.command.clone(),
                        log_path: process.log_path.clone(),
                        line_number: index + 1,
                        line: (*line).to_owned(),
                        context_before,
                        context_after,
                    });
                }
            }
        }
        Ok(matches)
    }

    pub fn list_terminal_summaries(&self, name: &str) -> Result<Vec<TerminalSessionSummary>> {
        self.list_terminals(name)?
            .into_iter()
            .map(|process| {
                let contents = fs::read_to_string(&process.log_path)
                    .with_context(|| format!("read log {}", process.log_path.display()))?;
                Ok(TerminalSessionSummary {
                    process,
                    line_count: contents.lines().count(),
                    byte_count: contents.len(),
                    preview: terminal_log_preview(&contents),
                })
            })
            .collect()
    }

    pub fn reconcile_terminal_processes(&self) -> Result<Vec<ProcessRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, workspace_id, kind, command, pid, log_path, status, started_at, exit_code, ended_at, session_harness_metadata
             FROM processes
             WHERE kind = ?1 AND status = 'running'
             ORDER BY id",
        )?;
        let running = stmt
            .query_map([ProcessKind::Terminal.as_str()], row_to_process)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);

        let mut reconciled = Vec::new();
        for process in running {
            if process_alive(process.pid) {
                continue;
            }
            let now = timestamp();
            self.conn.execute(
                "UPDATE processes
                 SET status = ?1, ended_at = ?2, exit_code = NULL
                 WHERE id = ?3 AND status = 'running'",
                params![ProcessStatus::Exited.as_str(), now, process.id],
            )?;
            reconciled.push(self.get_process(process.id)?);
        }
        Ok(reconciled)
    }

    pub fn terminal_command(&self, name: &str, command: &str) -> Result<TerminalCommandResult> {
        let command = command.trim();
        anyhow::ensure!(!command.is_empty(), "terminal command is required");
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let settings = load_repository_settings(&repository.root_path)?;
        let started_at = timestamp();
        let output = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&workspace.path)
            .envs(conductor_environment(&settings, &repository, &workspace))
            .stdin(Stdio::null())
            .output()
            .with_context(|| format!("run terminal command in {}", workspace.path.display()))?;
        let ended_at = timestamp();

        Ok(TerminalCommandResult {
            command: command.to_owned(),
            cwd: workspace.path,
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            started_at,
            ended_at,
        })
    }

    pub fn spotlight_start(&self, name: &str) -> Result<SpotlightSession> {
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let settings = load_repository_settings(&repository.root_path)?;
        anyhow::ensure!(
            settings.spotlight_testing.unwrap_or(false),
            "spotlight_testing must be enabled for this repository"
        );
        if let Some(active) = self.active_spotlight_for_repository(repository.id)? {
            if active.workspace_id == workspace.id {
                if let Some(updated) = self.spotlight_sync_if_changed(&workspace.name)? {
                    return Ok(updated);
                }
                return Ok(active);
            }
            self.stop_active_spotlight(&repository, &active)?;
        }
        ensure_clean_git_tree(&repository.root_path, "repository root")?;

        let patch = workspace_tracked_patch(&workspace)?;
        anyhow::ensure!(
            !patch.trim().is_empty(),
            "workspace {name} has no tracked changes to spotlight"
        );

        self.spotlight_checkpoint(&workspace, &patch)?;
        apply_git_patch(&repository.root_path, &patch)?;
        let now = timestamp_nanos();
        let patch_path = self
            .logs_dir
            .join(&workspace.name)
            .join(format!("spotlight-{now}.patch"));
        if let Some(parent) = patch_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create spotlight directory {}", parent.display()))?;
        }
        fs::write(&patch_path, patch).with_context(|| format!("write {}", patch_path.display()))?;

        self.conn.execute(
            "INSERT INTO spotlight_sessions (
                repository_id, workspace_id, workspace_name, patch_path, status, started_at, ended_at
            ) VALUES (?1, ?2, ?3, ?4, 'active', ?5, NULL)",
            params![
                repository.id,
                workspace.id,
                workspace.name,
                patch_path.to_string_lossy().to_string(),
                now,
            ],
        )?;
        self.get_spotlight_session(self.conn.last_insert_rowid())
    }

    pub fn spotlight_stop(&self, name: &str) -> Result<SpotlightSession> {
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let active = self
            .active_spotlight_for_repository(repository.id)?
            .with_context(|| format!("no active spotlight session for workspace {name}"))?;
        anyhow::ensure!(
            active.workspace_id == workspace.id,
            "active spotlight is for workspace {}, not {name}",
            active.workspace_name
        );

        let patch = fs::read_to_string(&active.patch_path)
            .with_context(|| format!("read {}", active.patch_path.display()))?;
        ensure_root_matches_spotlight_patch(&repository.root_path, &patch)?;
        reverse_git_patch(&repository.root_path, &patch)?;
        let now = timestamp();
        self.conn.execute(
            "UPDATE spotlight_sessions SET status = 'stopped', ended_at = ?1 WHERE id = ?2",
            params![now, active.id],
        )?;
        self.get_spotlight_session(active.id)
    }

    pub fn spotlight_repair_root(&self, name: &str) -> Result<SpotlightSession> {
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let active = self
            .active_spotlight_for_repository(repository.id)?
            .with_context(|| format!("no active spotlight session for workspace {name}"))?;
        anyhow::ensure!(
            active.workspace_id == workspace.id,
            "active spotlight is for workspace {}, not {name}",
            active.workspace_name
        );

        let patch = fs::read_to_string(&active.patch_path)
            .with_context(|| format!("read {}", active.patch_path.display()))?;
        git_dynamic(&repository.root_path, &["reset", "--hard", "HEAD"])?;
        git_dynamic(&repository.root_path, &["clean", "-fd"])?;
        apply_git_patch(&repository.root_path, &patch)?;
        self.get_spotlight_session(active.id)
    }

    pub fn spotlight_sync(&self, name: &str) -> Result<SpotlightSession> {
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let active = self
            .active_spotlight_for_repository(repository.id)?
            .with_context(|| format!("no active spotlight session for workspace {name}"))?;
        anyhow::ensure!(
            active.workspace_id == workspace.id,
            "active spotlight is for workspace {}, not {name}",
            active.workspace_name
        );

        let old_patch = fs::read_to_string(&active.patch_path)
            .with_context(|| format!("read {}", active.patch_path.display()))?;
        let patch = workspace_tracked_patch(&workspace)?;
        if patch.trim() == old_patch.trim() {
            return Ok(active);
        }

        ensure_root_matches_spotlight_patch(&repository.root_path, &old_patch)?;
        if !old_patch.trim().is_empty() {
            reverse_git_patch(&repository.root_path, &old_patch)?;
            ensure_clean_git_tree(&repository.root_path, "repository root")?;
        }
        if !patch.trim().is_empty() {
            apply_git_patch(&repository.root_path, &patch)?;
        }

        self.spotlight_checkpoint(&workspace, &patch)?;
        let now = timestamp_nanos();
        let patch_path = self
            .logs_dir
            .join(&workspace.name)
            .join(format!("spotlight-{now}.patch"));
        if let Some(parent) = patch_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create spotlight directory {}", parent.display()))?;
        }
        let expected_patch = if patch.trim().is_empty() {
            String::new()
        } else {
            root_tracked_patch(&repository.root_path)?
        };
        fs::write(&patch_path, &expected_patch)
            .with_context(|| format!("write {}", patch_path.display()))?;
        self.conn.execute(
            "UPDATE spotlight_sessions SET workspace_name = ?1, patch_path = ?2 WHERE id = ?3",
            params![
                workspace.name,
                patch_path.to_string_lossy().to_string(),
                active.id
            ],
        )?;
        self.get_spotlight_session(active.id)
    }

    pub fn spotlight_sync_if_changed(&self, name: &str) -> Result<Option<SpotlightSession>> {
        let workspace = self.get_by_name(name)?;
        let active = self
            .active_spotlight_for_repository(workspace.repository_id)?
            .filter(|session| session.workspace_id == workspace.id);
        let Some(active) = active else {
            return Ok(None);
        };
        self.spotlight_sync_if_changed_session(active.id)
    }

    fn spotlight_sync_if_changed_session(
        &self,
        session_id: i64,
    ) -> Result<Option<SpotlightSession>> {
        let active = self.get_spotlight_session(session_id)?;
        if active.status != "active" {
            return Ok(None);
        }
        let workspace = self.get_by_id(active.workspace_id)?;
        let active = self
            .active_spotlight_for_repository(workspace.repository_id)?
            .filter(|session| session.id == session_id);
        let Some(active) = active else {
            return Ok(None);
        };
        let active_patch = fs::read_to_string(&active.patch_path)
            .with_context(|| format!("read {}", active.patch_path.display()))?;
        let current_patch = workspace_tracked_patch(&workspace)?;
        if active_patch.trim() == current_patch.trim() {
            return Ok(None);
        }
        self.spotlight_sync(&workspace.name).map(Some)
    }

    pub fn spotlight_sync_active_sessions(&self) -> Result<Vec<SpotlightSession>> {
        let mut synced = Vec::new();
        for session in self.active_spotlight_sessions()? {
            if let Some(updated) = self.spotlight_sync_if_changed_session(session.id)? {
                synced.push(updated);
            }
        }
        Ok(synced)
    }

    pub fn spotlight_watch_targets(&self) -> Result<Vec<SpotlightWatchTarget>> {
        let mut stmt = self.conn.prepare(
            "SELECT ss.id, w.name, w.path
             FROM spotlight_sessions ss
             JOIN workspaces w ON w.id = ss.workspace_id
             WHERE ss.status = 'active'
             ORDER BY ss.id",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(SpotlightWatchTarget {
                    session_id: row.get(0)?,
                    workspace_name: row.get(1)?,
                    workspace_path: PathBuf::from(row.get::<_, String>(2)?),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("load spotlight watch targets")?;
        Ok(rows)
    }

    pub fn spotlight_status(&self, name: &str) -> Result<Option<SpotlightSession>> {
        let workspace = self.get_by_name(name)?;
        let active = self.active_spotlight_for_repository(workspace.repository_id)?;
        Ok(active.filter(|session| session.workspace_id == workspace.id))
    }

    pub fn spotlight_root_conflict_paths(&self, name: &str) -> Result<Vec<String>> {
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let Some(active) = self
            .active_spotlight_for_repository(repository.id)?
            .filter(|session| session.workspace_id == workspace.id)
        else {
            return Ok(Vec::new());
        };
        let expected_patch = fs::read_to_string(&active.patch_path)
            .with_context(|| format!("read {}", active.patch_path.display()))?;
        let current_patch = root_tracked_patch(&repository.root_path)?;
        if current_patch.trim() == expected_patch.trim() {
            return Ok(Vec::new());
        }
        Ok(spotlight_conflict_paths(&current_patch, &expected_patch)
            .into_iter()
            .collect())
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

    pub fn git_status_short(&self, name: &str) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        git_output_dynamic(&workspace.path, &["status", "--short"])
    }

    pub fn git_log_oneline(&self, name: &str, n: usize) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        git_output_dynamic(
            &workspace.path,
            &[
                "log",
                "--oneline",
                "--decorate",
                &format!("-{n}"),
                workspace.branch.as_str(),
            ],
        )
    }

    pub fn unified_diff(&self, name: &str, path: Option<&Path>) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        if let Some(path) = path {
            let path_value = path.to_string_lossy().to_string();
            return git_output_dynamic(&workspace.path, &["diff", "--", path_value.as_str()]);
        }
        git_output(&workspace.path, ["diff", "--"])
    }

    pub fn revert_workspace_file(&self, name: &str, relative_path: &str) -> Result<()> {
        let workspace = self.get_by_name(name)?;
        let validated = validate_workspace_relative_path(relative_path)?;
        ensure_tracked_in_head(&workspace.path, relative_path)?;
        let path_value = validated.to_string_lossy().to_string();
        git_dynamic(
            &workspace.path,
            &[
                "restore",
                "--source=HEAD",
                "--staged",
                "--worktree",
                "--",
                path_value.as_str(),
            ],
        )
    }

    pub fn diff_file_summaries(&self, name: &str) -> Result<Vec<DiffFileSummary>> {
        let workspace = self.get_by_name(name)?;
        let output = git_output(&workspace.path, ["diff", "--numstat", "--"])?;
        let mut summaries = parse_diff_numstat(&output);
        let known_paths = summaries
            .iter()
            .map(|summary| summary.path.clone())
            .collect::<BTreeSet<_>>();
        let status = git_output(
            &workspace.path,
            ["status", "--porcelain", "--untracked-files=all"],
        )?;
        for path in parse_untracked_status_paths(&status) {
            if known_paths.contains(&path) || is_conductor_context_path(&path) {
                continue;
            }
            let counts = untracked_file_counts(&workspace.path.join(&path))?;
            summaries.push(DiffFileSummary {
                path,
                additions: Some(counts.0),
                deletions: Some(0),
            });
        }
        summaries.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(summaries)
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

        let workspace = self.create(CreateWorkspace {
            repository_name: repository_name.to_owned(),
            name: workspace_name,
            branch,
            base_ref: None,
        })?;
        write_context_brief(
            &workspace.path,
            &format!(
                "# Brief\n\nGitHub issue #{issue_number}: {title}\n\n## Source\n\nGitHub Issue\n"
            ),
        )?;
        Ok(workspace)
    }

    pub fn create_from_pull_request(
        &self,
        repository_name: &str,
        pr_number: u64,
        workspace_name: Option<&str>,
        branch_name: Option<&str>,
    ) -> Result<Workspace> {
        let repository = self.load_repository(repository_name)?;
        let pr_number_text = pr_number.to_string();
        let output = command_output(
            &repository.root_path,
            "gh",
            &[
                "pr",
                "view",
                &pr_number_text,
                "--json",
                "title,url,state,number",
            ],
        )?;
        let title = extract_json_string_field(&output, "title")
            .unwrap_or_else(|| format!("pull-request-{pr_number}"));
        let url = extract_json_string_field(&output, "url");
        let state =
            extract_json_string_field(&output, "state").unwrap_or_else(|| "open".to_owned());
        let slug = slugify(&title);
        let remote_ref = format!("refs/linux-conductor/pull-requests/{pr_number}");
        let fetch_refspec = format!("pull/{pr_number}/head:{remote_ref}");
        git_dynamic(
            &repository.root_path,
            &[
                "fetch",
                repository.remote_name.as_str(),
                fetch_refspec.as_str(),
            ],
        )?;

        let workspace = self.create(CreateWorkspace {
            repository_name: repository_name.to_owned(),
            name: workspace_name
                .map(str::to_owned)
                .unwrap_or_else(|| format!("pr-{pr_number}")),
            branch: branch_name
                .map(str::to_owned)
                .unwrap_or_else(|| format!("lc/pr-{pr_number}-{slug}")),
            base_ref: Some(remote_ref),
        })?;

        if let Some(url) = url {
            self.record_pull_request(workspace.id, &url)?;
            if state != "open" {
                let now = timestamp();
                self.conn.execute(
                    "UPDATE pull_requests SET state = ?1, updated_at = ?2 WHERE workspace_id = ?3",
                    params![state, now, workspace.id],
                )?;
            }
        }

        write_context_brief(
            &workspace.path,
            &format!(
                "# Brief\n\nGitHub PR #{pr_number}: {title}\n\n{}\n\n## Source\n\nGitHub Pull Request\n",
                self.pull_request_by_workspace_id(workspace.id)?
                    .map(|pr| pr.url)
                    .unwrap_or_default()
            ),
        )?;

        Ok(workspace)
    }

    pub fn create_from_prompt(
        &self,
        repository_name: &str,
        prompt: &str,
        workspace_name: Option<&str>,
        branch_name: Option<&str>,
        base_ref: Option<&str>,
    ) -> Result<Workspace> {
        let prompt = prompt.trim();
        anyhow::ensure!(!prompt.is_empty(), "prompt is required");
        let slug = slugify(prompt);
        let workspace = self.create(CreateWorkspace {
            repository_name: repository_name.to_owned(),
            name: workspace_name
                .map(str::to_owned)
                .unwrap_or_else(|| slug.clone()),
            branch: branch_name
                .map(str::to_owned)
                .unwrap_or_else(|| format!("lc/{slug}")),
            base_ref: base_ref.map(str::to_owned),
        })?;
        write_context_brief(
            &workspace.path,
            &format!("# Brief\n\n{prompt}\n\n## Source\n\nPrompt\n"),
        )?;
        Ok(workspace)
    }

    pub fn create_from_linear_issue(
        &self,
        repository_name: &str,
        issue_id: &str,
        workspace_name: Option<&str>,
        branch_name: Option<&str>,
        base_ref: Option<&str>,
    ) -> Result<Workspace> {
        let issue_id = issue_id.trim();
        anyhow::ensure!(!issue_id.is_empty(), "Linear issue id is required");
        let issue = fetch_linear_issue(issue_id)?;
        let slug = slugify(&issue.title);
        let workspace = self.create(CreateWorkspace {
            repository_name: repository_name.to_owned(),
            name: workspace_name
                .map(str::to_owned)
                .unwrap_or_else(|| issue.identifier.to_ascii_lowercase()),
            branch: branch_name
                .map(str::to_owned)
                .or(issue.branch_name)
                .unwrap_or_else(|| format!("lc/{}-{slug}", issue.identifier.to_ascii_lowercase())),
            base_ref: base_ref.map(str::to_owned),
        })?;
        write_context_brief(
            &workspace.path,
            &format!(
                "# Brief\n\nLinear {}: {}\n\n{}\n",
                issue.identifier,
                issue.title,
                issue.url.unwrap_or_default()
            ),
        )?;
        Ok(workspace)
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

    pub fn pull_request_check_runs(&self, name: &str) -> Result<Vec<PullRequestCheckRun>> {
        self.pull_request_checks(name)
            .map(|output| parse_pull_request_check_runs(&output))
    }

    pub fn pull_request_checks_agent_prompt(&self, name: &str) -> Result<String> {
        let checks = self.pull_request_check_runs(name)?;
        Ok(format_pull_request_checks_agent_prompt(name, &checks))
    }

    pub fn pull_request_review_state(&self, name: &str) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        command_output(&workspace.path, "gh", &["pr", "view", "--comments"])
    }

    pub fn pull_request_review_agent_prompt(&self, name: &str) -> Result<String> {
        let review_state = self.pull_request_review_state(name)?;
        Ok(format_pull_request_review_agent_prompt(name, &review_state))
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

    pub fn review_comments_agent_prompt(&self, name: &str) -> Result<String> {
        let comments = self
            .list_review_comments(name)?
            .into_iter()
            .filter(|comment| comment.status == "open")
            .collect::<Vec<_>>();
        Ok(format_review_comments_agent_prompt(name, &comments))
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
        let message = message.trim();
        anyhow::ensure!(!message.is_empty(), "checkpoint message is required");
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

    fn stop_active_spotlight(
        &self,
        repository: &RepositoryRecord,
        active: &SpotlightSession,
    ) -> Result<SpotlightSession> {
        let patch = fs::read_to_string(&active.patch_path)
            .with_context(|| format!("read {}", active.patch_path.display()))?;
        ensure_root_matches_spotlight_patch(&repository.root_path, &patch)?;
        reverse_git_patch(&repository.root_path, &patch)?;
        let now = timestamp();
        self.conn.execute(
            "UPDATE spotlight_sessions SET status = 'stopped', ended_at = ?1 WHERE id = ?2",
            params![now, active.id],
        )?;
        self.get_spotlight_session(active.id)
    }

    fn spotlight_checkpoint(&self, workspace: &Workspace, patch: &str) -> Result<Checkpoint> {
        let now = timestamp_nanos();
        let git_ref = format!(
            "refs/linux-conductor/checkpoints/{}/spotlight-{now}",
            workspace.id
        );
        let message = "Spotlight checkpoint";
        let index_path = self
            .logs_dir
            .join(&workspace.name)
            .join(format!("spotlight-index-{now}"));
        if let Some(parent) = index_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("create spotlight index directory {}", parent.display())
            })?;
        }

        git_with_index(
            &workspace.path,
            &index_path,
            &["read-tree", &workspace.base_ref],
        )?;
        git_patch_with_index(
            &workspace.path,
            &index_path,
            &["apply", "--cached", "--binary", "-"],
            patch,
        )?;
        let tree = git_with_index_output(&workspace.path, &index_path, &["write-tree"])?;
        let head = git_output_dynamic(&workspace.path, &["rev-parse", "HEAD"])?;
        let commit = git_commit_tree(&workspace.path, tree.trim(), head.trim(), message)?;
        git_dynamic(&workspace.path, &["update-ref", &git_ref, commit.trim()])?;
        let _ = fs::remove_file(&index_path);

        self.conn.execute(
            "INSERT INTO checkpoints (workspace_id, session_id, git_ref, message, created_at)
             VALUES (?1, NULL, ?2, ?3, ?4)",
            params![workspace.id, git_ref, message, now],
        )?;
        self.get_checkpoint(self.conn.last_insert_rowid())
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
        let text = text.trim();
        anyhow::ensure!(!text.is_empty(), "todo text is required");
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
        let path_exists = workspace.path.exists();
        let changed_files = if path_exists {
            self.changed_files(name)?.len()
        } else {
            0
        };
        let run_status = self.latest_process_status(workspace.id, ProcessKind::Run)?;
        let session_status = self.latest_process_status(workspace.id, ProcessKind::Session)?;
        let active_sessions = self.count_running_processes(workspace.id, ProcessKind::Session)?;
        let pull_request = self.pull_request_by_workspace_id(workspace.id)?;
        let todos = self.list_todos(name)?;
        let open_todos = todos.iter().filter(|todo| todo.status == "open").count();
        let branch_push_state = if path_exists {
            self.branch_push_state(name).ok()
        } else {
            None
        };
        let comments = self.list_review_comments(name)?;
        let open_review_comments = comments.iter().filter(|c| c.status == "open").count();
        let conflicting_workspaces = if path_exists {
            self.find_conflicting_workspaces(name).unwrap_or_default()
        } else {
            Vec::new()
        };
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
            "SELECT id, workspace_id, kind, command, pid, log_path, status, started_at, exit_code, ended_at, session_harness_metadata
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
        let comments = self.list_review_comments(name)?;
        let open_comments = comments.iter().filter(|c| c.status == "open").count();
        if open_comments > 0 {
            anyhow::bail!(
                "{open_comments} open review comment(s) remain in workspace {name}; resolve them before merging"
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

    pub fn merge_and_maybe_archive_pull_request(
        &self,
        name: &str,
        method: &str,
    ) -> Result<MergePullRequestResult> {
        let merge_output = self.merge_pull_request(name, method)?;
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let settings = load_repository_settings(&repository.root_path)?;
        let archived_workspace = if settings.git.archive_on_merge.unwrap_or(false) {
            Some(self.archive(name, false)?)
        } else {
            None
        };
        Ok(MergePullRequestResult {
            merge_output,
            archived_workspace,
        })
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
            harness_metadata: None,
        })
    }

    pub fn session_launch(&self, name: &str, kind: SessionKind) -> Result<SessionLaunch> {
        self.session_launch_with_options(name, kind, SessionHarnessOptions::default())
    }

    pub fn session_launch_with_options(
        &self,
        name: &str,
        kind: SessionKind,
        harness: SessionHarnessOptions,
    ) -> Result<SessionLaunch> {
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let settings = load_repository_settings(&repository.root_path)?;
        let mut env = conductor_environment(&settings, &repository, &workspace);
        harness.apply_to_env(&mut env);
        let (program, args) = match kind {
            SessionKind::Shell => (
                std::env::var_os("SHELL")
                    .filter(|shell| !shell.is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("/bin/sh")),
                Vec::new(),
            ),
            SessionKind::Codex => (
                settings
                    .providers
                    .codex_executable_path
                    .as_deref()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("codex")),
                Vec::new(),
            ),
            SessionKind::Claude => (
                settings
                    .providers
                    .claude_code_executable_path
                    .as_deref()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("claude")),
                Vec::new(),
            ),
            SessionKind::Cursor => (
                PathBuf::from("cursor"),
                vec![workspace.path.to_string_lossy().to_string()],
            ),
        };

        Ok(SessionLaunch {
            kind,
            program,
            args,
            cwd: workspace.path.clone(),
            env,
            harness_metadata: harness.metadata(),
        })
    }

    pub fn list_sessions(&self, name: &str) -> Result<Vec<ProcessRecord>> {
        self.list_processes(name, ProcessKind::Session)
    }

    pub fn list_terminals(&self, name: &str) -> Result<Vec<ProcessRecord>> {
        self.list_processes(name, ProcessKind::Terminal)
    }

    pub fn list_runs(&self, name: &str) -> Result<Vec<ProcessRecord>> {
        self.list_processes(name, ProcessKind::Run)
    }

    pub fn list_setups(&self, name: &str) -> Result<Vec<ProcessRecord>> {
        self.list_processes(name, ProcessKind::Setup)
    }

    fn list_processes(&self, name: &str, kind: ProcessKind) -> Result<Vec<ProcessRecord>> {
        let workspace = self.get_by_name(name)?;
        let mut stmt = self.conn.prepare(
            "SELECT id, workspace_id, kind, command, pid, log_path, status, started_at, exit_code, ended_at, session_harness_metadata
             FROM processes WHERE workspace_id = ?1 AND kind = ?2
             ORDER BY id DESC",
        )?;
        let records = stmt
            .query_map(params![workspace.id, kind.as_str()], row_to_process)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(records)
    }

    pub fn start_session(&self, name: &str, kind: SessionKind) -> Result<ProcessRecord> {
        self.start_session_with_options(name, kind, SessionHarnessOptions::default())
    }

    pub fn start_session_with_options(
        &self,
        name: &str,
        kind: SessionKind,
        harness: SessionHarnessOptions,
    ) -> Result<ProcessRecord> {
        let launch = self.session_launch_with_options(name, kind, harness)?;
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
            launch.harness_metadata.as_deref(),
        )
    }

    pub fn record_session_process(
        &self,
        name: &str,
        launch: &SessionLaunch,
        pid: u32,
    ) -> Result<ProcessRecord> {
        anyhow::ensure!(pid > 0, "session process id is required");
        let workspace = self.get_by_name(name)?;
        let command = shell_words(&launch.program, &launch.args);
        self.record_process(
            ProcessKind::Session,
            &workspace,
            &command,
            pid,
            "session",
            launch.harness_metadata.as_deref(),
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

    pub fn workspace_path(&self, name: &str) -> Result<PathBuf> {
        Ok(self.get_by_name(name)?.path)
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

    fn get_by_id(&self, id: i64) -> Result<Workspace> {
        self.conn
            .query_row(
                "SELECT id, repository_id, name, path, branch, base_ref, port_base, status, archived_at, created_at, updated_at
                 FROM workspaces WHERE id = ?1",
                [id],
                row_to_workspace,
            )
            .with_context(|| format!("load workspace {id}"))
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
        session_harness_metadata: Option<&str>,
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
                workspace_id, kind, command, pid, log_path, status, started_at, exit_code, ended_at, session_harness_metadata
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, ?8)",
            params![
                workspace.id,
                kind.as_str(),
                script,
                i64::from(child.id()),
                log_path.to_string_lossy().to_string(),
                ProcessStatus::Running.as_str(),
                now,
                session_harness_metadata,
            ],
        )?;

        let process = self.latest_process(workspace.id, kind)?;
        spawn_process_monitor(self.db_path.clone(), process.id, child);
        Ok(process)
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
                "SELECT id, workspace_id, kind, command, pid, log_path, status, started_at, exit_code, ended_at, session_harness_metadata
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
                "SELECT id, workspace_id, kind, command, pid, log_path, status, started_at, exit_code, ended_at, session_harness_metadata
                 FROM processes WHERE id = ?1",
                [id],
                row_to_process,
            )
            .with_context(|| format!("load process {id}"))
    }

    fn active_spotlight_for_repository(
        &self,
        repository_id: i64,
    ) -> Result<Option<SpotlightSession>> {
        let result = self.conn.query_row(
            "SELECT id, repository_id, workspace_id, workspace_name, patch_path, status, started_at, ended_at
             FROM spotlight_sessions
             WHERE repository_id = ?1 AND status = 'active'
             ORDER BY id DESC LIMIT 1",
            [repository_id],
            row_to_spotlight_session,
        );
        match result {
            Ok(session) => Ok(Some(session)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    fn active_spotlight_sessions(&self) -> Result<Vec<SpotlightSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, repository_id, workspace_id, workspace_name, patch_path, status, started_at, ended_at
             FROM spotlight_sessions
             WHERE status = 'active'
             ORDER BY id",
        )?;
        let sessions = stmt
            .query_map([], row_to_spotlight_session)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(sessions)
    }

    fn get_spotlight_session(&self, id: i64) -> Result<SpotlightSession> {
        self.conn
            .query_row(
                "SELECT id, repository_id, workspace_id, workspace_name, patch_path, status, started_at, ended_at
                 FROM spotlight_sessions WHERE id = ?1",
                [id],
                row_to_spotlight_session,
            )
            .with_context(|| format!("load spotlight session {id}"))
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
              ended_at TEXT,
              session_harness_metadata TEXT
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

            CREATE TABLE IF NOT EXISTS spotlight_sessions (
              id INTEGER PRIMARY KEY,
              repository_id INTEGER NOT NULL REFERENCES repositories(id),
              workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
              workspace_name TEXT NOT NULL,
              patch_path TEXT NOT NULL,
              status TEXT NOT NULL,
              started_at TEXT NOT NULL,
              ended_at TEXT
            );
            ",
        )?;
        ensure_column(
            &self.conn,
            "processes",
            "exit_code",
            "ALTER TABLE processes ADD COLUMN exit_code INTEGER",
        )?;
        ensure_column(
            &self.conn,
            "processes",
            "session_harness_metadata",
            "ALTER TABLE processes ADD COLUMN session_harness_metadata TEXT",
        )?;
        Ok(())
    }
}

fn ensure_column(conn: &Connection, table: &str, column: &str, alter_sql: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !names.iter().any(|name| name == column) {
        conn.execute(alter_sql, [])?;
    }
    Ok(())
}

fn spawn_process_monitor(db_path: PathBuf, process_id: i64, mut child: Child) {
    std::thread::spawn(move || {
        let Ok(status) = child.wait() else {
            return;
        };
        let Ok(conn) = Connection::open(db_path) else {
            return;
        };
        let now = timestamp();
        let _ = conn.execute(
            "UPDATE processes
             SET status = ?1, ended_at = ?2, exit_code = ?3
             WHERE id = ?4 AND status = 'running'",
            params![
                ProcessStatus::Exited.as_str(),
                now,
                status.code(),
                process_id
            ],
        );
    });
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
        "setup" => ProcessKind::Setup,
        "run" => ProcessKind::Run,
        "session" => ProcessKind::Session,
        "terminal" => ProcessKind::Terminal,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let pid = row.get::<_, i64>(4)?;
    let session_harness_metadata = row
        .get::<_, Option<String>>("session_harness_metadata")
        .ok()
        .flatten();
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
        exit_code: row.get(8)?,
        ended_at: row.get(9)?,
        session_harness_metadata,
    })
}

fn row_to_spotlight_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<SpotlightSession> {
    Ok(SpotlightSession {
        id: row.get(0)?,
        repository_id: row.get(1)?,
        workspace_id: row.get(2)?,
        workspace_name: row.get(3)?,
        patch_path: PathBuf::from(row.get::<_, String>(4)?),
        status: row.get(5)?,
        started_at: row.get(6)?,
        ended_at: row.get(7)?,
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

fn parse_pull_request_check_runs(output: &str) -> Vec<PullRequestCheckRun> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let parts = line.split('\t').map(str::trim).collect::<Vec<_>>();
            if parts.len() >= 2 {
                return Some(PullRequestCheckRun {
                    name: parts[0].to_owned(),
                    status: parts[1].to_owned(),
                    detail: parts
                        .iter()
                        .skip(2)
                        .rev()
                        .find(|part| !part.is_empty())
                        .map(|part| (*part).to_owned()),
                });
            }
            let lower = line.to_ascii_lowercase();
            let status = [
                "fail",
                "failed",
                "failure",
                "error",
                "cancelled",
                "timed_out",
                "pass",
                "pending",
            ]
            .iter()
            .find(|status| lower.contains(**status))?;
            Some(PullRequestCheckRun {
                name: line.to_owned(),
                status: (*status).to_owned(),
                detail: None,
            })
        })
        .collect()
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

fn terminal_log_preview(contents: &str) -> String {
    contents
        .lines()
        .rev()
        .find_map(|line| {
            let trimmed = line.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        })
        .unwrap_or_else(|| "(empty transcript)".to_owned())
}

fn parse_diff_numstat(output: &str) -> Vec<DiffFileSummary> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let additions = parse_numstat_count(parts.next()?)?;
            let deletions = parse_numstat_count(parts.next()?)?;
            let path = parts.next()?.to_owned();
            Some(DiffFileSummary {
                path,
                additions,
                deletions,
            })
        })
        .collect()
}

fn parse_untracked_status_paths(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.strip_prefix("?? "))
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect()
}

fn is_conductor_context_path(path: &str) -> bool {
    path == ".context" || path.starts_with(".context/")
}

fn untracked_file_counts(path: &Path) -> Result<(usize, usize)> {
    let contents = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let additions = if contents.is_empty() {
        0
    } else {
        contents.iter().filter(|byte| **byte == b'\n').count()
            + usize::from(!contents.ends_with(b"\n"))
    };
    Ok((additions, contents.len()))
}

fn format_review_comments_agent_prompt(name: &str, comments: &[ReviewComment]) -> String {
    let mut prompt = format!("Address these open review comments for workspace {name}.\n");
    if comments.is_empty() {
        prompt.push_str("No open review comments.\n");
        return prompt;
    }
    prompt.push_str("Make the smallest safe changes, then run relevant tests.\n\n");
    for comment in comments {
        let line = comment
            .line_number
            .map(|line| format!(":{line}"))
            .unwrap_or_default();
        prompt.push_str(&format!(
            "- #{} {}{}: {}\n",
            comment.id, comment.file_path, line, comment.body
        ));
    }
    prompt
}

fn format_pull_request_checks_agent_prompt(name: &str, checks: &[PullRequestCheckRun]) -> String {
    let failures = checks
        .iter()
        .filter(|check| check.is_failure())
        .collect::<Vec<_>>();
    let mut prompt = format!("Fix these failing PR checks for workspace {name}.\n");
    if failures.is_empty() {
        prompt.push_str("No failing PR checks.\n");
        return prompt;
    }
    prompt.push_str("Make the smallest safe changes, then run relevant tests.\n\n");
    for check in failures {
        match check.detail.as_deref() {
            Some(detail) => prompt.push_str(&format!(
                "- {}: {} - {}\n",
                check.name, check.status, detail
            )),
            None => prompt.push_str(&format!("- {}: {}\n", check.name, check.status)),
        }
    }
    prompt
}

fn format_pull_request_review_agent_prompt(name: &str, review_state: &str) -> String {
    let mut prompt = format!("Address this GitHub PR review/comment state for workspace {name}.\n");
    let review_state = review_state.trim();
    if review_state.is_empty() {
        prompt.push_str("No GitHub PR review/comment output.\n");
        return prompt;
    }
    prompt.push_str("Make the smallest safe changes, then run relevant tests.\n\n");
    prompt.push_str(review_state);
    prompt.push('\n');
    prompt
}

fn parse_numstat_count(value: &str) -> Option<Option<usize>> {
    if value == "-" {
        return Some(None);
    }
    value.parse::<usize>().ok().map(Some)
}

fn validate_workspace_relative_path(relative_path: &str) -> Result<&Path> {
    let path = Path::new(relative_path);
    anyhow::ensure!(
        path.is_relative(),
        "workspace file path must be relative: {relative_path}",
    );
    for component in path.components() {
        anyhow::ensure!(
            !matches!(component, Component::ParentDir | Component::CurDir),
            "workspace file path may not use path traversal: {relative_path}",
        );
    }
    Ok(path)
}

fn ensure_tracked_in_head(cwd: &Path, relative_path: &str) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["ls-files", "--error-unmatch", "--", relative_path])
        .output()
        .with_context(|| format!("check tracked file in {}", cwd.display()))?;
    anyhow::ensure!(
        output.status.success(),
        "{relative_path} is not tracked in HEAD and cannot be safely reverted"
    );
    Ok(())
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
    if slug.is_empty() {
        "workspace".to_owned()
    } else if slug.len() > 40 {
        slug[..40].trim_end_matches('-').to_owned()
    } else {
        slug
    }
}

struct LinearIssue {
    identifier: String,
    title: String,
    branch_name: Option<String>,
    url: Option<String>,
}

fn fetch_linear_issue(issue_id: &str) -> Result<LinearIssue> {
    let api_key = std::env::var("LINEAR_API_KEY")
        .context("LINEAR_API_KEY is required to create a workspace from a Linear issue")?;
    let payload = format!(
        r#"{{"query":"query Issue($id: String!) {{ issue(id: $id) {{ identifier title branchName url }} }}","variables":{{"id":"{}"}}}}"#,
        json_escape(issue_id)
    );
    let output = Command::new("curl")
        .args([
            "-fsS",
            "https://api.linear.app/graphql",
            "-H",
            "Content-Type: application/json",
            "-H",
            &format!("Authorization: {api_key}"),
            "--data",
            &payload,
        ])
        .output()
        .context("run curl for Linear API")?;
    anyhow::ensure!(
        output.status.success(),
        "Linear API request failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body = String::from_utf8_lossy(&output.stdout);
    if body.contains("\"errors\"") {
        anyhow::bail!("Linear API returned errors: {body}");
    }
    let identifier = extract_json_string_field(&body, "identifier")
        .unwrap_or_else(|| issue_id.to_ascii_uppercase());
    let title = extract_json_string_field(&body, "title")
        .with_context(|| format!("Linear issue {issue_id} did not include a title"))?;
    let branch_name = extract_json_string_field(&body, "branchName");
    let url = extract_json_string_field(&body, "url");
    Ok(LinearIssue {
        identifier,
        title,
        branch_name,
        url,
    })
}

fn json_escape(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| match ch {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '"' => "\\\"".chars().collect(),
            '\n' => "\\n".chars().collect(),
            '\r' => "\\r".chars().collect(),
            '\t' => "\\t".chars().collect(),
            _ => vec![ch],
        })
        .collect()
}

fn write_context_brief(workspace_path: &Path, content: &str) -> Result<()> {
    let brief_path = workspace_path.join(".context/brief.md");
    fs::write(&brief_path, content).with_context(|| format!("write {}", brief_path.display()))
}

fn validate_workspace_name(name: &str) -> Result<()> {
    anyhow::ensure!(!name.trim().is_empty(), "workspace name must not be empty");
    anyhow::ensure!(
        name.chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_')),
        "workspace name may only contain ASCII letters, numbers, '-' and '_'"
    );
    Ok(())
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

fn ensure_clean_git_tree(cwd: &Path, label: &str) -> Result<()> {
    let status = git_output_dynamic(cwd, &["status", "--porcelain"])?;
    anyhow::ensure!(
        status.trim().is_empty(),
        "{label} must be clean before Spotlight testing"
    );
    Ok(())
}

fn workspace_tracked_patch(workspace: &Workspace) -> Result<String> {
    let mut patch = String::new();
    patch.push_str(&git_output_dynamic(
        &workspace.path,
        &["diff", "--binary", &workspace.base_ref, "HEAD"],
    )?);
    patch.push_str(&git_output_dynamic(
        &workspace.path,
        &["diff", "--binary", "--cached", "HEAD"],
    )?);
    patch.push_str(&git_output_dynamic(&workspace.path, &["diff", "--binary"])?);
    Ok(patch)
}

fn ensure_root_matches_spotlight_patch(root_path: &Path, expected_patch: &str) -> Result<()> {
    let current_patch = root_tracked_patch(root_path)?;
    let conflict_detail = spotlight_conflict_detail(&current_patch, expected_patch);
    anyhow::ensure!(
        current_patch.trim() == expected_patch.trim(),
        "repository root has changes outside the active Spotlight patch{conflict_detail}; clean or save root changes before changing Spotlight state"
    );
    Ok(())
}

fn root_tracked_patch(root_path: &Path) -> Result<String> {
    let index_path =
        std::env::temp_dir().join(format!("linux-conductor-root-index-{}", timestamp_nanos()));
    git_with_index(root_path, &index_path, &["read-tree", "HEAD"])?;
    git_with_index(root_path, &index_path, &["add", "-A"])?;
    let current_patch = git_with_index_output(
        root_path,
        &index_path,
        &["diff", "--cached", "--binary", "HEAD"],
    )?;
    let _ = fs::remove_file(&index_path);
    Ok(current_patch)
}

fn spotlight_conflict_detail(current_patch: &str, expected_patch: &str) -> String {
    let paths = spotlight_conflict_paths(current_patch, expected_patch);
    if paths.is_empty() {
        return String::new();
    }

    let shown = paths.into_iter().take(6).collect::<Vec<_>>();
    format!("; changed root paths: {}", shown.join(", "))
}

fn spotlight_conflict_paths(current_patch: &str, expected_patch: &str) -> BTreeSet<String> {
    let expected_paths = patch_changed_paths(expected_patch);
    let current_paths = patch_changed_paths(current_patch);
    let root_only_paths = current_paths
        .difference(&expected_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    if root_only_paths.is_empty() {
        current_paths.union(&expected_paths).cloned().collect()
    } else {
        root_only_paths
    }
}

fn patch_changed_paths(patch: &str) -> BTreeSet<String> {
    patch
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("diff --git a/")?;
            let (_, path) = rest.split_once(" b/")?;
            Some(path.to_owned())
        })
        .collect()
}

fn apply_git_patch(cwd: &Path, patch: &str) -> Result<()> {
    git_patch(cwd, &["apply", "--binary", "-"], patch)
}

fn reverse_git_patch(cwd: &Path, patch: &str) -> Result<()> {
    git_patch(cwd, &["apply", "--binary", "--reverse", "-"], patch)
}

fn git_patch(cwd: &Path, args: &[&str], patch: &str) -> Result<()> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("run git patch command in {}", cwd.display()))?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(patch.as_bytes())
            .context("write patch to git apply")?;
    }
    let output = child
        .wait_with_output()
        .with_context(|| format!("wait for git patch command in {}", cwd.display()))?;
    anyhow::ensure!(
        output.status.success(),
        "git patch command failed in {}: {}\n{}",
        cwd.display(),
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    Ok(())
}

fn git_with_index(cwd: &Path, index_path: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .env("GIT_INDEX_FILE", index_path)
        .args(args)
        .output()
        .with_context(|| format!("run git with temp index in {}", cwd.display()))?;
    anyhow::ensure!(
        output.status.success(),
        "git command failed in {}: {}\n{}",
        cwd.display(),
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    Ok(())
}

fn git_with_index_output(cwd: &Path, index_path: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .env("GIT_INDEX_FILE", index_path)
        .args(args)
        .output()
        .with_context(|| format!("run git with temp index in {}", cwd.display()))?;
    anyhow::ensure!(
        output.status.success(),
        "git command failed in {}: {}\n{}",
        cwd.display(),
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn git_patch_with_index(cwd: &Path, index_path: &Path, args: &[&str], patch: &str) -> Result<()> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .env("GIT_INDEX_FILE", index_path)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("run git patch command with temp index in {}", cwd.display()))?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(patch.as_bytes())
            .context("write patch to git apply")?;
    }
    let output = child
        .wait_with_output()
        .with_context(|| format!("wait for git patch command in {}", cwd.display()))?;
    anyhow::ensure!(
        output.status.success(),
        "git patch command failed in {}: {}\n{}",
        cwd.display(),
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    Ok(())
}

fn git_commit_tree(cwd: &Path, tree: &str, parent: &str, message: &str) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args([
            "-c",
            "user.name=Linux Conductor",
            "-c",
            "user.email=linux-conductor@example.test",
            "-c",
            "commit.gpgsign=false",
            "commit-tree",
            tree,
            "-p",
            parent,
            "-m",
            message,
        ])
        .output()
        .with_context(|| format!("create git checkpoint commit in {}", cwd.display()))?;
    anyhow::ensure!(
        output.status.success(),
        "git commit-tree failed in {}: {}\n{}",
        cwd.display(),
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
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

fn timestamp_nanos() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos().to_string())
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

fn process_alive(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn stop_process(pid: u32) -> Result<()> {
    // Try SIGTERM to process group first, then to process directly.
    let group_ok = Command::new("kill")
        .arg("-TERM")
        .arg(format!("-{pid}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("run kill")?
        .success();
    if !group_ok {
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    // Give the process up to 3 seconds to exit gracefully.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        if !process_alive(pid) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    // Still alive — send SIGKILL to process group, then process.
    if process_alive(pid) {
        let _ = Command::new("kill")
            .arg("-KILL")
            .arg(format!("-{pid}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        std::thread::sleep(Duration::from_millis(200));
        let _ = Command::new("kill")
            .arg("-KILL")
            .arg(pid.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

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
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

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
    fn create_from_prompt_writes_prompt_to_context_brief() {
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
            .create_from_prompt(
                "demo",
                "Build the real connector flow",
                None,
                None,
                Some("main"),
            )
            .unwrap();

        assert_eq!(workspace.name, "build-the-real-connector-flow");
        assert_eq!(workspace.branch, "lc/build-the-real-connector-flow");
        let brief = fs::read_to_string(workspace.path.join(".context/brief.md")).unwrap();
        assert!(brief.contains("Build the real connector flow"));
        assert!(brief.contains("Prompt"));
    }

    #[test]
    fn create_from_issue_uses_gh_title_and_writes_context_brief() {
        let _guard = env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        let db_path = temp.path().join("state.db");
        let old_path = install_fake_gh(
            temp.path(),
            r#"#!/bin/sh
if [ "$1" = "issue" ] && [ "$2" = "view" ]; then
  printf '{"title":"Fix honest connector validation","number":123}\n'
  exit 0
fi
echo "unexpected gh args: $*" >&2
exit 1
"#,
        );

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
        let workspace = store.create_from_issue("demo", 123, None).unwrap();

        restore_path(old_path);
        assert_eq!(workspace.name, "issue-123");
        assert_eq!(workspace.branch, "lc/123/fix-honest-connector-validation");
        let brief = fs::read_to_string(workspace.path.join(".context/brief.md")).unwrap();
        assert!(brief.contains("GitHub issue #123: Fix honest connector validation"));
        assert!(brief.contains("GitHub Issue"));
    }

    #[test]
    fn create_from_pull_request_fetches_pr_ref_records_pr_and_writes_context_brief() {
        let _guard = env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        let remote_path = temp.path().join("origin.git");
        Command::new("git")
            .args(["init", "--bare"])
            .arg(&remote_path)
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["remote", "add", "origin"])
            .arg(&remote_path)
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["push", "origin", "main"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["checkout", "-b", "contributor/pr-head"])
            .status()
            .unwrap();
        fs::write(repo_path.join("pr.txt"), "from pr\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", "pr.txt"])
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
                "pr head",
            ])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["push", "origin", "HEAD:refs/pull/42/head"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["checkout", "main"])
            .status()
            .unwrap();
        let old_path = install_fake_gh(
            temp.path(),
            r#"#!/bin/sh
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  printf '{"title":"Add real PR source","url":"https://github.com/example/demo/pull/42","state":"open","number":42}\n'
  exit 0
fi
echo "unexpected gh args: $*" >&2
exit 1
"#,
        );

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
            .create_from_pull_request("demo", 42, None, None)
            .unwrap();

        restore_path(old_path);
        assert_eq!(workspace.name, "pr-42");
        assert!(workspace.path.join("pr.txt").exists());
        let pr = store.pull_request("pr-42").unwrap().unwrap();
        assert_eq!(pr.number, 42);
        assert_eq!(pr.url, "https://github.com/example/demo/pull/42");
        let brief = fs::read_to_string(workspace.path.join(".context/brief.md")).unwrap();
        assert!(brief.contains("GitHub PR #42: Add real PR source"));
        assert!(brief.contains("https://github.com/example/demo/pull/42"));
    }

    fn install_fake_gh(temp: &Path, script: &str) -> Option<std::ffi::OsString> {
        let bin_dir = temp.join("bin");
        fs::create_dir(&bin_dir).unwrap();
        let gh_path = bin_dir.join("gh");
        fs::write(&gh_path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&gh_path).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&gh_path, permissions).unwrap();
        }
        let old_path = std::env::var_os("PATH");
        let new_path = match old_path.as_ref() {
            Some(path) => {
                let mut parts = vec![bin_dir];
                parts.extend(std::env::split_paths(path));
                std::env::join_paths(parts).unwrap()
            }
            None => bin_dir.into_os_string(),
        };
        std::env::set_var("PATH", new_path);
        old_path
    }

    fn restore_path(old_path: Option<std::ffi::OsString>) {
        match old_path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }
    }

    #[test]
    fn session_launch_uses_configured_provider_executables() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".conductor")).unwrap();
        fs::write(
            repo_path.join(".conductor/settings.toml"),
            r#"
codex_executable_path = "/opt/bin/codex-custom"
claude_code_executable_path = "/opt/bin/claude-custom"
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
            .args(["commit", "-m", "add conductor settings"])
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

        let store = WorkspaceStore::open(&db_path).unwrap();
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        assert_eq!(
            store
                .session_launch("berlin", SessionKind::Codex)
                .unwrap()
                .program,
            PathBuf::from("/opt/bin/codex-custom")
        );
        assert_eq!(
            store
                .session_launch("berlin", SessionKind::Claude)
                .unwrap()
                .program,
            PathBuf::from("/opt/bin/claude-custom")
        );
    }

    #[test]
    fn workspace_names_must_be_shell_safe() {
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
        let unsafe_create = store.create(CreateWorkspace {
            repository_name: "demo".to_owned(),
            name: "bad name; rm -rf /".to_owned(),
            branch: "lc/bad".to_owned(),
            base_ref: Some("main".to_owned()),
        });
        assert!(unsafe_create.is_err());

        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        assert!(store.rename("berlin", "../bad").is_err());
        assert!(store.rename("berlin", "oslo_2").is_ok());
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
        assert_eq!(stopped.exit_code, Some(143));
        assert!(stopped.ended_at.is_some());
    }

    #[test]
    fn run_workspace_records_exit_status_when_process_finishes() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".conductor")).unwrap();
        fs::write(
            repo_path.join(".conductor/settings.toml"),
            r#"
[scripts]
run = "printf 'done\n'; exit 3"
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
                "add exiting run script",
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
        wait_for_log(&run.log_path, "done");
        let exited =
            wait_for_process_status(&store, "berlin", ProcessKind::Run, ProcessStatus::Exited);

        assert_eq!(exited.id, run.id);
        assert_eq!(exited.exit_code, Some(3));
        assert!(exited.ended_at.is_some());
    }

    #[test]
    fn terminal_command_runs_in_workspace_with_conductor_environment_and_captures_output() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".conductor")).unwrap();
        fs::write(
            repo_path.join(".conductor/settings.toml"),
            r#"
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
                "add terminal env",
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

        let store = WorkspaceStore::open(&db_path).unwrap();
        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        let result = store
            .terminal_command(
                "berlin",
                "pwd; printf '%s:%s:%s\\n' \"$CONDUCTOR_WORKSPACE_NAME\" \"$CONDUCTOR_PORT\" \"$CUSTOM_VALUE\"; printf 'warn\\n' >&2; exit 7",
            )
            .unwrap();

        assert_eq!(result.command, "pwd; printf '%s:%s:%s\\n' \"$CONDUCTOR_WORKSPACE_NAME\" \"$CONDUCTOR_PORT\" \"$CUSTOM_VALUE\"; printf 'warn\\n' >&2; exit 7");
        assert_eq!(result.cwd, workspace.path);
        assert_eq!(result.exit_code, Some(7));
        assert!(result.stdout.contains("berlin:3000:from-settings"));
        assert!(result.stdout.contains(result.cwd.to_str().unwrap()));
        assert_eq!(result.stderr, "warn\n");
        assert!(!result.started_at.is_empty());
        assert!(!result.ended_at.is_empty());
    }

    #[test]
    fn terminal_process_records_track_running_and_stopped_shells() {
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
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        let running = store
            .record_terminal_process("berlin", "shell", 4242)
            .unwrap();

        assert_eq!(running.kind, ProcessKind::Terminal);
        assert_eq!(running.command, "shell");
        assert_eq!(running.pid, 4242);
        assert_eq!(running.status, ProcessStatus::Running);
        assert_eq!(running.exit_code, None);
        assert!(running.ended_at.is_none());
        assert_eq!(running.log_path.extension().unwrap(), "log");
        assert!(running
            .log_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("terminal-"));

        let terminals = store.list_terminals("berlin").unwrap();
        assert_eq!(terminals.len(), 1);
        assert_eq!(terminals[0].id, running.id);

        let stopped = store
            .mark_terminal_process_stopped(running.id, Some(143))
            .unwrap();

        assert_eq!(stopped.id, running.id);
        assert_eq!(stopped.status, ProcessStatus::Stopped);
        assert_eq!(stopped.exit_code, Some(143));
        assert!(stopped.ended_at.is_some());
    }

    #[test]
    fn terminal_process_reconciliation_marks_dead_shells_exited() {
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
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let stale = store
            .record_terminal_process("berlin", "shell", 999_999)
            .unwrap();

        let reconciled = store.reconcile_terminal_processes().unwrap();

        assert_eq!(reconciled.len(), 1);
        assert_eq!(reconciled[0].id, stale.id);
        assert_eq!(reconciled[0].status, ProcessStatus::Exited);
        assert_eq!(reconciled[0].exit_code, None);
        assert!(reconciled[0].ended_at.is_some());
        assert_eq!(
            store.list_terminals("berlin").unwrap()[0].status,
            ProcessStatus::Exited
        );
    }

    #[test]
    fn terminal_process_stop_by_id_marks_stopped() {
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
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let running = store
            .record_terminal_process("berlin", "shell", 999_999)
            .unwrap();

        let stopped = store.stop_terminal_process("berlin", running.id).unwrap();

        assert_eq!(stopped.id, running.id);
        assert_eq!(stopped.status, ProcessStatus::Stopped);
        assert_eq!(stopped.exit_code, Some(SIGTERM_EXIT_CODE));
        assert!(stopped.ended_at.is_some());
    }

    #[test]
    fn terminal_process_stop_by_id_noop_when_already_stopped() {
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
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let running = store
            .record_terminal_process("berlin", "shell", 999_999)
            .unwrap();
        store
            .mark_terminal_process_stopped(running.id, Some(1))
            .unwrap();

        let stopped = store.stop_terminal_process("berlin", running.id).unwrap();

        assert_eq!(stopped.id, running.id);
        assert_eq!(stopped.status, ProcessStatus::Stopped);
        assert_eq!(stopped.exit_code, Some(1));
        assert!(stopped.ended_at.is_some());
    }

    #[test]
    fn terminal_process_stop_by_id_respects_workspace() {
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
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "tokyo".to_owned(),
                branch: "lc/tokyo".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let berlin_terminal = store
            .record_terminal_process("berlin", "shell", 999_999)
            .unwrap();

        let result = store.stop_terminal_process("tokyo", berlin_terminal.id);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains(&format!(
            "terminal session {} not found",
            berlin_terminal.id
        )));
    }

    #[test]
    fn terminal_process_stop_by_id_rejects_invalid_pid() {
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
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let invalid_pid_terminal = store.record_terminal_process("berlin", "shell", 0).unwrap();

        let result = store.stop_terminal_process("berlin", invalid_pid_terminal.id);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid pid"));
    }

    #[test]
    fn copy_conflict_file_from_workspace_checks_relations_and_path_validation() {
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
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "tokyo".to_owned(),
                branch: "lc/tokyo".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        let berlin = store.get_by_name("berlin").unwrap();
        fs::write(berlin.path.join("README.md"), "from berlin\n").unwrap();

        let tokyo = store.get_by_name("tokyo").unwrap();
        fs::write(tokyo.path.join("README.md"), "from tokyo\n").unwrap();

        store
            .copy_conflict_file_from_workspace("berlin", "tokyo", "README.md")
            .unwrap();
        assert_eq!(
            fs::read_to_string(berlin.path.join("README.md")).unwrap(),
            "from tokyo\n"
        );

        let traversal_err =
            store.copy_conflict_file_from_workspace("berlin", "tokyo", "../outside.txt");
        assert!(traversal_err.is_err());
    }

    #[test]
    fn terminal_process_marked_as_exited_on_exit_update() {
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
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let running = store
            .record_terminal_process("berlin", "shell", 999_999)
            .unwrap();

        let exited = store
            .mark_terminal_process_exited(running.id, Some(42))
            .unwrap();

        assert_eq!(exited.id, running.id);
        assert_eq!(exited.status, ProcessStatus::Exited);
        assert_eq!(exited.exit_code, Some(42));
        assert!(exited.ended_at.is_some());
    }

    #[test]
    fn terminal_process_records_use_distinct_log_files() {
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
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        let first = store
            .record_terminal_process("berlin", "shell", 4242)
            .unwrap();
        let second = store
            .record_terminal_process("berlin", "shell", 4243)
            .unwrap();

        assert_ne!(first.log_path, second.log_path);
        assert!(first.log_path.exists());
        assert!(second.log_path.exists());
        assert_eq!(
            first.log_path.parent().unwrap(),
            second.log_path.parent().unwrap()
        );
    }

    #[test]
    fn terminal_process_logs_append_transcript_output() {
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
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let terminal = store
            .record_terminal_process("berlin", "shell", 4242)
            .unwrap();

        store
            .append_terminal_process_output(terminal.id, "first line\n")
            .unwrap();
        store
            .append_terminal_process_output(terminal.id, "second line\n")
            .unwrap();

        assert_eq!(
            fs::read_to_string(terminal.log_path).unwrap(),
            "first line\nsecond line\n"
        );
    }

    #[test]
    fn terminal_log_search_finds_matching_transcript_lines() {
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
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let first = store
            .record_terminal_process("berlin", "shell", 4242)
            .unwrap();
        let second = store
            .record_terminal_process("berlin", "shell", 4243)
            .unwrap();
        store
            .append_terminal_process_output(
                first.id,
                "alpha\nbuild ok\nneedle first\nafter one\nafter two\n",
            )
            .unwrap();
        store
            .append_terminal_process_output(
                second.id,
                "before one\nNEEDLE second\nafter one\nafter two\nafter three\n",
            )
            .unwrap();

        let matches = store.search_terminal_logs("berlin", "needle").unwrap();

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].process_id, second.id);
        assert_eq!(matches[0].line_number, 2);
        assert_eq!(matches[0].line, "NEEDLE second");
        assert_eq!(matches[0].context_before, Vec::<String>::new());
        assert_eq!(
            matches[0].context_after,
            vec![
                "after one".to_owned(),
                "after two".to_owned(),
                "after three".to_owned()
            ]
        );
        assert_eq!(matches[1].process_id, first.id);
        assert_eq!(matches[1].line_number, 3);
        assert_eq!(matches[1].line, "needle first");
        assert_eq!(
            matches[1].context_before,
            vec!["alpha".to_owned(), "build ok".to_owned()]
        );
        assert_eq!(
            matches[1].context_after,
            vec!["after one".to_owned(), "after two".to_owned()]
        );
    }

    #[test]
    fn read_latest_terminal_log_returns_newest_terminal_transcript() {
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
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let older = store
            .record_terminal_process("berlin", "older shell", 4242)
            .unwrap();
        let newer = store
            .record_terminal_process("berlin", "newer shell", 4243)
            .unwrap();
        store
            .append_terminal_process_output(older.id, "older transcript\n")
            .unwrap();
        store
            .append_terminal_process_output(newer.id, "newer transcript\n")
            .unwrap();

        let transcript = store.read_latest_terminal_log("berlin").unwrap();

        assert_eq!(transcript, "newer transcript\n");
    }

    #[test]
    fn read_terminal_log_returns_requested_workspace_transcript() {
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
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let older = store
            .record_terminal_process("berlin", "older shell", 4242)
            .unwrap();
        let newer = store
            .record_terminal_process("berlin", "newer shell", 4243)
            .unwrap();
        store
            .append_terminal_process_output(older.id, "older transcript\n")
            .unwrap();
        store
            .append_terminal_process_output(newer.id, "newer transcript\n")
            .unwrap();

        let transcript = store.read_terminal_log("berlin", older.id).unwrap();

        assert_eq!(transcript, "older transcript\n");
    }

    #[test]
    fn list_terminal_summaries_includes_counts_and_preview_newest_first() {
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
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let older = store
            .record_terminal_process("berlin", "older shell", 4242)
            .unwrap();
        let newer = store
            .record_terminal_process("berlin", "newer shell", 4243)
            .unwrap();
        store
            .append_terminal_process_output(older.id, "older first\nolder last\n")
            .unwrap();
        store
            .append_terminal_process_output(newer.id, "newer first\n\nnewer last\n")
            .unwrap();

        let summaries = store.list_terminal_summaries("berlin").unwrap();

        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].process.id, newer.id);
        assert_eq!(summaries[0].line_count, 3);
        assert_eq!(summaries[0].byte_count, 24);
        assert_eq!(summaries[0].preview, "newer last");
        assert_eq!(summaries[1].process.id, older.id);
        assert_eq!(summaries[1].line_count, 2);
        assert_eq!(summaries[1].byte_count, 23);
        assert_eq!(summaries[1].preview, "older last");
    }

    #[test]
    fn setup_workspace_executes_setup_script_and_captures_logs() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".conductor")).unwrap();
        fs::write(
            repo_path.join(".conductor/settings.toml"),
            r#"
[scripts]
setup = "printf 'setup:%s:%s\n' \"$CONDUCTOR_WORKSPACE_NAME\" \"$CONDUCTOR_PORT\""
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

        let setup = store.setup_workspace("berlin").unwrap();
        wait_for_log(&setup.log_path, "setup:berlin:3000");

        assert_eq!(setup.kind, ProcessKind::Setup);
        assert_eq!(setup.status, ProcessStatus::Running);
        assert!(store
            .read_latest_setup_log("berlin")
            .unwrap()
            .contains("setup:berlin:3000"));
    }

    #[test]
    fn spotlight_start_applies_workspace_tracked_changes_and_stop_restores_root() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".conductor")).unwrap();
        fs::write(
            repo_path.join(".conductor/settings.toml"),
            r#"
spotlight_testing = true
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
                "enable spotlight",
            ])
            .status()
            .unwrap();

        let db_path = temp.path().join("state.db");
        let store = WorkspaceStore::open_with_logs(&db_path, temp.path().join("logs")).unwrap();
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

        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        fs::write(workspace.path.join("README.md"), "spotlight change\n").unwrap();
        fs::write(workspace.path.join("new-tracked.txt"), "new file\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args(["add", "README.md", "new-tracked.txt"])
            .status()
            .unwrap();

        let active = store.spotlight_start("berlin").unwrap();

        assert_eq!(active.workspace_name, "berlin");
        assert_eq!(active.status, "active");
        assert_eq!(
            fs::read_to_string(repo_path.join("README.md")).unwrap(),
            "spotlight change\n"
        );
        assert_eq!(
            fs::read_to_string(repo_path.join("new-tracked.txt")).unwrap(),
            "new file\n"
        );
        assert_eq!(store.spotlight_status("berlin").unwrap().unwrap(), active);
        let checkpoints = store.checkpoint_list("berlin").unwrap();
        assert_eq!(checkpoints.len(), 1);
        assert_eq!(checkpoints[0].message, "Spotlight checkpoint");
        assert_eq!(
            git_output(
                &workspace.path,
                ["show", &format!("{}:README.md", checkpoints[0].git_ref)]
            ),
            "spotlight change\n"
        );
        assert_eq!(
            git_output(
                &workspace.path,
                [
                    "show",
                    &format!("{}:new-tracked.txt", checkpoints[0].git_ref)
                ]
            ),
            "new file\n"
        );

        let stopped = store.spotlight_stop("berlin").unwrap();

        assert_eq!(stopped.status, "stopped");
        assert_eq!(
            fs::read_to_string(repo_path.join("README.md")).unwrap(),
            "demo\n"
        );
        assert!(!repo_path.join("new-tracked.txt").exists());
        assert!(store.spotlight_status("berlin").unwrap().is_none());
    }

    #[test]
    fn spotlight_start_switches_active_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".conductor")).unwrap();
        fs::write(
            repo_path.join(".conductor/settings.toml"),
            r#"
spotlight_testing = true
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
                "enable spotlight",
            ])
            .status()
            .unwrap();

        let db_path = temp.path().join("state.db");
        let store = WorkspaceStore::open_with_logs(&db_path, temp.path().join("logs")).unwrap();
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

        let berlin = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let oslo = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "oslo".to_owned(),
                branch: "lc/oslo".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        fs::write(berlin.path.join("README.md"), "berlin change\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&berlin.path)
            .args(["add", "README.md"])
            .status()
            .unwrap();
        fs::write(oslo.path.join("README.md"), "oslo change\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&oslo.path)
            .args(["add", "README.md"])
            .status()
            .unwrap();

        let berlin_active = store.spotlight_start("berlin").unwrap();
        assert_eq!(
            fs::read_to_string(repo_path.join("README.md")).unwrap(),
            "berlin change\n"
        );

        let oslo_active = store.spotlight_start("oslo").unwrap();

        assert_eq!(oslo_active.workspace_name, "oslo");
        assert_eq!(
            fs::read_to_string(repo_path.join("README.md")).unwrap(),
            "oslo change\n"
        );
        assert!(store.spotlight_status("berlin").unwrap().is_none());
        assert_eq!(
            store.spotlight_status("oslo").unwrap().unwrap(),
            oslo_active
        );
        let stopped_berlin = store.get_spotlight_session(berlin_active.id).unwrap();
        assert_eq!(stopped_berlin.status, "stopped");
        assert!(stopped_berlin.ended_at.is_some());
        assert_eq!(store.checkpoint_list("oslo").unwrap().len(), 1);
    }

    #[test]
    fn spotlight_start_updates_same_active_workspace_when_tracked_changes_appear() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".conductor")).unwrap();
        fs::write(
            repo_path.join(".conductor/settings.toml"),
            r#"
spotlight_testing = true
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
                "enable spotlight",
            ])
            .status()
            .unwrap();

        let db_path = temp.path().join("state.db");
        let store = WorkspaceStore::open_with_logs(&db_path, temp.path().join("logs")).unwrap();
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

        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        fs::write(workspace.path.join("README.md"), "first change\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args(["add", "README.md"])
            .status()
            .unwrap();
        let first = store.spotlight_start("berlin").unwrap();

        let unchanged = store.spotlight_start("berlin").unwrap();
        assert_eq!(unchanged.id, first.id);
        assert_eq!(store.checkpoint_list("berlin").unwrap().len(), 1);

        fs::write(workspace.path.join("README.md"), "second change\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args(["add", "README.md"])
            .status()
            .unwrap();
        let synced = store.spotlight_start("berlin").unwrap();

        assert_eq!(synced.id, first.id);
        assert_ne!(synced.patch_path, first.patch_path);
        assert_eq!(store.checkpoint_list("berlin").unwrap().len(), 2);
    }

    #[test]
    fn spotlight_sync_updates_active_workspace_patch() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".conductor")).unwrap();
        fs::write(
            repo_path.join(".conductor/settings.toml"),
            r#"
spotlight_testing = true
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
                "enable spotlight",
            ])
            .status()
            .unwrap();

        let db_path = temp.path().join("state.db");
        let store = WorkspaceStore::open_with_logs(&db_path, temp.path().join("logs")).unwrap();
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

        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        fs::write(workspace.path.join("README.md"), "first change\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args(["add", "README.md"])
            .status()
            .unwrap();
        let active = store.spotlight_start("berlin").unwrap();
        assert_eq!(
            fs::read_to_string(repo_path.join("README.md")).unwrap(),
            "first change\n"
        );

        fs::write(workspace.path.join("README.md"), "second change\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args(["add", "README.md"])
            .status()
            .unwrap();

        let synced = store.spotlight_sync("berlin").unwrap();

        assert_eq!(synced.id, active.id);
        assert_eq!(synced.status, "active");
        assert_ne!(synced.patch_path, active.patch_path);
        assert_eq!(
            fs::read_to_string(repo_path.join("README.md")).unwrap(),
            "second change\n"
        );
        let checkpoints = store.checkpoint_list("berlin").unwrap();
        assert_eq!(checkpoints.len(), 2);
        assert_eq!(
            git_output(
                &workspace.path,
                ["show", &format!("{}:README.md", checkpoints[0].git_ref)]
            ),
            "second change\n"
        );
    }

    #[test]
    fn spotlight_sync_updates_root_to_empty_when_workspace_changes_are_removed() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".conductor")).unwrap();
        fs::write(
            repo_path.join(".conductor/settings.toml"),
            r#"
spotlight_testing = true
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
                "enable spotlight",
            ])
            .status()
            .unwrap();

        let db_path = temp.path().join("state.db");
        let store = WorkspaceStore::open_with_logs(&db_path, temp.path().join("logs")).unwrap();
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

        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        fs::write(workspace.path.join("README.md"), "spotlight change\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args(["add", "README.md"])
            .status()
            .unwrap();
        let active = store.spotlight_start("berlin").unwrap();
        assert_eq!(
            fs::read_to_string(repo_path.join("README.md")).unwrap(),
            "spotlight change\n"
        );

        fs::write(workspace.path.join("README.md"), "demo\n").unwrap();

        let synced = store.spotlight_sync("berlin").unwrap();

        assert_eq!(synced.id, active.id);
        assert_eq!(synced.status, "active");
        assert_eq!(
            fs::read_to_string(repo_path.join("README.md")).unwrap(),
            "demo\n"
        );
        assert!(store
            .spotlight_root_conflict_paths("berlin")
            .unwrap()
            .is_empty());
        assert_eq!(store.checkpoint_list("berlin").unwrap().len(), 2);
    }

    #[test]
    fn spotlight_sync_if_changed_skips_unchanged_patch_and_syncs_new_patch() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".conductor")).unwrap();
        fs::write(
            repo_path.join(".conductor/settings.toml"),
            r#"
spotlight_testing = true
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
                "enable spotlight",
            ])
            .status()
            .unwrap();

        let db_path = temp.path().join("state.db");
        let store = WorkspaceStore::open_with_logs(&db_path, temp.path().join("logs")).unwrap();
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

        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        fs::write(workspace.path.join("README.md"), "first change\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args(["add", "README.md"])
            .status()
            .unwrap();
        let active = store.spotlight_start("berlin").unwrap();

        assert_eq!(store.spotlight_sync_if_changed("berlin").unwrap(), None);
        assert_eq!(store.checkpoint_list("berlin").unwrap().len(), 1);

        fs::write(workspace.path.join("README.md"), "second change\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args(["add", "README.md"])
            .status()
            .unwrap();

        let synced = store
            .spotlight_sync_if_changed("berlin")
            .unwrap()
            .expect("changed patch should sync");

        assert_eq!(synced.id, active.id);
        assert_eq!(
            fs::read_to_string(repo_path.join("README.md")).unwrap(),
            "second change\n"
        );
        assert_eq!(store.checkpoint_list("berlin").unwrap().len(), 2);
    }

    #[test]
    fn spotlight_sync_active_sessions_syncs_changed_active_workspaces() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".conductor")).unwrap();
        fs::write(
            repo_path.join(".conductor/settings.toml"),
            r#"
spotlight_testing = true
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
                "enable spotlight",
            ])
            .status()
            .unwrap();

        let db_path = temp.path().join("state.db");
        let store = WorkspaceStore::open_with_logs(&db_path, temp.path().join("logs")).unwrap();
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

        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        fs::write(workspace.path.join("README.md"), "first change\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args(["add", "README.md"])
            .status()
            .unwrap();
        let active = store.spotlight_start("berlin").unwrap();

        assert!(store.spotlight_sync_active_sessions().unwrap().is_empty());

        fs::write(workspace.path.join("README.md"), "background change\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args(["add", "README.md"])
            .status()
            .unwrap();

        let synced = store.spotlight_sync_active_sessions().unwrap();

        assert_eq!(synced.len(), 1);
        assert_eq!(synced[0].id, active.id);
        assert_eq!(
            fs::read_to_string(repo_path.join("README.md")).unwrap(),
            "background change\n"
        );
        assert_eq!(store.checkpoint_list("berlin").unwrap().len(), 2);
    }

    #[test]
    fn spotlight_watch_targets_return_active_workspace_paths() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".conductor")).unwrap();
        fs::write(
            repo_path.join(".conductor/settings.toml"),
            r#"
spotlight_testing = true
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
                "enable spotlight",
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
        fs::write(workspace.path.join("README.md"), "first change\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args(["add", "README.md"])
            .status()
            .unwrap();
        let active = store.spotlight_start("berlin").unwrap();

        let targets = store.spotlight_watch_targets().unwrap();

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].session_id, active.id);
        assert_eq!(targets[0].workspace_name, "berlin");
        assert_eq!(targets[0].workspace_path, workspace.path);
    }

    #[test]
    fn spotlight_stop_refuses_when_root_has_extra_changes() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".conductor")).unwrap();
        fs::write(
            repo_path.join(".conductor/settings.toml"),
            r#"
spotlight_testing = true
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
                "enable spotlight",
            ])
            .status()
            .unwrap();

        let db_path = temp.path().join("state.db");
        let store = WorkspaceStore::open_with_logs(&db_path, temp.path().join("logs")).unwrap();
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

        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        fs::write(workspace.path.join("README.md"), "spotlight change\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args(["add", "README.md"])
            .status()
            .unwrap();
        let active = store.spotlight_start("berlin").unwrap();
        fs::write(repo_path.join("root-only.txt"), "root edit\n").unwrap();

        let err = store.spotlight_stop("berlin").unwrap_err();

        assert!(err
            .to_string()
            .contains("repository root has changes outside the active Spotlight patch"));
        assert!(err.to_string().contains("root-only.txt"));
        assert_eq!(store.spotlight_status("berlin").unwrap().unwrap(), active);
        assert_eq!(
            fs::read_to_string(repo_path.join("README.md")).unwrap(),
            "spotlight change\n"
        );
        assert_eq!(
            fs::read_to_string(repo_path.join("root-only.txt")).unwrap(),
            "root edit\n"
        );
    }

    #[test]
    fn spotlight_root_conflict_paths_reports_extra_root_edits_without_stopping_session() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".conductor")).unwrap();
        fs::write(
            repo_path.join(".conductor/settings.toml"),
            r#"
spotlight_testing = true
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
                "enable spotlight",
            ])
            .status()
            .unwrap();

        let db_path = temp.path().join("state.db");
        let store = WorkspaceStore::open_with_logs(&db_path, temp.path().join("logs")).unwrap();
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

        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        fs::write(workspace.path.join("README.md"), "spotlight change\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args(["add", "README.md"])
            .status()
            .unwrap();
        let active = store.spotlight_start("berlin").unwrap();
        assert!(store
            .spotlight_root_conflict_paths("berlin")
            .unwrap()
            .is_empty());

        fs::write(repo_path.join("root-only.txt"), "root edit\n").unwrap();

        let paths = store.spotlight_root_conflict_paths("berlin").unwrap();

        assert_eq!(paths, vec!["root-only.txt".to_owned()]);
        assert_eq!(store.spotlight_status("berlin").unwrap().unwrap(), active);
    }

    #[test]
    fn spotlight_conflict_detail_prefers_root_only_paths_over_active_patch_paths() {
        let expected_patch = "\
diff --git a/README.md b/README.md
index 1111111..2222222 100644
--- a/README.md
+++ b/README.md
@@ -1 +1 @@
-old
+spotlight
";
        let current_patch = "\
diff --git a/README.md b/README.md
index 1111111..2222222 100644
--- a/README.md
+++ b/README.md
@@ -1 +1 @@
-old
+spotlight
diff --git a/root-only.txt b/root-only.txt
new file mode 100644
index 0000000..3333333
--- /dev/null
+++ b/root-only.txt
@@ -0,0 +1 @@
+root edit
";

        let detail = spotlight_conflict_detail(current_patch, expected_patch);

        assert!(detail.contains("root-only.txt"));
        assert!(!detail.contains("README.md"));
    }

    #[test]
    fn spotlight_repair_root_discards_root_only_edits_and_reapplies_active_patch() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".conductor")).unwrap();
        fs::write(
            repo_path.join(".conductor/settings.toml"),
            r#"
spotlight_testing = true
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
                "enable spotlight",
            ])
            .status()
            .unwrap();

        let db_path = temp.path().join("state.db");
        let store = WorkspaceStore::open_with_logs(&db_path, temp.path().join("logs")).unwrap();
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

        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        fs::write(workspace.path.join("README.md"), "spotlight change\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args(["add", "README.md"])
            .status()
            .unwrap();
        let active = store.spotlight_start("berlin").unwrap();
        fs::write(repo_path.join("root-only.txt"), "root edit\n").unwrap();

        let repaired = store.spotlight_repair_root("berlin").unwrap();

        assert_eq!(repaired.id, active.id);
        assert_eq!(
            fs::read_to_string(repo_path.join("README.md")).unwrap(),
            "spotlight change\n"
        );
        assert!(!repo_path.join("root-only.txt").exists());
        assert_eq!(
            git_output(&repo_path, ["diff", "--", "README.md"]),
            git_output(&workspace.path, ["diff", "--cached", "--", "README.md"])
        );
        assert_eq!(store.spotlight_stop("berlin").unwrap().status, "stopped");
        assert_eq!(
            fs::read_to_string(repo_path.join("README.md")).unwrap(),
            "demo\n"
        );
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
    fn session_launch_for_cursor_opens_workspace_path() {
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

        let launch = store.session_launch("berlin", SessionKind::Cursor).unwrap();

        assert_eq!(launch.program, PathBuf::from("cursor"));
        assert_eq!(
            launch.args,
            vec![workspace.path.to_string_lossy().to_string()]
        );
        assert_eq!(launch.cwd, workspace.path);
        assert_eq!(launch.env_value("CONDUCTOR_WORKSPACE_NAME"), Some("berlin"));
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
    fn stop_session_process_targets_explicit_session() {
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
            let stopped = store
                .stop_session_process("berlin", session.id)
                .expect("stop_session_process should mark a running session");
            assert_eq!(stopped.id, session.id);
            assert_eq!(stopped.status, ProcessStatus::Stopped);

            let idempotent = store
                .stop_session_process("berlin", session.id)
                .expect("stop_session_process should be idempotent");
            assert_eq!(idempotent.id, session.id);
            assert_eq!(idempotent.status, ProcessStatus::Stopped);
        });
    }

    #[test]
    fn reconcile_session_processes_marks_dead_process_as_exited() {
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
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        let launch = store.session_launch("berlin", SessionKind::Shell).unwrap();
        let process = store
            .record_session_process("berlin", &launch, u32::MAX)
            .expect("seed dead session record");

        let reconciled = store
            .reconcile_session_processes()
            .expect("reconcile should process dead records");
        let reconciled = reconciled
            .into_iter()
            .find(|entry| entry.id == process.id)
            .expect("dead session should be marked exited");
        assert_eq!(reconciled.id, process.id);
        assert_eq!(reconciled.status, ProcessStatus::Exited);
        assert!(reconciled.ended_at.is_some());
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
    fn diff_file_summaries_include_additions_and_deletions() {
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

        let summaries = store.diff_file_summaries("berlin").unwrap();

        assert_eq!(
            summaries,
            vec![
                DiffFileSummary {
                    path: "README.md".to_owned(),
                    additions: Some(1),
                    deletions: Some(0),
                },
                DiffFileSummary {
                    path: "notes.txt".to_owned(),
                    additions: Some(1),
                    deletions: Some(0),
                },
            ]
        );
    }

    #[test]
    fn review_comments_agent_prompt_includes_only_open_comments() {
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
        let open = store
            .add_review_comment("berlin", "src/lib.rs", Some(12), "handle empty input")
            .unwrap();
        let resolved = store
            .add_review_comment("berlin", "README.md", None, "clarify setup")
            .unwrap();
        store.resolve_review_comment(resolved.id).unwrap();

        let prompt = store.review_comments_agent_prompt("berlin").unwrap();

        assert!(prompt.contains("Address these open review comments for workspace berlin."));
        assert!(prompt.contains(&format!("- #{} src/lib.rs:12: handle empty input", open.id)));
        assert!(!prompt.contains("clarify setup"));
    }

    #[test]
    fn pull_request_checks_agent_prompt_includes_failing_checks() {
        let output = "\
build\tpass\t1m\thttps://github.com/example/demo/actions/1
lint\tfail\t30s\thttps://github.com/example/demo/actions/2
deploy\tpending\t10s\thttps://github.com/example/demo/actions/3
";

        let checks = parse_pull_request_check_runs(output);
        let prompt = format_pull_request_checks_agent_prompt("berlin", &checks);

        assert_eq!(checks.len(), 3);
        assert_eq!(checks.iter().filter(|check| check.is_failure()).count(), 1);
        assert!(prompt.contains("Fix these failing PR checks for workspace berlin."));
        assert!(prompt.contains("- lint: fail - https://github.com/example/demo/actions/2"));
        assert!(!prompt.contains("build"));
        assert!(!prompt.contains("deploy"));
    }

    #[test]
    fn pull_request_review_agent_prompt_wraps_github_comment_state() {
        let raw = "Reviewers: changes requested\nalice: please add a test\n";

        let prompt = format_pull_request_review_agent_prompt("berlin", raw);

        assert!(
            prompt.contains("Address this GitHub PR review/comment state for workspace berlin.")
        );
        assert!(prompt.contains("Reviewers: changes requested"));
        assert!(prompt.contains("alice: please add a test"));
    }

    #[test]
    fn merge_pull_request_blocks_open_review_comments() {
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
        store
            .record_pull_request(workspace.id, "https://github.com/example/demo/pull/42")
            .unwrap();
        store
            .add_review_comment("berlin", "src/lib.rs", Some(12), "fix edge case")
            .unwrap();

        let err = store.merge_pull_request("berlin", "squash").unwrap_err();

        assert!(err.to_string().contains("open review comment(s) remain"));
    }

    #[test]
    fn merge_and_maybe_archive_archives_when_repository_setting_is_enabled() {
        let _guard = env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".conductor")).unwrap();
        fs::write(
            repo_path.join(".conductor/settings.toml"),
            "[git]\narchive_on_merge = true\n",
        )
        .unwrap();
        let old_path = install_fake_gh(
            temp.path(),
            r#"#!/bin/sh
if [ "$1" = "pr" ] && [ "$2" = "merge" ]; then
  printf 'merged pull request\n'
  exit 0
fi
echo "unexpected gh args: $*" >&2
exit 1
"#,
        );
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
        store
            .record_pull_request(workspace.id, "https://github.com/example/demo/pull/42")
            .unwrap();

        let result = store
            .merge_and_maybe_archive_pull_request("berlin", "squash")
            .unwrap();

        restore_path(old_path);
        assert_eq!(result.merge_output.trim(), "merged pull request");
        assert_eq!(result.archived_workspace.unwrap().status, "archived");
        assert_eq!(store.get_by_name("berlin").unwrap().status, "archived");
    }

    #[test]
    fn revert_workspace_file_restores_tracked_changes() {
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
        assert!(store
            .changed_files("berlin")
            .unwrap()
            .contains(&"README.md".to_owned()));

        store.revert_workspace_file("berlin", "README.md").unwrap();

        assert_eq!(
            fs::read_to_string(workspace.path.join("README.md")).unwrap(),
            "demo\n"
        );
        assert!(!store
            .changed_files("berlin")
            .unwrap()
            .contains(&"README.md".to_owned()));
    }

    #[test]
    fn revert_workspace_file_rejects_untracked_paths() {
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

        fs::write(workspace.path.join("notes.txt"), "new\n").unwrap();

        let err = store
            .revert_workspace_file("berlin", "notes.txt")
            .unwrap_err()
            .to_string();

        assert!(err.contains("is not tracked in HEAD"));
        assert_eq!(
            fs::read_to_string(workspace.path.join("notes.txt")).unwrap(),
            "new\n"
        );
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
    fn empty_todos_are_rejected() {
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

        let err = store.add_todo("berlin", "   ").unwrap_err();

        assert!(err.to_string().contains("todo text is required"));
        assert!(store.list_todos("berlin").unwrap().is_empty());
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
    fn spotlight_rename_updates_watch_and_sync_works_with_stale_session_name() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".conductor")).unwrap();
        fs::write(
            repo_path.join(".conductor/settings.toml"),
            r#"
spotlight_testing = true
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
                "enable spotlight",
            ])
            .status()
            .unwrap();

        let db_path = temp.path().join("state.db");
        let store = WorkspaceStore::open_with_logs(&db_path, temp.path().join("logs")).unwrap();
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

        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        fs::write(workspace.path.join("README.md"), "first change\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args(["add", "README.md"])
            .status()
            .unwrap();

        let active = store.spotlight_start("berlin").unwrap();
        let renamed = store.rename("berlin", "oslo").unwrap();
        store
            .conn
            .execute(
                "UPDATE spotlight_sessions SET workspace_name = 'berlin' WHERE id = ?1",
                [active.id],
            )
            .unwrap();

        fs::write(renamed.path.join("README.md"), "second change\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&renamed.path)
            .args(["add", "README.md"])
            .status()
            .unwrap();

        let synced = store.spotlight_sync_active_sessions().unwrap();
        let targets = store.spotlight_watch_targets().unwrap();

        assert_eq!(synced.len(), 1);
        assert_eq!(synced[0].workspace_name, "oslo");
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].workspace_name, "oslo");
        assert_eq!(targets[0].workspace_path, renamed.path);
        assert_eq!(
            fs::read_to_string(repo_path.join("README.md")).unwrap(),
            "second change\n"
        );
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
    fn empty_checkpoint_messages_are_rejected() {
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

        let err = store.checkpoint_create("berlin", "   ", None).unwrap_err();

        assert!(err.to_string().contains("checkpoint message is required"));
        assert!(store.checkpoint_list("berlin").unwrap().is_empty());
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

    fn wait_for_process_status(
        store: &WorkspaceStore,
        workspace: &str,
        kind: ProcessKind,
        status: ProcessStatus,
    ) -> ProcessRecord {
        for _ in 0..50 {
            let records = match kind {
                ProcessKind::Setup => store.list_setups(workspace).unwrap(),
                ProcessKind::Run => store.list_runs(workspace).unwrap(),
                ProcessKind::Session => store.list_sessions(workspace).unwrap(),
                ProcessKind::Terminal => store.list_terminals(workspace).unwrap(),
            };
            if let Some(record) = records.into_iter().find(|record| record.status == status) {
                return record;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("timed out waiting for {kind:?} process to become {status:?}");
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
