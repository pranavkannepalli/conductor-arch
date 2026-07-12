use crate::codex_tui::{
    merge_message_content, parse_codex_screen_delta, CodexFileChangeAction, CodexParseBenchmark,
    CodexParseCursor, CodexParsedItem, CodexTranscriptEvent, ScreenMessageRole,
};
use crate::github_pr::{
    append_unique_checks, append_unique_deployments, extract_json_string_field,
    extract_pull_request_url, format_pull_request_checks_agent_prompt,
    format_pull_request_readiness, format_pull_request_readiness_agent_prompt,
    format_pull_request_review_agent_prompt, json_root_string, parse_github_commit_status_checks,
    parse_github_deployment_entries, parse_github_deployment_latest_status,
    parse_pull_request_check_runs, parse_pull_request_number, parse_pull_request_readiness,
    parse_pull_request_review_thread_mutation, parse_pull_request_review_threads,
};
use crate::harness;
use crate::linear::fetch_linear_issue;
use crate::local_chat::{
    local_chat_agent_type, parse_local_chat_transcript, session_events_to_local_chat_messages,
    truncate_chars,
};
use crate::session_event::{
    codex_parsed_item_to_session_event, SessionEvent, SessionEventPayload, SessionEventSource,
};
use crate::session_pipeline::{
    process_codex_pty_pipeline, PtyChunkInput, SessionPipelineInput, SessionPipelineOutput,
};
use crate::session_state::AgentSessionState;
use crate::settings::{
    ensure_repository_config, gitignore_pattern_key, load_repository_settings,
    save_local_default_agent_provider, RepositorySettings,
};
use crate::terminal_logs::{
    search_terminal_logs as search_terminal_logs_in_processes, summarize_terminal_sessions,
    terminal_log_preview,
};
use crate::todos::parse_context_todos;
use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Read;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::warn;
use uuid::Uuid;
use walkdir::WalkDir;

const ARCHDUCTOR_METADATA_OPEN: &str = "<archductor_metadata>";
const ARCHDUCTOR_METADATA_CLOSE: &str = "</archductor_metadata>";

pub use crate::github_pr::{
    parse_github_numbered_stateful_choices, GitHubNumberedChoice, PullRequestCheckRun,
    PullRequestCommentEntry, PullRequestDeployment, PullRequestReadiness, PullRequestReviewEntry,
    PullRequestReviewThread, PullRequestThreadComment,
};
pub use crate::local_chat::{
    LocalChatHistoryMessage, LocalChatHistorySummary, LocalChatThreadSummary,
};
pub use crate::terminal_logs::{TerminalLogMatch, TerminalSessionSummary};

const SIGTERM_EXIT_CODE: i32 = 143;
const UNTRACKED_FILE_COUNT_BYTE_LIMIT: usize = 1024 * 1024;
const DIFF_HUNK_PATCH_LIMIT_BYTES: usize = 200 * 1024;
const TURN_CHECKPOINT_DIFF_LIMIT: usize = 25;
const TURN_CHECKPOINT_DIFF_MAX_BYTES: usize = 64 * 1024;
const WORKSPACE_CITY_NAMES: [&str; 200] = [
    "berlin",
    "tokyo",
    "helsinki",
    "lisbon",
    "nairobi",
    "seoul",
    "oslo",
    "kyoto",
    "zurich",
    "sydney",
    "dublin",
    "vancouver",
    "london",
    "paris",
    "rome",
    "madrid",
    "barcelona",
    "amsterdam",
    "vienna",
    "prague",
    "budapest",
    "athens",
    "istanbul",
    "copenhagen",
    "stockholm",
    "edinburgh",
    "glasgow",
    "manchester",
    "liverpool",
    "birmingham",
    "brussels",
    "antwerp",
    "rotterdam",
    "hamburg",
    "munich",
    "frankfurt",
    "cologne",
    "milan",
    "naples",
    "florence",
    "venice",
    "turin",
    "bologna",
    "palermo",
    "valencia",
    "seville",
    "granada",
    "malaga",
    "porto",
    "warsaw",
    "krakow",
    "wroclaw",
    "gdansk",
    "bergen",
    "reykjavik",
    "tallinn",
    "riga",
    "vilnius",
    "ljubljana",
    "zagreb",
    "dubrovnik",
    "split",
    "belgrade",
    "sarajevo",
    "skopje",
    "sofia",
    "bucharest",
    "cluj-napoca",
    "chisinau",
    "tirana",
    "ankara",
    "izmir",
    "antalya",
    "tbilisi",
    "yerevan",
    "baku",
    "jerusalem",
    "tel-aviv",
    "haifa",
    "beirut",
    "amman",
    "cairo",
    "alexandria",
    "casablanca",
    "marrakesh",
    "rabat",
    "fes",
    "tunis",
    "algiers",
    "lagos",
    "accra",
    "dakar",
    "abidjan",
    "addis-ababa",
    "kampala",
    "kigali",
    "dar-es-salaam",
    "zanzibar",
    "johannesburg",
    "cape-town",
    "durban",
    "pretoria",
    "gaborone",
    "windhoek",
    "lusaka",
    "harare",
    "maputo",
    "luanda",
    "dubai",
    "abu-dhabi",
    "doha",
    "riyadh",
    "jeddah",
    "muscat",
    "manama",
    "kuwait-city",
    "tehran",
    "shiraz",
    "isfahan",
    "karachi",
    "lahore",
    "islamabad",
    "delhi",
    "mumbai",
    "bangalore",
    "kolkata",
    "chennai",
    "hyderabad",
    "pune",
    "jaipur",
    "udaipur",
    "varanasi",
    "agra",
    "kochi",
    "goa",
    "colombo",
    "kathmandu",
    "thimphu",
    "dhaka",
    "yangon",
    "bangkok",
    "chiang-mai",
    "phuket",
    "hanoi",
    "ho-chi-minh-city",
    "hoi-an",
    "da-nang",
    "singapore",
    "kuala-lumpur",
    "penang",
    "jakarta",
    "bandung",
    "yogyakarta",
    "bali",
    "surabaya",
    "manila",
    "cebu",
    "davao",
    "taipei",
    "kaohsiung",
    "hong-kong",
    "macau",
    "shanghai",
    "beijing",
    "guangzhou",
    "shenzhen",
    "chengdu",
    "chongqing",
    "xian",
    "hangzhou",
    "suzhou",
    "nanjing",
    "wuhan",
    "qingdao",
    "tianjin",
    "osaka",
    "nara",
    "kobe",
    "fukuoka",
    "sapporo",
    "hiroshima",
    "nagoya",
    "busan",
    "incheon",
    "jeju",
    "ulaanbaatar",
    "perth",
    "melbourne",
    "brisbane",
    "adelaide",
    "auckland",
    "wellington",
    "queenstown",
    "christchurch",
    "suva",
    "honolulu",
    "seattle",
    "portland",
    "san-francisco",
    "los-angeles",
];

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
pub struct ProjectRepositoryModel {
    pub id: i64,
    pub root_path: PathBuf,
    pub default_branch: String,
    pub remote_name: String,
    pub workspace_parent_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectModel {
    pub id: i64,
    pub name: String,
    pub repository: ProjectRepositoryModel,
    pub scripts: crate::settings::ScriptSettings,
    pub environment_variables: BTreeMap<String, String>,
    pub prompts: crate::settings::PromptSettings,
    pub workspace_ids: Vec<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceModelStatus {
    Active,
    Paused,
    Review,
    Merged,
    Archived,
    Failed,
}

impl WorkspaceModelStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Review => "review",
            Self::Merged => "merged",
            Self::Archived => "archived",
            Self::Failed => "failed",
        }
    }

    fn from_workspace_status(value: &str) -> Self {
        match value {
            "paused" => Self::Paused,
            "review" => Self::Review,
            "merged" => Self::Merged,
            "archived" => Self::Archived,
            "failed" => Self::Failed,
            _ => Self::Active,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceModel {
    pub workspace: Workspace,
    pub project_id: i64,
    pub branch: String,
    pub worktree_path: PathBuf,
    pub session_ids: Vec<i64>,
    pub checkpoint_ids: Vec<i64>,
    pub open_todos: usize,
    pub open_review_comments: usize,
    pub status: WorkspaceModelStatus,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSourcePreflight {
    pub github_cli_installed: bool,
    pub github_authenticated: bool,
    pub linear_api_key_set: bool,
}

impl WorkspaceSourcePreflight {
    pub fn github_ready(&self) -> bool {
        self.github_cli_installed && self.github_authenticated
    }

    pub fn linear_ready(&self) -> bool {
        self.linear_api_key_set
    }

    pub fn github_status(&self) -> &'static str {
        match (self.github_cli_installed, self.github_authenticated) {
            (true, true) => "ready",
            (false, _) => "gh missing",
            (true, false) => "gh auth required",
        }
    }

    pub fn linear_status(&self) -> &'static str {
        if self.linear_api_key_set {
            "ready"
        } else {
            "LINEAR_API_KEY missing"
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
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
    pub harness_metadata: Option<String>,
    pub session_resume_id: Option<String>,
}

impl SessionLaunch {
    pub fn env_value(&self, key: &str) -> Option<&str> {
        self.env
            .iter()
            .find(|(name, _)| name == key)
            .and_then(|(_, value)| value.to_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
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
                "ARCHDUCTOR_SESSION_PLAN_MODE".to_owned(),
                OsString::from("true"),
            ));
        }
        if self.fast_mode {
            env.push((
                "ARCHDUCTOR_SESSION_FAST_MODE".to_owned(),
                OsString::from("true"),
            ));
        }
        if let Some(value) = sanitize_empty_text(self.approval_mode.as_deref()) {
            env.push((
                "ARCHDUCTOR_SESSION_APPROVAL_MODE".to_owned(),
                OsString::from(value),
            ));
        }
        if let Some(value) = sanitize_empty_text(self.reasoning_mode.as_deref()) {
            env.push((
                "ARCHDUCTOR_SESSION_REASONING_MODE".to_owned(),
                OsString::from(value),
            ));
        }
        if let Some(value) = sanitize_empty_text(self.effort_mode.as_deref()) {
            env.push((
                "ARCHDUCTOR_SESSION_EFFORT_MODE".to_owned(),
                OsString::from(value),
            ));
        }
        if let Some(value) = sanitize_empty_text(self.codex_personality.as_deref()) {
            env.push((
                "ARCHDUCTOR_SESSION_CODEX_PERSONALITY".to_owned(),
                OsString::from(value),
            ));
        }
        if let Some(value) = sanitize_empty_text(self.codex_goals.as_deref()) {
            env.push((
                "ARCHDUCTOR_SESSION_CODEX_GOALS".to_owned(),
                OsString::from(value),
            ));
        }
        if let Some(value) = sanitize_empty_text(self.codex_skills.as_deref()) {
            env.push((
                "ARCHDUCTOR_SESSION_CODEX_SKILLS".to_owned(),
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

    pub fn from_metadata(metadata: Option<&str>) -> Self {
        let mut options = Self::default();
        let Some(metadata) = metadata else {
            return options;
        };

        for entry in metadata.split(';') {
            let Some((key, value)) = entry.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            match key {
                "plan" => options.plan_mode = value.eq_ignore_ascii_case("true"),
                "fast" => options.fast_mode = value.eq_ignore_ascii_case("true"),
                "approval" | "approvals" => {
                    options.approval_mode = (!value.is_empty()).then(|| value.to_owned());
                }
                "reasoning" => {
                    options.reasoning_mode = (!value.is_empty()).then(|| value.to_owned());
                }
                "effort" => {
                    options.effort_mode = (!value.is_empty()).then(|| value.to_owned());
                }
                "personality" => {
                    options.codex_personality = (!value.is_empty()).then(|| value.to_owned());
                }
                "goals" => {
                    options.codex_goals = (!value.is_empty()).then(|| value.to_owned());
                }
                "skills" => {
                    options.codex_skills = (!value.is_empty()).then(|| value.to_owned());
                }
                _ => {}
            }
        }

        options
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

fn env_key_fragment(value: &str) -> String {
    let fragment = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    if fragment.is_empty() {
        "WORKSPACE".to_owned()
    } else {
        fragment
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessKind {
    Setup,
    Run,
    Check,
    Session,
    Terminal,
}

impl ProcessKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Run => "run",
            Self::Check => "check",
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
    pub chat_thread_id: Option<i64>,
    pub kind: ProcessKind,
    pub command: String,
    pub pid: u32,
    pub log_path: PathBuf,
    pub status: ProcessStatus,
    pub started_at: String,
    pub exit_code: Option<i32>,
    pub ended_at: Option<String>,
    pub session_harness_metadata: Option<String>,
    pub session_resume_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtyChunkRecord {
    pub id: i64,
    pub process_id: i64,
    pub sequence: u64,
    pub occurred_at_ms: u64,
    pub stream: String,
    pub text: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatThreadRecord {
    pub id: i64,
    pub workspace_id: i64,
    pub provider: String,
    pub title: String,
    pub status: String,
    pub native_thread_id: Option<String>,
    pub harness_metadata: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessageRecord {
    pub id: i64,
    pub thread_id: i64,
    pub role: String,
    pub content: String,
    pub source: String,
    pub timeline_seq: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatEventRecord {
    pub id: i64,
    pub thread_id: i64,
    pub process_id: Option<i64>,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub path: Option<String>,
    pub payload_json: String,
    pub timeline_seq: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatThreadContextSummary {
    pub title: String,
    pub provider: String,
    pub message_count: usize,
    pub event_count: usize,
    pub transcript_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedDirectory {
    pub id: i64,
    pub workspace_id: i64,
    pub workspace_name: String,
    pub workspace_path: PathBuf,
    pub target_workspace_id: i64,
    pub target_workspace_name: String,
    pub target_workspace_path: PathBuf,
    pub link_path: PathBuf,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkspaceViewDefaults {
    pub default_visible_tab: Option<String>,
    pub theme: Option<String>,
    pub accent_color: Option<String>,
    pub colors: std::collections::BTreeMap<String, String>,
    pub density: Option<String>,
    pub keybindings: Option<String>,
    pub terminal_font: Option<String>,
    pub terminal_scrollback: Option<u32>,
    pub command_palette_presets: Vec<String>,
    pub agent_profile_names: Vec<String>,
    pub notification_rules: Vec<String>,
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

fn repository_command_palette_presets(settings: &RepositorySettings) -> Vec<String> {
    let mut presets = Vec::new();
    push_script_command_preset(
        &mut presets,
        "Test",
        settings.scripts.test.as_deref().or(settings
            .customization
            .automation
            .test_command
            .as_deref()),
    );
    push_script_command_preset(
        &mut presets,
        "Lint",
        settings.scripts.lint.as_deref().or(settings
            .customization
            .automation
            .lint_command
            .as_deref()),
    );
    push_script_command_preset(
        &mut presets,
        "Typecheck",
        settings.scripts.typecheck.as_deref().or(settings
            .customization
            .automation
            .typecheck_command
            .as_deref()),
    );
    push_script_command_preset(
        &mut presets,
        "Build",
        settings.scripts.build.as_deref().or(settings
            .customization
            .automation
            .build_command
            .as_deref()),
    );

    presets.extend(settings.customization.view.command_palette_presets.clone());
    presets
}

fn push_script_command_preset(presets: &mut Vec<String>, label: &str, command: Option<&str>) {
    if let Some(command) = command.map(str::trim).filter(|command| !command.is_empty()) {
        presets.push(format!("{label}={command}"));
    }
}

fn configured_check_commands_from_settings(
    settings: &RepositorySettings,
) -> Vec<ConfiguredCheckCommand> {
    [
        (
            "test",
            "Test",
            settings.scripts.test.as_deref().or(settings
                .customization
                .automation
                .test_command
                .as_deref()),
        ),
        (
            "lint",
            "Lint",
            settings.scripts.lint.as_deref().or(settings
                .customization
                .automation
                .lint_command
                .as_deref()),
        ),
        (
            "typecheck",
            "Typecheck",
            settings.scripts.typecheck.as_deref().or(settings
                .customization
                .automation
                .typecheck_command
                .as_deref()),
        ),
        (
            "build",
            "Build",
            settings.scripts.build.as_deref().or(settings
                .customization
                .automation
                .build_command
                .as_deref()),
        ),
    ]
    .into_iter()
    .filter_map(|(key, label, command)| {
        command
            .map(str::trim)
            .filter(|command| !command.is_empty())
            .map(|command| ConfiguredCheckCommand {
                key: key.to_owned(),
                label: label.to_owned(),
                command: command.to_owned(),
            })
    })
    .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffFileSummary {
    pub path: String,
    pub additions: Option<usize>,
    pub deletions: Option<usize>,
    pub staged: bool,
    pub unstaged: bool,
    pub untracked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffHunkSummary {
    pub index: usize,
    pub header: String,
    pub additions: usize,
    pub deletions: usize,
    pub staged: bool,
    pub unsupported_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceTimelineEvent {
    pub id: i64,
    pub workspace_id: i64,
    pub workspace_name: String,
    pub kind: String,
    pub summary: String,
    pub created_at: String,
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
pub struct PullRequestPanelState {
    pub pull_request: Option<PullRequest>,
    pub readiness: Option<PullRequestReadiness>,
    pub readiness_text: String,
    pub review_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestTemplate {
    pub title: String,
    pub body: String,
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
    pub check_status: Option<ProcessStatus>,
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
pub struct ConfiguredCheckCommand {
    pub key: String,
    pub label: String,
    pub command: String,
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
    pub diff_additions: usize,
    pub diff_deletions: usize,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnCheckpointDiff {
    pub checkpoint: Checkpoint,
    pub end_checkpoint: Option<Checkpoint>,
    pub diff: String,
    pub truncated: bool,
}

struct RepositoryRecord {
    id: i64,
    root_path: PathBuf,
    default_branch: String,
    remote_name: String,
    workspace_parent_path: PathBuf,
}

struct LocalChatHistoryRow {
    process: ProcessRecord,
    repository_name: String,
    workspace_name: String,
    workspace_path: PathBuf,
}

struct LocalChatThreadRow {
    thread: ChatThreadRecord,
    repository_name: String,
    workspace_name: String,
    workspace_path: PathBuf,
}

struct ProcessSessionMetadata<'a> {
    harness_metadata: Option<&'a str>,
    resume_id: Option<&'a str>,
}

struct RecordProcessInput<'a> {
    kind: ProcessKind,
    workspace: &'a Workspace,
    chat_thread_id: Option<i64>,
    command: &'a str,
    pid: u32,
    file_prefix: &'a str,
    session: ProcessSessionMetadata<'a>,
}

struct StartProcessInput<'a> {
    kind: ProcessKind,
    script: &'a str,
    settings: &'a crate::settings::RepositorySettings,
    repository: &'a RepositoryRecord,
    workspace: &'a Workspace,
    chat_thread_id: Option<i64>,
    extra_env: &'a [(String, OsString)],
    session: ProcessSessionMetadata<'a>,
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

    pub(crate) fn owns_process_log_path(&self, log_path: &Path) -> bool {
        log_path.starts_with(&self.logs_dir)
    }

    pub fn create(&self, input: CreateWorkspace) -> Result<Workspace> {
        self.create_with_progress(input, || {})
    }

    pub fn create_with_progress(
        &self,
        input: CreateWorkspace,
        after_insert: impl FnOnce(),
    ) -> Result<Workspace> {
        let repository = self.load_repository(&input.repository_name)?;
        ensure_repository_config(&repository.root_path)?;
        let settings = load_repository_settings(&repository.root_path)?;
        let name = self.resolve_workspace_name(&repository, &settings, &input.name)?;
        validate_workspace_name(&name)?;
        let branch = self.resolve_workspace_branch(&settings, &input.branch, &name);
        let remote_available = remote_exists(&repository.root_path, &repository.remote_name);
        let default_base_branch = settings
            .customization
            .workspace_defaults
            .base_branch
            .as_deref()
            .unwrap_or(&repository.default_branch)
            .to_owned();
        let base_ref = if let Some(base_ref) = input.base_ref {
            base_ref
        } else if remote_available {
            sync_repository_default_branch(
                &repository.root_path,
                &repository.remote_name,
                &default_base_branch,
            )?
        } else {
            default_base_branch
        };

        let path = repository.workspace_parent_path.join(&name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create workspace parent {}", parent.display()))?;
        }
        let port_block_size = settings
            .customization
            .workspace_defaults
            .port_block_size
            .unwrap_or(10);
        let port_base = self.next_port_base(port_block_size)?;
        let now = timestamp();
        self.conn.execute(
            "INSERT INTO workspaces (
                repository_id, name, path, branch, base_ref, port_base, status, archived_at, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'creating', NULL, ?7, ?8)",
            params![
                repository.id,
                name,
                path.to_string_lossy().to_string(),
                branch,
                base_ref,
                i64::from(port_base),
                now,
                now,
            ],
        )?;
        let workspace = self.get_by_path(&path)?;
        self.record_workspace_event(
            workspace.id,
            &workspace.name,
            "workspace.creating",
            &format!("Creating workspace on branch {}", workspace.branch),
        )?;
        after_insert();

        let create_result = (|| -> Result<()> {
            git_dynamic(
                &repository.root_path,
                &[
                    "worktree",
                    "add",
                    "-b",
                    workspace.branch.as_str(),
                    workspace.path.to_string_lossy().as_ref(),
                    workspace.base_ref.as_str(),
                ],
            )?;
            std::fs::create_dir_all(workspace.path.join(".context")).with_context(|| {
                format!(
                    "create workspace context directory {}",
                    workspace.path.display()
                )
            })?;
            initialize_context_files(&workspace.path, &settings)?;
            copy_included_ignored_files(&repository.root_path, &workspace.path)?;

            let auto_setup = settings
                .customization
                .automation
                .auto_setup
                .unwrap_or(false);
            if auto_setup {
                self.setup_workspace(&workspace.name)?;
            }
            Ok(())
        })();

        if let Err(err) = create_result {
            let _ = self.mark_workspace_status(workspace.id, "failed");
            let _ = self.record_workspace_event(
                workspace.id,
                &workspace.name,
                "workspace.create_failed",
                &format!("Workspace creation failed: {err:#}"),
            );
            return Err(err);
        }

        self.mark_workspace_status(workspace.id, "active")?;
        let workspace = self.get_by_id(workspace.id)?;
        self.record_workspace_event(
            workspace.id,
            &workspace.name,
            "workspace.created",
            &format!("Created workspace on branch {}", workspace.branch),
        )?;
        Ok(workspace)
    }

    fn mark_workspace_status(&self, workspace_id: i64, status: &str) -> Result<()> {
        let now = timestamp();
        self.conn.execute(
            "UPDATE workspaces SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status, now, workspace_id],
        )?;
        Ok(())
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

    pub fn project_model(&self, name: &str) -> Result<ProjectModel> {
        let repository = self.load_repository(name)?;
        let settings = load_repository_settings(&repository.root_path)?;
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM workspaces WHERE repository_id = ?1 ORDER BY id")?;
        let workspace_ids = stmt
            .query_map([repository.id], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(ProjectModel {
            id: repository.id,
            name: name.to_owned(),
            repository: ProjectRepositoryModel {
                id: repository.id,
                root_path: repository.root_path,
                default_branch: repository.default_branch,
                remote_name: repository.remote_name,
                workspace_parent_path: repository.workspace_parent_path,
            },
            scripts: settings.scripts,
            environment_variables: settings.environment_variables.into_iter().collect(),
            prompts: settings.prompts.unwrap_or_default(),
            workspace_ids,
        })
    }

    pub fn workspace_model(&self, name: &str) -> Result<WorkspaceModel> {
        let workspace = self.get_by_name(name)?;
        let session_ids = self.ids_for_workspace_kind(workspace.id, ProcessKind::Session)?;
        let checkpoint_ids = self.ids_for_table(
            "SELECT id FROM checkpoints WHERE workspace_id = ?1 ORDER BY id",
            workspace.id,
        )?;
        let open_todos = self.count_workspace_rows(
            "SELECT COUNT(*) FROM todos WHERE workspace_id = ?1 AND status = 'open'",
            workspace.id,
        )?;
        let open_review_comments = self.count_workspace_rows(
            "SELECT COUNT(*) FROM review_comments WHERE workspace_id = ?1 AND status = 'open'",
            workspace.id,
        )?;

        Ok(WorkspaceModel {
            project_id: workspace.repository_id,
            branch: workspace.branch.clone(),
            worktree_path: workspace.path.clone(),
            session_ids,
            checkpoint_ids,
            open_todos,
            open_review_comments,
            status: WorkspaceModelStatus::from_workspace_status(&workspace.status),
            created_at: workspace.created_at.clone(),
            updated_at: workspace.updated_at.clone(),
            workspace,
        })
    }

    fn ids_for_workspace_kind(&self, workspace_id: i64, kind: ProcessKind) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM processes WHERE workspace_id = ?1 AND kind = ?2 ORDER BY id",
        )?;
        let ids = stmt
            .query_map(params![workspace_id, kind.as_str()], |row| {
                row.get::<_, i64>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    }

    fn ids_for_table(&self, query: &str, workspace_id: i64) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare(query)?;
        let ids = stmt
            .query_map([workspace_id], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    }

    fn count_workspace_rows(&self, query: &str, workspace_id: i64) -> Result<usize> {
        let count = self
            .conn
            .query_row(query, [workspace_id], |row| row.get::<_, i64>(0))?;
        Ok(count as usize)
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
            let (diff_additions, diff_deletions) = if workspace.status == "active" {
                workspace_diff_stats_against_base(&workspace).unwrap_or_default()
            } else {
                (0, 0)
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
                diff_additions,
                diff_deletions,
            });
        }
        Ok(lines)
    }

    pub fn rename(&self, name: &str, new_name: &str) -> Result<Workspace> {
        validate_workspace_name(new_name)?;
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        anyhow::ensure!(
            self.workspace_name_available_for_rename(&repository, workspace.id, new_name)?,
            "workspace {new_name} already exists"
        );

        let now = timestamp();
        let changed = self.conn.execute(
            "UPDATE workspaces SET name = ?1, updated_at = ?2 WHERE id = ?3",
            params![new_name, now, workspace.id],
        )?;
        self.conn.execute(
            "UPDATE spotlight_sessions SET workspace_name = ?1 WHERE workspace_id = ?2",
            params![new_name, workspace.id],
        )?;
        anyhow::ensure!(changed > 0, "workspace {name} not found");
        let renamed = self.get_by_name(new_name)?;
        self.record_workspace_event(
            renamed.id,
            &renamed.name,
            "workspace.renamed",
            &format!("Renamed workspace from {name} to {new_name}"),
        )?;
        Ok(renamed)
    }

    pub fn apply_first_message_workspace_naming(
        &self,
        name: &str,
        message: &str,
    ) -> Result<Option<Workspace>> {
        let message = message.trim();
        if message.is_empty() {
            return Ok(None);
        }

        let workspace = self.get_by_name(name)?;
        if self.workspace_has_chat_messages(workspace.id)? {
            return Ok(None);
        }

        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let settings = load_repository_settings(&repository.root_path)?;
        let base = slugify(message);
        let workspace_name =
            self.unique_message_workspace_name(&repository, workspace.id, &base)?;
        let prefix = settings
            .customization
            .workspace_defaults
            .branch_prefix
            .as_deref()
            .unwrap_or("lc");
        let branch_base = format!("{prefix}/{base}");
        let branch =
            unique_message_branch_name(&repository.root_path, &branch_base, &workspace.branch)?;

        let mut updated = workspace;
        if updated.branch != branch {
            updated = self.rename_branch(&updated.name, &branch)?;
        }
        if updated.name != workspace_name {
            updated = self.rename(&updated.name, &workspace_name)?;
        }
        Ok(Some(updated))
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

    pub fn delete(
        &self,
        name: &str,
        remove_worktree: bool,
        delete_branch: bool,
    ) -> Result<Workspace> {
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;

        self.stop_workspace_processes(workspace.id)?;

        if remove_worktree {
            remove_workspace_worktree(&repository.root_path, &workspace.path)?;
        }

        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<()> {
            self.record_workspace_event(
                workspace.id,
                &workspace.name,
                "workspace.deleted",
                "Deleted workspace metadata",
            )?;
            self.delete_workspace_rows(workspace.id)?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn.execute_batch("COMMIT")?;
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                return Err(err);
            }
        }

        if delete_branch {
            let _ = git_dynamic(
                &repository.root_path,
                &["branch", "-D", workspace.branch.as_str()],
            );
        }

        Ok(workspace)
    }

    pub fn cleanup_deleted_workspace_artifacts(
        &self,
        workspace: &Workspace,
        remove_worktree: bool,
        delete_branch: bool,
    ) -> Result<()> {
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        if remove_worktree {
            remove_workspace_worktree(&repository.root_path, &workspace.path)?;
        }
        if delete_branch {
            let _ = git_dynamic(
                &repository.root_path,
                &["branch", "-D", workspace.branch.as_str()],
            );
        }
        Ok(())
    }

    pub fn archive(&self, name: &str, remove_worktree: bool) -> Result<Workspace> {
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let settings = load_repository_settings(&repository.root_path)?;

        self.stop_workspace_processes(workspace.id)?;

        if let Some(archive_script) = &settings.scripts.archive {
            if workspace.path.exists() {
                run_shell_script(
                    archive_script,
                    &settings,
                    &repository,
                    &workspace,
                    &self.linked_directory_env(&workspace)?,
                )?;
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
        let archived = self.get_by_name(name)?;
        self.record_workspace_event(
            archived.id,
            &archived.name,
            "workspace.archived",
            "Archived workspace",
        )?;

        Ok(archived)
    }

    fn stop_workspace_processes(&self, workspace_id: i64) -> Result<()> {
        let mut stmt = self.conn.prepare(
            "SELECT id, workspace_id, chat_thread_id, kind, command, pid, log_path, status, started_at, exit_code, ended_at, session_harness_metadata, session_resume_id
             FROM processes
             WHERE workspace_id = ?1 AND status = ?2",
        )?;
        let processes = stmt
            .query_map(
                params![workspace_id, ProcessStatus::Running.as_str()],
                row_to_process,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for process in processes {
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
            self.record_workspace_event(
                workspace_id,
                &self.workspace_name_by_id(workspace_id)?,
                "session.stopped",
                &format!("Stopped session process #{}", process.id),
            )?;
        }
        Ok(())
    }

    fn delete_workspace_rows(&self, workspace_id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM chat_events
             WHERE process_id IN (SELECT id FROM processes WHERE workspace_id = ?1)",
            [workspace_id],
        )?;
        self.conn.execute(
            "DELETE FROM codex_parse_cursors
             WHERE process_id IN (SELECT id FROM processes WHERE workspace_id = ?1)",
            [workspace_id],
        )?;
        self.conn.execute(
            "DELETE FROM checkpoints WHERE workspace_id = ?1 OR session_id IN (
               SELECT id FROM processes WHERE workspace_id = ?1
             )",
            [workspace_id],
        )?;
        self.conn.execute(
            "DELETE FROM processes WHERE workspace_id = ?1",
            [workspace_id],
        )?;
        self.conn.execute(
            "DELETE FROM chat_messages
             WHERE thread_id IN (SELECT id FROM chat_threads WHERE workspace_id = ?1)",
            [workspace_id],
        )?;
        self.conn.execute(
            "DELETE FROM chat_threads WHERE workspace_id = ?1",
            [workspace_id],
        )?;
        self.conn.execute(
            "DELETE FROM linked_directories WHERE workspace_id = ?1 OR target_workspace_id = ?1",
            [workspace_id],
        )?;
        self.conn.execute(
            "DELETE FROM pull_requests WHERE workspace_id = ?1",
            [workspace_id],
        )?;
        self.conn
            .execute("DELETE FROM todos WHERE workspace_id = ?1", [workspace_id])?;
        self.conn.execute(
            "DELETE FROM review_comments WHERE workspace_id = ?1",
            [workspace_id],
        )?;
        self.conn.execute(
            "DELETE FROM spotlight_sessions WHERE workspace_id = ?1",
            [workspace_id],
        )?;
        let changed = self
            .conn
            .execute("DELETE FROM workspaces WHERE id = ?1", [workspace_id])?;
        anyhow::ensure!(changed > 0, "workspace id {workspace_id} not found");
        Ok(())
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
        let restored = self.get_by_name(name)?;
        self.record_workspace_event(
            restored.id,
            &restored.name,
            "workspace.restored",
            "Restored workspace",
        )?;
        Ok(restored)
    }

    pub fn duplicate(
        &self,
        name: &str,
        new_name: &str,
        new_branch: Option<&str>,
    ) -> Result<Workspace> {
        validate_workspace_name(new_name)?;
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let branch = new_branch
            .map(str::trim)
            .filter(|branch| !branch.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{}-copy", workspace.branch));
        validate_branch_name(&branch)?;
        let duplicated = self.create(CreateWorkspace {
            repository_name: self.repository_name_by_id(repository.id)?,
            name: new_name.to_owned(),
            branch,
            base_ref: Some(workspace.branch.clone()),
        })?;
        self.record_workspace_event(
            duplicated.id,
            &duplicated.name,
            "workspace.duplicated",
            &format!("Duplicated workspace {name}"),
        )?;
        Ok(duplicated)
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

        self.start_process(StartProcessInput {
            kind: ProcessKind::Run,
            script: run,
            settings: &settings,
            repository: &repository,
            workspace: &workspace,
            chat_thread_id: None,
            extra_env: &[],
            session: ProcessSessionMetadata {
                harness_metadata: None,
                resume_id: None,
            },
        })
    }

    pub fn setup_workspace(&self, name: &str) -> Result<ProcessRecord> {
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let settings = load_repository_settings(&repository.root_path)?;
        let Some(setup) = &settings.scripts.setup else {
            anyhow::bail!("workspace {name} has no scripts.setup configured");
        };

        self.start_process(StartProcessInput {
            kind: ProcessKind::Setup,
            script: setup,
            settings: &settings,
            repository: &repository,
            workspace: &workspace,
            chat_thread_id: None,
            extra_env: &[],
            session: ProcessSessionMetadata {
                harness_metadata: None,
                resume_id: None,
            },
        })
    }

    pub fn configured_check_commands(&self, name: &str) -> Result<Vec<ConfiguredCheckCommand>> {
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let settings = load_repository_settings(&repository.root_path)?;
        Ok(configured_check_commands_from_settings(&settings))
    }

    pub fn run_workspace_check(&self, name: &str, key: &str) -> Result<ProcessRecord> {
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let settings = load_repository_settings(&repository.root_path)?;
        let check = configured_check_commands_from_settings(&settings)
            .into_iter()
            .find(|check| check.key == key)
            .with_context(|| format!("workspace {name} has no configured {key} check"))?;

        self.start_process(StartProcessInput {
            kind: ProcessKind::Check,
            script: &check.command,
            settings: &settings,
            repository: &repository,
            workspace: &workspace,
            chat_thread_id: None,
            extra_env: &[],
            session: ProcessSessionMetadata {
                harness_metadata: None,
                resume_id: None,
            },
        })
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

    pub fn read_latest_check_log(&self, name: &str) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        let process = self.latest_process(workspace.id, ProcessKind::Check)?;
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
        let process = self.get_process(process_id)?;
        self.record_workspace_event(
            process.workspace_id,
            &self.workspace_name_by_id(process.workspace_id)?,
            "session.stopped",
            &format!("Stopped session process #{}", process.id),
        )?;
        Ok(process)
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
        let process = self.get_process(process_id)?;
        self.record_workspace_event(
            process.workspace_id,
            &self.workspace_name_by_id(process.workspace_id)?,
            "session.exited",
            &format!("Session process #{} exited", process.id),
        )?;
        Ok(process)
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
            .with_context(|| format!("write log {}", process.log_path.display()))?;
        Ok(())
    }

    pub fn append_pty_chunk(
        &self,
        process_id: i64,
        stream: &str,
        text: &str,
    ) -> Result<PtyChunkRecord> {
        if text.is_empty() {
            anyhow::bail!("pty chunk text is empty");
        }
        let stream = stream.trim();
        anyhow::ensure!(!stream.is_empty(), "pty chunk stream is required");
        let process = self.get_process(process_id)?;
        anyhow::ensure!(
            process.kind == ProcessKind::Session,
            "process {process_id} is not a session process"
        );
        let sequence = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) + 1 FROM pty_chunks WHERE process_id = ?1",
                [process_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(1);
        let occurred_at_ms = timestamp_millis() as i64;
        let now = timestamp();
        self.conn.execute(
            "INSERT INTO pty_chunks (
                process_id, sequence, occurred_at_ms, stream, text, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![process_id, sequence, occurred_at_ms, stream, text, now],
        )?;
        self.get_pty_chunk(self.conn.last_insert_rowid())
    }

    pub fn list_pty_chunks(&self, process_id: i64) -> Result<Vec<PtyChunkRecord>> {
        let process = self.get_process(process_id)?;
        anyhow::ensure!(
            process.kind == ProcessKind::Session,
            "process {process_id} is not a session process"
        );
        let mut stmt = self.conn.prepare(
            "SELECT id, process_id, sequence, occurred_at_ms, stream, text, created_at
             FROM pty_chunks
             WHERE process_id = ?1
             ORDER BY sequence ASC",
        )?;
        let rows = stmt.query_map([process_id], row_to_pty_chunk)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn get_pty_chunk(&self, id: i64) -> Result<PtyChunkRecord> {
        self.conn
            .query_row(
                "SELECT id, process_id, sequence, occurred_at_ms, stream, text, created_at
                 FROM pty_chunks
                 WHERE id = ?1",
                [id],
                row_to_pty_chunk,
            )
            .with_context(|| format!("load PTY chunk {id}"))
    }

    pub fn append_session_events(
        &self,
        process_id: i64,
        events: Vec<SessionEvent>,
    ) -> Result<Vec<SessionEvent>> {
        if events.is_empty() {
            return Ok(Vec::new());
        }
        let process = self.get_process(process_id)?;
        anyhow::ensure!(
            process.kind == ProcessKind::Session,
            "process {process_id} is not a session process"
        );
        let next_sequence = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) + 1 FROM session_events WHERE process_id = ?1",
                [process_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(1) as u64;
        let occurred_at_ms = timestamp_millis();
        let now = timestamp();
        let mut saved = Vec::with_capacity(events.len());

        for (offset, event) in events.into_iter().enumerate() {
            let sequence = next_sequence + offset as u64;
            let occurred_at_ms = event.occurred_at_ms.unwrap_or(occurred_at_ms);
            let event = event
                .with_sequence(sequence)
                .with_occurred_at_ms(occurred_at_ms);
            let payload_json = serde_json::to_string(&event.payload)?;
            self.conn.execute(
                "INSERT INTO session_events (
                    process_id, sequence, occurred_at_ms, source, raw_text, payload_json, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    process_id,
                    sequence as i64,
                    occurred_at_ms as i64,
                    session_event_source_to_str(event.source),
                    event.raw_text,
                    payload_json,
                    now,
                ],
            )?;
            saved.push(event);
        }

        Ok(saved)
    }

    pub fn list_session_events(&self, process_id: i64) -> Result<Vec<SessionEvent>> {
        let process = self.get_process(process_id)?;
        anyhow::ensure!(
            process.kind == ProcessKind::Session,
            "process {process_id} is not a session process"
        );
        let mut stmt = self.conn.prepare(
            "SELECT sequence, occurred_at_ms, source, raw_text, payload_json
             FROM session_events
             WHERE process_id = ?1
             ORDER BY sequence",
        )?;
        let rows = stmt.query_map([process_id], |row| {
            let source = session_event_source_from_str(row.get::<_, String>(2)?.as_str())?;
            let payload_json: String = row.get(4)?;
            let payload: SessionEventPayload = serde_json::from_str(&payload_json)
                .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
            Ok(SessionEvent {
                sequence: Some(row.get::<_, i64>(0)? as u64),
                occurred_at_ms: Some(row.get::<_, i64>(1)? as u64),
                source,
                raw_text: row.get(3)?,
                payload,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn reconcile_session_processes(&self) -> Result<Vec<ProcessRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, workspace_id, chat_thread_id, kind, command, pid, log_path, status, started_at, exit_code, ended_at, session_harness_metadata, session_resume_id
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
        self.record_process(RecordProcessInput {
            kind: ProcessKind::Terminal,
            workspace: &workspace,
            chat_thread_id: None,
            command,
            pid,
            file_prefix: "terminal",
            session: ProcessSessionMetadata {
                harness_metadata: None,
                resume_id: None,
            },
        })
    }

    fn record_process(&self, input: RecordProcessInput<'_>) -> Result<ProcessRecord> {
        let RecordProcessInput {
            kind,
            workspace,
            chat_thread_id,
            command,
            pid,
            file_prefix,
            session,
        } = input;
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
                workspace_id, chat_thread_id, kind, command, pid, log_path, status, started_at, exit_code, ended_at, session_harness_metadata, session_resume_id
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, ?9, ?10)",
            params![
                workspace.id,
                chat_thread_id,
                kind.as_str(),
                command,
                i64::from(pid),
                log_path.to_string_lossy().to_string(),
                ProcessStatus::Running.as_str(),
                now,
                session.harness_metadata,
                session.resume_id,
            ],
        )?;
        let process = self.get_process(self.conn.last_insert_rowid())?;
        if process.kind == ProcessKind::Session {
            self.record_workspace_event(
                process.workspace_id,
                &workspace.name,
                "session.started",
                &format!("Started {} session process #{}", command, process.id),
            )?;
        }
        Ok(process)
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
        stop_process(process.pid)?;
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
        search_terminal_logs_in_processes(self.list_terminals(name)?, query)
    }

    pub fn list_terminal_summaries(&self, name: &str) -> Result<Vec<TerminalSessionSummary>> {
        summarize_terminal_sessions(self.list_terminals(name)?)
    }

    pub fn reconcile_terminal_processes(&self) -> Result<Vec<ProcessRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, workspace_id, chat_thread_id, kind, command, pid, log_path, status, started_at, exit_code, ended_at, session_harness_metadata, session_resume_id
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
        let cwd = workspace_working_directory(&settings, &workspace)?;
        let started_at = timestamp();
        let mut env = conductor_environment(&settings, &repository, &workspace)?;
        env.extend(self.linked_directory_env(&workspace)?);
        let output = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&cwd)
            .envs(env)
            .stdin(Stdio::null())
            .output()
            .with_context(|| format!("run terminal command in {}", cwd.display()))?;
        let ended_at = timestamp();

        Ok(TerminalCommandResult {
            command: command.to_owned(),
            cwd,
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
        let current_patch = spotlight_meaningful_patch(&current_patch);
        let expected_patch = spotlight_meaningful_patch(&expected_patch);
        let conflict_paths = spotlight_conflict_paths(&current_patch, &expected_patch);
        if current_patch.trim() == expected_patch.trim() {
            return Ok(Vec::new());
        }
        Ok(conflict_paths.into_iter().collect())
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

    pub fn staged_diff(&self, name: &str, path: Option<&Path>) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        if let Some(path) = path {
            let path_value = path.to_string_lossy().to_string();
            return git_output_dynamic(
                &workspace.path,
                &["diff", "--cached", "--", path_value.as_str()],
            );
        }
        git_output(&workspace.path, ["diff", "--cached", "--"])
    }

    pub fn diff_hunks(
        &self,
        name: &str,
        relative_path: &str,
        staged: bool,
    ) -> Result<Vec<DiffHunkSummary>> {
        let workspace = self.get_by_name(name)?;
        let validated = validate_workspace_relative_path(relative_path)?;
        let path_value = validated.to_string_lossy().to_string();
        let diff = if staged {
            git_output_dynamic(
                &workspace.path,
                &["diff", "--cached", "--", path_value.as_str()],
            )?
        } else {
            git_output_dynamic(&workspace.path, &["diff", "--", path_value.as_str()])?
        };
        Ok(diff_hunk_summaries(&diff, staged))
    }

    pub fn stage_workspace_hunk(
        &self,
        name: &str,
        relative_path: &str,
        hunk_index: usize,
    ) -> Result<()> {
        let workspace = self.get_by_name(name)?;
        let validated = validate_workspace_relative_path(relative_path)?;
        let path_value = validated.to_string_lossy().to_string();
        let diff = git_output_dynamic(&workspace.path, &["diff", "--", path_value.as_str()])?;
        let patch = diff_hunk_patch(&diff, hunk_index)?;
        validate_hunk_patch_supported(&patch)?;
        git_patch(&workspace.path, &["apply", "--cached", "-"], &patch)?;
        self.record_workspace_event(
            workspace.id,
            &workspace.name,
            "git.hunk_staged",
            &format!("Staged hunk {} in {relative_path}", hunk_index + 1),
        )?;
        Ok(())
    }

    pub fn unstage_workspace_hunk(
        &self,
        name: &str,
        relative_path: &str,
        hunk_index: usize,
    ) -> Result<()> {
        let workspace = self.get_by_name(name)?;
        let validated = validate_workspace_relative_path(relative_path)?;
        let path_value = validated.to_string_lossy().to_string();
        let diff = git_output_dynamic(
            &workspace.path,
            &["diff", "--cached", "--", path_value.as_str()],
        )?;
        let patch = diff_hunk_patch(&diff, hunk_index)?;
        validate_hunk_patch_supported(&patch)?;
        git_patch(
            &workspace.path,
            &["apply", "--cached", "--reverse", "-"],
            &patch,
        )?;
        self.record_workspace_event(
            workspace.id,
            &workspace.name,
            "git.hunk_unstaged",
            &format!("Unstaged hunk {} in {relative_path}", hunk_index + 1),
        )?;
        Ok(())
    }

    pub fn unified_diff_against_base(&self, name: &str, path: Option<&Path>) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        let base_ref = workspace_base_ref(&workspace);
        if let Some(path) = path {
            let path_value = path.to_string_lossy().to_string();
            return git_output_dynamic(
                &workspace.path,
                &["diff", base_ref, "--", path_value.as_str()],
            );
        }
        git_output_dynamic(&workspace.path, &["diff", base_ref, "--"])
    }

    pub fn set_workspace_base_ref(&self, name: &str, base_ref: &str) -> Result<Workspace> {
        let workspace = self.get_by_name(name)?;
        let base_ref = base_ref.trim();
        validate_branch_name(base_ref)?;
        let commit_ref = format!("{base_ref}^{{commit}}");
        git_dynamic(
            &workspace.path,
            &["rev-parse", "--verify", commit_ref.as_str()],
        )
        .with_context(|| format!("verify base ref {base_ref}"))?;
        let now = timestamp();
        self.conn.execute(
            "UPDATE workspaces SET base_ref = ?1, updated_at = ?2 WHERE id = ?3",
            params![base_ref, now, workspace.id],
        )?;
        let updated = self.get_by_name(name)?;
        self.record_workspace_event(
            updated.id,
            &updated.name,
            "base_ref.updated",
            &format!("Updated base branch to {base_ref}"),
        )?;
        Ok(updated)
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
        )?;
        self.record_workspace_event(
            workspace.id,
            &workspace.name,
            "file.reverted",
            &format!("Reverted {relative_path} to HEAD"),
        )?;
        Ok(())
    }

    pub fn stage_workspace_file(&self, name: &str, relative_path: &str) -> Result<()> {
        let workspace = self.get_by_name(name)?;
        let validated = validate_workspace_relative_path(relative_path)?;
        let path_value = validated.to_string_lossy().to_string();
        git_dynamic(&workspace.path, &["add", "--", path_value.as_str()])?;
        self.record_workspace_event(
            workspace.id,
            &workspace.name,
            "git.staged",
            &format!("Staged {relative_path}"),
        )?;
        Ok(())
    }

    pub fn stage_all_workspace_files(&self, name: &str) -> Result<()> {
        let workspace = self.get_by_name(name)?;
        git_dynamic(&workspace.path, &["add", "-A"])?;
        self.record_workspace_event(
            workspace.id,
            &workspace.name,
            "git.staged_all",
            "Staged all changed files",
        )?;
        Ok(())
    }

    pub fn unstage_workspace_file(&self, name: &str, relative_path: &str) -> Result<()> {
        let workspace = self.get_by_name(name)?;
        let validated = validate_workspace_relative_path(relative_path)?;
        let path_value = validated.to_string_lossy().to_string();
        git_dynamic(
            &workspace.path,
            &["restore", "--staged", "--", path_value.as_str()],
        )?;
        self.record_workspace_event(
            workspace.id,
            &workspace.name,
            "git.unstaged",
            &format!("Unstaged {relative_path}"),
        )?;
        Ok(())
    }

    pub fn unstage_all_workspace_files(&self, name: &str) -> Result<()> {
        let workspace = self.get_by_name(name)?;
        git_dynamic(&workspace.path, &["restore", "--staged", "--", "."])?;
        self.record_workspace_event(
            workspace.id,
            &workspace.name,
            "git.unstaged_all",
            "Unstaged all files",
        )?;
        Ok(())
    }

    pub fn commit_message_draft(&self, name: &str) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        let staged = git_output_dynamic(&workspace.path, &["diff", "--cached", "--name-only"])?;
        let files = if staged.trim().is_empty() {
            self.changed_files(name)?
        } else {
            staged
                .lines()
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(str::to_owned)
                .collect()
        };
        Ok(commit_message_draft_for_files(&workspace.name, &files))
    }

    pub fn generated_commit_message_from_staged_diff(&self, name: &str) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        let staged = git_output_dynamic(&workspace.path, &["diff", "--cached", "--name-only"])?;
        let files = staged
            .lines()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        anyhow::ensure!(
            !files.is_empty(),
            "stage at least one file before generating a commit message"
        );
        Ok(commit_message_draft_for_files(&workspace.name, &files))
    }

    pub fn commit_workspace_changes(&self, name: &str, message: &str) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        let message = message.trim();
        anyhow::ensure!(!message.is_empty(), "commit message cannot be empty");
        let staged = git_output_dynamic(&workspace.path, &["diff", "--cached", "--name-only"])?;
        anyhow::ensure!(
            !staged.trim().is_empty(),
            "stage at least one file before committing"
        );
        let output = git_output_dynamic(&workspace.path, &["commit", "-m", message])?;
        self.record_workspace_event(
            workspace.id,
            &workspace.name,
            "commit.created",
            &format!("Committed staged changes: {message}"),
        )?;
        Ok(output)
    }

    pub fn diff_file_summaries(&self, name: &str) -> Result<Vec<DiffFileSummary>> {
        let workspace = self.get_by_name(name)?;
        let unstaged = git_output(&workspace.path, ["diff", "--numstat", "--"])?;
        let staged = git_output(&workspace.path, ["diff", "--cached", "--numstat", "--"])?;
        let mut summaries = merge_diff_summaries(
            parse_diff_numstat(&staged)
                .into_iter()
                .chain(parse_diff_numstat(&unstaged))
                .collect(),
        );
        let mut known_paths = summaries
            .iter()
            .map(|summary| summary.path.clone())
            .collect::<BTreeSet<_>>();
        let status = git_output(
            &workspace.path,
            ["status", "--porcelain", "--untracked-files=all"],
        )?;
        apply_diff_file_status(&mut summaries, &status);
        for path in parse_untracked_status_paths(&status) {
            if known_paths.contains(&path) || is_conductor_context_path(&path) {
                continue;
            }
            let counts = untracked_file_counts(&workspace.path.join(&path))?;
            known_paths.insert(path.clone());
            summaries.push(DiffFileSummary {
                path,
                additions: Some(counts.0),
                deletions: Some(0),
                staged: false,
                unstaged: false,
                untracked: true,
            });
        }
        summaries.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(summaries)
    }

    pub fn diff_stats_against_base(&self, name: &str) -> Result<(usize, usize)> {
        let workspace = self.get_by_name(name)?;
        workspace_diff_stats_against_base(&workspace)
    }

    pub fn workspace_base_ref(&self, name: &str) -> Result<String> {
        Ok(self.get_by_name(name)?.base_ref)
    }

    pub fn untracked_files(&self, name: &str) -> Result<Vec<String>> {
        let workspace = self.get_by_name(name)?;
        let status = git_output(
            &workspace.path,
            ["status", "--porcelain", "--untracked-files=all"],
        )?;
        Ok(parse_untracked_status_paths(&status))
    }

    pub fn git_show_commit(&self, name: &str, commit: &str) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        git_output_dynamic(
            &workspace.path,
            &["show", "--stat", "--patch", "--format=medium", commit],
        )
    }

    pub fn push_branch(&self, name: &str) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        let output = git_output_dynamic(
            &workspace.path,
            &["push", "-u", "origin", workspace.branch.as_str()],
        )?;
        self.record_workspace_event(
            workspace.id,
            &workspace.name,
            "branch.pushed",
            &format!("Pushed branch {}", workspace.branch),
        )?;
        Ok(output)
    }

    pub fn force_push_branch_with_lease(&self, name: &str) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let output = git_output_dynamic(
            &workspace.path,
            &[
                "push",
                "--force-with-lease",
                "-u",
                repository.remote_name.as_str(),
                workspace.branch.as_str(),
            ],
        )?;
        self.record_workspace_event(
            workspace.id,
            &workspace.name,
            "branch.force_pushed",
            &format!("Force pushed branch {} with lease", workspace.branch),
        )?;
        Ok(output)
    }

    pub fn create_pull_request(
        &self,
        name: &str,
        title: Option<&str>,
        body: Option<&str>,
        draft: bool,
    ) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        if let Some(existing) = self.existing_pull_request_for_workspace(&workspace)? {
            return Ok(format!("Existing PR: {}\n", existing.url));
        }
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

    pub fn render_pull_request_template(&self, name: &str) -> Result<PullRequestTemplate> {
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let settings = load_repository_settings(&repository.root_path)?;
        let changed_files = self
            .changed_files(name)?
            .into_iter()
            .filter(|path| !is_conductor_context_path(path))
            .collect::<Vec<_>>();
        let changed_files_text = if changed_files.is_empty() {
            "No changed files detected.".to_owned()
        } else {
            changed_files
                .iter()
                .map(|path| format!("- {path}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let changed_files_inline = if changed_files.is_empty() {
            "no changed files".to_owned()
        } else {
            changed_files.join(", ")
        };
        let session_summary = self
            .latest_session_summary(&workspace)?
            .unwrap_or_else(|| "No saved agent session summary yet.".to_owned());
        let context_brief = self
            .read_context_brief(name)?
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#'))
            .unwrap_or(&workspace.name)
            .to_owned();

        let default_title = format!("{}: {}", workspace.name, context_brief);
        let title_template = settings
            .customization
            .naming
            .pr_title_template
            .as_deref()
            .filter(|template| !template.trim().is_empty())
            .unwrap_or(&default_title);
        let title = render_pr_template_text(
            title_template,
            &workspace,
            changed_files.len(),
            &changed_files_inline,
            &session_summary,
            &context_brief,
        );

        let sections = if settings.customization.naming.pr_body_sections.is_empty() {
            vec!["Summary".to_owned(), "Tests".to_owned(), "Risk".to_owned()]
        } else {
            settings.customization.naming.pr_body_sections
        };
        let mut body = String::new();
        for section in sections {
            let normalized = section.to_ascii_lowercase();
            body.push_str(&format!("## {section}\n\n"));
            if normalized.contains("summary") {
                body.push_str(&format!(
                    "- Workspace: {}\n- Branch: {}\n- Changed files: {}\n\n{}\n\nSession summary:\n{}\n",
                    workspace.name,
                    workspace.branch,
                    changed_files.len(),
                    changed_files_text,
                    session_summary
                ));
            } else if normalized.contains("test") || normalized.contains("verification") {
                body.push_str("- TODO: Add checks/tests run before creating the PR.\n");
            } else if normalized.contains("risk") {
                body.push_str("- TODO: Note migration, data, rollout, or UX risk.\n");
            } else {
                body.push_str("- TODO\n");
            }
            body.push('\n');
        }

        Ok(PullRequestTemplate {
            title: title.trim().to_owned(),
            body: body.trim().to_owned(),
        })
    }

    pub fn create_from_issue(
        &self,
        repository_name: &str,
        issue_number: u64,
        branch_prefix: Option<&str>,
    ) -> Result<Workspace> {
        self.create_from_issue_with_progress(repository_name, issue_number, branch_prefix, || {})
    }

    pub fn create_from_issue_with_progress(
        &self,
        repository_name: &str,
        issue_number: u64,
        branch_prefix: Option<&str>,
        after_insert: impl FnOnce(),
    ) -> Result<Workspace> {
        let preflight = self.source_preflight();
        anyhow::ensure!(
            preflight.github_ready(),
            "GitHub source creation requires {}",
            preflight.github_status()
        );
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
        let settings = load_repository_settings(&repository.root_path)?;

        // Slugify title for branch name
        let slug = slugify(&title);
        let configured_prefix = settings
            .customization
            .workspace_defaults
            .branch_prefix
            .as_deref()
            .unwrap_or("lc");
        let prefix = branch_prefix.unwrap_or(configured_prefix);
        let branch = format!("{prefix}/{issue_number}/{slug}");
        let workspace_name = format!("issue-{issue_number}");

        let workspace = self.create_with_progress(
            CreateWorkspace {
                repository_name: repository_name.to_owned(),
                name: workspace_name,
                branch,
                base_ref: None,
            },
            after_insert,
        )?;
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
        self.create_from_pull_request_with_progress(
            repository_name,
            pr_number,
            workspace_name,
            branch_name,
            || {},
        )
    }

    pub fn create_from_pull_request_with_progress(
        &self,
        repository_name: &str,
        pr_number: u64,
        workspace_name: Option<&str>,
        branch_name: Option<&str>,
        after_insert: impl FnOnce(),
    ) -> Result<Workspace> {
        let preflight = self.source_preflight();
        anyhow::ensure!(
            preflight.github_ready(),
            "GitHub source creation requires {}",
            preflight.github_status()
        );
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
        let settings = load_repository_settings(&repository.root_path)?;
        let prefix = settings
            .customization
            .workspace_defaults
            .branch_prefix
            .as_deref()
            .unwrap_or("lc");
        let slug = slugify(&title);
        let remote_ref = format!("refs/linux-archductor/pull-requests/{pr_number}");
        let fetch_refspec = format!("pull/{pr_number}/head:{remote_ref}");
        git_dynamic(
            &repository.root_path,
            &[
                "fetch",
                repository.remote_name.as_str(),
                fetch_refspec.as_str(),
            ],
        )?;

        let workspace = self.create_with_progress(
            CreateWorkspace {
                repository_name: repository_name.to_owned(),
                name: workspace_name
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("pr-{pr_number}")),
                branch: branch_name
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("{prefix}/pr-{pr_number}-{slug}")),
                base_ref: Some(remote_ref),
            },
            after_insert,
        )?;

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
        self.create_from_prompt_with_progress(
            repository_name,
            prompt,
            workspace_name,
            branch_name,
            base_ref,
            || {},
        )
    }

    pub fn create_from_prompt_with_progress(
        &self,
        repository_name: &str,
        prompt: &str,
        workspace_name: Option<&str>,
        branch_name: Option<&str>,
        base_ref: Option<&str>,
        after_insert: impl FnOnce(),
    ) -> Result<Workspace> {
        let prompt = prompt.trim();
        anyhow::ensure!(!prompt.is_empty(), "prompt is required");
        let repository = self.load_repository(repository_name)?;
        let settings = load_repository_settings(&repository.root_path)?;
        let prefix = settings
            .customization
            .workspace_defaults
            .branch_prefix
            .as_deref()
            .unwrap_or("lc");
        let slug = slugify(prompt);
        let workspace = self.create_with_progress(
            CreateWorkspace {
                repository_name: repository_name.to_owned(),
                name: workspace_name
                    .map(str::to_owned)
                    .unwrap_or_else(|| slug.clone()),
                branch: branch_name
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("{prefix}/{slug}")),
                base_ref: base_ref.map(str::to_owned),
            },
            after_insert,
        )?;
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
        self.create_from_linear_issue_with_progress(
            repository_name,
            issue_id,
            workspace_name,
            branch_name,
            base_ref,
            || {},
        )
    }

    pub fn create_from_linear_issue_with_progress(
        &self,
        repository_name: &str,
        issue_id: &str,
        workspace_name: Option<&str>,
        branch_name: Option<&str>,
        base_ref: Option<&str>,
        after_insert: impl FnOnce(),
    ) -> Result<Workspace> {
        let preflight = self.source_preflight();
        anyhow::ensure!(
            preflight.linear_ready(),
            "Linear source creation requires {}",
            preflight.linear_status()
        );
        let issue_id = issue_id.trim();
        anyhow::ensure!(!issue_id.is_empty(), "Linear issue id is required");
        let issue = fetch_linear_issue(issue_id)?;
        let repository = self.load_repository(repository_name)?;
        let settings = load_repository_settings(&repository.root_path)?;
        let prefix = settings
            .customization
            .workspace_defaults
            .branch_prefix
            .as_deref()
            .unwrap_or("lc");
        let slug = slugify(&issue.title);
        let workspace = self.create_with_progress(
            CreateWorkspace {
                repository_name: repository_name.to_owned(),
                name: workspace_name
                    .map(str::to_owned)
                    .unwrap_or_else(|| issue.identifier.to_ascii_lowercase()),
                branch: branch_name
                    .map(str::to_owned)
                    .or(issue.branch_name)
                    .unwrap_or_else(|| {
                        format!("{prefix}/{}-{slug}", issue.identifier.to_ascii_lowercase())
                    }),
                base_ref: base_ref.map(str::to_owned),
            },
            after_insert,
        )?;
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

    pub fn source_preflight(&self) -> WorkspaceSourcePreflight {
        WorkspaceSourcePreflight {
            github_cli_installed: command_exists("gh"),
            github_authenticated: command_success("gh", &["auth", "status"]),
            linear_api_key_set: std::env::var_os("LINEAR_API_KEY")
                .map(|value| !value.is_empty())
                .unwrap_or(false),
        }
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
        let args = self.gh_pr_args_for_workspace(&workspace, "checks", &[])?;
        let output = command_output_owned(&workspace.path, "gh", &args)?;
        self.record_workspace_event(
            workspace.id,
            &workspace.name,
            "checks.refreshed",
            "Refreshed pull request checks",
        )?;
        Ok(output)
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
        let args = self.gh_pr_args_for_workspace(&workspace, "view", &["--comments"])?;
        command_output_owned(&workspace.path, "gh", &args)
    }

    pub fn pull_request_review_agent_prompt(&self, name: &str) -> Result<String> {
        let review_state = self.pull_request_review_state(name)?;
        Ok(format_pull_request_review_agent_prompt(name, &review_state))
    }

    pub fn pull_request_readiness(&self, name: &str) -> Result<PullRequestReadiness> {
        let workspace = self.get_by_name(name)?;
        let args = self.gh_pr_args_for_workspace(
            &workspace,
            "view",
            &[
                "--json",
                "id,headRefOid,reviewDecision,latestReviews,comments,statusCheckRollup",
            ],
        )?;
        let output = command_output_owned(&workspace.path, "gh", &args)?;
        let mut readiness = parse_pull_request_readiness(&output)?;
        if let Some(pr_id) = json_root_string(&output, "id")? {
            readiness.review_threads =
                self.pull_request_review_threads_by_id(&workspace, &pr_id)?;
        }
        if let Some(head_sha) = json_root_string(&output, "headRefOid")? {
            append_unique_checks(
                &mut readiness.checks,
                self.pull_request_statuses_for_head(&workspace, &head_sha)?,
            );
            append_unique_deployments(
                &mut readiness.deployments,
                self.pull_request_deployments_for_head(&workspace, &head_sha)?,
            );
        }
        Ok(readiness)
    }

    pub fn pull_request_readiness_text(&self, name: &str) -> Result<String> {
        let readiness = self.pull_request_readiness(name)?;
        Ok(format_pull_request_readiness(name, &readiness))
    }

    pub fn pull_request_readiness_agent_prompt(&self, name: &str) -> Result<String> {
        let readiness = self.pull_request_readiness(name)?;
        Ok(format_pull_request_readiness_agent_prompt(name, &readiness))
    }

    fn pull_request_review_threads_by_id(
        &self,
        workspace: &Workspace,
        pr_id: &str,
    ) -> Result<Vec<PullRequestReviewThread>> {
        let query = "\
query($prId: ID!) {
  node(id: $prId) {
    ... on PullRequest {
      reviewThreads(first: 50) {
        nodes {
          id
          isResolved
          path
          line
          startLine
          comments(first: 20) {
            nodes {
              author { login }
              body
              url
              createdAt
            }
          }
        }
      }
    }
  }
}
";
        let output = command_output_owned(
            &workspace.path,
            "gh",
            &[
                "api".to_owned(),
                "graphql".to_owned(),
                "-f".to_owned(),
                format!("query={query}"),
                "-F".to_owned(),
                format!("prId={pr_id}"),
            ],
        )?;
        parse_pull_request_review_threads(&output)
    }

    fn pull_request_statuses_for_head(
        &self,
        workspace: &Workspace,
        head_sha: &str,
    ) -> Result<Vec<PullRequestCheckRun>> {
        let output = command_output_owned(
            &workspace.path,
            "gh",
            &[
                "api".to_owned(),
                format!("repos/{{owner}}/{{repo}}/commits/{head_sha}/status"),
            ],
        )?;
        parse_github_commit_status_checks(&output)
    }

    fn pull_request_deployments_for_head(
        &self,
        workspace: &Workspace,
        head_sha: &str,
    ) -> Result<Vec<PullRequestDeployment>> {
        let output = command_output_owned(
            &workspace.path,
            "gh",
            &[
                "api".to_owned(),
                format!("repos/{{owner}}/{{repo}}/deployments?sha={head_sha}"),
            ],
        )?;
        let mut deployments = Vec::new();
        for deployment in parse_github_deployment_entries(&output)? {
            let statuses = command_output_owned(
                &workspace.path,
                "gh",
                &[
                    "api".to_owned(),
                    format!(
                        "repos/{{owner}}/{{repo}}/deployments/{}/statuses",
                        deployment.id
                    ),
                ],
            )?;
            let latest = parse_github_deployment_latest_status(&statuses)?;
            deployments.push(PullRequestDeployment {
                environment: deployment.environment,
                status: latest
                    .as_ref()
                    .map(|status| status.state.clone())
                    .or(deployment.status)
                    .unwrap_or_else(|| "UNKNOWN".to_owned()),
                url: latest.and_then(|status| status.url),
            });
        }
        Ok(deployments)
    }

    pub fn set_pull_request_review_thread_resolution(
        &self,
        name: &str,
        thread_id: &str,
        resolved: bool,
    ) -> Result<PullRequestReviewThread> {
        let workspace = self.get_by_name(name)?;
        let thread_id = thread_id.trim();
        anyhow::ensure!(!thread_id.is_empty(), "review thread id is required");
        let (mutation_name, state_field) = if resolved {
            ("resolveReviewThread", "resolved")
        } else {
            ("unresolveReviewThread", "unresolved")
        };
        let query = format!(
            "\
mutation($threadId: ID!) {{
  {mutation_name}(input: {{threadId: $threadId}}) {{
    thread {{
      id
      isResolved
      path
      line
      startLine
      comments(first: 20) {{
        nodes {{
          author {{ login }}
          body
          url
          createdAt
        }}
      }}
    }}
  }}
}}
"
        );
        let output = command_output_owned(
            &workspace.path,
            "gh",
            &[
                "api".to_owned(),
                "graphql".to_owned(),
                "-f".to_owned(),
                format!("query={query}"),
                "-F".to_owned(),
                format!("threadId={thread_id}"),
            ],
        )
        .with_context(|| format!("mark GitHub review thread {thread_id} {state_field}"))?;
        parse_pull_request_review_thread_mutation(&output, mutation_name)
    }

    pub fn pull_request(&self, name: &str) -> Result<Option<PullRequest>> {
        let workspace = self.get_by_name(name)?;
        self.pull_request_by_workspace_id(workspace.id)
    }

    pub fn pull_request_panel_state(&self, name: &str) -> Result<PullRequestPanelState> {
        let pull_request = self.pull_request(name)?;
        let readiness = match pull_request.as_ref() {
            Some(_) => Some(self.pull_request_readiness(name)?),
            None => None,
        };
        let readiness_text = readiness
            .as_ref()
            .map(|readiness| format_pull_request_readiness(name, readiness))
            .unwrap_or_else(|| "No pull request yet.".to_owned());
        let review_text = match pull_request.as_ref() {
            Some(_) => Some(self.pull_request_review_state(name)?),
            None => None,
        };
        Ok(PullRequestPanelState {
            pull_request,
            readiness,
            readiness_text,
            review_text,
        })
    }

    pub fn refresh_pull_request_state(&self, name: &str) -> Result<Option<PullRequest>> {
        let workspace = self.get_by_name(name)?;
        if self.pull_request_by_workspace_id(workspace.id)?.is_none() {
            return Ok(None);
        }
        let args = self.gh_pr_args_for_workspace(&workspace, "view", &["--json", "state"])?;
        let state = command_output_owned(&workspace.path, "gh", &args)?;
        let state = extract_json_string_field(&state, "state").unwrap_or_else(|| "open".to_owned());
        let now = timestamp();
        self.conn.execute(
            "UPDATE pull_requests SET state = ?1, updated_at = ?2 WHERE workspace_id = ?3",
            params![state, now, workspace.id],
        )?;
        self.pull_request_by_workspace_id(workspace.id)
    }

    fn gh_pr_args_for_workspace(
        &self,
        workspace: &Workspace,
        subcommand: &str,
        extra: &[&str],
    ) -> Result<Vec<String>> {
        let mut args = vec!["pr".to_owned(), subcommand.to_owned()];
        if let Some(pr) = self.pull_request_by_workspace_id(workspace.id)? {
            args.push(pr.number.to_string());
        }
        args.extend(extra.iter().map(|arg| (*arg).to_owned()));
        Ok(args)
    }

    fn existing_pull_request_for_workspace(
        &self,
        workspace: &Workspace,
    ) -> Result<Option<PullRequest>> {
        if let Some(pr) = self.pull_request_by_workspace_id(workspace.id)? {
            if pr.state == "open" {
                return Ok(Some(pr));
            }
        }
        let output = command_output(
            &workspace.path,
            "gh",
            &[
                "pr",
                "list",
                "--head",
                workspace.branch.as_str(),
                "--state",
                "open",
                "--json",
                "url",
                "--limit",
                "1",
            ],
        )?;
        match first_pull_request_url_from_json(&output) {
            Some(url) => self.record_pull_request(workspace.id, &url).map(Some),
            None => Ok(None),
        }
    }

    fn latest_session_summary(&self, workspace: &Workspace) -> Result<Option<String>> {
        let Some(process) = self
            .list_processes(&workspace.name, ProcessKind::Session)?
            .into_iter()
            .next()
        else {
            return Ok(None);
        };
        let transcript = fs::read_to_string(&process.log_path).unwrap_or_default();
        let summary = terminal_log_preview(&transcript);
        Ok((!summary.trim().is_empty()).then_some(summary))
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
        let pull_request = self
            .pull_request_by_workspace_id(workspace_id)?
            .context("load recorded pull request")?;
        self.record_workspace_event(
            workspace_id,
            &self.workspace_name_by_id(workspace_id)?,
            "pr.created",
            &format!("Recorded GitHub PR #{}", pull_request.number),
        )?;
        Ok(pull_request)
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
        for todo in parse_context_todos(&contents) {
            let status = if todo.done { "done" } else { "open" };
            let existing: Option<(i64, String)> = self
                .conn
                .query_row(
                    "SELECT id, status FROM todos
                     WHERE workspace_id = ?1 AND text = ?2 AND source = 'context'",
                    params![workspace.id, todo.text],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()?;
            if let Some((id, existing_status)) = existing {
                if existing_status != status {
                    let now = timestamp();
                    self.conn.execute(
                        "UPDATE todos SET status = ?1, updated_at = ?2 WHERE id = ?3",
                        params![status, now, id],
                    )?;
                }
            } else {
                let now = timestamp();
                self.conn.execute(
                    "INSERT INTO todos (workspace_id, text, status, source, created_at, updated_at)
                     VALUES (?1, ?2, ?3, 'context', ?4, ?5)",
                    params![workspace.id, todo.text, status, now, now],
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
        let checkpoint = create_worktree_checkpoint_commit(&workspace, message, "manual")?;

        self.conn.execute(
            "INSERT INTO checkpoints (workspace_id, session_id, git_ref, message, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                workspace.id,
                session_id,
                checkpoint.git_ref,
                message,
                checkpoint.created_at
            ],
        )?;
        self.get_checkpoint(self.conn.last_insert_rowid())
    }

    pub fn checkpoint_create_turn_start(
        &self,
        name: &str,
        thread_id: i64,
        session_id: Option<i64>,
        prompt_kind: &str,
    ) -> Result<Checkpoint> {
        let prompt_kind = prompt_kind.trim();
        let prompt_kind = if prompt_kind.is_empty() {
            "user"
        } else {
            prompt_kind
        };
        let message = format!("Turn start: thread #{thread_id} {prompt_kind}");
        let workspace = self.get_by_name(name)?;
        let checkpoint = create_worktree_checkpoint_commit(&workspace, &message, "turn")?;

        self.conn.execute(
            "INSERT INTO checkpoints (workspace_id, session_id, git_ref, message, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                workspace.id,
                session_id,
                checkpoint.git_ref,
                message,
                checkpoint.created_at
            ],
        )?;
        let checkpoint = self.get_checkpoint(self.conn.last_insert_rowid())?;
        self.record_workspace_event(
            workspace.id,
            &workspace.name,
            "checkpoint.turn_start",
            &format!(
                "Created turn checkpoint #{} for thread #{thread_id}",
                checkpoint.id
            ),
        )?;
        Ok(checkpoint)
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

    pub fn latest_turn_checkpoint_diff(&self, name: &str) -> Result<Option<TurnCheckpointDiff>> {
        Ok(self.turn_checkpoint_diffs(name, 1)?.into_iter().next())
    }

    pub fn turn_checkpoint_diffs(
        &self,
        name: &str,
        limit: usize,
    ) -> Result<Vec<TurnCheckpointDiff>> {
        let workspace = self.get_by_name(name)?;
        let limit = limit.min(TURN_CHECKPOINT_DIFF_LIMIT);
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut checkpoints = self
            .checkpoint_list(name)?
            .into_iter()
            .filter(|checkpoint| checkpoint.message.starts_with("Turn start:"))
            .collect::<Vec<_>>();
        if checkpoints.is_empty() {
            return Ok(Vec::new());
        }
        checkpoints.sort_by_key(|checkpoint| checkpoint.id);

        let mut rows = Vec::new();
        let start = checkpoints.len().saturating_sub(limit);
        for index in start..checkpoints.len() {
            let checkpoint = checkpoints[index].clone();
            let end_checkpoint = checkpoints.get(index + 1).cloned();
            let raw_diff = match end_checkpoint.as_ref() {
                Some(end) => diff_checkpoint_refs(&workspace, &checkpoint.git_ref, &end.git_ref)?,
                None => diff_worktree_against_ref(&workspace, &checkpoint.git_ref)?,
            };
            let (diff, truncated) =
                truncate_text_at_char_boundary(raw_diff, TURN_CHECKPOINT_DIFF_MAX_BYTES);
            rows.push(TurnCheckpointDiff {
                checkpoint,
                end_checkpoint,
                diff,
                truncated,
            });
        }
        rows.reverse();
        Ok(rows)
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

    pub fn checkpoint_delete(&self, name: &str, checkpoint_id: i64) -> Result<()> {
        let workspace = self.get_by_name(name)?;
        let checkpoint = self.get_checkpoint(checkpoint_id)?;
        anyhow::ensure!(
            checkpoint.workspace_id == workspace.id,
            "checkpoint {checkpoint_id} does not belong to workspace {name}"
        );
        let _ = git_dynamic(&workspace.path, &["update-ref", "-d", &checkpoint.git_ref]);
        self.conn
            .execute("DELETE FROM checkpoints WHERE id = ?1", [checkpoint_id])
            .with_context(|| format!("delete checkpoint {checkpoint_id}"))?;
        Ok(())
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
            "refs/linux-archductor/checkpoints/{}/spotlight-{now}",
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

    pub fn create_branch(&self, name: &str, branch: &str) -> Result<()> {
        let workspace = self.get_by_name(name)?;
        validate_branch_name(branch)?;
        git_dynamic(&workspace.path, &["branch", branch])?;
        self.record_workspace_event(
            workspace.id,
            &workspace.name,
            "branch.created",
            &format!("Created branch {branch}"),
        )?;
        Ok(())
    }

    pub fn checkout_branch(&self, name: &str, branch: &str) -> Result<Workspace> {
        let workspace = self.get_by_name(name)?;
        validate_branch_name(branch)?;
        ensure_clean_git_tree(&workspace.path, "checkout branch")?;
        git_dynamic(&workspace.path, &["checkout", branch])?;
        let now = timestamp();
        self.conn.execute(
            "UPDATE workspaces SET branch = ?1, updated_at = ?2 WHERE id = ?3",
            params![branch, now, workspace.id],
        )?;
        let updated = self.get_by_name(name)?;
        self.record_workspace_event(
            updated.id,
            &updated.name,
            "branch.checked_out",
            &format!("Checked out branch {branch}"),
        )?;
        Ok(updated)
    }

    pub fn rename_branch(&self, name: &str, new_branch: &str) -> Result<Workspace> {
        let workspace = self.get_by_name(name)?;
        validate_branch_name(new_branch)?;
        ensure_clean_git_tree(&workspace.path, "rename branch")?;
        git_dynamic(&workspace.path, &["branch", "-m", new_branch])?;
        let old_branch = workspace.branch.clone();
        let now = timestamp();
        self.conn.execute(
            "UPDATE workspaces SET branch = ?1, updated_at = ?2 WHERE id = ?3",
            params![new_branch, now, workspace.id],
        )?;
        let updated = self.get_by_name(name)?;
        self.record_workspace_event(
            updated.id,
            &updated.name,
            "branch.renamed",
            &format!("Renamed branch {old_branch} to {new_branch}"),
        )?;
        Ok(updated)
    }

    pub fn delete_branch(&self, name: &str, branch: &str) -> Result<()> {
        let workspace = self.get_by_name(name)?;
        validate_branch_name(branch)?;
        anyhow::ensure!(
            branch != workspace.branch,
            "cannot delete current workspace branch {branch}"
        );
        ensure_clean_git_tree(&workspace.path, "delete branch")?;
        git_dynamic(&workspace.path, &["branch", "-D", branch])?;
        self.record_workspace_event(
            workspace.id,
            &workspace.name,
            "branch.deleted",
            &format!("Deleted branch {branch}"),
        )?;
        Ok(())
    }

    pub fn workspace_timeline(
        &self,
        name: &str,
        kind: Option<&str>,
    ) -> Result<Vec<WorkspaceTimelineEvent>> {
        let workspace = self.get_by_name(name)?;
        self.sync_workspace_commit_events(&workspace)?;
        let mut events = self.workspace_timeline_by_id(workspace.id, kind)?;
        events.sort_by_key(|event| event.id);
        Ok(events)
    }

    fn workspace_timeline_by_id(
        &self,
        workspace_id: i64,
        kind: Option<&str>,
    ) -> Result<Vec<WorkspaceTimelineEvent>> {
        match kind.map(str::trim).filter(|kind| !kind.is_empty()) {
            Some(kind) => {
                let mut stmt = self.conn.prepare(
                    "SELECT id, workspace_id, workspace_name, kind, summary, created_at
                     FROM workspace_timeline
                     WHERE workspace_id = ?1 AND kind = ?2
                     ORDER BY id ASC",
                )?;
                let events = stmt
                    .query_map(params![workspace_id, kind], row_to_workspace_timeline_event)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(events)
            }
            None => {
                let mut stmt = self.conn.prepare(
                    "SELECT id, workspace_id, workspace_name, kind, summary, created_at
                     FROM workspace_timeline
                     WHERE workspace_id = ?1
                     ORDER BY id ASC",
                )?;
                let events = stmt
                    .query_map([workspace_id], row_to_workspace_timeline_event)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(events)
            }
        }
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
        let check_status = self.latest_process_status(workspace.id, ProcessKind::Check)?;
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
            check_status,
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
            "SELECT id, workspace_id, chat_thread_id, kind, command, pid, log_path, status, started_at, exit_code, ended_at, session_harness_metadata, session_resume_id
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

    pub fn merge_pull_request(&self, name: &str, method: Option<&str>) -> Result<String> {
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let settings = load_repository_settings(&repository.root_path)?;
        let pr = self
            .pull_request_by_workspace_id(workspace.id)?
            .with_context(|| format!("no pull request recorded for workspace {name}"))?;
        let method = merge_method(&settings, method)?;
        let merge_rules = &settings.customization.merge_rules;

        if merge_rules.block_on_open_todos.unwrap_or(true) {
            let todos = self.list_todos(name)?;
            let open_todos = todos.iter().filter(|t| t.status == "open").count();
            if open_todos > 0 {
                anyhow::bail!(
                    "{open_todos} open todo(s) remain in workspace {name}; complete them before merging"
                );
            }
        }
        if merge_rules.block_on_open_comments.unwrap_or(true) {
            let comments = self.list_review_comments(name)?;
            let open_comments = comments.iter().filter(|c| c.status == "open").count();
            if open_comments > 0 {
                anyhow::bail!(
                    "{open_comments} open review comment(s) remain in workspace {name}; resolve them before merging"
                );
            }
        }
        if merge_rules.block_on_failed_checks.unwrap_or(false)
            || merge_rules.block_on_pending_checks.unwrap_or(false)
        {
            let readiness = self.pull_request_readiness(name)?;
            if merge_rules.block_on_failed_checks.unwrap_or(false) {
                let failing = readiness
                    .checks
                    .iter()
                    .filter(|check| check.is_failure())
                    .collect::<Vec<_>>();
                if !failing.is_empty() {
                    let names = failing
                        .iter()
                        .map(|check| check.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    anyhow::bail!(
                        "{} failing check(s) remain in workspace {name}: {names}",
                        failing.len()
                    );
                }
            }
            if merge_rules.block_on_pending_checks.unwrap_or(false) {
                let pending = readiness
                    .checks
                    .iter()
                    .filter(|check| check.is_pending())
                    .collect::<Vec<_>>();
                if !pending.is_empty() {
                    let names = pending
                        .iter()
                        .map(|check| check.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    anyhow::bail!(
                        "{} pending check(s) remain in workspace {name}: {names}",
                        pending.len()
                    );
                }
            }
        }

        let method_flag = format!("--{method}");
        let output = command_output(
            &workspace.path,
            "gh",
            &["pr", "merge", pr.number.to_string().as_str(), &method_flag],
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
        method: Option<&str>,
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
        let cwd = workspace_working_directory(&settings, &workspace)?;
        let mut env = conductor_environment(&settings, &repository, &workspace)?;
        env.extend(self.linked_directory_env(&workspace)?);
        Ok(SessionLaunch {
            kind: SessionKind::Shell,
            program: PathBuf::from(editor),
            args: vec![cwd.to_string_lossy().to_string()],
            cwd,
            env,
            harness_metadata: None,
            session_resume_id: None,
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
        self.build_session_launch(name, kind, harness, None, false)
    }

    pub fn session_launch_with_options_and_resume(
        &self,
        name: &str,
        kind: SessionKind,
        harness: SessionHarnessOptions,
        session_resume_id: Option<&str>,
    ) -> Result<SessionLaunch> {
        self.build_session_launch(name, kind, harness, session_resume_id, true)
    }

    fn build_session_launch(
        &self,
        name: &str,
        kind: SessionKind,
        harness: SessionHarnessOptions,
        session_resume_id: Option<&str>,
        prefer_resume: bool,
    ) -> Result<SessionLaunch> {
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let settings = load_repository_settings(&repository.root_path)?;
        let cwd = workspace_working_directory(&settings, &workspace)?;
        let mut env = conductor_environment(&settings, &repository, &workspace)?;
        env.extend(self.linked_directory_env(&workspace)?);
        let (program, mut args) = match kind {
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
        };
        let harness::SessionHarnessLaunchPlan {
            args: harness_args,
            env: harness_env,
            harness_metadata,
            session_resume_id: launch_session_resume_id,
            ..
        } = if prefer_resume {
            harness::build_session_resume_launch_plan(kind, &cwd, &harness, session_resume_id)
        } else {
            harness::build_session_harness_launch_plan(kind, &cwd, &harness)
        };
        env.extend(harness_env);
        args.extend(harness_args);

        Ok(SessionLaunch {
            kind,
            program,
            args,
            cwd,
            env,
            harness_metadata,
            session_resume_id: launch_session_resume_id,
        })
    }

    pub fn list_sessions(&self, name: &str) -> Result<Vec<ProcessRecord>> {
        self.list_processes(name, ProcessKind::Session)
    }

    pub fn get_process_record(&self, id: i64) -> Result<ProcessRecord> {
        self.get_process(id)
    }

    pub fn get_workspace_record(&self, id: i64) -> Result<Workspace> {
        self.get_by_id(id)
    }

    pub fn get_workspace_record_by_name(&self, name: &str) -> Result<Workspace> {
        self.get_by_name(name)
    }

    pub fn list_local_chat_history(
        &self,
        workspace_path: Option<&Path>,
    ) -> Result<Vec<LocalChatHistorySummary>> {
        let rows = self.local_chat_history_rows(workspace_path)?;
        rows.into_iter()
            .map(|row| {
                let event_messages = session_events_to_local_chat_messages(
                    &self.list_session_events(row.process.id)?,
                );
                let (messages, transcript) = if event_messages.is_empty() {
                    let transcript = fs::read_to_string(&row.process.log_path).unwrap_or_default();
                    (parse_local_chat_transcript(&transcript), transcript)
                } else {
                    (event_messages, String::new())
                };
                let preview = messages
                    .iter()
                    .rev()
                    .find_map(|message| {
                        let trimmed = message.content.trim();
                        (!trimmed.is_empty()).then(|| truncate_chars(trimmed, 160))
                    })
                    .unwrap_or_else(|| terminal_log_preview(&transcript));
                Ok(LocalChatHistorySummary {
                    process_id: row.process.id,
                    chat_thread_id: row.process.chat_thread_id,
                    repository_name: row.repository_name,
                    workspace_name: row.workspace_name,
                    workspace_path: row.workspace_path,
                    agent_type: local_chat_agent_type(&row.process.command),
                    status: row.process.status.as_str().to_owned(),
                    started_at: row.process.started_at.clone(),
                    updated_at: row
                        .process
                        .ended_at
                        .clone()
                        .unwrap_or_else(|| row.process.started_at.clone()),
                    message_count: messages.len(),
                    preview,
                    harness: row.process.session_harness_metadata.clone(),
                })
            })
            .collect()
    }

    pub fn local_chat_history_messages(
        &self,
        process_id: i64,
    ) -> Result<Vec<LocalChatHistoryMessage>> {
        let process = self.get_process(process_id)?;
        anyhow::ensure!(
            process.kind == ProcessKind::Session,
            "process {process_id} is not a session process"
        );
        let session_events = self.list_session_events(process_id)?;
        let messages = session_events_to_local_chat_messages(&session_events);
        if !messages.is_empty() {
            return Ok(messages);
        }
        let transcript = fs::read_to_string(&process.log_path)
            .with_context(|| format!("read log {}", process.log_path.display()))?;
        Ok(parse_local_chat_transcript(&transcript))
    }

    pub fn list_local_chat_threads(
        &self,
        workspace_path: Option<&Path>,
    ) -> Result<Vec<LocalChatThreadSummary>> {
        let rows = self.local_chat_thread_rows(workspace_path)?;
        rows.into_iter()
            .map(|row| {
                let messages = self.list_chat_messages(row.thread.id)?;
                let preview = messages
                    .iter()
                    .rev()
                    .find_map(|message| {
                        let trimmed = message.content.trim();
                        (!trimmed.is_empty()).then(|| truncate_chars(trimmed, 160))
                    })
                    .unwrap_or_else(|| row.thread.title.clone());
                let status = self
                    .latest_thread_process(row.thread.id)?
                    .map(|process| process.status.as_str().to_owned())
                    .unwrap_or_else(|| row.thread.status.clone());
                Ok(LocalChatThreadSummary {
                    thread_id: row.thread.id,
                    repository_name: row.repository_name,
                    workspace_name: row.workspace_name,
                    workspace_path: row.workspace_path,
                    provider: row.thread.provider,
                    title: row.thread.title,
                    status,
                    updated_at: row.thread.updated_at,
                    message_count: messages.len(),
                    preview,
                    native_thread_id: row.thread.native_thread_id,
                })
            })
            .collect()
    }

    pub fn local_chat_thread_messages(
        &self,
        thread_id: i64,
    ) -> Result<Vec<LocalChatHistoryMessage>> {
        Ok(self
            .list_chat_messages(thread_id)?
            .into_iter()
            .map(|message| LocalChatHistoryMessage {
                role: message.role,
                content: message.content,
            })
            .collect())
    }

    pub fn link_workspace_directory(
        &self,
        name: &str,
        target_name: &str,
    ) -> Result<LinkedDirectory> {
        let workspace = self.get_by_name(name)?;
        let target = self.get_by_name(target_name)?;
        anyhow::ensure!(
            workspace.id != target.id,
            "workspace {name} cannot link to itself"
        );
        anyhow::ensure!(
            workspace.status == "active",
            "workspace {name} must be active to link directories"
        );
        anyhow::ensure!(
            target.status == "active",
            "target workspace {target_name} must be active to link directories"
        );
        let now = timestamp();
        self.conn.execute(
            "INSERT INTO linked_directories (
                workspace_id, target_workspace_id, created_at
             ) VALUES (?1, ?2, ?3)
             ON CONFLICT(workspace_id, target_workspace_id) DO UPDATE SET
                created_at = linked_directories.created_at",
            params![workspace.id, target.id, now],
        )?;
        let link = self.linked_directory_by_workspaces(workspace.id, target.id)?;
        materialize_linked_directory(&link)?;
        Ok(link)
    }

    pub fn unlink_workspace_directory(
        &self,
        name: &str,
        target_name: &str,
    ) -> Result<LinkedDirectory> {
        let workspace = self.get_by_name(name)?;
        let target = self.get_by_name(target_name)?;
        let link = self.linked_directory_by_workspaces(workspace.id, target.id)?;
        self.conn.execute(
            "DELETE FROM linked_directories WHERE workspace_id = ?1 AND target_workspace_id = ?2",
            params![workspace.id, target.id],
        )?;
        remove_linked_directory_path(&link)?;
        Ok(link)
    }

    pub fn list_linked_directories(&self, name: &str) -> Result<Vec<LinkedDirectory>> {
        let workspace = self.get_by_name(name)?;
        let mut stmt = self.conn.prepare(
            "SELECT ld.id,
                    source.id, source.name, source.path,
                    target.id, target.name, target.path,
                    ld.created_at
             FROM linked_directories ld
             JOIN workspaces source ON source.id = ld.workspace_id
             JOIN workspaces target ON target.id = ld.target_workspace_id
             WHERE source.id = ?1
             ORDER BY target.name",
        )?;
        let links = stmt
            .query_map([workspace.id], row_to_linked_directory)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        for link in &links {
            materialize_linked_directory(link)?;
        }
        Ok(links)
    }

    fn linked_directory_by_workspaces(
        &self,
        workspace_id: i64,
        target_workspace_id: i64,
    ) -> Result<LinkedDirectory> {
        self.conn
            .query_row(
                "SELECT ld.id,
                        source.id, source.name, source.path,
                        target.id, target.name, target.path,
                        ld.created_at
                 FROM linked_directories ld
                 JOIN workspaces source ON source.id = ld.workspace_id
                 JOIN workspaces target ON target.id = ld.target_workspace_id
                 WHERE source.id = ?1 AND target.id = ?2",
                params![workspace_id, target_workspace_id],
                row_to_linked_directory,
            )
            .with_context(|| format!("load linked directory {workspace_id}->{target_workspace_id}"))
    }

    fn linked_directory_env(&self, workspace: &Workspace) -> Result<Vec<(String, OsString)>> {
        let links = self.list_linked_directories(&workspace.name)?;
        if links.is_empty() {
            return Ok(Vec::new());
        }
        let mut env = Vec::new();
        let manifest = links
            .iter()
            .map(|link| {
                format!(
                    "{}={}",
                    link.target_workspace_name,
                    link.target_workspace_path.display()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        env.push((
            "ARCHDUCTOR_LINKED_DIRECTORIES".to_owned(),
            OsString::from(manifest),
        ));
        for link in links {
            env.push((
                format!(
                    "ARCHDUCTOR_LINKED_DIRECTORY_{}",
                    env_key_fragment(&link.target_workspace_name)
                ),
                link.target_workspace_path.as_os_str().to_owned(),
            ));
        }
        Ok(env)
    }

    fn local_chat_history_rows(
        &self,
        workspace_path: Option<&Path>,
    ) -> Result<Vec<LocalChatHistoryRow>> {
        let mut sql = String::from(
            "SELECT p.id, p.workspace_id, p.chat_thread_id, p.kind, p.command, p.pid, p.log_path, p.status,
                    p.started_at, p.exit_code, p.ended_at, p.session_harness_metadata, p.session_resume_id,
                    r.name, w.name, w.path
             FROM processes p
             JOIN workspaces w ON w.id = p.workspace_id
             JOIN repositories r ON r.id = w.repository_id
             WHERE p.kind = ?1",
        );
        if workspace_path.is_some() {
            sql.push_str(" AND w.path = ?2");
        }
        sql.push_str(" ORDER BY p.id DESC LIMIT 200");

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = if let Some(path) = workspace_path {
            stmt.query_map(
                params![ProcessKind::Session.as_str(), path.to_string_lossy()],
                row_to_local_chat_history_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            stmt.query_map(
                [ProcessKind::Session.as_str()],
                row_to_local_chat_history_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        Ok(rows)
    }

    fn local_chat_thread_rows(
        &self,
        workspace_path: Option<&Path>,
    ) -> Result<Vec<LocalChatThreadRow>> {
        let mut sql = String::from(
            "SELECT t.id, t.workspace_id, t.provider, t.title, t.status, t.native_thread_id,
                    t.harness_metadata, t.created_at, t.updated_at, t.archived_at,
                    r.name, w.name, w.path
             FROM chat_threads t
             JOIN workspaces w ON w.id = t.workspace_id
             JOIN repositories r ON r.id = w.repository_id",
        );
        if workspace_path.is_some() {
            sql.push_str(" WHERE w.path = ?1");
        }
        sql.push_str(" ORDER BY t.updated_at DESC, t.id DESC LIMIT 200");

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = if let Some(path) = workspace_path {
            stmt.query_map(
                [path.to_string_lossy().to_string()],
                row_to_local_chat_thread_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            stmt.query_map([], row_to_local_chat_thread_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        Ok(rows)
    }

    pub fn list_terminals(&self, name: &str) -> Result<Vec<ProcessRecord>> {
        self.list_processes(name, ProcessKind::Terminal)
    }

    pub fn list_runs(&self, name: &str) -> Result<Vec<ProcessRecord>> {
        self.list_processes(name, ProcessKind::Run)
    }

    pub fn list_checks(&self, name: &str) -> Result<Vec<ProcessRecord>> {
        self.list_processes(name, ProcessKind::Check)
    }

    pub fn list_setups(&self, name: &str) -> Result<Vec<ProcessRecord>> {
        self.list_processes(name, ProcessKind::Setup)
    }

    fn list_processes(&self, name: &str, kind: ProcessKind) -> Result<Vec<ProcessRecord>> {
        let workspace = self.get_by_name(name)?;
        let mut stmt = self.conn.prepare(
            "SELECT id, workspace_id, chat_thread_id, kind, command, pid, log_path, status, started_at, exit_code, ended_at, session_harness_metadata, session_resume_id
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
        anyhow::ensure!(
            !matches!(kind, SessionKind::Codex),
            "Codex sessions are owned by archcar; use ArchcarRequest::SpawnSession"
        );
        let launch = self.session_launch_with_options(name, kind, harness)?;
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let settings = load_repository_settings(&repository.root_path)?;
        let command = shell_words(&launch.program, &launch.args);
        self.start_process(StartProcessInput {
            kind: ProcessKind::Session,
            script: &command,
            settings: &settings,
            repository: &repository,
            workspace: &workspace,
            chat_thread_id: None,
            extra_env: &launch.env,
            session: ProcessSessionMetadata {
                harness_metadata: launch.harness_metadata.as_deref(),
                resume_id: launch.session_resume_id.as_deref(),
            },
        })
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
        self.record_process(RecordProcessInput {
            kind: ProcessKind::Session,
            workspace: &workspace,
            chat_thread_id: None,
            command: &command,
            pid,
            file_prefix: "session",
            session: ProcessSessionMetadata {
                harness_metadata: launch.harness_metadata.as_deref(),
                resume_id: launch.session_resume_id.as_deref(),
            },
        })
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

    pub fn workspace_repository_root(&self, name: &str) -> Result<PathBuf> {
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        Ok(repository.root_path)
    }

    pub fn save_local_default_agent_provider(
        &self,
        workspace_name: &str,
        provider: &str,
    ) -> Result<()> {
        let repo_path = self.workspace_repository_root(workspace_name)?;
        save_local_default_agent_provider(&repo_path, provider)
    }

    pub fn save_local_default_agent_provider_for_database(
        database_path: &Path,
        workspace_name: &str,
        provider: &str,
    ) -> Result<()> {
        Self::open(database_path)?.save_local_default_agent_provider(workspace_name, provider)
    }

    pub fn workspace_view_defaults(&self, name: &str) -> Result<WorkspaceViewDefaults> {
        let workspace = self.get_by_name(name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let settings = load_repository_settings(&repository.root_path)?;
        Ok(WorkspaceViewDefaults {
            default_visible_tab: settings
                .customization
                .workspace_defaults
                .default_visible_tab
                .clone(),
            theme: settings.customization.view.theme.clone(),
            accent_color: settings.customization.view.accent_color.clone(),
            colors: settings.customization.view.colors.clone(),
            density: settings.customization.view.density.clone(),
            keybindings: settings.customization.view.keybindings.clone(),
            terminal_font: settings.customization.view.terminal_font.clone(),
            terminal_scrollback: settings.customization.view.terminal_scrollback,
            command_palette_presets: repository_command_palette_presets(&settings),
            agent_profile_names: settings
                .customization
                .agent_profiles
                .keys()
                .cloned()
                .collect(),
            notification_rules: settings.customization.view.notification_rules.clone(),
        })
    }

    pub fn workspace_repo_settings(
        &self,
        workspace_name: &str,
    ) -> Result<crate::settings::RepositorySettings> {
        let workspace = self.get_by_name(workspace_name)?;
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        load_repository_settings(&repository.root_path)
    }

    pub fn create_chat_thread(
        &self,
        workspace_name: &str,
        provider: &str,
        title: &str,
        harness_metadata: Option<&str>,
    ) -> Result<ChatThreadRecord> {
        let workspace = self.get_by_name(workspace_name)?;
        let now = timestamp();
        self.conn.execute(
            "INSERT INTO chat_threads (
                workspace_id, provider, title, status, native_thread_id, harness_metadata, created_at, updated_at, archived_at
             ) VALUES (?1, ?2, ?3, 'active', NULL, ?4, ?5, ?5, NULL)",
            params![workspace.id, provider, title, harness_metadata, now],
        )?;
        self.get_chat_thread(self.conn.last_insert_rowid())
    }

    pub fn list_chat_threads(&self, workspace_name: &str) -> Result<Vec<ChatThreadRecord>> {
        let workspace = self.get_by_name(workspace_name)?;
        let mut stmt = self.conn.prepare(
            "SELECT id, workspace_id, provider, title, status, native_thread_id, harness_metadata, created_at, updated_at, archived_at
             FROM chat_threads
             WHERE workspace_id = ?1
             ORDER BY updated_at DESC, id DESC",
        )?;
        let threads = stmt
            .query_map([workspace.id], row_to_chat_thread)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(threads)
    }

    pub fn chat_thread_context_summaries(
        &self,
        workspace_name: &str,
    ) -> Result<Vec<ChatThreadContextSummary>> {
        let workspace = self.get_by_name(workspace_name)?;
        let mut stmt = self.conn.prepare(
            "SELECT
                t.title,
                t.provider,
                COALESCE(m.message_count, 0),
                COALESCE(m.message_bytes, 0),
                COALESCE(e.event_count, 0),
                COALESCE(e.event_bytes, 0)
             FROM chat_threads t
             LEFT JOIN (
                SELECT thread_id, COUNT(*) AS message_count, SUM(LENGTH(CAST(content AS BLOB))) AS message_bytes
                FROM chat_messages
                GROUP BY thread_id
             ) m ON m.thread_id = t.id
             LEFT JOIN (
                SELECT
                    thread_id,
                    COUNT(*) AS event_count,
                    SUM(
                        LENGTH(CAST(title AS BLOB)) +
                        LENGTH(CAST(body AS BLOB)) +
                        LENGTH(CAST(payload_json AS BLOB))
                    ) AS event_bytes
                FROM chat_events
                GROUP BY thread_id
             ) e ON e.thread_id = t.id
             WHERE t.workspace_id = ?1
             ORDER BY t.updated_at DESC, t.id DESC",
        )?;
        let rows = stmt
            .query_map([workspace.id], |row| {
                let message_count: i64 = row.get(2)?;
                let message_bytes: i64 = row.get(3)?;
                let event_count: i64 = row.get(4)?;
                let event_bytes: i64 = row.get(5)?;
                Ok(ChatThreadContextSummary {
                    title: row.get(0)?,
                    provider: row.get(1)?,
                    message_count: message_count.max(0) as usize,
                    event_count: event_count.max(0) as usize,
                    transcript_bytes: message_bytes.max(0) as usize + event_bytes.max(0) as usize,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_chat_thread_record(&self, thread_id: i64) -> Result<ChatThreadRecord> {
        self.get_chat_thread(thread_id)
    }

    pub fn append_chat_message(
        &self,
        thread_id: i64,
        role: &str,
        content: &str,
        source: &str,
    ) -> Result<ChatMessageRecord> {
        if let Some(existing) =
            self.latest_matching_adjacent_chat_message(thread_id, role, content, source)?
        {
            return Ok(existing);
        }
        if let Some(existing) =
            self.update_latest_mergeable_chat_message(thread_id, role, content, source)?
        {
            return Ok(existing);
        }

        let now = timestamp();
        let timeline_seq = self.next_chat_timeline_seq()?;
        self.conn.execute(
            "INSERT INTO chat_messages (thread_id, role, content, source, timeline_seq, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![thread_id, role, content, source, timeline_seq, now],
        )?;
        self.touch_chat_thread(thread_id, &now)?;
        self.get_chat_message(self.conn.last_insert_rowid())
    }

    fn append_agent_chat_message_with_metadata(
        &self,
        thread_id: i64,
        content: &str,
        source: &str,
    ) -> Result<()> {
        let accepts_metadata = !self.thread_has_agent_message(thread_id)?;
        let (content, directive) = if accepts_metadata {
            extract_archductor_metadata_directive(content)
        } else {
            (content.to_owned(), None)
        };
        if let Some(directive) = directive {
            if let Err(err) = self.apply_archductor_metadata_directive(thread_id, directive) {
                warn!(
                    thread_id,
                    error = %err,
                    "failed to apply archductor metadata directive"
                );
            }
        }
        if !content.trim().is_empty() {
            self.append_chat_message(thread_id, "agent", &content, source)?;
        }
        Ok(())
    }

    fn apply_archductor_metadata_directive(
        &self,
        thread_id: i64,
        directive: ArchductorMetadataDirective,
    ) -> Result<()> {
        let thread = self.get_chat_thread(thread_id)?;
        let workspace = self.get_by_id(thread.workspace_id)?;
        if !self.workspace_has_user_message(thread_id)? {
            return Ok(());
        }

        if let Some(chat_title) = directive
            .chat_title
            .as_deref()
            .and_then(normalize_chat_title)
        {
            if is_default_chat_thread_title_core(&thread.title) || thread.title == chat_title {
                self.update_chat_thread_title(thread_id, &chat_title)?;
            }
        }

        let mut workspace = workspace;
        if let Some(branch_name) = directive.branch_name.as_deref() {
            let branch_name = self.metadata_branch_name(&workspace, branch_name)?;
            if branch_name != workspace.branch {
                match self.rename_branch(&workspace.name, &branch_name) {
                    Ok(updated) => workspace = updated,
                    Err(err) => warn!(
                        workspace = %workspace.name,
                        branch = %branch_name,
                        error = %err,
                        "failed to apply archductor branch metadata"
                    ),
                }
            }
        }

        if let Some(workspace_name) = directive.workspace_name.as_deref() {
            let repository = self.load_repository_by_id(workspace.repository_id)?;
            let workspace_name =
                self.metadata_workspace_name(&repository, workspace.id, workspace_name)?;
            if workspace_name != workspace.name {
                self.rename(&workspace.name, &workspace_name)?;
            }
        }

        Ok(())
    }

    fn metadata_workspace_name(
        &self,
        repository: &RepositoryRecord,
        workspace_id: i64,
        raw: &str,
    ) -> Result<String> {
        let slug = slugify(raw);
        self.unique_message_workspace_name(repository, workspace_id, &slug)
    }

    fn metadata_branch_name(&self, workspace: &Workspace, raw: &str) -> Result<String> {
        let repository = self.load_repository_by_id(workspace.repository_id)?;
        let settings = load_repository_settings(&repository.root_path)?;
        let prefix = settings
            .customization
            .workspace_defaults
            .branch_prefix
            .as_deref()
            .unwrap_or("lc");
        let raw = raw.trim();
        let base = if validate_branch_name(raw).is_ok() && raw.contains('/') {
            if let Some(suffix) = raw.strip_prefix("lc/") {
                format!("{prefix}/{}", slugify(suffix))
            } else {
                raw.to_owned()
            }
        } else {
            format!("{prefix}/{}", slugify(raw))
        };
        unique_message_branch_name(&repository.root_path, &base, &workspace.branch)
    }

    fn workspace_has_user_message(&self, thread_id: i64) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM chat_messages WHERE thread_id = ?1 AND role = 'user'",
            [thread_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    fn thread_has_agent_message(&self, thread_id: i64) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM chat_messages WHERE thread_id = ?1 AND role = 'agent'",
            [thread_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    fn latest_matching_adjacent_chat_message(
        &self,
        thread_id: i64,
        role: &str,
        content: &str,
        source: &str,
    ) -> Result<Option<ChatMessageRecord>> {
        let latest = self
            .conn
            .query_row(
                "SELECT item_type, id, thread_id, role, content, source, timeline_seq, created_at, updated_at
                 FROM (
                   SELECT 'message' AS item_type, id, thread_id, role, content, source, timeline_seq, created_at, updated_at, COALESCE(timeline_seq, id) AS timeline_order
                   FROM chat_messages
                   WHERE thread_id = ?1
                   UNION ALL
                   SELECT 'event' AS item_type, id, thread_id, NULL AS role, NULL AS content, NULL AS source, timeline_seq, NULL AS created_at, NULL AS updated_at, timeline_seq AS timeline_order
                   FROM chat_events
                   WHERE thread_id = ?1
                 )
                 ORDER BY timeline_order DESC, id DESC
                 LIMIT 1",
                [thread_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                    ))
                },
            )
            .optional()?;
        Ok(latest.and_then(
            |(
                item_type,
                id,
                thread_id,
                latest_role,
                latest_content,
                latest_source,
                timeline_seq,
                created_at,
                updated_at,
            )| {
                (item_type == "message"
                    && latest_role.as_deref() == Some(role)
                    && latest_content.as_deref() == Some(content)
                    && latest_source.as_deref() == Some(source))
                .then(|| ChatMessageRecord {
                    id,
                    thread_id,
                    role: latest_role.unwrap(),
                    content: latest_content.unwrap(),
                    source: latest_source.unwrap(),
                    timeline_seq,
                    created_at: created_at.unwrap(),
                    updated_at: updated_at.unwrap(),
                })
            },
        ))
    }

    fn update_latest_mergeable_chat_message(
        &self,
        thread_id: i64,
        role: &str,
        content: &str,
        source: &str,
    ) -> Result<Option<ChatMessageRecord>> {
        let Some(existing) = self.latest_adjacent_chat_message(thread_id, role, source)? else {
            return Ok(None);
        };
        let Some(merged) = merge_message_content(&existing.content, content) else {
            return Ok(None);
        };
        if merged == existing.content {
            return Ok(Some(existing));
        }

        let now = timestamp();
        self.conn.execute(
            "UPDATE chat_messages
             SET content = ?1,
                 updated_at = ?2
             WHERE id = ?3",
            params![merged, now, existing.id],
        )?;
        self.touch_chat_thread(thread_id, &now)?;
        self.get_chat_message(existing.id).map(Some)
    }

    fn latest_adjacent_chat_message(
        &self,
        thread_id: i64,
        role: &str,
        source: &str,
    ) -> Result<Option<ChatMessageRecord>> {
        let latest = self
            .conn
            .query_row(
                "SELECT item_type, id, thread_id, role, content, source, timeline_seq, created_at, updated_at
                 FROM (
                   SELECT 'message' AS item_type, id, thread_id, role, content, source, timeline_seq, created_at, updated_at, COALESCE(timeline_seq, id) AS timeline_order
                   FROM chat_messages
                   WHERE thread_id = ?1
                   UNION ALL
                   SELECT 'event' AS item_type, id, thread_id, NULL AS role, NULL AS content, NULL AS source, timeline_seq, NULL AS created_at, NULL AS updated_at, timeline_seq AS timeline_order
                   FROM chat_events
                   WHERE thread_id = ?1
                 )
                 ORDER BY timeline_order DESC, id DESC
                 LIMIT 1",
                [thread_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                    ))
                },
            )
            .optional()?;
        Ok(latest.and_then(
            |(
                item_type,
                id,
                thread_id,
                latest_role,
                latest_content,
                latest_source,
                timeline_seq,
                created_at,
                updated_at,
            )| {
                (item_type == "message"
                    && latest_role.as_deref() == Some(role)
                    && latest_source.as_deref() == Some(source))
                .then(|| ChatMessageRecord {
                    id,
                    thread_id,
                    role: latest_role.unwrap(),
                    content: latest_content.unwrap(),
                    source: latest_source.unwrap(),
                    timeline_seq,
                    created_at: created_at.unwrap(),
                    updated_at: updated_at.unwrap(),
                })
            },
        ))
    }

    pub fn get_codex_parse_cursor(&self, process_id: i64) -> Result<Option<CodexParseCursor>> {
        let fingerprint = self
            .conn
            .query_row(
                "SELECT fingerprint
                 FROM codex_parse_cursors
                 WHERE process_id = ?1",
                [process_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?;
        Ok(fingerprint.map(|fingerprint| CodexParseCursor { fingerprint }))
    }

    pub fn set_codex_parse_cursor(&self, process_id: i64, cursor: &CodexParseCursor) -> Result<()> {
        let now = timestamp();
        self.conn.execute(
            "INSERT INTO codex_parse_cursors (process_id, fingerprint, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(process_id) DO UPDATE SET
               fingerprint = excluded.fingerprint,
               updated_at = excluded.updated_at",
            params![process_id, cursor.fingerprint.as_deref(), now],
        )?;
        Ok(())
    }

    pub fn append_chat_event(
        &self,
        thread_id: i64,
        process_id: i64,
        event: &CodexTranscriptEvent,
    ) -> Result<ChatEventRecord> {
        let fields = chat_event_fields(event)?;
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let outcome = (|| -> Result<ChatEventRecord> {
            if let Some(existing) = self.find_latest_exact_chat_event(
                thread_id,
                process_id,
                &fields.kind,
                &fields.title,
                &fields.body,
                &fields.payload_json,
            )? {
                return Ok(existing);
            }

            if let Some(existing) =
                self.find_latest_updatable_chat_event(thread_id, process_id, &fields)?
            {
                let now = timestamp();
                self.conn.execute(
                    "UPDATE chat_events
                     SET body = ?1,
                         path = ?2,
                         payload_json = ?3,
                         updated_at = ?4
                     WHERE id = ?5",
                    params![
                        fields.body,
                        fields.path,
                        fields.payload_json,
                        now,
                        existing.id
                    ],
                )?;
                self.touch_chat_thread(thread_id, &now)?;
                return self.get_chat_event(existing.id);
            }

            let now = timestamp();
            let timeline_seq = self.next_chat_timeline_seq()?;
            self.conn.execute(
                "INSERT INTO chat_events (
                    thread_id,
                    process_id,
                    kind,
                    title,
                    body,
                    path,
                    payload_json,
                    timeline_seq,
                    created_at,
                    updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
                params![
                    thread_id,
                    process_id,
                    fields.kind,
                    fields.title,
                    fields.body,
                    fields.path,
                    fields.payload_json,
                    timeline_seq,
                    now
                ],
            )?;
            self.touch_chat_thread(thread_id, &now)?;
            let event_id = self.conn.last_insert_rowid();
            self.get_chat_event(event_id)
        })();

        match outcome {
            Ok(event) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(event)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    pub fn persist_codex_screen_delta(
        &self,
        thread_id: i64,
        process_id: i64,
        screen: &str,
    ) -> Result<()> {
        let messages = self.list_chat_messages(thread_id)?;
        let benchmark = codex_parse_benchmark_from_messages(&messages);
        let previous_cursor = self.get_codex_parse_cursor(process_id)?;
        let delta = parse_codex_screen_delta(screen, &benchmark, previous_cursor.as_ref());
        let session_events = delta
            .items
            .iter()
            .cloned()
            .map(codex_parsed_item_to_session_event)
            .collect::<Vec<_>>();

        for item in delta.items {
            match item {
                CodexParsedItem::Message(message) => {
                    if message.role == ScreenMessageRole::Agent {
                        self.append_agent_chat_message_with_metadata(
                            thread_id,
                            &message.content,
                            "agent_screen_parse",
                        )?;
                    }
                }
                CodexParsedItem::Event(event) => {
                    self.append_chat_event(thread_id, process_id, &event)?;
                }
            }
        }
        self.append_session_events(process_id, session_events)?;

        self.set_codex_parse_cursor(process_id, &delta.cursor)?;
        Ok(())
    }

    pub fn persist_codex_pipeline_update(
        &self,
        thread_id: i64,
        process_id: i64,
        chunks: Vec<PtyChunkInput>,
        screen: &str,
        previous_state: AgentSessionState,
    ) -> Result<SessionPipelineOutput> {
        let messages = self.list_chat_messages(thread_id)?;
        let benchmark = codex_parse_benchmark_from_messages(&messages);
        let previous_cursor = self.get_codex_parse_cursor(process_id)?;
        let output = process_codex_pty_pipeline(SessionPipelineInput {
            chunks,
            screen: screen.to_owned(),
            benchmark: benchmark.clone(),
            previous_cursor: previous_cursor.clone(),
            previous_state,
        });

        let delta = parse_codex_screen_delta(screen, &benchmark, previous_cursor.as_ref());
        for item in delta.items {
            match item {
                CodexParsedItem::Message(message) => {
                    if message.role == ScreenMessageRole::Agent {
                        self.append_agent_chat_message_with_metadata(
                            thread_id,
                            &message.content,
                            "agent_screen_parse",
                        )?;
                    }
                }
                CodexParsedItem::Event(event) => {
                    self.append_chat_event(thread_id, process_id, &event)?;
                }
            }
        }
        self.append_session_events(process_id, output.events.clone())?;
        self.set_codex_parse_cursor(process_id, &output.cursor)?;
        Ok(output)
    }

    pub fn list_chat_events(&self, thread_id: i64) -> Result<Vec<ChatEventRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, thread_id, process_id, kind, title, body, path, payload_json, timeline_seq, created_at, updated_at
             FROM chat_events
             WHERE thread_id = ?1
             ORDER BY timeline_seq ASC, id ASC",
        )?;
        let events = stmt
            .query_map([thread_id], row_to_chat_event)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(events)
    }

    pub fn update_chat_thread_title(&self, thread_id: i64, title: &str) -> Result<()> {
        let title = title.trim();
        anyhow::ensure!(!title.is_empty(), "chat thread title is required");
        let now = timestamp();
        self.conn.execute(
            "UPDATE chat_threads
             SET title = ?1, updated_at = ?2
             WHERE id = ?3",
            params![title, now, thread_id],
        )?;
        Ok(())
    }

    pub fn close_chat_thread(&self, thread_id: i64) -> Result<()> {
        self.update_chat_thread_status(thread_id, "closed", true)
    }

    pub fn reopen_chat_thread(&self, thread_id: i64) -> Result<()> {
        self.update_chat_thread_status(thread_id, "active", false)
    }

    fn update_chat_thread_status(
        &self,
        thread_id: i64,
        status: &str,
        archived: bool,
    ) -> Result<()> {
        let now = timestamp();
        let archived_at = archived.then_some(now.as_str());
        let changed = self.conn.execute(
            "UPDATE chat_threads
             SET status = ?1, updated_at = ?2, archived_at = ?3
             WHERE id = ?4",
            params![status, now, archived_at, thread_id],
        )?;
        anyhow::ensure!(changed > 0, "chat thread {thread_id} not found");
        Ok(())
    }

    pub fn list_chat_messages(&self, thread_id: i64) -> Result<Vec<ChatMessageRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, thread_id, role, content, source, timeline_seq, created_at, updated_at
             FROM chat_messages
             WHERE thread_id = ?1
             ORDER BY COALESCE(timeline_seq, id) ASC, id ASC",
        )?;
        let messages = stmt
            .query_map([thread_id], row_to_chat_message)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(messages)
    }

    pub fn update_chat_thread_native_id(
        &self,
        thread_id: i64,
        native_thread_id: &str,
    ) -> Result<ChatThreadRecord> {
        let native_thread_id = native_thread_id.trim();
        anyhow::ensure!(!native_thread_id.is_empty(), "native thread id is required");
        let now = timestamp();
        self.conn.execute(
            "UPDATE chat_threads
             SET native_thread_id = ?1, updated_at = ?2
             WHERE id = ?3",
            params![native_thread_id, now, thread_id],
        )?;
        self.get_chat_thread(thread_id)
    }

    pub fn record_session_process_for_thread(
        &self,
        name: &str,
        thread_id: i64,
        launch: &SessionLaunch,
        pid: u32,
    ) -> Result<ProcessRecord> {
        anyhow::ensure!(pid > 0, "session process id is required");
        let workspace = self.get_by_name(name)?;
        let thread = self.get_chat_thread(thread_id)?;
        anyhow::ensure!(
            thread.workspace_id == workspace.id,
            "chat thread {thread_id} does not belong to workspace {name}"
        );
        let command = shell_words(&launch.program, &launch.args);
        let resume_id = match launch.kind {
            SessionKind::Codex => thread.native_thread_id.as_deref(),
            _ => launch.session_resume_id.as_deref(),
        };
        self.record_process(RecordProcessInput {
            kind: ProcessKind::Session,
            workspace: &workspace,
            chat_thread_id: Some(thread_id),
            command: &command,
            pid,
            file_prefix: "session",
            session: ProcessSessionMetadata {
                harness_metadata: launch.harness_metadata.as_deref(),
                resume_id,
            },
        })
    }

    pub fn list_thread_processes(&self, thread_id: i64) -> Result<Vec<ProcessRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, workspace_id, chat_thread_id, kind, command, pid, log_path, status, started_at, exit_code, ended_at, session_harness_metadata, session_resume_id
             FROM processes
             WHERE chat_thread_id = ?1
             ORDER BY id DESC",
        )?;
        let processes = stmt
            .query_map([thread_id], row_to_process)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(processes)
    }

    pub fn latest_thread_process(&self, thread_id: i64) -> Result<Option<ProcessRecord>> {
        let result = self.conn.query_row(
            "SELECT id, workspace_id, chat_thread_id, kind, command, pid, log_path, status, started_at, exit_code, ended_at, session_harness_metadata, session_resume_id
             FROM processes
             WHERE chat_thread_id = ?1
             ORDER BY id DESC LIMIT 1",
            [thread_id],
            row_to_process,
        );
        match result {
            Ok(process) => Ok(Some(process)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    pub fn resolve_codex_native_thread_id_for_process(
        &self,
        process_id: i64,
    ) -> Result<Option<String>> {
        let process = self.get_process(process_id)?;
        anyhow::ensure!(
            process.kind == ProcessKind::Session,
            "process {process_id} is not a session process"
        );
        let Some(thread_id) = process.chat_thread_id else {
            return Ok(None);
        };
        let thread = self.get_chat_thread(thread_id)?;
        if let Some(native_thread_id) = thread.native_thread_id {
            return Ok(Some(native_thread_id));
        }
        let cwd = self.get_by_id(process.workspace_id)?.path;
        let Some(native_thread_id) = find_codex_rollout_session_id(&cwd, &process.started_at)?
        else {
            return Ok(None);
        };
        self.update_chat_thread_native_id(thread_id, &native_thread_id)?;
        self.conn.execute(
            "UPDATE processes
             SET session_resume_id = ?1
             WHERE id = ?2",
            params![native_thread_id, process_id],
        )?;
        Ok(Some(native_thread_id))
    }

    fn touch_chat_thread(&self, thread_id: i64, now: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE chat_threads SET updated_at = ?1 WHERE id = ?2",
            params![now, thread_id],
        )?;
        Ok(())
    }

    fn next_chat_timeline_seq(&self) -> Result<i64> {
        self.conn
            .execute("INSERT INTO chat_timeline_seq DEFAULT VALUES", [])?;
        Ok(self.conn.last_insert_rowid())
    }

    fn latest_chat_timeline_seq(&self, thread_id: i64) -> Result<Option<i64>> {
        self.conn
            .query_row(
                "SELECT timeline_seq
                 FROM (
                   SELECT COALESCE(timeline_seq, id) AS timeline_seq
                   FROM chat_messages
                   WHERE thread_id = ?1
                   UNION ALL
                   SELECT timeline_seq
                   FROM chat_events
                   WHERE thread_id = ?1
                 )
                 ORDER BY timeline_seq DESC
                 LIMIT 1",
                [thread_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    fn find_latest_updatable_chat_event(
        &self,
        thread_id: i64,
        process_id: i64,
        fields: &ChatEventFields,
    ) -> Result<Option<ChatEventRecord>> {
        let existing = self
            .conn
            .query_row(
                "SELECT id, thread_id, process_id, kind, title, body, path, payload_json, timeline_seq, created_at, updated_at
                 FROM chat_events
                 WHERE thread_id = ?1
                   AND process_id = ?2
                   AND kind = ?3
                   AND title = ?4
                   AND COALESCE(path, '') = COALESCE(?5, '')
                   AND (?3 != 'file_change' OR body = ?6)
                 ORDER BY timeline_seq DESC, id DESC
                 LIMIT 1",
                params![
                    thread_id,
                    process_id,
                    fields.kind,
                    fields.title,
                    fields.path,
                    fields.body,
                ],
                row_to_chat_event,
            )
            .optional()?;
        let Some(existing) = existing else {
            return Ok(None);
        };
        let is_latest = Some(existing.timeline_seq) == self.latest_chat_timeline_seq(thread_id)?;
        Ok(
            (is_latest && is_streaming_body_growth(&existing.body, &fields.body))
                .then_some(existing),
        )
    }

    fn find_latest_exact_chat_event(
        &self,
        thread_id: i64,
        process_id: i64,
        kind: &str,
        title: &str,
        body: &str,
        payload_json: &str,
    ) -> Result<Option<ChatEventRecord>> {
        let existing = self
            .conn
            .query_row(
                "SELECT id, thread_id, process_id, kind, title, body, path, payload_json, timeline_seq, created_at, updated_at
                 FROM chat_events
                 WHERE thread_id = ?1
                   AND process_id = ?2
                   AND kind = ?3
                   AND title = ?4
                   AND body = ?5
                   AND payload_json = ?6
                 ORDER BY timeline_seq DESC, id DESC
                 LIMIT 1",
                params![thread_id, process_id, kind, title, body, payload_json],
                row_to_chat_event,
            )
            .optional()?;
        let Some(existing) = existing else {
            return Ok(None);
        };
        Ok(
            (Some(existing.timeline_seq) == self.latest_chat_timeline_seq(thread_id)?)
                .then_some(existing),
        )
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

    fn get_chat_thread(&self, id: i64) -> Result<ChatThreadRecord> {
        self.conn
            .query_row(
                "SELECT id, workspace_id, provider, title, status, native_thread_id, harness_metadata, created_at, updated_at, archived_at
                 FROM chat_threads WHERE id = ?1",
                [id],
                row_to_chat_thread,
            )
            .with_context(|| format!("load chat thread {id}"))
    }

    fn get_chat_message(&self, id: i64) -> Result<ChatMessageRecord> {
        self.conn
            .query_row(
                "SELECT id, thread_id, role, content, source, timeline_seq, created_at, updated_at
                 FROM chat_messages WHERE id = ?1",
                [id],
                row_to_chat_message,
            )
            .with_context(|| format!("load chat message {id}"))
    }

    fn get_chat_event(&self, id: i64) -> Result<ChatEventRecord> {
        self.conn
            .query_row(
                "SELECT id, thread_id, process_id, kind, title, body, path, payload_json, timeline_seq, created_at, updated_at
                 FROM chat_events WHERE id = ?1",
                [id],
                row_to_chat_event,
            )
            .with_context(|| format!("load chat event {id}"))
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

    fn next_port_base(&self, port_block_size: u16) -> Result<u16> {
        anyhow::ensure!(
            port_block_size > 0,
            "workspace port block size must be greater than 0"
        );
        let next = self
            .conn
            .query_row("SELECT MAX(port_base) FROM workspaces", [], |row| {
                row.get::<_, Option<i64>>(0)
            })?
            .map(|port| port + i64::from(port_block_size))
            .unwrap_or(3000);
        u16::try_from(next).context("workspace port base exceeded u16 range")
    }

    fn resolve_workspace_name(
        &self,
        repository: &RepositoryRecord,
        settings: &crate::settings::RepositorySettings,
        requested_name: &str,
    ) -> Result<String> {
        let requested_name = requested_name.trim();
        if !requested_name.is_empty() {
            anyhow::ensure!(
                self.workspace_name_available(repository, requested_name)?,
                "workspace {requested_name} already exists"
            );
            return Ok(requested_name.to_owned());
        }

        let city_mode = settings
            .customization
            .naming
            .workspace_name_style
            .as_deref()
            .unwrap_or("city")
            == "city";
        let bases = if city_mode {
            WORKSPACE_CITY_NAMES.as_slice()
        } else {
            &["workspace"]
        };
        let active_names = self.active_workspace_names()?;
        let available = bases
            .iter()
            .copied()
            .filter(|candidate| !active_names.contains(*candidate))
            .filter(|candidate| {
                self.workspace_name_available(repository, candidate)
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();

        if !available.is_empty() {
            let index = random_index(available.len());
            return Ok(available[index].to_owned());
        }

        for suffix in 0.. {
            for base in bases {
                let candidate = if suffix == 0 {
                    (*base).to_owned()
                } else {
                    format!("{base}-{suffix}")
                };
                if self.workspace_name_available(repository, &candidate)? {
                    return Ok(candidate);
                }
            }
        }

        unreachable!("workspace name generation should always return")
    }

    fn workspace_name_available(&self, repository: &RepositoryRecord, name: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM workspaces WHERE name = ?1",
            params![name],
            |row| row.get(0),
        )?;
        Ok(count == 0 && !repository.workspace_parent_path.join(name).exists())
    }

    fn workspace_name_available_for_rename(
        &self,
        repository: &RepositoryRecord,
        workspace_id: i64,
        name: &str,
    ) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM workspaces WHERE name = ?1 AND id != ?2",
            params![name, workspace_id],
            |row| row.get(0),
        )?;
        if count > 0 {
            return Ok(false);
        }
        let candidate_path = repository.workspace_parent_path.join(name);
        if !candidate_path.exists() {
            return Ok(true);
        }
        let current_path: String = self.conn.query_row(
            "SELECT path FROM workspaces WHERE id = ?1",
            [workspace_id],
            |row| row.get(0),
        )?;
        Ok(Path::new(&current_path) == candidate_path)
    }

    fn workspace_has_chat_messages(&self, workspace_id: i64) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*)
             FROM chat_messages
             WHERE thread_id IN (SELECT id FROM chat_threads WHERE workspace_id = ?1)",
            [workspace_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    fn unique_message_workspace_name(
        &self,
        repository: &RepositoryRecord,
        workspace_id: i64,
        base: &str,
    ) -> Result<String> {
        for suffix in 0.. {
            let candidate = if suffix == 0 {
                base.to_owned()
            } else {
                format!("{base}-{suffix}")
            };
            validate_workspace_name(&candidate)?;
            if self.workspace_name_available_for_rename(repository, workspace_id, &candidate)? {
                return Ok(candidate);
            }
        }

        unreachable!("workspace name generation should always return")
    }

    fn active_workspace_names(&self) -> Result<HashSet<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM workspaces WHERE status = 'active' ORDER BY id")?;
        let names = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(names.into_iter().collect())
    }

    fn resolve_workspace_branch(
        &self,
        settings: &crate::settings::RepositorySettings,
        requested_branch: &str,
        workspace_name: &str,
    ) -> String {
        let requested_branch = requested_branch.trim();
        if !requested_branch.is_empty() {
            return requested_branch.to_owned();
        }

        let prefix = settings
            .customization
            .workspace_defaults
            .branch_prefix
            .as_deref()
            .unwrap_or("lc");
        format!("{prefix}/{}", slugify(workspace_name))
    }

    fn start_process(&self, input: StartProcessInput<'_>) -> Result<ProcessRecord> {
        let StartProcessInput {
            kind,
            script,
            settings,
            repository,
            workspace,
            chat_thread_id,
            extra_env,
            session,
        } = input;
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
        let cwd = workspace_working_directory(settings, workspace)?;
        let mut env = conductor_environment(settings, repository, workspace)?;
        env.extend(self.linked_directory_env(workspace)?);
        env.extend(extra_env.iter().cloned());

        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg(script)
            .current_dir(&cwd)
            .envs(env)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(stderr));
        #[cfg(unix)]
        {
            command.process_group(0);
        }
        let child = command
            .spawn()
            .with_context(|| format!("start script in {}", cwd.display()))?;

        self.conn.execute(
            "INSERT INTO processes (
                workspace_id, chat_thread_id, kind, command, pid, log_path, status, started_at, exit_code, ended_at, session_harness_metadata, session_resume_id
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, ?9, ?10)",
            params![
                workspace.id,
                chat_thread_id,
                kind.as_str(),
                script,
                i64::from(child.id()),
                log_path.to_string_lossy().to_string(),
                ProcessStatus::Running.as_str(),
                now,
                session.harness_metadata,
                session.resume_id,
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
                "SELECT id, workspace_id, chat_thread_id, kind, command, pid, log_path, status, started_at, exit_code, ended_at, session_harness_metadata, session_resume_id
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
                "SELECT id, workspace_id, chat_thread_id, kind, command, pid, log_path, status, started_at, exit_code, ended_at, session_harness_metadata, session_resume_id
                 FROM processes WHERE id = ?1",
                [id],
                row_to_process,
            )
            .with_context(|| format!("load process {id}"))
    }

    fn workspace_name_by_id(&self, id: i64) -> Result<String> {
        self.conn
            .query_row("SELECT name FROM workspaces WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .with_context(|| format!("load workspace name for id {id}"))
    }

    fn repository_name_by_id(&self, id: i64) -> Result<String> {
        self.conn
            .query_row("SELECT name FROM repositories WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .with_context(|| format!("load repository name for id {id}"))
    }

    fn record_workspace_event(
        &self,
        workspace_id: i64,
        workspace_name: &str,
        kind: &str,
        summary: &str,
    ) -> Result<WorkspaceTimelineEvent> {
        let now = timestamp();
        self.conn.execute(
            "INSERT INTO workspace_timeline (
                workspace_id, workspace_name, kind, summary, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![workspace_id, workspace_name, kind, summary, now],
        )?;
        self.get_workspace_timeline_event(self.conn.last_insert_rowid())
    }

    fn get_workspace_timeline_event(&self, id: i64) -> Result<WorkspaceTimelineEvent> {
        self.conn
            .query_row(
                "SELECT id, workspace_id, workspace_name, kind, summary, created_at
                 FROM workspace_timeline WHERE id = ?1",
                [id],
                row_to_workspace_timeline_event,
            )
            .with_context(|| format!("load workspace timeline event {id}"))
    }

    fn sync_workspace_commit_events(&self, workspace: &Workspace) -> Result<()> {
        let output = git_output_dynamic(
            &workspace.path,
            &[
                "log",
                "--date=iso-strict",
                "--format=%H%x1f%h%x1f%ad%x1f%s",
                "--max-count=50",
                workspace.branch.as_str(),
            ],
        )?;
        for line in output.lines().rev() {
            let mut parts = line.splitn(4, '\x1f');
            let Some(hash) = parts.next().filter(|value| !value.is_empty()) else {
                continue;
            };
            let short = parts.next().unwrap_or(hash);
            let committed_at = parts.next().unwrap_or("");
            let subject = parts.next().unwrap_or("");
            let summary = format!("Commit {short}: {subject}");
            let exists = self.conn.query_row(
                "SELECT 1 FROM workspace_timeline
                 WHERE workspace_id = ?1 AND kind = 'commit.created' AND summary LIKE ?2
                 LIMIT 1",
                params![workspace.id, format!("Commit {short}:%")],
                |_| Ok(()),
            );
            if matches!(exists, Err(rusqlite::Error::QueryReturnedNoRows)) {
                self.conn.execute(
                    "INSERT INTO workspace_timeline (
                        workspace_id, workspace_name, kind, summary, created_at
                     ) VALUES (?1, ?2, 'commit.created', ?3, ?4)",
                    params![
                        workspace.id,
                        workspace.name,
                        summary,
                        if committed_at.is_empty() {
                            timestamp()
                        } else {
                            committed_at.to_owned()
                        },
                    ],
                )?;
            }
        }
        Ok(())
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
        crate::storage::migrate_workspace_db(&self.conn)?;
        self.backfill_chat_message_timeline_seq()?;
        Ok(())
    }

    fn backfill_chat_message_timeline_seq(&self) -> Result<()> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<()> {
            let mut stmt = self.conn.prepare(
                "SELECT id
                 FROM chat_messages
                 WHERE timeline_seq IS NULL
                 ORDER BY created_at ASC, id ASC",
            )?;
            let message_ids = stmt
                .query_map([], |row| row.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            for message_id in message_ids {
                let timeline_seq = self.next_chat_timeline_seq()?;
                self.conn.execute(
                    "UPDATE chat_messages
                     SET timeline_seq = ?1
                     WHERE id = ?2",
                    params![timeline_seq, message_id],
                )?;
            }

            Ok(())
        })();

        match result {
            Ok(()) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(())
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }
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
        let updated = conn.execute(
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
        if matches!(updated, Ok(1)) {
            let _ = record_process_exit_timeline_event(&conn, process_id, status.code());
        }
    });
}

fn record_process_exit_timeline_event(
    conn: &Connection,
    process_id: i64,
    exit_code: Option<i32>,
) -> Result<()> {
    let (workspace_id, workspace_name, kind): (i64, String, String) = conn
        .query_row(
            "SELECT p.workspace_id, w.name, p.kind
             FROM processes p
             JOIN workspaces w ON w.id = p.workspace_id
             WHERE p.id = ?1",
            [process_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .with_context(|| format!("load process {process_id} for timeline"))?;
    if !matches!(kind.as_str(), "setup" | "run" | "check") {
        return Ok(());
    }
    let status = if exit_code == Some(0) {
        "completed"
    } else {
        "failed"
    };
    let event_kind = format!("{kind}.{status}");
    let summary = match exit_code {
        Some(code) => format!("{kind} script {status} with exit code {code}"),
        None => format!("{kind} script {status} without an exit code"),
    };
    let now = timestamp();
    conn.execute(
        "INSERT INTO workspace_timeline (
            workspace_id, workspace_name, kind, summary, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![workspace_id, workspace_name, event_kind, summary, now],
    )?;
    Ok(())
}

pub fn format_codex_raw_output(raw: &str) -> String {
    format!("[codex raw]\n{raw}\n[/codex raw]\n")
}

pub fn format_codex_screen_snapshot(screen: &str) -> String {
    format!(
        "[codex screen]\n{}\n[/codex screen]\n",
        screen.trim_end_matches('\n')
    )
}

fn codex_parse_benchmark_from_messages(messages: &[ChatMessageRecord]) -> CodexParseBenchmark {
    CodexParseBenchmark {
        last_user_message: messages
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .map(|message| message.content.clone()),
        last_agent_message: messages
            .iter()
            .rev()
            .find(|message| message.role == "agent")
            .map(|message| message.content.clone()),
    }
}

fn is_streaming_body_growth(existing: &str, incoming: &str) -> bool {
    !incoming.is_empty() && incoming != existing && incoming.starts_with(existing)
}

struct ChatEventFields {
    kind: String,
    title: String,
    body: String,
    path: Option<String>,
    payload_json: String,
}

fn chat_event_fields(event: &CodexTranscriptEvent) -> Result<ChatEventFields> {
    let payload_json = match event {
        CodexTranscriptEvent::Tool { title, body } => json!({
            "type": "tool",
            "title": title,
            "body": body,
        })
        .to_string(),
        CodexTranscriptEvent::Skill { title, body } => json!({
            "type": "skill",
            "title": title,
            "body": body,
        })
        .to_string(),
        CodexTranscriptEvent::FileChange(change) => json!({
            "type": "file_change",
            "action": match change.action {
                CodexFileChangeAction::Added => "added",
                CodexFileChangeAction::Edited => "edited",
                CodexFileChangeAction::Deleted => "deleted",
            },
            "path": &change.path,
            "additions": change.additions,
            "deletions": change.deletions,
            "lines": &change.lines,
        })
        .to_string(),
    };

    Ok(match event {
        CodexTranscriptEvent::Tool { title, body } => ChatEventFields {
            kind: "tool".to_owned(),
            title: title.clone(),
            body: body.clone(),
            path: None,
            payload_json,
        },
        CodexTranscriptEvent::Skill { title, body } => ChatEventFields {
            kind: "skill".to_owned(),
            title: title.clone(),
            body: body.clone(),
            path: None,
            payload_json,
        },
        CodexTranscriptEvent::FileChange(change) => ChatEventFields {
            kind: "file_change".to_owned(),
            title: change.path.clone(),
            body: match change.action {
                CodexFileChangeAction::Added => "added",
                CodexFileChangeAction::Edited => "edited",
                CodexFileChangeAction::Deleted => "deleted",
            }
            .to_owned(),
            path: Some(change.path.clone()),
            payload_json,
        },
    })
}

fn find_codex_rollout_session_id(cwd: &Path, started_at: &str) -> Result<Option<String>> {
    let Some(home) = std::env::var_os("HOME") else {
        return Ok(None);
    };
    let root = PathBuf::from(home).join(".codex/sessions");
    if !root.exists() {
        return Ok(None);
    }
    let started_at = started_at.parse::<u64>().unwrap_or(0);
    let min_started_at = started_at.saturating_sub(5);
    let max_started_at = started_at.saturating_add(600);
    let mut candidates = Vec::new();
    for entry in WalkDir::new(&root)
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy();
        if !file_name.starts_with("rollout-") || !file_name.ends_with(".jsonl") {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let modified_at = match metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
        {
            Some(value) if value >= min_started_at && value <= max_started_at => value,
            _ => continue,
        };
        let Some(meta) = read_codex_rollout_session_meta(entry.path())? else {
            continue;
        };
        if meta.cwd != cwd {
            continue;
        }
        candidates.push((modified_at, meta.session_id));
    }
    candidates.sort_by_key(|candidate| candidate.0);
    Ok(candidates.into_iter().next().map(|candidate| candidate.1))
}

struct CodexRolloutMeta {
    cwd: PathBuf,
    session_id: String,
}

fn read_codex_rollout_session_meta(path: &Path) -> Result<Option<CodexRolloutMeta>> {
    let Some(line) = fs::read_to_string(path)
        .with_context(|| format!("read codex rollout {}", path.display()))?
        .lines()
        .next()
        .map(str::to_owned)
    else {
        return Ok(None);
    };
    let value: Value = serde_json::from_str(&line)
        .with_context(|| format!("parse codex rollout {}", path.display()))?;
    if value.get("type").and_then(Value::as_str) != Some("session_meta") {
        return Ok(None);
    }
    let Some(payload) = value.get("payload") else {
        return Ok(None);
    };
    let Some(session_id) = payload.get("session_id").and_then(Value::as_str) else {
        return Ok(None);
    };
    let Some(cwd) = payload.get("cwd").and_then(Value::as_str) else {
        return Ok(None);
    };
    Ok(Some(CodexRolloutMeta {
        cwd: PathBuf::from(cwd),
        session_id: session_id.to_owned(),
    }))
}

fn remove_workspace_worktree(repository_root: &Path, workspace_path: &Path) -> Result<()> {
    if !workspace_path.exists() {
        let _ = git_dynamic(repository_root, &["worktree", "prune"]);
        return Ok(());
    }

    let workspace_path_arg = workspace_path.to_string_lossy();
    match git_dynamic(
        repository_root,
        &["worktree", "remove", "--force", workspace_path_arg.as_ref()],
    ) {
        Ok(()) => Ok(()),
        Err(err) => {
            let top_level = git_output_dynamic(workspace_path, &["rev-parse", "--show-toplevel"])
                .with_context(|| {
                format!(
                    "confirm fallback worktree path {} after git worktree remove failed: {err:#}",
                    workspace_path.display()
                )
            })?;
            let top_level = PathBuf::from(top_level.trim());
            let canonical_workspace_path = workspace_path.canonicalize().with_context(|| {
                format!(
                    "canonicalize workspace path {} after git worktree remove failed: {err:#}",
                    workspace_path.display()
                )
            })?;
            anyhow::ensure!(
                top_level == canonical_workspace_path,
                "refusing fallback delete for {} because git top-level is {} after git worktree remove failed: {err:#}",
                workspace_path.display(),
                top_level.display()
            );
            let repository_common_dir = git_common_dir(repository_root).with_context(|| {
                format!(
                    "confirm repository git common dir {} after git worktree remove failed: {err:#}",
                    repository_root.display()
                )
            })?;
            let workspace_common_dir = git_common_dir(workspace_path).with_context(|| {
                format!(
                    "confirm workspace git common dir {} after git worktree remove failed: {err:#}",
                    workspace_path.display()
                )
            })?;
            anyhow::ensure!(
                workspace_common_dir == repository_common_dir,
                "refusing fallback delete for {} because git common dir {} does not match repository common dir {} after git worktree remove failed: {err:#}",
                workspace_path.display(),
                workspace_common_dir.display(),
                repository_common_dir.display()
            );
            fs::remove_dir_all(workspace_path).with_context(|| {
                format!(
                    "remove moved worktree directory {} after git worktree remove failed: {err:#}",
                    workspace_path.display()
                )
            })?;
            let _ = git_dynamic(repository_root, &["worktree", "prune"]);
            Ok(())
        }
    }
}

fn git_common_dir(path: &Path) -> Result<PathBuf> {
    let raw = git_output_dynamic(path, &["rev-parse", "--git-common-dir"])?;
    let common_dir = PathBuf::from(raw.trim());
    let common_dir = if common_dir.is_absolute() {
        common_dir
    } else {
        path.join(common_dir)
    };
    common_dir
        .canonicalize()
        .with_context(|| format!("canonicalize git common dir {}", common_dir.display()))
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
    let kind = match row.get::<_, String>(3)?.as_str() {
        "setup" => ProcessKind::Setup,
        "run" => ProcessKind::Run,
        "check" => ProcessKind::Check,
        "session" => ProcessKind::Session,
        "terminal" => ProcessKind::Terminal,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let pid = row.get::<_, i64>(5)?;
    let chat_thread_id = row.get::<_, Option<i64>>("chat_thread_id").ok().flatten();
    let session_harness_metadata = row
        .get::<_, Option<String>>("session_harness_metadata")
        .ok()
        .flatten();
    let session_resume_id = row
        .get::<_, Option<String>>("session_resume_id")
        .ok()
        .flatten();
    Ok(ProcessRecord {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        chat_thread_id,
        kind,
        command: row.get(4)?,
        pid: u32::try_from(pid).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Integer,
                Box::new(err),
            )
        })?,
        log_path: PathBuf::from(row.get::<_, String>(6)?),
        status: ProcessStatus::from_str(&row.get::<_, String>(7)?)?,
        started_at: row.get(8)?,
        exit_code: row.get(9)?,
        ended_at: row.get(10)?,
        session_harness_metadata,
        session_resume_id,
    })
}

fn session_event_source_to_str(source: SessionEventSource) -> &'static str {
    match source {
        SessionEventSource::User => "user",
        SessionEventSource::Assistant => "assistant",
        SessionEventSource::Runtime => "runtime",
        SessionEventSource::System => "system",
    }
}

fn session_event_source_from_str(value: &str) -> rusqlite::Result<SessionEventSource> {
    match value {
        "user" => Ok(SessionEventSource::User),
        "assistant" => Ok(SessionEventSource::Assistant),
        "runtime" => Ok(SessionEventSource::Runtime),
        "system" => Ok(SessionEventSource::System),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn row_to_local_chat_history_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LocalChatHistoryRow> {
    Ok(LocalChatHistoryRow {
        process: row_to_process(row)?,
        repository_name: row.get(13)?,
        workspace_name: row.get(14)?,
        workspace_path: PathBuf::from(row.get::<_, String>(15)?),
    })
}

fn row_to_local_chat_thread_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LocalChatThreadRow> {
    Ok(LocalChatThreadRow {
        thread: row_to_chat_thread(row)?,
        repository_name: row.get(10)?,
        workspace_name: row.get(11)?,
        workspace_path: PathBuf::from(row.get::<_, String>(12)?),
    })
}

fn row_to_chat_thread(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatThreadRecord> {
    Ok(ChatThreadRecord {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        provider: row.get(2)?,
        title: row.get(3)?,
        status: row.get(4)?,
        native_thread_id: row.get(5)?,
        harness_metadata: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
        archived_at: row.get(9)?,
    })
}

fn row_to_chat_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatMessageRecord> {
    Ok(ChatMessageRecord {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        role: row.get(2)?,
        content: row.get(3)?,
        source: row.get(4)?,
        timeline_seq: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn row_to_chat_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatEventRecord> {
    Ok(ChatEventRecord {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        process_id: row.get(2)?,
        kind: row.get(3)?,
        title: row.get(4)?,
        body: row.get(5)?,
        path: row.get(6)?,
        payload_json: row.get(7)?,
        timeline_seq: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

fn row_to_workspace_timeline_event(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<WorkspaceTimelineEvent> {
    Ok(WorkspaceTimelineEvent {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        workspace_name: row.get(2)?,
        kind: row.get(3)?,
        summary: row.get(4)?,
        created_at: row.get(5)?,
    })
}

fn row_to_pty_chunk(row: &rusqlite::Row<'_>) -> rusqlite::Result<PtyChunkRecord> {
    Ok(PtyChunkRecord {
        id: row.get(0)?,
        process_id: row.get(1)?,
        sequence: row.get::<_, i64>(2)? as u64,
        occurred_at_ms: row.get::<_, i64>(3)? as u64,
        stream: row.get(4)?,
        text: row.get(5)?,
        created_at: row.get(6)?,
    })
}

fn row_to_linked_directory(row: &rusqlite::Row<'_>) -> rusqlite::Result<LinkedDirectory> {
    let workspace_name: String = row.get(2)?;
    let workspace_path = PathBuf::from(row.get::<_, String>(3)?);
    let target_workspace_name: String = row.get(5)?;
    Ok(LinkedDirectory {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        workspace_name,
        workspace_path: workspace_path.clone(),
        target_workspace_id: row.get(4)?,
        target_workspace_name: target_workspace_name.clone(),
        target_workspace_path: PathBuf::from(row.get::<_, String>(6)?),
        link_path: linked_directory_path(&workspace_path, &target_workspace_name),
        created_at: row.get(7)?,
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

fn linked_directory_root(workspace_path: &Path) -> PathBuf {
    workspace_path.join(".context/linked-directories")
}

fn linked_directory_path(workspace_path: &Path, target_workspace_name: &str) -> PathBuf {
    linked_directory_root(workspace_path).join(target_workspace_name)
}

fn materialize_linked_directory(link: &LinkedDirectory) -> Result<()> {
    anyhow::ensure!(
        link.target_workspace_path.is_dir(),
        "target workspace {} does not exist at {}",
        link.target_workspace_name,
        link.target_workspace_path.display()
    );
    if let Some(parent) = link.link_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create linked directory root {}", parent.display()))?;
    }
    if let Ok(existing) = fs::read_link(&link.link_path) {
        if existing == link.target_workspace_path {
            return Ok(());
        }
        fs::remove_file(&link.link_path)
            .with_context(|| format!("replace linked directory {}", link.link_path.display()))?;
    } else if link.link_path.exists() {
        anyhow::bail!(
            "linked directory path {} already exists and is not a symlink",
            link.link_path.display()
        );
    }
    create_directory_symlink(&link.target_workspace_path, &link.link_path).with_context(|| {
        format!(
            "link {} to {}",
            link.link_path.display(),
            link.target_workspace_path.display()
        )
    })
}

fn remove_linked_directory_path(link: &LinkedDirectory) -> Result<()> {
    match fs::read_link(&link.link_path) {
        Ok(_) => fs::remove_file(&link.link_path)
            .with_context(|| format!("remove linked directory {}", link.link_path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) if !link.link_path.exists() => Ok(()),
        Err(_) => anyhow::bail!(
            "linked directory path {} exists and is not a symlink",
            link.link_path.display()
        ),
    }
}

#[cfg(unix)]
fn create_directory_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_directory_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
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
                staged: false,
                unstaged: false,
                untracked: false,
            })
        })
        .collect()
}

fn diff_hunk_summaries(diff: &str, staged: bool) -> Vec<DiffHunkSummary> {
    if diff.trim().is_empty() {
        return Vec::new();
    }
    if let Some(reason) = hunk_unsupported_reason(diff) {
        return vec![DiffHunkSummary {
            index: 0,
            header: "Unsupported hunks".to_owned(),
            additions: 0,
            deletions: 0,
            staged,
            unsupported_reason: Some(reason),
        }];
    }
    let (_header, hunks) = split_diff_header_and_hunks(diff);
    hunks
        .iter()
        .enumerate()
        .map(|(index, hunk)| {
            let mut additions = 0;
            let mut deletions = 0;
            for line in hunk.lines() {
                if line.starts_with('+') && !line.starts_with("+++") {
                    additions += 1;
                } else if line.starts_with('-') && !line.starts_with("---") {
                    deletions += 1;
                }
            }
            DiffHunkSummary {
                index,
                header: hunk.lines().next().unwrap_or("@@").to_owned(),
                additions,
                deletions,
                staged,
                unsupported_reason: None,
            }
        })
        .collect()
}

fn diff_hunk_patch(diff: &str, hunk_index: usize) -> Result<String> {
    validate_hunk_diff_supported(diff)?;
    let (header, hunks) = split_diff_header_and_hunks(diff);
    let hunk = hunks
        .get(hunk_index)
        .with_context(|| format!("hunk {} was not found", hunk_index + 1))?;
    Ok(format!("{header}{hunk}"))
}

fn split_diff_header_and_hunks(diff: &str) -> (String, Vec<String>) {
    let mut header = String::new();
    let mut hunks = Vec::new();
    let mut current_hunk: Option<String> = None;
    for line in diff.lines() {
        if line.starts_with("@@") {
            if let Some(hunk) = current_hunk.take() {
                hunks.push(hunk);
            }
            current_hunk = Some(String::new());
        }
        if let Some(hunk) = current_hunk.as_mut() {
            hunk.push_str(line);
            hunk.push('\n');
        } else {
            header.push_str(line);
            header.push('\n');
        }
    }
    if let Some(hunk) = current_hunk {
        hunks.push(hunk);
    }
    (header, hunks)
}

fn validate_hunk_diff_supported(diff: &str) -> Result<()> {
    if let Some(reason) = hunk_unsupported_reason(diff) {
        anyhow::bail!("{reason}; use file-level stage/unstage instead");
    }
    Ok(())
}

fn validate_hunk_patch_supported(patch: &str) -> Result<()> {
    anyhow::ensure!(
        patch.contains("\n@@"),
        "selected hunk patch is invalid; use file-level stage/unstage instead"
    );
    Ok(())
}

fn hunk_unsupported_reason(diff: &str) -> Option<String> {
    if diff.len() > DIFF_HUNK_PATCH_LIMIT_BYTES {
        return Some("large file hunks are unsupported".to_owned());
    }
    if diff.contains("GIT binary patch") || diff.contains("Binary files ") {
        return Some("binary file hunks are unsupported".to_owned());
    }
    None
}

fn workspace_diff_stats_against_base(workspace: &Workspace) -> Result<(usize, usize)> {
    let base_ref = workspace_base_ref(workspace);
    let diff = git_output_dynamic(&workspace.path, &["diff", "--numstat", base_ref, "--"])
        .or_else(|err| {
            if base_ref == "main" {
                Err(err)
            } else {
                git_output(&workspace.path, ["diff", "--numstat", "main", "--"])
            }
        })?;
    let mut additions = 0;
    let mut deletions = 0;
    for summary in parse_diff_numstat(&diff) {
        additions += summary.additions.unwrap_or_default();
        deletions += summary.deletions.unwrap_or_default();
    }

    let status = git_output(
        &workspace.path,
        ["status", "--porcelain", "--untracked-files=all"],
    )?;
    for path in parse_untracked_status_paths(&status) {
        if is_conductor_context_path(&path) {
            continue;
        }
        let counts = untracked_file_counts(&workspace.path.join(&path))?;
        additions += counts.0;
    }

    Ok((additions, deletions))
}

fn workspace_base_ref(workspace: &Workspace) -> &str {
    if workspace.base_ref.trim().is_empty() {
        "main"
    } else {
        workspace.base_ref.as_str()
    }
}

fn merge_diff_summaries(summaries: Vec<DiffFileSummary>) -> Vec<DiffFileSummary> {
    let mut merged = BTreeMap::<String, DiffFileSummary>::new();
    for summary in summaries {
        let entry = merged
            .entry(summary.path.clone())
            .or_insert_with(|| DiffFileSummary {
                path: summary.path.clone(),
                additions: Some(0),
                deletions: Some(0),
                staged: false,
                unstaged: false,
                untracked: false,
            });
        entry.additions = match (entry.additions, summary.additions) {
            (Some(left), Some(right)) => Some(left + right),
            _ => None,
        };
        entry.deletions = match (entry.deletions, summary.deletions) {
            (Some(left), Some(right)) => Some(left + right),
            _ => None,
        };
        entry.staged |= summary.staged;
        entry.unstaged |= summary.unstaged;
        entry.untracked |= summary.untracked;
    }
    merged.into_values().collect()
}

fn apply_diff_file_status(summaries: &mut [DiffFileSummary], output: &str) {
    for line in output.lines() {
        let Some((path, staged, unstaged, untracked)) = parse_status_summary_line(line) else {
            continue;
        };
        for summary in summaries.iter_mut() {
            if summary.path == path || summary.path.contains(&path) {
                summary.staged |= staged;
                summary.unstaged |= unstaged;
                summary.untracked |= untracked;
            }
        }
    }
}

fn parse_status_summary_line(line: &str) -> Option<(String, bool, bool, bool)> {
    let bytes = line.as_bytes();
    if bytes.len() < 4 {
        return None;
    }
    let index = bytes[0] as char;
    let worktree = bytes[1] as char;
    let path = line.get(3..)?.trim();
    if path.is_empty() {
        return None;
    }
    let path = path
        .rsplit_once(" -> ")
        .map(|(_, new_path)| new_path)
        .unwrap_or(path)
        .to_owned();
    Some((
        path,
        index != ' ' && index != '?',
        worktree != ' ' && worktree != '?',
        index == '?' && worktree == '?',
    ))
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
    path == ".context"
        || path.starts_with(".context/")
        || path == ".archductor/prompt-packs"
        || path.starts_with(".archductor/prompt-packs/")
}

fn untracked_file_counts(path: &Path) -> Result<(usize, usize)> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("inspect {}", path.display()))?;
    if !metadata.file_type().is_file() {
        return Ok((0, 0));
    }
    let mut file = fs::File::open(path).with_context(|| format!("read {}", path.display()))?;
    let mut buffer = [0_u8; 8192];
    let mut additions = 0;
    let mut bytes_read = 0;
    let mut last_byte = None;
    while bytes_read < UNTRACKED_FILE_COUNT_BYTE_LIMIT {
        let remaining = UNTRACKED_FILE_COUNT_BYTE_LIMIT - bytes_read;
        let read_capacity = buffer.len().min(remaining);
        let read_len = file
            .read(&mut buffer[..read_capacity])
            .with_context(|| format!("read {}", path.display()))?;
        if read_len == 0 {
            break;
        }
        bytes_read += read_len;
        additions += buffer[..read_len]
            .iter()
            .filter(|byte| **byte == b'\n')
            .count();
        last_byte = Some(buffer[read_len - 1]);
    }
    if bytes_read > 0 && last_byte != Some(b'\n') {
        additions += 1;
    }
    Ok((additions, bytes_read))
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArchductorMetadataDirective {
    workspace_name: Option<String>,
    branch_name: Option<String>,
    chat_title: Option<String>,
}

fn extract_archductor_metadata_directive(
    content: &str,
) -> (String, Option<ArchductorMetadataDirective>) {
    let Some(start) = content.find(ARCHDUCTOR_METADATA_OPEN) else {
        return (content.to_owned(), None);
    };
    let json_start = start + ARCHDUCTOR_METADATA_OPEN.len();
    let Some(relative_end) = content[json_start..].find(ARCHDUCTOR_METADATA_CLOSE) else {
        return (content.to_owned(), None);
    };
    let end = json_start + relative_end;
    let json_text = content[json_start..end].trim();
    let directive = parse_archductor_metadata_directive(json_text);
    let mut cleaned = String::new();
    cleaned.push_str(&content[..start]);
    cleaned.push_str(&content[end + ARCHDUCTOR_METADATA_CLOSE.len()..]);
    (trim_metadata_blank_edges(&cleaned), directive)
}

fn parse_archductor_metadata_directive(json_text: &str) -> Option<ArchductorMetadataDirective> {
    let value = serde_json::from_str::<Value>(json_text).ok()?;
    Some(ArchductorMetadataDirective {
        workspace_name: value
            .get("workspace_name")
            .and_then(Value::as_str)
            .map(str::to_owned),
        branch_name: value
            .get("branch_name")
            .and_then(Value::as_str)
            .map(str::to_owned),
        chat_title: value
            .get("chat_title")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn trim_metadata_blank_edges(content: &str) -> String {
    let lines = content.lines().collect::<Vec<_>>();
    let start = lines
        .iter()
        .position(|line| !line.trim().is_empty())
        .unwrap_or(lines.len());
    let end = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map(|index| index + 1)
        .unwrap_or(start);
    lines[start..end].join("\n")
}

fn normalize_chat_title(raw: &str) -> Option<String> {
    let mut title = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.chars().count() > 48 {
        title = title.chars().take(48).collect::<String>();
    }
    let title = title
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ':' | ';' | ',' | '.'))
        .trim()
        .to_owned();
    (!title.is_empty()).then_some(title)
}

fn is_default_chat_thread_title_core(title: &str) -> bool {
    let title = title.trim();
    title == "New Chat"
        || title
            .strip_prefix("New Chat ")
            .and_then(|suffix| suffix.parse::<usize>().ok())
            .is_some()
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

fn validate_branch_name(branch: &str) -> Result<()> {
    let branch = branch.trim();
    anyhow::ensure!(!branch.is_empty(), "branch name is required");
    anyhow::ensure!(
        !branch.starts_with('-'),
        "branch name cannot start with '-'"
    );
    anyhow::ensure!(
        !branch.contains("..")
            && !branch.contains(' ')
            && !branch.contains('~')
            && !branch.contains('^')
            && !branch.contains(':')
            && !branch.contains('?')
            && !branch.contains('*')
            && !branch.contains('[')
            && !branch.contains('\\'),
        "branch name contains unsupported characters"
    );
    Ok(())
}

fn unique_message_branch_name(
    repository_root: &Path,
    base: &str,
    current_branch: &str,
) -> Result<String> {
    for suffix in 0.. {
        let candidate = if suffix == 0 {
            base.to_owned()
        } else {
            format!("{base}-{suffix}")
        };
        validate_branch_name(&candidate)?;
        if candidate == current_branch {
            return Ok(candidate);
        }
        if !local_branch_exists(repository_root, &candidate)? {
            return Ok(candidate);
        }
    }

    unreachable!("branch name generation should always return")
}

fn local_branch_exists(repository_root: &Path, branch: &str) -> Result<bool> {
    let ref_name = format!("refs/heads/{branch}");
    let output = Command::new("git")
        .arg("-C")
        .arg(repository_root)
        .args(["show-ref", "--verify", "--quiet", &ref_name])
        .output()
        .with_context(|| format!("check branch {branch} in {}", repository_root.display()))?;
    Ok(output.status.success())
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

fn sync_repository_default_branch(
    root_path: &Path,
    remote_name: &str,
    default_branch: &str,
) -> Result<String> {
    git_dynamic(
        root_path,
        &["fetch", remote_name, default_branch, "--prune"],
    )?;
    let remote_ref = format!("refs/remotes/{remote_name}/{default_branch}");
    git_dynamic(root_path, &["rev-parse", "--verify", &remote_ref])?;
    let local_ref = format!("refs/heads/{default_branch}");
    if git_dynamic(root_path, &["rev-parse", "--verify", &local_ref]).is_ok()
        && git_dynamic(
            root_path,
            &["merge-base", "--is-ancestor", &local_ref, &remote_ref],
        )
        .is_err()
    {
        warn!(
            repository = %root_path.display(),
            branch = default_branch,
            "local default branch is ahead or diverged; using remote branch as workspace base"
        );
        return Ok(format!("{remote_name}/{default_branch}"));
    }
    let current_branch = git_output_dynamic(root_path, &["branch", "--show-current"])
        .unwrap_or_default()
        .trim()
        .to_owned();
    if current_branch == default_branch {
        git_dynamic(
            root_path,
            &["pull", "--ff-only", remote_name, default_branch],
        )?;
        return Ok(default_branch.to_owned());
    }
    match git_dynamic(root_path, &["update-ref", &local_ref, &remote_ref]) {
        Ok(()) => Ok(default_branch.to_owned()),
        Err(err) => {
            warn!(
                repository = %root_path.display(),
                branch = default_branch,
                error = %err,
                "failed to fast-forward local default branch; using remote branch as workspace base"
            );
            Ok(format!("{remote_name}/{default_branch}"))
        }
    }
}

#[cfg(test)]
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
    let dirty = status
        .lines()
        .map(|line| line.get(3..).unwrap_or(line).trim())
        .filter(|path| !path.is_empty())
        .filter(|path| !is_conductor_context_path(path))
        .filter(|path| {
            !(path == &".gitignore" && gitignore_has_only_managed_changes(cwd).unwrap_or(false))
        })
        .collect::<Vec<_>>();
    anyhow::ensure!(
        dirty.is_empty(),
        "{label} requires a clean working tree; commit, stash, or discard changes first"
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
    let current_patch = spotlight_meaningful_patch(&current_patch);
    let expected_patch = spotlight_meaningful_patch(expected_patch);
    let conflict_detail = spotlight_conflict_detail(&current_patch, &expected_patch);
    anyhow::ensure!(
        current_patch.trim() == expected_patch.trim(),
        "repository root has changes outside the active Spotlight patch{conflict_detail}; clean or save root changes before changing Spotlight state"
    );
    Ok(())
}

fn root_tracked_patch(root_path: &Path) -> Result<String> {
    let index_path =
        std::env::temp_dir().join(format!("linux-archductor-root-index-{}", timestamp_nanos()));
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

struct CreatedCheckpointRef {
    git_ref: String,
    created_at: String,
}

fn create_worktree_checkpoint_commit(
    workspace: &Workspace,
    message: &str,
    ref_kind: &str,
) -> Result<CreatedCheckpointRef> {
    let now = timestamp();
    let ref_suffix = timestamp_nanos();
    let git_ref = format!(
        "refs/linux-archductor/checkpoints/{}/{ref_kind}-{ref_suffix}",
        workspace.id
    );
    let index_path = std::env::temp_dir().join(format!(
        "linux-archductor-checkpoint-index-{}-{ref_suffix}",
        workspace.id
    ));
    let result = (|| -> Result<CreatedCheckpointRef> {
        let tree = worktree_snapshot_tree(&workspace.path, &index_path)?;
        let head = git_output_dynamic(&workspace.path, &["rev-parse", "HEAD"])?;
        let commit = git_commit_tree(&workspace.path, tree.trim(), head.trim(), message)?;
        git_dynamic(&workspace.path, &["update-ref", &git_ref, commit.trim()])?;
        Ok(CreatedCheckpointRef {
            git_ref,
            created_at: now,
        })
    })();
    let _ = fs::remove_file(&index_path);
    result
}

fn diff_worktree_against_ref(workspace: &Workspace, base_ref: &str) -> Result<String> {
    let now = timestamp_nanos();
    let index_path = std::env::temp_dir().join(format!(
        "linux-archductor-turn-diff-index-{}-{now}",
        workspace.id
    ));
    let result = (|| -> Result<String> {
        let tree = worktree_snapshot_tree(&workspace.path, &index_path)?;
        git_output_dynamic(
            &workspace.path,
            &["diff", "--binary", base_ref, tree.trim()],
        )
    })();
    let _ = fs::remove_file(&index_path);
    result
}

fn diff_checkpoint_refs(workspace: &Workspace, base_ref: &str, head_ref: &str) -> Result<String> {
    git_output_dynamic(&workspace.path, &["diff", "--binary", base_ref, head_ref])
}

fn truncate_text_at_char_boundary(value: String, max_bytes: usize) -> (String, bool) {
    if value.len() <= max_bytes {
        return (value, false);
    }
    const MARKER: &str = "\n[Diff truncated at hard limit]\n";
    let retain_bytes = max_bytes.saturating_sub(MARKER.len());
    let mut end = retain_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = value[..end].to_owned();
    truncated.push_str(MARKER);
    (truncated, true)
}

fn worktree_snapshot_tree(cwd: &Path, index_path: &Path) -> Result<String> {
    git_with_index(cwd, index_path, &["read-tree", "HEAD"])?;
    git_with_index(cwd, index_path, &["add", "-A"])?;
    git_with_index_output(cwd, index_path, &["write-tree"])
}

fn spotlight_conflict_detail(current_patch: &str, expected_patch: &str) -> String {
    let paths = spotlight_conflict_paths(current_patch, expected_patch);
    spotlight_conflict_detail_from_paths(&paths)
}

fn spotlight_conflict_detail_from_paths(paths: &BTreeSet<String>) -> String {
    if paths.is_empty() {
        return String::new();
    }

    let shown = paths.iter().take(6).cloned().collect::<Vec<_>>();
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
    patch_file_chunks(patch)
        .into_iter()
        .filter(|(path, chunk)| !(path == ".gitignore" && gitignore_patch_is_managed(chunk)))
        .map(|(path, _)| path)
        .filter(|path| !is_conductor_context_path(path))
        .collect()
}

fn spotlight_meaningful_patch(patch: &str) -> String {
    let mut filtered = String::new();
    for (path, chunk) in patch_file_chunks(patch) {
        if !(is_conductor_context_path(&path)
            || path == ".gitignore" && gitignore_patch_is_managed(&chunk))
        {
            filtered.push_str(&chunk);
            if !chunk.ends_with('\n') {
                filtered.push('\n');
            }
        }
    }
    filtered
}

fn patch_path_from_diff_line(line: &str) -> Option<String> {
    let rest = line.strip_prefix("diff --git ")?;
    let (_old_path, rest) = parse_diff_path_token(rest)?;
    let (new_path, _) = parse_diff_path_token(rest.trim_start())?;
    new_path.strip_prefix("b/").map(str::to_owned)
}

fn parse_diff_path_token(input: &str) -> Option<(String, &str)> {
    if let Some(rest) = input.strip_prefix('"') {
        let mut escaped = false;
        for (index, ch) in rest.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == '"' {
                let token = &rest[..index];
                let remaining = &rest[index + ch.len_utf8()..];
                return Some((unescape_git_quoted_path(token), remaining));
            }
        }
        return None;
    }
    let (token, remaining) = input.split_once(' ').unwrap_or((input, ""));
    Some((token.to_owned(), remaining))
}

fn unescape_git_quoted_path(path: &str) -> String {
    let mut result = String::new();
    let mut chars = path.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            result.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => result.push('\n'),
            Some('t') => result.push('\t'),
            Some('r') => result.push('\r'),
            Some('\\') => result.push('\\'),
            Some('"') => result.push('"'),
            Some(other) => {
                result.push('\\');
                result.push(other);
            }
            None => result.push('\\'),
        }
    }
    result
}

fn patch_file_chunks(patch: &str) -> Vec<(String, String)> {
    let mut chunks = Vec::new();
    let mut current_path = None::<String>;
    let mut current_chunk = String::new();
    for line in patch.lines() {
        if let Some(path) = patch_path_from_diff_line(line) {
            if let Some(previous_path) = current_path.replace(path) {
                chunks.push((previous_path, std::mem::take(&mut current_chunk)));
            }
        }
        if current_path.is_some() {
            current_chunk.push_str(line);
            current_chunk.push('\n');
        }
    }
    if let Some(path) = current_path {
        chunks.push((path, current_chunk));
    }
    chunks
}

fn gitignore_patch_is_managed(chunk: &str) -> bool {
    let mut changed = false;
    for line in chunk.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        let Some(prefix) = line.chars().next() else {
            continue;
        };
        if prefix != '+' && prefix != '-' {
            continue;
        }
        let pattern = line[1..].trim();
        match (prefix, gitignore_pattern_key(pattern).as_deref()) {
            (
                '+',
                Some(
                    ".context"
                    | ".archductor"
                    | ".archductor/settings.toml"
                    | ".archductor/prompt-packs"
                    | ".archductor/prompt-packs/*.toml"
                    | ".archductor/settings.local.toml*",
                ),
            )
            | ('-', Some(".archductor" | ".archductor/*" | ".archductor/**")) => changed = true,
            _ => return false,
        }
    }
    changed
}

fn gitignore_has_only_managed_changes(repo_path: &Path) -> Result<bool> {
    let current = fs::read_to_string(repo_path.join(".gitignore")).unwrap_or_default();
    let base = git_output_dynamic(repo_path, &["show", "HEAD:.gitignore"]).unwrap_or_default();
    Ok(gitignore_without_managed_patterns(&current) == gitignore_without_managed_patterns(&base))
}

fn gitignore_without_managed_patterns(contents: &str) -> Vec<String> {
    contents
        .lines()
        .filter(|line| {
            !matches!(
                gitignore_pattern_key(line).as_deref(),
                Some(
                    ".context"
                        | ".archductor"
                        | ".archductor/*"
                        | ".archductor/**"
                        | ".archductor/settings.toml"
                        | ".archductor/prompt-packs"
                        | ".archductor/prompt-packs/*.toml"
                        | ".archductor/settings.local.toml*"
                )
            )
        })
        .map(str::to_owned)
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
            "user.name=Linux Archductor",
            "-c",
            "user.email=linux-archductor@example.test",
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

fn command_output_owned(cwd: &Path, program: &str, args: &[String]) -> Result<String> {
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    command_output(cwd, program, &refs)
}

fn command_success(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn command_exists(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_owned())
}

fn timestamp_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
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

#[cfg(unix)]
fn process_group_matches_pid(pid: u32) -> bool {
    Command::new("ps")
        .args(["-o", "pgid=", "-p", &pid.to_string()])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|stdout| stdout.trim().parse::<u32>().ok())
        .map(|pgid| pgid == pid)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn process_group_matches_pid(_pid: u32) -> bool {
    false
}

fn stop_process(pid: u32) -> Result<()> {
    // Try SIGTERM to the process group only when the child owns a distinct group.
    let group_ok = if process_group_matches_pid(pid) {
        Command::new("kill")
            .arg("-TERM")
            .arg(format!("-{pid}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("run kill")?
            .success()
    } else {
        false
    };
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
        if process_group_matches_pid(pid) {
            let _ = Command::new("kill")
                .arg("-KILL")
                .arg(format!("-{pid}"))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
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

fn first_pull_request_url_from_json(output: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(output).ok()?;
    value
        .as_array()?
        .first()?
        .get("url")?
        .as_str()
        .map(str::to_owned)
}

fn render_pr_template_text(
    template: &str,
    workspace: &Workspace,
    changed_files_count: usize,
    changed_files: &str,
    session_summary: &str,
    summary: &str,
) -> String {
    template
        .replace("{workspace}", &workspace.name)
        .replace("{branch}", &workspace.branch)
        .replace("{changed_files_count}", &changed_files_count.to_string())
        .replace("{changed_files}", changed_files)
        .replace("{session_summary}", session_summary)
        .replace("{summary}", summary)
        .replace("{type}", "feat")
}

fn commit_message_draft_for_files(workspace_name: &str, files: &[String]) -> String {
    let focus = files
        .iter()
        .find(|path| !is_conductor_context_path(path))
        .or_else(|| files.first());
    match focus {
        Some(path) => format!("chore: update {path}"),
        None => format!("chore: update {workspace_name}"),
    }
}

fn random_index(len: usize) -> usize {
    debug_assert!(len > 0);
    let value = u128::from_le_bytes(*Uuid::new_v4().as_bytes());
    (value % len as u128) as usize
}

fn run_shell_script(
    script: &str,
    settings: &crate::settings::RepositorySettings,
    repository: &RepositoryRecord,
    workspace: &Workspace,
    extra_env: &[(String, OsString)],
) -> Result<()> {
    let cwd = workspace_working_directory(settings, workspace)?;
    let mut env = conductor_environment(settings, repository, workspace)?;
    env.extend(extra_env.iter().cloned());
    let mut command = Command::new("sh");
    command.arg("-c").arg(script).current_dir(&cwd).envs(env);

    let output = command
        .output()
        .with_context(|| format!("run script in {}", cwd.display()))?;
    anyhow::ensure!(
        output.status.success(),
        "script failed in {}: {}\n{}",
        cwd.display(),
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );

    Ok(())
}

fn conductor_environment(
    settings: &crate::settings::RepositorySettings,
    repository: &RepositoryRecord,
    workspace: &Workspace,
) -> Result<Vec<(String, OsString)>> {
    let working_directory =
        workspace_working_directory(settings, workspace).unwrap_or_else(|_| workspace.path.clone());
    let mut env = vec![
        (
            "ARCHDUCTOR_WORKSPACE_NAME".to_owned(),
            OsString::from(&workspace.name),
        ),
        (
            "ARCHDUCTOR_WORKSPACE_PATH".to_owned(),
            workspace.path.as_os_str().to_owned(),
        ),
        (
            "ARCHDUCTOR_WORKING_DIRECTORY".to_owned(),
            working_directory.as_os_str().to_owned(),
        ),
        (
            "ARCHDUCTOR_ROOT_PATH".to_owned(),
            repository.root_path.as_os_str().to_owned(),
        ),
        (
            "ARCHDUCTOR_DEFAULT_BRANCH".to_owned(),
            OsString::from(&repository.default_branch),
        ),
        (
            "ARCHDUCTOR_PORT".to_owned(),
            OsString::from(workspace.port_base.to_string()),
        ),
        ("ARCHDUCTOR_IS_LOCAL".to_owned(), OsString::from("1")),
    ];
    env.extend(load_env_file_refs(
        settings,
        &repository.root_path,
        &workspace.path,
    )?);
    env.extend(
        settings
            .environment_variables
            .iter()
            .map(|(key, value)| (key.clone(), OsString::from(value))),
    );
    Ok(env)
}

fn load_env_file_refs(
    settings: &crate::settings::RepositorySettings,
    repository_root: &Path,
    workspace_path: &Path,
) -> Result<Vec<(String, OsString)>> {
    let mut values = Vec::new();
    for relative in &settings.env_file_refs {
        validate_relative_workspace_path(relative)?;
        let path = if workspace_path.join(relative).exists() {
            resolve_env_file_ref(workspace_path, relative)?
        } else if repository_root.join(relative).exists() {
            resolve_env_file_ref(repository_root, relative)?
        } else {
            anyhow::bail!("env file reference {relative} does not exist");
        };
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("read env file {}", path.display()))?;
        values.extend(parse_env_file_contents(relative, &contents)?);
    }
    Ok(values)
}

fn resolve_env_file_ref(root: &Path, relative: &str) -> Result<PathBuf> {
    let root = root
        .canonicalize()
        .with_context(|| format!("resolve env file root {}", root.display()))?;
    let path = root.join(relative);
    let canonical = path
        .canonicalize()
        .with_context(|| format!("resolve env file {}", path.display()))?;
    anyhow::ensure!(
        canonical.starts_with(&root),
        "env file reference {relative} resolves outside {}",
        root.display()
    );
    Ok(canonical)
}

fn parse_env_file_contents(source: &str, contents: &str) -> Result<Vec<(String, OsString)>> {
    let mut values = Vec::new();
    for (index, raw) in contents.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim();
        let Some((key, value)) = line.split_once('=') else {
            anyhow::bail!("invalid env line {} in {source}", index + 1);
        };
        let key = key.trim();
        anyhow::ensure!(
            crate::settings::is_valid_environment_key(key),
            "environment variable key {key:?} in {source} is invalid"
        );
        values.push((
            key.to_owned(),
            OsString::from(unquote_env_value(value.trim())),
        ));
    }
    Ok(values)
}

fn unquote_env_value(value: &str) -> String {
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        value[1..value.len() - 1].to_owned()
    } else {
        value.to_owned()
    }
}

fn workspace_working_directory(
    settings: &crate::settings::RepositorySettings,
    workspace: &Workspace,
) -> Result<PathBuf> {
    let Some(relative) = settings
        .customization
        .workspace_defaults
        .working_directory
        .as_deref()
    else {
        return Ok(workspace.path.clone());
    };
    validate_relative_workspace_path(relative)?;
    let cwd = resolve_workspace_directory(&workspace.path, relative)?;
    anyhow::ensure!(
        cwd.is_dir(),
        "workspace working_directory {} does not exist in {}",
        relative,
        workspace.path.display()
    );
    Ok(cwd)
}

fn resolve_workspace_directory(root: &Path, relative: &str) -> Result<PathBuf> {
    let root = root
        .canonicalize()
        .with_context(|| format!("resolve workspace root {}", root.display()))?;
    let path = root.join(relative);
    let canonical = path
        .canonicalize()
        .with_context(|| format!("resolve workspace working_directory {}", path.display()))?;
    anyhow::ensure!(
        canonical.starts_with(&root),
        "workspace working_directory {relative} resolves outside {}",
        root.display()
    );
    Ok(canonical)
}

fn merge_method(
    settings: &crate::settings::RepositorySettings,
    method: Option<&str>,
) -> Result<String> {
    let method = method
        .filter(|method| !method.trim().is_empty())
        .map(str::trim)
        .or(settings
            .customization
            .naming
            .default_merge_method
            .as_deref())
        .unwrap_or("squash");
    anyhow::ensure!(
        matches!(method, "squash" | "merge" | "rebase"),
        "merge method must be squash, merge, or rebase"
    );
    Ok(method.to_owned())
}

fn validate_relative_workspace_path(path: &str) -> Result<()> {
    let path = Path::new(path);
    anyhow::ensure!(
        !path.as_os_str().is_empty()
            && !path.as_os_str().as_encoded_bytes().contains(&0)
            && path.is_relative(),
        "workspace working_directory must be a relative path"
    );
    anyhow::ensure!(
        path.components()
            .all(|component| matches!(component, std::path::Component::Normal(_))),
        "workspace working_directory cannot contain parent/current/root components"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::{AddRepository, RepositoryStore};
    use std::fs;
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn exited_child_pid() -> u32 {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("exit 0")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(!process_alive(pid));
        pid
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
    fn create_workspace_recovers_missing_repository_config() {
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
        fs::remove_dir_all(repo_path.join(".archductor")).unwrap();

        let store = WorkspaceStore::open(&db_path).unwrap();
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        assert!(repo_path.join(".archductor/settings.toml").exists());
    }

    #[test]
    fn create_workspace_records_failed_row_when_git_setup_fails() {
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
                workspace_parent_path: Some(workspace_parent),
            })
            .unwrap();

        let store = WorkspaceStore::open(&db_path).unwrap();
        let err = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "broken".to_owned(),
                branch: "lc/broken".to_owned(),
                base_ref: Some("missing-base-ref".to_owned()),
            })
            .unwrap_err();

        assert!(err.to_string().contains("git command failed"));
        let workspace = store.get_by_name("broken").unwrap();
        assert_eq!(workspace.status, "failed");
        assert_eq!(workspace.branch, "lc/broken");
    }

    #[test]
    fn create_workspace_syncs_default_branch_before_worktree_add() {
        let temp = tempfile::tempdir().unwrap();
        let seed_path = init_repo(temp.path().join("seed"));
        let remote_path = temp.path().join("origin.git");
        Command::new("git")
            .args(["init", "--bare", "--initial-branch", "main"])
            .arg(&remote_path)
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&seed_path)
            .args(["remote", "add", "origin"])
            .arg(&remote_path)
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&seed_path)
            .args(["push", "-u", "origin", "main"])
            .status()
            .unwrap();

        let repo_path = temp.path().join("demo");
        Command::new("git")
            .args(["clone"])
            .arg(&remote_path)
            .arg(&repo_path)
            .status()
            .unwrap();

        fs::write(seed_path.join("REMOTE.md"), "new on main\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&seed_path)
            .args(["add", "REMOTE.md"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&seed_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "advance main",
            ])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&seed_path)
            .args(["push", "origin", "main"])
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
                base_ref: None,
            })
            .unwrap();

        assert_eq!(workspace.status, "active");
        assert_eq!(workspace.base_ref, "main");
        assert!(workspace.path.join("REMOTE.md").exists());
        assert_eq!(
            git_output(&repo_path, ["rev-parse", "main"]),
            git_output(&seed_path, ["rev-parse", "main"])
        );
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
                root_path: repo_path.clone(),
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
    fn create_from_prompt_uses_configured_branch_prefix() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
[customization.workspace_defaults]
branch_prefix = "team"
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["commit", "-m", "add archductor settings"])
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
            .create_from_prompt("demo", "Build source defaults", None, None, None)
            .unwrap();

        assert_eq!(workspace.branch, "team/build-source-defaults");
    }

    #[test]
    fn first_message_workspace_naming_renames_workspace_and_branch() {
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
        store
            .create_chat_thread(&workspace.name, "codex", "New Chat", None)
            .unwrap();

        let renamed = store
            .apply_first_message_workspace_naming(
                &workspace.name,
                "Fix the customer billing webhook failure",
            )
            .unwrap()
            .unwrap();

        assert_eq!(renamed.name, "fix-the-customer-billing-webhook-failure");
        assert_eq!(
            renamed.branch,
            "lc/fix-the-customer-billing-webhook-failure"
        );
        assert_eq!(renamed.path, workspace.path);
        assert!(renamed.path.is_dir());
        let branch = git_output(&renamed.path, ["branch", "--show-current"]);
        assert_eq!(branch.trim(), "lc/fix-the-customer-billing-webhook-failure");
        let worktrees = Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["worktree", "list", "--porcelain"])
            .output()
            .unwrap();
        let worktrees = String::from_utf8_lossy(&worktrees.stdout);
        assert!(worktrees.contains(workspace.path.to_str().unwrap()));
        assert!(!worktrees.contains(
            workspace_parent
                .join("fix-the-customer-billing-webhook-failure")
                .to_str()
                .unwrap()
        ));
    }

    #[test]
    fn first_message_workspace_naming_skips_after_existing_message() {
        let (_temp, store) = test_workspace_store();
        let workspace = store.get_by_name("berlin").unwrap();
        let thread = store
            .create_chat_thread("berlin", "codex", "New Chat", None)
            .unwrap();
        store
            .append_chat_message(thread.id, "user", "existing message", "user_send")
            .unwrap();

        let renamed = store
            .apply_first_message_workspace_naming("berlin", "Rename from later message")
            .unwrap();

        assert!(renamed.is_none());
        assert_eq!(store.get_by_name("berlin").unwrap(), workspace);
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
                root_path: repo_path.clone(),
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
    fn create_from_issue_uses_configured_branch_prefix_when_not_explicit() {
        let _guard = env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
[customization.workspace_defaults]
branch_prefix = "team"
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["commit", "-m", "add archductor settings"])
            .status()
            .unwrap();
        let old_path = install_fake_gh(
            temp.path(),
            r#"#!/bin/sh
if [ "$1" = "issue" ] && [ "$2" = "view" ]; then
  printf '{"title":"Fix configured branch prefixes","number":123}\n'
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
        let workspace = store.create_from_issue("demo", 123, None).unwrap();

        restore_path(old_path);
        assert_eq!(workspace.branch, "team/123/fix-configured-branch-prefixes");
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
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
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
        let script_body = script.strip_prefix("#!/bin/sh").unwrap_or(script);
        fs::write(
            &gh_path,
            format!(
                "#!/bin/sh\n\
if [ \"$1\" = \"--version\" ]; then\n\
  printf 'gh version fake\\n'\n\
  exit 0\n\
fi\n\
if [ \"$1\" = \"auth\" ] && [ \"$2\" = \"status\" ]; then\n\
  printf 'Logged in to github.com account fake\\n'\n\
  exit 0\n\
fi\n\
{script_body}"
            ),
        )
        .unwrap();
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
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
codex_executable_path = "/opt/bin/codex-custom"
claude_code_executable_path = "/opt/bin/claude-custom"
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["commit", "-m", "add archductor settings"])
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
    fn create_workspace_uses_configured_workspace_base_branch_default() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["checkout", "-b", "develop"])
            .status()
            .unwrap();
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
[customization.workspace_defaults]
base_branch = "develop"
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["commit", "-m", "add archductor settings"])
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
                base_ref: None,
            })
            .unwrap();

        assert_eq!(workspace.base_ref, "develop");
    }

    #[test]
    fn create_workspace_uses_configured_port_block_size() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
[customization.workspace_defaults]
port_block_size = 25
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["commit", "-m", "add archductor settings"])
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
        assert_eq!(second.port_base, 3025);
    }

    #[test]
    fn create_workspace_generates_city_name_and_branch_when_empty() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
[customization.naming]
workspace_name_style = "city"

[customization.workspace_defaults]
branch_prefix = "team"
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["commit", "-m", "add archductor settings"])
            .status()
            .unwrap();
        let db_path = temp.path().join("state.db");
        let workspace_parent = temp.path().join("workspaces/demo");

        RepositoryStore::open(&db_path)
            .unwrap()
            .add(AddRepository {
                name: Some("demo".to_owned()),
                root_path: repo_path,
                default_branch: Some("main".to_owned()),
                remote_name: "origin".to_owned(),
                workspace_parent_path: Some(workspace_parent.clone()),
            })
            .unwrap();

        let workspace = WorkspaceStore::open(&db_path)
            .unwrap()
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: String::new(),
                branch: String::new(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        assert!(WORKSPACE_CITY_NAMES.contains(&workspace.name.as_str()));
        assert_eq!(
            workspace.branch,
            format!("team/{}", slugify(&workspace.name))
        );
        assert_eq!(workspace.path, workspace_parent.join(&workspace.name));
    }

    #[test]
    fn create_workspace_uses_global_active_city_pool_across_repositories() {
        let temp = tempfile::tempdir().unwrap();
        let repo_one_path = init_repo(temp.path().join("demo-one"));
        let repo_two_path = init_repo(temp.path().join("demo-two"));
        let db_path = temp.path().join("state.db");

        let repo_store = RepositoryStore::open(&db_path).unwrap();
        repo_store
            .add(AddRepository {
                name: Some("demo-one".to_owned()),
                root_path: repo_one_path,
                default_branch: Some("main".to_owned()),
                remote_name: "origin".to_owned(),
                workspace_parent_path: Some(temp.path().join("workspaces/demo-one")),
            })
            .unwrap();
        repo_store
            .add(AddRepository {
                name: Some("demo-two".to_owned()),
                root_path: repo_two_path,
                default_branch: Some("main".to_owned()),
                remote_name: "origin".to_owned(),
                workspace_parent_path: Some(temp.path().join("workspaces/demo-two")),
            })
            .unwrap();

        let store = WorkspaceStore::open(&db_path).unwrap();
        store
            .create(CreateWorkspace {
                repository_name: "demo-one".to_owned(),
                name: WORKSPACE_CITY_NAMES[0].to_owned(),
                branch: format!("lc/{}", WORKSPACE_CITY_NAMES[0]),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        for city in &WORKSPACE_CITY_NAMES[1..WORKSPACE_CITY_NAMES.len() - 1] {
            store
                .create(CreateWorkspace {
                    repository_name: "demo-two".to_owned(),
                    name: (*city).to_owned(),
                    branch: format!("lc/{city}"),
                    base_ref: Some("main".to_owned()),
                })
                .unwrap();
        }

        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo-one".to_owned(),
                name: String::new(),
                branch: String::new(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        assert_eq!(
            workspace.name,
            WORKSPACE_CITY_NAMES[WORKSPACE_CITY_NAMES.len() - 1]
        );
        assert_eq!(
            workspace.branch,
            format!(
                "lc/{}",
                WORKSPACE_CITY_NAMES[WORKSPACE_CITY_NAMES.len() - 1]
            )
        );
    }

    #[test]
    fn create_workspace_does_not_treat_repository_name_as_selected_city() {
        let temp = tempfile::tempdir().unwrap();
        let repo_one_path = init_repo(temp.path().join("repo-berlin"));
        let repo_two_path = init_repo(temp.path().join("repo-two"));
        let db_path = temp.path().join("state.db");

        let repo_store = RepositoryStore::open(&db_path).unwrap();
        repo_store
            .add(AddRepository {
                name: Some("berlin".to_owned()),
                root_path: repo_one_path,
                default_branch: Some("main".to_owned()),
                remote_name: "origin".to_owned(),
                workspace_parent_path: Some(temp.path().join("workspaces/berlin")),
            })
            .unwrap();
        repo_store
            .add(AddRepository {
                name: Some("demo-two".to_owned()),
                root_path: repo_two_path,
                default_branch: Some("main".to_owned()),
                remote_name: "origin".to_owned(),
                workspace_parent_path: Some(temp.path().join("workspaces/demo-two")),
            })
            .unwrap();

        let store = WorkspaceStore::open(&db_path).unwrap();
        for city in &WORKSPACE_CITY_NAMES[1..] {
            store
                .create(CreateWorkspace {
                    repository_name: "demo-two".to_owned(),
                    name: (*city).to_owned(),
                    branch: format!("lc/{city}"),
                    base_ref: Some("main".to_owned()),
                })
                .unwrap();
        }

        let workspace = store
            .create(CreateWorkspace {
                repository_name: "berlin".to_owned(),
                name: String::new(),
                branch: String::new(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        assert_eq!(workspace.name, "berlin");
        assert_eq!(workspace.branch, "lc/berlin");
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
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
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
                ".archductor/settings.toml",
            ])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "add archductor settings",
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
    fn create_workspace_does_not_run_setup_script_by_default() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
[scripts]
setup = "printf 'setup-ran\n' > .context/setup-env"
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
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

        assert!(!workspace.path.join(".context/setup-env").exists());
    }

    #[test]
    fn create_workspace_auto_setup_starts_background_setup_process_with_conductor_environment() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
[scripts]
setup = "printf '%s\n' \"$ARCHDUCTOR_WORKSPACE_NAME\" \"$ARCHDUCTOR_WORKSPACE_PATH\" \"$ARCHDUCTOR_ROOT_PATH\" \"$ARCHDUCTOR_DEFAULT_BRANCH\" \"$ARCHDUCTOR_PORT\" \"$ARCHDUCTOR_IS_LOCAL\" \"$CUSTOM_VALUE\" > .context/setup-env"

[customization.automation]
auto_setup = true

[environment_variables]
CUSTOM_VALUE = "from-settings"
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
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

        let store = WorkspaceStore::open_with_logs(&db_path, temp.path().join("logs")).unwrap();
        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        let setups = store.list_setups("berlin").unwrap();
        assert_eq!(setups.len(), 1);
        wait_for_path(&workspace.path.join(".context/setup-env"));
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
    fn archive_failure_leaves_workspace_active() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            "[scripts]\narchive = \"false\"\n",
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "add archive script",
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
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        let err = store.archive("berlin", false).unwrap_err();

        assert!(err.to_string().contains("script failed"));
        let workspace = store.get_by_name("berlin").unwrap();
        assert_eq!(workspace.status, "active");
        assert!(workspace.archived_at.is_none());
    }

    #[test]
    fn delete_workspace_removes_record_dependents_and_keeps_worktree_by_default() {
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
        store.add_todo("berlin", "clean up").unwrap();
        let thread = store
            .create_chat_thread("berlin", "codex", "Bugfix", None)
            .unwrap();
        store
            .append_chat_message(thread.id, "user", "hi", "user_send")
            .unwrap();
        let launch = store.session_launch("berlin", SessionKind::Codex).unwrap();
        let process = store
            .record_session_process_for_thread("berlin", thread.id, &launch, exited_child_pid())
            .unwrap();
        store
            .append_pty_chunk(process.id, "stdout_pty", "raw codex output\n")
            .unwrap();
        store
            .append_session_events(
                process.id,
                vec![crate::session_event::SessionEvent::new(
                    crate::session_event::SessionEventSource::Assistant,
                    Some("parsed output".to_owned()),
                    crate::session_event::SessionEventPayload::AssistantText {
                        text: "parsed output".to_owned(),
                    },
                )],
            )
            .unwrap();

        let deleted = store.delete("berlin", false, false).unwrap();

        assert_eq!(deleted.name, "berlin");
        assert!(workspace.path.exists());
        assert!(store.get_by_name("berlin").is_err());
        assert!(store.list_todos("berlin").is_err());
        let orphan_threads: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM chat_threads WHERE workspace_id = ?1",
                [workspace.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(orphan_threads, 0);
        let orphan_pty_chunks: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM pty_chunks", [], |row| row.get(0))
            .unwrap();
        let orphan_session_events: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM session_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(orphan_pty_chunks, 0);
        assert_eq!(orphan_session_events, 0);
    }

    #[test]
    fn delete_worktree_failure_keeps_workspace_metadata() {
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
        let not_a_worktree = temp.path().join("not-a-worktree");
        fs::create_dir(&not_a_worktree).unwrap();
        store
            .conn
            .execute(
                "UPDATE workspaces SET path = ?1 WHERE id = ?2",
                params![not_a_worktree.to_string_lossy(), workspace.id],
            )
            .unwrap();

        assert!(store.delete("berlin", true, false).is_err());
        assert!(store.get_by_name("berlin").is_ok());
    }

    #[test]
    fn create_with_explicit_base_ref_skips_default_branch_sync() {
        let temp = tempfile::tempdir().unwrap();
        let remote_path = temp.path().join("origin.git");
        Command::new("git")
            .args(["init", "--bare", "--initial-branch", "main"])
            .arg(&remote_path)
            .status()
            .unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["remote", "add", "origin"])
            .arg(&remote_path)
            .status()
            .unwrap();
        let db_path = temp.path().join("state.db");

        RepositoryStore::open(&db_path)
            .unwrap()
            .add(AddRepository {
                name: Some("demo".to_owned()),
                root_path: repo_path,
                default_branch: Some("missing-default".to_owned()),
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

        assert_eq!(workspace.base_ref, "main");
    }

    #[test]
    fn delete_workspace_can_remove_worktree_and_branch() {
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

        store.delete("berlin", true, true).unwrap();

        assert!(!workspace.path.exists());
        let branches = Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["branch", "--list", "lc/berlin"])
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&branches.stdout).trim().is_empty());
    }

    #[test]
    fn delete_workspace_can_remove_moved_worktree_with_stale_git_metadata() {
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
        let moved_path = workspace.path.parent().unwrap().join("moved-berlin");
        fs::rename(&workspace.path, &moved_path).unwrap();
        store
            .conn
            .execute(
                "UPDATE workspaces SET name = ?1, path = ?2 WHERE id = ?3",
                params!["moved-berlin", moved_path.to_string_lossy(), workspace.id],
            )
            .unwrap();

        store.delete("moved-berlin", true, true).unwrap();

        assert!(!moved_path.exists());
        assert!(store.get_by_name("moved-berlin").is_err());
        let worktrees = Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["worktree", "list", "--porcelain"])
            .output()
            .unwrap();
        assert!(!String::from_utf8_lossy(&worktrees.stdout).contains("berlin"));
    }

    #[test]
    fn sync_default_branch_preserves_local_commits() {
        let temp = tempfile::tempdir().unwrap();
        let remote_path = temp.path().join("origin.git");
        Command::new("git")
            .args(["init", "--bare", "--initial-branch", "main"])
            .arg(&remote_path)
            .status()
            .unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
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
            .args(["push", "-u", "origin", "main"])
            .status()
            .unwrap();
        let local_head_before = git_output(&repo_path, ["rev-parse", "main"]);
        fs::write(repo_path.join("local.txt"), "local\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", "local.txt"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "local only",
            ])
            .status()
            .unwrap();
        let local_head_after_commit = git_output(&repo_path, ["rev-parse", "main"]);
        assert_ne!(local_head_before, local_head_after_commit);
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["checkout", "-b", "work"])
            .status()
            .unwrap();

        let base = sync_repository_default_branch(&repo_path, "origin", "main").unwrap();

        assert_eq!(base, "origin/main");
        assert_eq!(
            git_output(&repo_path, ["rev-parse", "main"]),
            local_head_after_commit
        );
    }

    #[test]
    fn patch_changed_paths_handles_quoted_diff_paths() {
        let patch = "diff --git \"a/path with spaces.txt\" \"b/path with spaces.txt\"\n\
--- \"a/path with spaces.txt\"\n\
+++ \"b/path with spaces.txt\"\n\
@@ -1 +1 @@\n\
-old\n\
+new\n";

        let paths = patch_changed_paths(patch);

        assert!(paths.contains("path with spaces.txt"));
    }

    #[test]
    fn run_workspace_executes_run_script_with_conductor_environment() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
[scripts]
run = "printf '%s\n' \"$ARCHDUCTOR_WORKSPACE_NAME\" \"$ARCHDUCTOR_WORKSPACE_PATH\" \"$ARCHDUCTOR_ROOT_PATH\" \"$ARCHDUCTOR_DEFAULT_BRANCH\" \"$ARCHDUCTOR_PORT\" \"$ARCHDUCTOR_IS_LOCAL\" \"$CUSTOM_VALUE\" > .context/run-env"

[environment_variables]
CUSTOM_VALUE = "from-settings"
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
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
    fn run_workspace_loads_env_file_refs_without_logging_secret_values() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".env.local"),
            "SECRET_TOKEN=from-file\nCUSTOM_VALUE=from-file\n",
        )
        .unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
env_file_refs = ".env.local"

[scripts]
run = "printf '%s:%s\n' \"$SECRET_TOKEN\" \"$CUSTOM_VALUE\" > .context/env-file-result"

[environment_variables]
CUSTOM_VALUE = "from-settings"
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", "."])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "add env file run script",
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
                remote_name: "upstream".to_owned(),
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
        wait_for_path(&workspace.path.join(".context/env-file-result"));

        assert_eq!(
            fs::read_to_string(workspace.path.join(".context/env-file-result")).unwrap(),
            "from-file:from-settings\n"
        );
        assert!(!fs::read_to_string(&run.log_path)
            .unwrap()
            .contains("from-file"));
    }

    #[cfg(unix)]
    #[test]
    fn run_workspace_rejects_env_file_ref_symlink_escape() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
env_file_refs = ".env.local"

[scripts]
run = "true"
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", "."])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "add settings",
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
        let outside_env = temp.path().join("outside.env");
        fs::write(&outside_env, "SECRET_TOKEN=outside\n").unwrap();
        std::os::unix::fs::symlink(&outside_env, workspace.path.join(".env.local")).unwrap();

        let err = store.run_workspace("berlin").unwrap_err();

        assert!(format!("{err:#}").contains("resolves outside"));
    }

    #[test]
    fn run_workspace_captures_logs() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
[scripts]
run = "printf 'started\n'"
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
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
        let exited =
            wait_for_process_status(&store, "berlin", ProcessKind::Run, ProcessStatus::Exited);
        assert_eq!(exited.exit_code, Some(0));
    }

    #[test]
    fn run_workspace_records_exit_status_when_process_finishes() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
[scripts]
run = "printf 'done\n'; exit 3"
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
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
        let events = store
            .workspace_timeline("berlin", Some("run.failed"))
            .unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].summary.contains("exit code 3"));
    }

    #[test]
    fn run_workspace_check_executes_configured_command_and_records_status() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
[scripts]
test = "printf 'test ok\n'"
lint = "printf 'lint failed\n'; exit 7"
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "add check scripts",
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

        let commands = store.configured_check_commands("berlin").unwrap();
        assert_eq!(
            commands
                .iter()
                .map(|command| command.key.as_str())
                .collect::<Vec<_>>(),
            vec!["test", "lint"]
        );

        let check = store.run_workspace_check("berlin", "test").unwrap();
        wait_for_log(&check.log_path, "test ok");
        let exited =
            wait_for_process_status(&store, "berlin", ProcessKind::Check, ProcessStatus::Exited);

        assert_eq!(check.kind, ProcessKind::Check);
        assert_eq!(exited.exit_code, Some(0));
        assert!(store
            .read_latest_check_log("berlin")
            .unwrap()
            .contains("test ok"));
        assert_eq!(
            store.checks_summary("berlin").unwrap().check_status,
            Some(ProcessStatus::Exited)
        );
    }

    #[test]
    fn terminal_command_runs_in_workspace_with_conductor_environment_and_captures_output() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
[environment_variables]
CUSTOM_VALUE = "from-settings"
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
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
                "pwd; printf '%s:%s:%s\\n' \"$ARCHDUCTOR_WORKSPACE_NAME\" \"$ARCHDUCTOR_PORT\" \"$CUSTOM_VALUE\"; printf 'warn\\n' >&2; exit 7",
            )
            .unwrap();

        assert_eq!(result.command, "pwd; printf '%s:%s:%s\\n' \"$ARCHDUCTOR_WORKSPACE_NAME\" \"$ARCHDUCTOR_PORT\" \"$CUSTOM_VALUE\"; printf 'warn\\n' >&2; exit 7");
        assert_eq!(result.cwd, workspace.path);
        assert_eq!(result.exit_code, Some(7));
        assert!(result.stdout.contains("berlin:3000:from-settings"));
        assert!(result.stdout.contains(result.cwd.to_str().unwrap()));
        assert_eq!(result.stderr, "warn\n");
        assert!(!result.started_at.is_empty());
        assert!(!result.ended_at.is_empty());
    }

    #[test]
    fn terminal_command_uses_configured_monorepo_working_directory() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir_all(repo_path.join("apps/web")).unwrap();
        fs::write(repo_path.join("apps/web/package.json"), "{}\n").unwrap();
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
[customization.workspace_defaults]
working_directory = "apps/web"
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", "."])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "add monorepo app",
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
                "pwd; printf 'root=%s\\nwork=%s\\n' \"$ARCHDUCTOR_WORKSPACE_PATH\" \"$ARCHDUCTOR_WORKING_DIRECTORY\"",
            )
            .unwrap();

        assert_eq!(result.cwd, workspace.path.join("apps/web"));
        assert!(result.stdout.contains(result.cwd.to_str().unwrap()));
        assert!(result
            .stdout
            .contains(&format!("root={}", workspace.path.to_string_lossy())));
        assert!(result.stdout.contains(&format!(
            "work={}",
            workspace.path.join("apps/web").to_string_lossy()
        )));
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
        let missing = store
            .record_terminal_process("berlin", "missing log", 4244)
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
        fs::remove_file(&missing.log_path).unwrap();

        let matches = store.search_terminal_logs("berlin", "needle").unwrap();

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].process_id, second.id);
        assert_eq!(matches[0].line_number, 2);
        assert_eq!(matches[0].line, "NEEDLE second");
        assert_eq!(matches[0].context_before, vec!["before one".to_owned()]);
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
    fn list_terminal_summaries_includes_missing_logs_without_failing_history() {
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
        let newer_missing = store
            .record_terminal_process("berlin", "missing shell", 4243)
            .unwrap();
        store
            .append_terminal_process_output(older.id, "older transcript\n")
            .unwrap();
        fs::remove_file(&newer_missing.log_path).unwrap();

        let summaries = store.list_terminal_summaries("berlin").unwrap();

        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].process.id, newer_missing.id);
        assert_eq!(summaries[0].line_count, 0);
        assert_eq!(summaries[0].byte_count, 0);
        assert_eq!(summaries[0].preview, "(missing transcript)");
        assert_eq!(summaries[1].process.id, older.id);
        assert_eq!(summaries[1].preview, "older transcript");
    }

    #[test]
    fn local_chat_history_summaries_include_saved_agent_sessions_newest_first() {
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
                name: "zurich".to_owned(),
                branch: "lc/zurich".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let berlin = store
            .record_session_process(
                "berlin",
                &SessionLaunch {
                    kind: SessionKind::Codex,
                    program: PathBuf::from("codex"),
                    args: Vec::new(),
                    cwd: temp.path().join("workspaces/demo/berlin"),
                    env: Vec::new(),
                    harness_metadata: Some("plan=true".to_owned()),
                    session_resume_id: None,
                },
                exited_child_pid(),
            )
            .unwrap();
        let zurich = store
            .record_session_process(
                "zurich",
                &SessionLaunch {
                    kind: SessionKind::Claude,
                    program: PathBuf::from("claude"),
                    args: Vec::new(),
                    cwd: temp.path().join("workspaces/demo/zurich"),
                    env: Vec::new(),
                    harness_metadata: None,
                    session_resume_id: None,
                },
                exited_child_pid(),
            )
            .unwrap();
        store
            .append_session_process_output(berlin.id, "older response\n")
            .unwrap();
        store
            .append_session_process_output(zurich.id, "newer response\n")
            .unwrap();

        let all = store.list_local_chat_history(None).unwrap();
        let berlin_only = store
            .list_local_chat_history(Some(temp.path().join("workspaces/demo/berlin").as_path()))
            .unwrap();

        assert_eq!(all.len(), 2);
        assert_eq!(all[0].workspace_name, "zurich");
        assert_eq!(all[0].agent_type, "Claude");
        assert_eq!(all[0].message_count, 1);
        assert_eq!(all[0].preview, "newer response");
        assert_eq!(all[1].workspace_name, "berlin");
        assert_eq!(all[1].agent_type, "Codex");
        assert_eq!(all[1].harness, Some("plan=true".to_owned()));
        assert_eq!(berlin_only.len(), 1);
        assert_eq!(berlin_only[0].process_id, berlin.id);
    }

    #[test]
    fn local_chat_history_summaries_prefer_structured_session_events() {
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
        let session = store
            .record_session_process(
                "berlin",
                &SessionLaunch {
                    kind: SessionKind::Codex,
                    program: PathBuf::from("codex"),
                    args: Vec::new(),
                    cwd: temp.path().join("workspaces/demo/berlin"),
                    env: Vec::new(),
                    harness_metadata: None,
                    session_resume_id: None,
                },
                exited_child_pid(),
            )
            .unwrap();
        store
            .append_session_events(
                session.id,
                vec![
                    SessionEvent::new(
                        SessionEventSource::User,
                        None,
                        SessionEventPayload::UserInput {
                            text: "run tests".to_owned(),
                            kind: crate::session_event::SessionInputKind::User,
                        },
                    ),
                    SessionEvent::new(
                        SessionEventSource::Assistant,
                        None,
                        SessionEventPayload::AssistantText {
                            text: "tests passed".to_owned(),
                        },
                    ),
                ],
            )
            .unwrap();

        let summaries = store.list_local_chat_history(None).unwrap();
        let messages = store.local_chat_history_messages(session.id).unwrap();

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].process_id, session.id);
        assert_eq!(summaries[0].message_count, 2);
        assert_eq!(summaries[0].preview, "tests passed");
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn local_chat_history_messages_render_saved_transcript_events() {
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
        let session = store
            .record_session_process(
                "berlin",
                &SessionLaunch {
                    kind: SessionKind::Shell,
                    program: PathBuf::from("/bin/sh"),
                    args: Vec::new(),
                    cwd: temp.path().join("workspaces/demo/berlin"),
                    env: Vec::new(),
                    harness_metadata: Some("fast=true".to_owned()),
                    session_resume_id: None,
                },
                exited_child_pid(),
            )
            .unwrap();
        store
            .append_session_process_output(
                session.id,
                "[session started] #1 kind=Shell pid=123\nagent preface\n[user input berlin#1]\nrun tests\n[/user input]\n[session finished] #1\nagent reply\n",
            )
            .unwrap();

        let messages = store.local_chat_history_messages(session.id).unwrap();

        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0].role, "system");
        assert!(messages[0].content.contains("kind=Shell"));
        assert_eq!(messages[1].role, "agent");
        assert_eq!(messages[1].content, "agent preface");
        assert_eq!(messages[2].role, "user");
        assert_eq!(messages[2].content, "run tests");
        assert_eq!(messages[3].role, "system");
        assert_eq!(messages[4].role, "agent");
        assert_eq!(messages[4].content, "agent reply");
    }

    #[test]
    fn local_chat_history_messages_parse_codex_screen_snapshots() {
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
        let session = store
            .record_session_process(
                "berlin",
                &SessionLaunch {
                    kind: SessionKind::Codex,
                    program: PathBuf::from("codex"),
                    args: Vec::new(),
                    cwd: temp.path().join("workspaces/demo/berlin"),
                    env: Vec::new(),
                    harness_metadata: None,
                    session_resume_id: None,
                },
                exited_child_pid(),
            )
            .unwrap();
        store
            .append_session_process_output(
                session.id,
                "[codex raw]\nwrapper:codex\narg:--no-alt-screen\n[/codex raw]\n[codex screen]\n╭─ You ─╮\n│ run tests\n╰────\n╭─ Codex ─╮\n│ Running now.\n╰────\n[/codex screen]\n[codex raw]\narg:-C\n[/codex raw]\n[codex screen]\n╭─ You ─╮\n│ run tests\n╰────\n╭─ Codex ─╮\n│ Running now.\n│ Tests passed.\n╰────\n[/codex screen]\n",
            )
            .unwrap();

        let messages = store.local_chat_history_messages(session.id).unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "run tests");
        assert_eq!(messages[1].role, "agent");
        assert_eq!(messages[1].content, "Running now.\nTests passed.");
    }

    #[test]
    fn linked_directory_links_workspace_context_to_target_workspace() {
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
        let frontend = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "frontend".to_owned(),
                branch: "lc/frontend".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let backend = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "backend".to_owned(),
                branch: "lc/backend".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        let link = store
            .link_workspace_directory("frontend", "backend")
            .unwrap();
        let links = store.list_linked_directories("frontend").unwrap();

        assert_eq!(links, vec![link.clone()]);
        assert_eq!(link.workspace_name, "frontend");
        assert_eq!(link.target_workspace_name, "backend");
        assert_eq!(link.target_workspace_path, backend.path);
        assert_eq!(
            link.link_path,
            frontend.path.join(".context/linked-directories/backend")
        );
        assert!(link.link_path.exists());
        assert_eq!(
            fs::read_link(&link.link_path).unwrap(),
            link.target_workspace_path
        );
    }

    #[test]
    fn session_launch_exposes_linked_directory_environment() {
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
                name: "frontend".to_owned(),
                branch: "lc/frontend".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let backend = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "backend".to_owned(),
                branch: "lc/backend".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        store
            .link_workspace_directory("frontend", "backend")
            .unwrap();

        let launch = store
            .session_launch("frontend", SessionKind::Codex)
            .unwrap();

        assert_eq!(
            launch.env_value("ARCHDUCTOR_LINKED_DIRECTORIES"),
            Some(format!("backend={}", backend.path.display()).as_str())
        );
        assert_eq!(
            launch.env_value("ARCHDUCTOR_LINKED_DIRECTORY_BACKEND"),
            Some(backend.path.to_str().unwrap())
        );
    }

    #[test]
    fn workspace_view_defaults_read_repository_customization() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r##"
[customization.workspace_defaults]
default_visible_tab = "checks"

[scripts]
test = "cargo test --workspace"
typecheck = "cargo check --workspace"

[customization.view]
theme = "dark"
accent_color = "green"
density = "compact"
keybindings = "vim"
terminal_font = "JetBrains Mono 13"
terminal_scrollback = 5000
command_palette_presets = ["test", "Preview=pnpm dev"]

[customization.view.colors]
accent = "#0ea5e9"
"##,
        )
        .unwrap();
        git(&repo_path, ["add", ".archductor/settings.toml"]).unwrap();
        git(
            &repo_path,
            [
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "add settings",
            ],
        )
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

        let store = WorkspaceStore::open_with_logs(&db_path, temp.path().join("logs")).unwrap();
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        let defaults = store.workspace_view_defaults("berlin").unwrap();

        assert_eq!(defaults.default_visible_tab.as_deref(), Some("checks"));
        assert_eq!(defaults.theme.as_deref(), Some("dark"));
        assert_eq!(defaults.accent_color.as_deref(), Some("green"));
        assert_eq!(defaults.colors.get("accent"), Some(&"#0ea5e9".to_owned()));
        assert_eq!(defaults.density.as_deref(), Some("compact"));
        assert_eq!(defaults.keybindings.as_deref(), Some("vim"));
        assert_eq!(defaults.terminal_font.as_deref(), Some("JetBrains Mono 13"));
        assert_eq!(defaults.terminal_scrollback, Some(5000));
        assert_eq!(
            defaults.command_palette_presets,
            vec![
                "Test=cargo test --workspace".to_owned(),
                "Typecheck=cargo check --workspace".to_owned(),
                "test".to_owned(),
                "Preview=pnpm dev".to_owned()
            ]
        );
        assert!(defaults.agent_profile_names.is_empty());
        assert!(defaults.notification_rules.is_empty());
    }

    #[test]
    fn setup_workspace_executes_setup_script_and_captures_logs() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
[scripts]
setup = "printf 'setup:%s:%s\n' \"$ARCHDUCTOR_WORKSPACE_NAME\" \"$ARCHDUCTOR_PORT\""
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
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
    fn setup_workspace_uses_configured_monorepo_working_directory() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir_all(repo_path.join("apps/api")).unwrap();
        fs::write(repo_path.join("apps/api/Cargo.toml"), "[package]\n").unwrap();
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
[scripts]
setup = "pwd; printf 'work=%s\n' \"$ARCHDUCTOR_WORKING_DIRECTORY\""

[customization.workspace_defaults]
working_directory = "apps/api"
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", "."])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "add monorepo setup",
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
        let setup = store.setup_workspace("berlin").unwrap();
        wait_for_log(&setup.log_path, "work=");
        let log = store.read_latest_setup_log("berlin").unwrap();

        assert!(log.contains(workspace.path.join("apps/api").to_str().unwrap()));
        assert!(log.contains(&format!(
            "work={}",
            workspace.path.join("apps/api").to_string_lossy()
        )));
    }

    #[test]
    fn spotlight_start_applies_workspace_tracked_changes_and_stop_restores_root() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
spotlight_testing = true
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
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
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
spotlight_testing = true
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
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
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
spotlight_testing = true
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
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
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
spotlight_testing = true
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
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
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
spotlight_testing = true
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
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
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
spotlight_testing = true
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
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
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
spotlight_testing = true
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
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
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
spotlight_testing = true
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
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
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
spotlight_testing = true
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
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
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
spotlight_testing = true
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
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
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
spotlight_testing = true
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
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
        assert_eq!(
            launch.env_value("ARCHDUCTOR_WORKSPACE_NAME"),
            Some("berlin")
        );
        assert_eq!(launch.env_value("ARCHDUCTOR_PORT"), Some("3000"));
        assert_eq!(
            launch.env_value("ARCHDUCTOR_ROOT_PATH"),
            repo_path.canonicalize().unwrap().to_str()
        );
    }

    #[test]
    fn session_launch_uses_configured_monorepo_working_directory() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir_all(repo_path.join("apps/worker")).unwrap();
        fs::write(repo_path.join("apps/worker/main.rs"), "fn main() {}\n").unwrap();
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
[customization.workspace_defaults]
working_directory = "apps/worker"
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", "."])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "add worker",
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

        let launch = store.session_launch("berlin", SessionKind::Shell).unwrap();

        assert_eq!(launch.cwd, workspace.path.join("apps/worker"));
        assert_eq!(
            launch.env_value("ARCHDUCTOR_WORKSPACE_PATH"),
            workspace.path.to_str()
        );
        assert_eq!(
            launch.env_value("ARCHDUCTOR_WORKING_DIRECTORY"),
            workspace.path.join("apps/worker").to_str()
        );
    }

    #[test]
    fn session_launch_for_codex_uses_documented_flags_without_bootstrap_payload() {
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
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        let launch = store
            .session_launch_with_options(
                "berlin",
                SessionKind::Codex,
                SessionHarnessOptions {
                    plan_mode: true,
                    fast_mode: true,
                    approval_mode: Some("ask".to_owned()),
                    reasoning_mode: Some("high".to_owned()),
                    effort_mode: Some("medium".to_owned()),
                    codex_personality: Some("pragmatic".to_owned()),
                    codex_goals: Some("ship the fix".to_owned()),
                    codex_skills: Some("tests".to_owned()),
                },
            )
            .unwrap();

        let codex_cwd = launch.cwd.to_str().unwrap().to_owned();
        let codex_trust_arg = format!(r#"projects."{codex_cwd}".trust_level="trusted""#);
        assert_eq!(&launch.program, &PathBuf::from("codex"));
        assert_eq!(
            &launch.args,
            &vec![
                "--no-alt-screen",
                "--dangerously-bypass-approvals-and-sandbox",
                "-c",
                "check_for_update_on_startup=false",
                "-c",
                codex_trust_arg.as_str(),
                "-C",
                codex_cwd.as_str(),
                "-c",
                r#"model_reasoning_effort="high""#,
                "-c",
                r#"personality="pragmatic""#,
                "-c",
                r#"service_tier="fast""#,
                "--ask-for-approval",
                "on-request",
                "--enable",
                "goals",
                "ship the fix",
            ]
        );
        assert_eq!(
            launch.harness_metadata.as_deref(),
            Some(
                "harness=codex;plan=true;fast=true;approval=ask;reasoning=high;effort=medium;personality=pragmatic;goals=ship the fix;skills=tests"
            )
        );
        assert!(launch.session_resume_id.is_none());
        assert!(launch.env_value("ARCHDUCTOR_SESSION_BOOTSTRAP").is_none());
    }

    #[test]
    fn session_launch_for_claude_uses_documented_flags_and_bootstrap() {
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
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        let launch = store
            .session_launch_with_options(
                "berlin",
                SessionKind::Claude,
                SessionHarnessOptions {
                    plan_mode: true,
                    fast_mode: true,
                    approval_mode: Some("never".to_owned()),
                    reasoning_mode: Some("low".to_owned()),
                    effort_mode: Some("high".to_owned()),
                    codex_personality: Some("friendly".to_owned()),
                    codex_goals: Some("stabilize the fix".to_owned()),
                    codex_skills: Some("rust, tests".to_owned()),
                },
            )
            .unwrap();

        let claude_bootstrap = launch
            .env_value("ARCHDUCTOR_SESSION_BOOTSTRAP")
            .unwrap()
            .to_owned();
        assert_eq!(&launch.program, &PathBuf::from("claude"));
        assert_eq!(
            &launch.args,
            &vec![
                "--permission-mode",
                "plan",
                "--effort",
                "high",
                "--session-id",
                launch.session_resume_id.as_deref().unwrap(),
                "--append-system-prompt",
                claude_bootstrap.as_str(),
            ]
        );
        assert_eq!(
            launch.harness_metadata.as_deref(),
            Some(
                "harness=claude;plan=true;fast=true;approval=never;reasoning=low;effort=high;personality=friendly;goals=stabilize the fix;skills=rust, tests"
            )
        );
        assert!(launch.session_resume_id.is_some());
    }

    #[test]
    fn session_resume_launch_uses_resume_subcommand_and_session_id() {
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
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        let launch = store
            .session_launch_with_options_and_resume(
                "berlin",
                SessionKind::Claude,
                SessionHarnessOptions::default(),
                Some("019ef6b1-8a1b-78f0-ae17-0db46572decf"),
            )
            .unwrap();

        assert_eq!(
            launch.args,
            vec![
                "--resume".to_owned(),
                "019ef6b1-8a1b-78f0-ae17-0db46572decf".to_owned()
            ]
        );
        assert_eq!(
            launch.session_resume_id.as_deref(),
            Some("019ef6b1-8a1b-78f0-ae17-0db46572decf")
        );
    }

    #[test]
    fn codex_resume_launch_without_id_uses_last_and_preserves_harness() {
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
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        let launch = store
            .session_launch_with_options_and_resume(
                "berlin",
                SessionKind::Codex,
                SessionHarnessOptions {
                    fast_mode: true,
                    approval_mode: Some("ask".to_owned()),
                    reasoning_mode: Some("high".to_owned()),
                    codex_personality: Some("pragmatic".to_owned()),
                    codex_goals: Some("ship the fix".to_owned()),
                    ..SessionHarnessOptions::default()
                },
                None,
            )
            .unwrap();

        assert_eq!(
            launch.args,
            vec![
                "--no-alt-screen".to_owned(),
                "--dangerously-bypass-approvals-and-sandbox".to_owned(),
                "-c".to_owned(),
                "check_for_update_on_startup=false".to_owned(),
                "-c".to_owned(),
                format!(
                    r#"projects."{}".trust_level="trusted""#,
                    launch.cwd.to_string_lossy()
                ),
                "-C".to_owned(),
                launch.cwd.to_string_lossy().to_string(),
                "-c".to_owned(),
                r#"model_reasoning_effort="high""#.to_owned(),
                "-c".to_owned(),
                r#"personality="pragmatic""#.to_owned(),
                "-c".to_owned(),
                r#"service_tier="fast""#.to_owned(),
                "--ask-for-approval".to_owned(),
                "on-request".to_owned(),
                "--enable".to_owned(),
                "goals".to_owned(),
                "ship the fix".to_owned(),
                "resume".to_owned(),
                "--last".to_owned(),
            ]
        );
        assert_eq!(
            launch.harness_metadata.as_deref(),
            Some(
                "harness=codex;fast=true;approval=ask;reasoning=high;personality=pragmatic;goals=ship the fix"
            )
        );
        assert!(launch.session_resume_id.is_none());
    }

    #[test]
    fn start_session_with_options_rejects_codex_runtime_ownership() {
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

        let err = store
            .start_session_with_options(
                "berlin",
                SessionKind::Codex,
                SessionHarnessOptions {
                    plan_mode: true,
                    codex_goals: Some("ship the fix".to_owned()),
                    ..SessionHarnessOptions::default()
                },
            )
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("Codex sessions are owned by archcar"),
            "{err:#}"
        );
    }

    #[test]
    fn record_session_process_does_not_guess_codex_resume_id() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        let db_path = temp.path().join("state.db");
        let fake_home = temp.path().join("home");
        fs::create_dir_all(fake_home.join(".codex")).unwrap();
        fs::write(
            fake_home.join(".codex/session_index.jsonl"),
            "{\"id\":\"guessed-session-id\"}\n",
        )
        .unwrap();

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

        temp_env_var("HOME", &fake_home, || {
            let process = store
                .record_session_process(
                    "berlin",
                    &SessionLaunch {
                        kind: SessionKind::Codex,
                        program: PathBuf::from("codex"),
                        args: vec!["resume".to_owned(), "--last".to_owned()],
                        cwd: workspace.path.clone(),
                        env: Vec::new(),
                        harness_metadata: Some("harness=codex".to_owned()),
                        session_resume_id: None,
                    },
                    exited_child_pid(),
                )
                .unwrap();

            assert_eq!(process.session_resume_id, None);
        });
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
            .record_session_process("berlin", &launch, exited_child_pid())
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
    fn session_events_are_persisted_with_raw_logs_and_reload_after_restart() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        let db_path = temp.path().join("state.db");
        let logs_dir = temp.path().join("logs");

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

        let store = WorkspaceStore::open_with_logs(&db_path, &logs_dir).unwrap();
        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let launch = store.session_launch("berlin", SessionKind::Shell).unwrap();
        let process = store
            .record_session_process("berlin", &launch, exited_child_pid())
            .unwrap();

        store
            .append_session_process_output(process.id, "[codex raw]\nhello\n[/codex raw]\n")
            .unwrap();
        store
            .append_session_events(
                process.id,
                vec![
                    crate::session_event::SessionEvent::new(
                        crate::session_event::SessionEventSource::User,
                        Some("› run tests".to_owned()),
                        crate::session_event::SessionEventPayload::UserInput {
                            text: "run tests".to_owned(),
                            kind: crate::session_event::SessionInputKind::User,
                        },
                    ),
                    crate::session_event::SessionEvent::new(
                        crate::session_event::SessionEventSource::Assistant,
                        Some("Tests passed.".to_owned()),
                        crate::session_event::SessionEventPayload::AssistantText {
                            text: "Tests passed.".to_owned(),
                        },
                    ),
                ],
            )
            .unwrap();

        drop(store);
        let reopened = WorkspaceStore::open_with_logs(&db_path, &logs_dir).unwrap();
        let raw = fs::read_to_string(&process.log_path).unwrap();
        let events = reopened.list_session_events(process.id).unwrap();
        let history_messages = reopened.local_chat_history_messages(process.id).unwrap();

        assert!(raw.contains("[codex raw]"));
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].sequence, Some(1));
        assert_eq!(events[1].sequence, Some(2));
        assert_eq!(events[0].raw_text.as_deref(), Some("› run tests"));
        assert_eq!(events[1].render_text(), "Tests passed.");
        assert_eq!(events[0].occurred_at_ms, events[1].occurred_at_ms);
        assert_eq!(
            events[0].source,
            crate::session_event::SessionEventSource::User
        );
        assert_eq!(
            events[1].source,
            crate::session_event::SessionEventSource::Assistant
        );
        assert_eq!(process.workspace_id, workspace.id);
        assert_eq!(
            history_messages,
            vec![
                LocalChatHistoryMessage {
                    role: "user".to_owned(),
                    content: "run tests".to_owned(),
                },
                LocalChatHistoryMessage {
                    role: "agent".to_owned(),
                    content: "Tests passed.".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn codex_screen_delta_persists_typed_session_events_for_ui_runtime() {
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
        let _workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let thread = store
            .create_chat_thread("berlin", "codex", "Parser work", None)
            .unwrap();
        let launch = store.session_launch("berlin", SessionKind::Codex).unwrap();
        let process = store
            .record_session_process_for_thread("berlin", thread.id, &launch, exited_child_pid())
            .unwrap();

        store
            .append_chat_message(thread.id, "user", "run tests", "composer")
            .unwrap();
        store
            .persist_codex_screen_delta(
                thread.id,
                process.id,
                "› run tests\n• Running now.\nRan cargo test\nok\n",
            )
            .unwrap();

        let session_events = store.list_session_events(process.id).unwrap();
        assert_eq!(
            session_events
                .iter()
                .map(crate::session_event::SessionEvent::render_text)
                .collect::<Vec<_>>(),
            vec!["Running now.", "cargo test\nok"]
        );
        assert!(matches!(
            session_events[0].payload,
            crate::session_event::SessionEventPayload::AssistantText { .. }
        ));
        assert!(matches!(
            session_events[1].payload,
            crate::session_event::SessionEventPayload::CommandOutput { .. }
        ));
    }

    #[test]
    fn pty_chunks_are_persisted_with_monotonic_session_sequences() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        let db_path = temp.path().join("state.db");
        let logs_dir = temp.path().join("logs");

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

        let store = WorkspaceStore::open_with_logs(&db_path, &logs_dir).unwrap();
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let launch = store.session_launch("berlin", SessionKind::Codex).unwrap();
        let process = store
            .record_session_process("berlin", &launch, exited_child_pid())
            .unwrap();

        store
            .append_session_process_output(process.id, "[codex raw]\nformatted\n[/codex raw]\n")
            .unwrap();
        let first = store
            .append_pty_chunk(process.id, "stdout_pty", "› run tests\n")
            .unwrap();
        let second = store
            .append_pty_chunk(process.id, "stdout_pty", "• Done.\n")
            .unwrap();

        drop(store);
        let reopened = WorkspaceStore::open_with_logs(&db_path, &logs_dir).unwrap();
        let chunks = reopened.list_pty_chunks(process.id).unwrap();
        let log = fs::read_to_string(&process.log_path).unwrap();

        assert_eq!(first.sequence, 1);
        assert_eq!(second.sequence, 2);
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.sequence)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.text.as_str())
                .collect::<String>(),
            "› run tests\n• Done.\n"
        );
        assert!(chunks
            .iter()
            .all(|chunk| chunk.stream == "stdout_pty" && chunk.process_id == process.id));
        assert!(
            log.contains("[codex raw]"),
            "human-readable log should remain independent from chunk rows"
        );
        assert!(
            !log.contains("› run tests\n• Done.\n"),
            "raw chunks should not be reconstructed from formatted log text"
        );

        let replay_screen = chunks
            .iter()
            .map(|chunk| chunk.text.as_str())
            .collect::<String>();
        let replay = process_codex_pty_pipeline(SessionPipelineInput {
            chunks: chunks
                .into_iter()
                .map(|chunk| PtyChunkInput {
                    sequence: chunk.sequence,
                    text: chunk.text,
                })
                .collect(),
            screen: replay_screen,
            benchmark: CodexParseBenchmark {
                last_user_message: Some("run tests".to_owned()),
                last_agent_message: None,
            },
            previous_cursor: None,
            previous_state: AgentSessionState::Running,
        });
        assert!(
            replay
                .events
                .iter()
                .any(|event| event.render_text() == "Done."),
            "persisted PTY chunks should replay into typed parser events"
        );
    }

    #[test]
    fn project_and_workspace_models_include_runtime_review_and_config_boundaries() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir_all(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
[scripts]
setup = "make setup"
run = "make dev"

[environment_variables]
API_BASE_URL = "http://localhost:3000"

[prompts]
general = "Keep changes focused."
"#,
        )
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

        let store = WorkspaceStore::open_with_logs(&db_path, temp.path().join("logs")).unwrap();
        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let launch = store.session_launch("berlin", SessionKind::Shell).unwrap();
        let first_session = store
            .record_session_process("berlin", &launch, exited_child_pid())
            .unwrap();
        let second_session = store
            .record_session_process("berlin", &launch, exited_child_pid())
            .unwrap();
        let checkpoint = store
            .checkpoint_create("berlin", "before refactor", Some(first_session.id))
            .unwrap();
        store.add_todo("berlin", "finish parser").unwrap();
        store
            .add_review_comment("berlin", "src/lib.rs", Some(12), "handle failure")
            .unwrap();

        let project = store.project_model("demo").unwrap();
        let model = store.workspace_model("berlin").unwrap();

        assert_eq!(project.id, workspace.repository_id);
        assert_eq!(project.name, "demo");
        assert_eq!(project.repository.default_branch, "main");
        assert_eq!(project.scripts.setup.as_deref(), Some("make setup"));
        assert_eq!(project.scripts.run.as_deref(), Some("make dev"));
        assert_eq!(
            project
                .environment_variables
                .get("API_BASE_URL")
                .map(String::as_str),
            Some("http://localhost:3000")
        );
        assert_eq!(
            project.prompts.general.as_deref(),
            Some("Keep changes focused.")
        );
        assert_eq!(project.workspace_ids, vec![workspace.id]);

        assert_eq!(model.workspace.id, workspace.id);
        assert_eq!(model.project_id, workspace.repository_id);
        assert_eq!(model.branch, "lc/berlin");
        assert_eq!(model.worktree_path, workspace.path);
        assert_eq!(model.session_ids, vec![first_session.id, second_session.id]);
        assert_eq!(model.checkpoint_ids, vec![checkpoint.id]);
        assert_eq!(model.open_todos, 1);
        assert_eq!(model.open_review_comments, 1);
        assert_eq!(model.status.as_str(), "active");
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
                    staged: false,
                    unstaged: true,
                    untracked: false,
                },
                DiffFileSummary {
                    path: "notes.txt".to_owned(),
                    additions: Some(1),
                    deletions: Some(0),
                    staged: false,
                    unstaged: false,
                    untracked: true,
                },
            ]
        );
    }

    #[test]
    fn diff_stats_against_base_include_committed_and_untracked_changes() {
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
        fs::write(workspace.path.join("README.md"), "changed\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args(["add", "README.md"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "change readme",
            ])
            .status()
            .unwrap();
        fs::write(workspace.path.join("notes.txt"), "new\nnotes\n").unwrap();

        assert_eq!(store.diff_stats_against_base("berlin").unwrap(), (3, 1));
        let line = store
            .list_status()
            .unwrap()
            .into_iter()
            .find(|line| line.workspace.name == "berlin")
            .unwrap();
        assert_eq!((line.diff_additions, line.diff_deletions), (3, 1));
    }

    #[test]
    fn set_workspace_base_ref_changes_base_diff_output() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["checkout", "-b", "develop"])
            .status()
            .unwrap();
        fs::write(repo_path.join("README.md"), "develop\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", "README.md"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "develop readme",
            ])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["checkout", "main"])
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
        fs::write(workspace.path.join("README.md"), "feature\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args(["add", "README.md"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "feature readme",
            ])
            .status()
            .unwrap();

        let main_diff = store
            .unified_diff_against_base("berlin", Some(Path::new("README.md")))
            .unwrap();
        assert!(main_diff.contains("-demo"));

        let updated = store.set_workspace_base_ref("berlin", "develop").unwrap();
        assert_eq!(updated.base_ref, "develop");
        let develop_diff = store
            .unified_diff_against_base("berlin", Some(Path::new("README.md")))
            .unwrap();

        assert!(develop_diff.contains("-develop"));
        assert!(!develop_diff.contains("-demo"));
        assert_eq!(
            store
                .workspace_timeline("berlin", Some("base_ref.updated"))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn force_push_branch_with_lease_updates_remote_branch() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        let remote_path = temp.path().join("upstream.git");
        Command::new("git")
            .args(["init", "--bare", "--initial-branch", "main"])
            .arg(&remote_path)
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["remote", "add", "upstream"])
            .arg(&remote_path)
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["push", "-u", "upstream", "main"])
            .status()
            .unwrap();
        let db_path = temp.path().join("state.db");

        RepositoryStore::open(&db_path)
            .unwrap()
            .add(AddRepository {
                name: Some("demo".to_owned()),
                root_path: repo_path,
                default_branch: Some("main".to_owned()),
                remote_name: "upstream".to_owned(),
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
        fs::write(workspace.path.join("README.md"), "force push\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args(["add", "README.md"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "force push readme",
            ])
            .status()
            .unwrap();

        let output = store.force_push_branch_with_lease("berlin").unwrap();
        let workspace_head = git_output(&workspace.path, ["rev-parse", "HEAD"]);
        let remote_head = Command::new("git")
            .arg("--git-dir")
            .arg(&remote_path)
            .args(["rev-parse", "refs/heads/lc/berlin"])
            .output()
            .unwrap();

        assert!(output.contains("lc/berlin"));
        assert_eq!(
            String::from_utf8(remote_head.stdout).unwrap().trim(),
            workspace_head.trim()
        );
        assert_eq!(
            store
                .workspace_timeline("berlin", Some("branch.force_pushed"))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn truncate_text_at_char_boundary_reserves_marker_space() {
        let (diff, truncated) = truncate_text_at_char_boundary("a".repeat(80), 64);

        assert!(truncated);
        assert!(diff.len() <= 64);
        assert!(diff.ends_with("[Diff truncated at hard limit]\n"));
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
    fn source_preflight_statuses_explain_missing_github_and_linear_inputs() {
        let missing = WorkspaceSourcePreflight {
            github_cli_installed: false,
            github_authenticated: false,
            linear_api_key_set: false,
        };
        assert!(!missing.github_ready());
        assert_eq!(missing.github_status(), "gh missing");
        assert!(!missing.linear_ready());
        assert_eq!(missing.linear_status(), "LINEAR_API_KEY missing");

        let unauthenticated = WorkspaceSourcePreflight {
            github_cli_installed: true,
            github_authenticated: false,
            linear_api_key_set: true,
        };
        assert!(!unauthenticated.github_ready());
        assert_eq!(unauthenticated.github_status(), "gh auth required");
        assert!(unauthenticated.linear_ready());
        assert_eq!(unauthenticated.linear_status(), "ready");

        let ready = WorkspaceSourcePreflight {
            github_cli_installed: true,
            github_authenticated: true,
            linear_api_key_set: true,
        };
        assert!(ready.github_ready());
        assert_eq!(ready.github_status(), "ready");
        assert!(ready.linear_ready());
        assert_eq!(ready.linear_status(), "ready");
    }

    #[test]
    fn pull_request_readiness_summary_formats_reviews_checks_and_deployments() {
        let json = r#"
{
  "reviewDecision": "CHANGES_REQUESTED",
  "latestReviews": [
    {
      "author": {"login": "alice"},
      "state": "CHANGES_REQUESTED",
      "body": "Please add a regression test.",
      "submittedAt": "2026-06-20T12:00:00Z"
    },
    {
      "author": {"login": "bob"},
      "state": "APPROVED",
      "body": "",
      "submittedAt": "2026-06-20T12:05:00Z"
    }
  ],
  "comments": [
    {
      "author": {"login": "carol"},
      "body": "This also needs docs.",
      "createdAt": "2026-06-20T12:10:00Z"
    }
  ],
  "statusCheckRollup": [
    {
      "__typename": "CheckRun",
      "name": "unit",
      "status": "COMPLETED",
      "conclusion": "FAILURE",
      "detailsUrl": "https://github.com/example/demo/actions/runs/1"
    },
    {
      "__typename": "StatusContext",
      "context": "lint",
      "state": "SUCCESS",
      "targetUrl": "https://github.com/example/demo/actions/runs/2"
    },
    {
      "__typename": "Deployment",
      "environment": "Preview",
      "state": "ACTIVE",
      "latestStatus": {"state": "SUCCESS"},
      "url": "https://preview.example.com"
    }
  ]
}
"#;

        let readiness = parse_pull_request_readiness(json).unwrap();
        assert_eq!(
            readiness.review_decision.as_deref(),
            Some("CHANGES_REQUESTED")
        );
        assert_eq!(readiness.latest_reviews.len(), 2);
        assert_eq!(readiness.comments.len(), 1);
        assert_eq!(readiness.checks.len(), 2);
        assert_eq!(readiness.deployments.len(), 1);

        let text = format_pull_request_readiness("berlin", &readiness);
        assert!(text.contains("PR readiness for workspace berlin."));
        assert!(text.contains("Review decision: CHANGES_REQUESTED"));
        assert!(text.contains("alice: CHANGES_REQUESTED - Please add a regression test."));
        assert!(text.contains("carol: This also needs docs."));
        assert!(text.contains("unit: FAILURE - https://github.com/example/demo/actions/runs/1"));
        assert!(text.contains("lint: SUCCESS - https://github.com/example/demo/actions/runs/2"));
        assert!(text.contains("Preview: SUCCESS - https://preview.example.com"));
    }

    #[test]
    fn github_pull_request_parsing_is_extracted_from_workspace_store() {
        let source = include_str!("workspace.rs");
        let parser_fn = concat!("fn ", "parse_pull_request_readiness(");
        let formatter_fn = concat!("fn ", "format_pull_request_readiness(");

        assert!(
            !source.contains(parser_fn),
            "GitHub PR parsing should live outside the workspace store module"
        );
        assert!(
            !source.contains(formatter_fn),
            "GitHub PR readiness formatting should live outside the workspace store module"
        );
    }

    #[test]
    fn terminal_log_aggregation_is_extracted_from_workspace_store() {
        let source = include_str!("workspace.rs");
        let preview_fn = concat!("fn ", "terminal_log_preview(");
        let search_context_const = concat!("const ", "TERMINAL_SEARCH_CONTEXT_LINES");
        let match_struct = concat!("struct ", "TerminalLogMatch");

        assert!(
            !source.contains(preview_fn),
            "terminal log preview formatting should live outside the workspace store module"
        );
        assert!(
            !source.contains(search_context_const),
            "terminal search context policy should live outside the workspace store module"
        );
        assert!(
            !source.contains(match_struct),
            "terminal search result types should live outside the workspace store module"
        );
    }

    #[test]
    fn local_chat_parsing_is_extracted_from_workspace_store() {
        let source = include_str!("workspace.rs");
        let transcript_parser = concat!("fn ", "parse_local_chat_transcript(");
        let event_mapper = concat!("fn ", "session_events_to_local_chat_messages(");
        let agent_classifier = concat!("fn ", "local_chat_agent_type(");

        assert!(
            !source.contains(transcript_parser),
            "local chat transcript parsing should live outside the workspace store module"
        );
        assert!(
            !source.contains(event_mapper),
            "local chat event mapping should live outside the workspace store module"
        );
        assert!(
            !source.contains(agent_classifier),
            "local chat agent classification should live outside the workspace store module"
        );
    }

    #[test]
    fn context_todo_parsing_is_extracted_from_workspace_store() {
        let source = include_str!("workspace.rs");
        let parser_fn = concat!("fn ", "parse_context_todos(");

        assert!(
            !source.contains(parser_fn),
            "context todo markdown parsing should live outside the workspace store module"
        );
    }

    #[test]
    fn pull_request_readiness_parses_nested_deployment_rollup_shapes() {
        let json = r#"
{
  "reviewDecision": "UNKNOWN",
  "latestReviews": [],
  "comments": [],
  "statusCheckRollup": [
    {
      "__typename": "DeploymentStatus",
      "environment": {"name": "Preview"},
      "status": {"state": "FAILURE"},
      "targetUrl": "https://preview.example.com/status"
    }
  ]
}
"#;

        let readiness = parse_pull_request_readiness(json).unwrap();

        assert_eq!(readiness.checks.len(), 0);
        assert_eq!(
            readiness.deployments,
            vec![PullRequestDeployment {
                environment: "Preview".to_owned(),
                status: "FAILURE".to_owned(),
                url: Some("https://preview.example.com/status".to_owned()),
            }]
        );
        let text = format_pull_request_readiness("berlin", &readiness);
        assert!(text.contains("Preview: FAILURE - https://preview.example.com/status"));
        assert!(text.contains("- Deployments: 0 passing, 1 failing, 0 pending, 0 other"));
    }

    #[test]
    fn pull_request_readiness_parses_latest_status_deployment_urls() {
        let json = r#"
{
  "reviewDecision": "UNKNOWN",
  "latestReviews": [],
  "comments": [],
  "statusCheckRollup": [
    {
      "__typename": "Deployment",
      "environment": {"name": "Preview"},
      "latestStatus": {
        "state": "SUCCESS",
        "environmentUrl": "https://preview.example.com",
        "logUrl": "https://github.com/example/demo/deployments/activity_log"
      }
    }
  ]
}
"#;

        let readiness = parse_pull_request_readiness(json).unwrap();

        assert_eq!(
            readiness.deployments,
            vec![PullRequestDeployment {
                environment: "Preview".to_owned(),
                status: "SUCCESS".to_owned(),
                url: Some("https://preview.example.com".to_owned()),
            }]
        );
    }

    #[test]
    fn pull_request_readiness_parses_connection_status_rollup_shapes() {
        let json = r#"
{
  "reviewDecision": "UNKNOWN",
  "latestReviews": [],
  "comments": [],
  "statusCheckRollup": {
    "contexts": {
      "nodes": [
        {
          "__typename": "CheckRun",
          "name": "unit",
          "status": "COMPLETED",
          "conclusion": "SUCCESS",
          "detailsUrl": "https://github.com/example/demo/actions/runs/1"
        },
        {
          "__typename": "StatusContext",
          "context": "lint",
          "state": "PENDING",
          "targetUrl": "https://github.com/example/demo/actions/runs/2"
        },
        {
          "__typename": "Deployment",
          "environment": {"name": "Preview"},
          "latestStatus": {"state": "SUCCESS"},
          "latestEnvironmentUrl": "https://preview.example.com"
        }
      ]
    }
  }
}
"#;

        let readiness = parse_pull_request_readiness(json).unwrap();

        assert_eq!(
            readiness.checks,
            vec![
                PullRequestCheckRun {
                    name: "unit".to_owned(),
                    status: "SUCCESS".to_owned(),
                    detail: Some("https://github.com/example/demo/actions/runs/1".to_owned()),
                },
                PullRequestCheckRun {
                    name: "lint".to_owned(),
                    status: "PENDING".to_owned(),
                    detail: Some("https://github.com/example/demo/actions/runs/2".to_owned()),
                },
            ]
        );
        assert_eq!(
            readiness.deployments,
            vec![PullRequestDeployment {
                environment: "Preview".to_owned(),
                status: "SUCCESS".to_owned(),
                url: Some("https://preview.example.com".to_owned()),
            }]
        );
    }

    #[test]
    fn pull_request_review_threads_parse_graphql_response() {
        let json = r#"
{
  "data": {
    "node": {
      "reviewThreads": {
        "nodes": [
          {
            "id": "PRRT_kwDOexample1",
            "isResolved": false,
            "path": "src/lib.rs",
            "line": 42,
            "comments": {
              "nodes": [
                {
                  "author": {"login": "alice"},
                  "body": "This needs a regression test.",
                  "url": "https://github.com/example/demo/pull/7#discussion_r1",
                  "createdAt": "2026-06-20T12:00:00Z"
                }
              ]
            }
          },
          {
            "isResolved": true,
            "path": "README.md",
            "startLine": 5,
            "comments": {
              "nodes": [
                {
                  "author": {"login": "bob"},
                  "body": "Resolved doc note.",
                  "url": "https://github.com/example/demo/pull/7#discussion_r2",
                  "createdAt": "2026-06-20T12:05:00Z"
                }
              ]
            }
          }
        ]
      }
    }
  }
}
"#;

        let threads = parse_pull_request_review_threads(json).unwrap();

        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].path.as_deref(), Some("src/lib.rs"));
        assert_eq!(threads[0].line, Some(42));
        assert!(!threads[0].resolved);
        assert_eq!(threads[0].comments[0].author, "alice");
        assert_eq!(threads[1].path.as_deref(), Some("README.md"));
        assert_eq!(threads[1].line, Some(5));
        assert!(threads[1].resolved);

        let readiness = PullRequestReadiness {
            review_decision: None,
            latest_reviews: Vec::new(),
            comments: Vec::new(),
            review_threads: threads,
            checks: Vec::new(),
            deployments: Vec::new(),
        };
        let text = format_pull_request_readiness("berlin", &readiness);
        assert!(text.contains("PRRT_kwDOexample1"));
    }

    #[test]
    fn pull_request_readiness_agent_prompt_includes_actionable_summary() {
        let readiness = PullRequestReadiness {
            review_decision: Some("REVIEW_REQUIRED".to_owned()),
            latest_reviews: vec![PullRequestReviewEntry {
                author: "alice".to_owned(),
                state: "COMMENTED".to_owned(),
                body: Some("Please explain the rollback path.".to_owned()),
                submitted_at: Some("2026-06-20T12:00:00Z".to_owned()),
            }],
            comments: vec![PullRequestCommentEntry {
                author: "bob".to_owned(),
                body: "Check the preview deployment.".to_owned(),
                created_at: Some("2026-06-20T12:10:00Z".to_owned()),
            }],
            review_threads: vec![PullRequestReviewThread {
                id: Some("PRRT_kwDOexample1".to_owned()),
                path: Some("src/lib.rs".to_owned()),
                line: Some(42),
                resolved: false,
                comments: vec![PullRequestThreadComment {
                    author: "carol".to_owned(),
                    body: "Threaded comment still needs a fix.".to_owned(),
                    url: Some("https://github.com/example/demo/pull/7#discussion_r1".to_owned()),
                    created_at: Some("2026-06-20T12:15:00Z".to_owned()),
                }],
            }],
            checks: vec![PullRequestCheckRun {
                name: "build".to_owned(),
                status: "FAILURE".to_owned(),
                detail: Some("https://github.com/example/demo/actions/runs/3".to_owned()),
            }],
            deployments: vec![PullRequestDeployment {
                environment: "Preview".to_owned(),
                status: "FAILURE".to_owned(),
                url: Some("https://preview.example.com".to_owned()),
            }],
        };

        let prompt = format_pull_request_readiness_agent_prompt("berlin", &readiness);

        assert!(prompt.contains("Address this PR readiness state for workspace berlin."));
        assert!(prompt.contains("build: FAILURE"));
        assert!(prompt.contains("Preview: FAILURE"));
        assert!(prompt.contains("Please explain the rollback path."));
        assert!(prompt.contains("Check the preview deployment."));
        assert!(prompt.contains("src/lib.rs:42"));
        assert!(prompt.contains("unresolved"));
        assert!(prompt.contains("PRRT_kwDOexample1"));
        assert!(prompt.contains("Threaded comment still needs a fix."));
    }

    #[test]
    fn pull_request_readiness_summary_promotes_blockers_and_pending_gates() {
        let readiness = PullRequestReadiness {
            review_decision: Some("CHANGES_REQUESTED".to_owned()),
            latest_reviews: Vec::new(),
            comments: Vec::new(),
            review_threads: vec![
                PullRequestReviewThread {
                    id: Some("PRRT_open".to_owned()),
                    path: Some("src/lib.rs".to_owned()),
                    line: Some(42),
                    resolved: false,
                    comments: Vec::new(),
                },
                PullRequestReviewThread {
                    id: Some("PRRT_done".to_owned()),
                    path: Some("README.md".to_owned()),
                    line: Some(5),
                    resolved: true,
                    comments: Vec::new(),
                },
            ],
            checks: vec![
                PullRequestCheckRun {
                    name: "unit".to_owned(),
                    status: "FAILURE".to_owned(),
                    detail: Some("https://github.com/example/demo/actions/runs/1".to_owned()),
                },
                PullRequestCheckRun {
                    name: "e2e".to_owned(),
                    status: "IN_PROGRESS".to_owned(),
                    detail: None,
                },
                PullRequestCheckRun {
                    name: "lint".to_owned(),
                    status: "SUCCESS".to_owned(),
                    detail: None,
                },
            ],
            deployments: vec![
                PullRequestDeployment {
                    environment: "Preview".to_owned(),
                    status: "FAILURE".to_owned(),
                    url: Some("https://preview.example.com".to_owned()),
                },
                PullRequestDeployment {
                    environment: "Docs".to_owned(),
                    status: "PENDING".to_owned(),
                    url: None,
                },
            ],
        };

        let text = format_pull_request_readiness("berlin", &readiness);

        assert!(text.contains("Attention needed:"));
        assert!(text.contains("- Review decision: CHANGES_REQUESTED"));
        assert!(text.contains("- Unresolved review thread PRRT_open at src/lib.rs:42"));
        assert!(text.contains(
            "- Failing check unit: FAILURE - https://github.com/example/demo/actions/runs/1"
        ));
        assert!(text.contains("- Pending check e2e: IN_PROGRESS"));
        assert!(
            text.contains("- Failing deployment Preview: FAILURE - https://preview.example.com")
        );
        assert!(text.contains("- Pending deployment Docs: PENDING"));
        assert!(!text.contains("PRRT_done at README.md:5"));
        assert!(!text.contains("lint: SUCCESS -"));
    }

    #[test]
    fn pull_request_readiness_summary_includes_compact_rollup_counts() {
        let readiness = PullRequestReadiness {
            review_decision: Some("APPROVED".to_owned()),
            latest_reviews: vec![
                PullRequestReviewEntry {
                    author: "alice".to_owned(),
                    state: "APPROVED".to_owned(),
                    body: None,
                    submitted_at: None,
                },
                PullRequestReviewEntry {
                    author: "bob".to_owned(),
                    state: "COMMENTED".to_owned(),
                    body: None,
                    submitted_at: None,
                },
            ],
            comments: vec![PullRequestCommentEntry {
                author: "carol".to_owned(),
                body: "Please check this.".to_owned(),
                created_at: None,
            }],
            review_threads: vec![
                PullRequestReviewThread {
                    id: Some("PRRT_open".to_owned()),
                    path: Some("src/lib.rs".to_owned()),
                    line: Some(7),
                    resolved: false,
                    comments: Vec::new(),
                },
                PullRequestReviewThread {
                    id: Some("PRRT_done".to_owned()),
                    path: Some("README.md".to_owned()),
                    line: Some(1),
                    resolved: true,
                    comments: Vec::new(),
                },
            ],
            checks: vec![
                PullRequestCheckRun {
                    name: "unit".to_owned(),
                    status: "SUCCESS".to_owned(),
                    detail: None,
                },
                PullRequestCheckRun {
                    name: "lint".to_owned(),
                    status: "FAILURE".to_owned(),
                    detail: None,
                },
                PullRequestCheckRun {
                    name: "e2e".to_owned(),
                    status: "IN_PROGRESS".to_owned(),
                    detail: None,
                },
            ],
            deployments: vec![
                PullRequestDeployment {
                    environment: "Preview".to_owned(),
                    status: "SUCCESS".to_owned(),
                    url: None,
                },
                PullRequestDeployment {
                    environment: "Docs".to_owned(),
                    status: "PENDING".to_owned(),
                    url: None,
                },
            ],
        };

        let text = format_pull_request_readiness("berlin", &readiness);

        assert!(text.contains("Rollup:"));
        assert!(text.contains("- Reviews: 2 latest, 1 top-level comment"));
        assert!(text.contains("- Review threads: 1 unresolved / 2 total"));
        assert!(text.contains("- Checks: 1 passing, 1 failing, 1 pending, 0 other"));
        assert!(text.contains("- Deployments: 1 passing, 0 failing, 1 pending, 0 other"));
    }

    #[test]
    fn github_pr_state_uses_recorded_pr_number_when_workspace_branch_differs() {
        let _guard = env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        let db_path = temp.path().join("state.db");
        let old_path = install_fake_gh(
            temp.path(),
            r#"#!/bin/sh
if [ "$1" = "pr" ] && [ "$2" = "checks" ] && [ "$3" = "42" ]; then
  printf 'unit\tpass\t1m\thttps://github.com/example/demo/actions/1\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "view" ] && [ "$3" = "42" ] && [ "$4" = "--comments" ]; then
  printf 'alice: looks good\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "view" ] && [ "$3" = "42" ] && [ "$4" = "--json" ] && [ "$5" = "state" ]; then
  printf '{"state":"merged"}\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "view" ] && [ "$3" = "42" ] && [ "$4" = "--json" ]; then
  printf '{"id":"PR_fake","reviewDecision":"APPROVED","latestReviews":[],"comments":[],"statusCheckRollup":[]}\n'
  exit 0
fi
if [ "$1" = "api" ] && [ "$2" = "graphql" ]; then
  printf '{"data":{"node":{"reviewThreads":{"nodes":[]}}}}\n'
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
        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/local-copy-of-pr".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        store
            .record_pull_request(workspace.id, "https://github.com/example/demo/pull/42")
            .unwrap();

        assert!(store
            .pull_request_checks("berlin")
            .unwrap()
            .contains("unit"));
        assert!(store
            .pull_request_review_state("berlin")
            .unwrap()
            .contains("alice"));
        assert!(store
            .pull_request_readiness_text("berlin")
            .unwrap()
            .contains("Review decision: APPROVED"));
        assert_eq!(
            store
                .refresh_pull_request_state("berlin")
                .unwrap()
                .unwrap()
                .state,
            "merged"
        );

        restore_path(old_path);
    }

    #[test]
    fn pull_request_panel_state_collects_pr_readiness_and_review_text() {
        let _guard = env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        let db_path = temp.path().join("state.db");
        let old_path = install_fake_gh(
            temp.path(),
            r#"#!/bin/sh
if [ "$1" = "pr" ] && [ "$2" = "view" ] && [ "$3" = "42" ] && [ "$4" = "--json" ]; then
  printf '{"id":"PR_fake","reviewDecision":"APPROVED","latestReviews":[],"comments":[],"statusCheckRollup":[]}\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "view" ] && [ "$3" = "42" ] && [ "$4" = "--comments" ]; then
  printf 'alice: looks good\n'
  exit 0
fi
if [ "$1" = "api" ] && [ "$2" = "graphql" ]; then
  printf '{"data":{"node":{"reviewThreads":{"nodes":[]}}}}\n'
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

        let panel = store.pull_request_panel_state("berlin").unwrap();

        assert_eq!(panel.pull_request.unwrap().number, 42);
        assert_eq!(
            panel
                .readiness
                .as_ref()
                .and_then(|state| state.review_decision.as_deref()),
            Some("APPROVED")
        );
        assert!(panel
            .readiness_text
            .contains("PR readiness for workspace berlin."));
        assert_eq!(panel.review_text.as_deref(), Some("alice: looks good\n"));

        restore_path(old_path);
    }

    #[test]
    fn github_source_choices_parse_number_title_and_state() {
        let raw = "\
12\tOPEN\tFix auth loop\n\
18\tDRAFT\tShip checks ui\n";

        let choices = parse_github_numbered_stateful_choices(raw);

        assert_eq!(choices.len(), 2);
        assert_eq!(choices[0].number, 12);
        assert_eq!(choices[0].state, "OPEN");
        assert_eq!(choices[0].title, "Fix auth loop");
        assert_eq!(choices[1].number, 18);
        assert_eq!(choices[1].state, "DRAFT");
        assert_eq!(choices[1].title, "Ship checks ui");
    }

    #[test]
    fn pull_request_readiness_fetches_head_deployments_from_github_api() {
        let _guard = env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        let db_path = temp.path().join("state.db");
        let old_path = install_fake_gh(
            temp.path(),
            r#"#!/bin/sh
if [ "$1" = "pr" ] && [ "$2" = "view" ] && [ "$3" = "42" ] && [ "$4" = "--json" ]; then
  printf '{"id":"PR_fake","headRefOid":"abc123","reviewDecision":"APPROVED","latestReviews":[],"comments":[],"statusCheckRollup":[]}\n'
  exit 0
fi
if [ "$1" = "api" ] && [ "$2" = "graphql" ]; then
  printf '{"data":{"node":{"reviewThreads":{"nodes":[]}}}}\n'
  exit 0
fi
if [ "$1" = "api" ] && [ "$2" = "repos/{owner}/{repo}/commits/abc123/status" ]; then
  printf '{"statuses":[]}\n'
  exit 0
fi
if [ "$1" = "api" ] && [ "$2" = "repos/{owner}/{repo}/deployments?sha=abc123" ]; then
  printf '[{"id":99,"environment":"Preview"}]\n'
  exit 0
fi
if [ "$1" = "api" ] && [ "$2" = "repos/{owner}/{repo}/deployments/99/statuses" ]; then
  printf '[{"state":"success","environment_url":"https://preview.example.test"}]\n'
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
        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/local-copy-of-pr".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        store
            .record_pull_request(workspace.id, "https://github.com/example/demo/pull/42")
            .unwrap();

        let text = store.pull_request_readiness_text("berlin").unwrap();

        assert!(text.contains("Preview: success - https://preview.example.test"));
        assert!(text.contains("- Deployments: 1 passing, 0 failing, 0 pending, 0 other"));

        restore_path(old_path);
    }

    #[test]
    fn pull_request_readiness_fetches_head_statuses_from_github_api() {
        let _guard = env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        let db_path = temp.path().join("state.db");
        let old_path = install_fake_gh(
            temp.path(),
            r#"#!/bin/sh
if [ "$1" = "pr" ] && [ "$2" = "view" ] && [ "$3" = "42" ] && [ "$4" = "--json" ]; then
  printf '{"id":"PR_fake","headRefOid":"abc123","reviewDecision":"APPROVED","latestReviews":[],"comments":[],"statusCheckRollup":[]}\n'
  exit 0
fi
if [ "$1" = "api" ] && [ "$2" = "graphql" ]; then
  printf '{"data":{"node":{"reviewThreads":{"nodes":[]}}}}\n'
  exit 0
fi
if [ "$1" = "api" ] && [ "$2" = "repos/{owner}/{repo}/commits/abc123/status" ]; then
  printf '{"statuses":[{"context":"matrix-success","state":"success","target_url":"https://example.test/success"},{"context":"matrix-failure","state":"failure","target_url":"https://example.test/failure"},{"context":"matrix-pending","state":"pending","target_url":"https://example.test/pending"}]}\n'
  exit 0
fi
if [ "$1" = "api" ] && [ "$2" = "repos/{owner}/{repo}/deployments?sha=abc123" ]; then
  printf '[]\n'
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
        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/local-copy-of-pr".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        store
            .record_pull_request(workspace.id, "https://github.com/example/demo/pull/42")
            .unwrap();

        let text = store.pull_request_readiness_text("berlin").unwrap();

        assert!(text.contains("matrix-success: success - https://example.test/success"));
        assert!(text.contains("matrix-failure: failure - https://example.test/failure"));
        assert!(text.contains("matrix-pending: pending - https://example.test/pending"));
        assert!(text.contains("- Checks: 1 passing, 1 failing, 1 pending, 0 other"));

        restore_path(old_path);
    }

    #[test]
    fn pull_request_readiness_deduplicates_rollup_and_head_status_checks() {
        let _guard = env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        let db_path = temp.path().join("state.db");
        let old_path = install_fake_gh(
            temp.path(),
            r#"#!/bin/sh
if [ "$1" = "pr" ] && [ "$2" = "view" ] && [ "$3" = "42" ] && [ "$4" = "--json" ]; then
  printf '{"id":"PR_fake","headRefOid":"abc123","reviewDecision":"APPROVED","latestReviews":[],"comments":[],"statusCheckRollup":[{"__typename":"StatusContext","context":"unit","state":"SUCCESS","targetUrl":"https://example.test/unit"}]}\n'
  exit 0
fi
if [ "$1" = "api" ] && [ "$2" = "graphql" ]; then
  printf '{"data":{"node":{"reviewThreads":{"nodes":[]}}}}\n'
  exit 0
fi
if [ "$1" = "api" ] && [ "$2" = "repos/{owner}/{repo}/commits/abc123/status" ]; then
  printf '{"statuses":[{"context":"unit","state":"success","target_url":"https://example.test/unit"}]}\n'
  exit 0
fi
if [ "$1" = "api" ] && [ "$2" = "repos/{owner}/{repo}/deployments?sha=abc123" ]; then
  printf '[]\n'
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
        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/local-copy-of-pr".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        store
            .record_pull_request(workspace.id, "https://github.com/example/demo/pull/42")
            .unwrap();

        let readiness = store.pull_request_readiness("berlin").unwrap();

        assert_eq!(
            readiness.checks,
            vec![PullRequestCheckRun {
                name: "unit".to_owned(),
                status: "SUCCESS".to_owned(),
                detail: Some("https://example.test/unit".to_owned()),
            }]
        );

        restore_path(old_path);
    }

    #[test]
    fn github_review_threads_can_be_resolved_and_reopened() {
        let _guard = env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        let db_path = temp.path().join("state.db");
        let old_path = install_fake_gh(
            temp.path(),
            r#"#!/bin/sh
if [ "$1" = "api" ] && [ "$2" = "graphql" ]; then
  case "$*" in
    *unresolveReviewThread*threadId=PRRT_fake*)
      printf '{"data":{"unresolveReviewThread":{"thread":{"id":"PRRT_fake","isResolved":false}}}}\n'
      exit 0
      ;;
    *resolveReviewThread*threadId=PRRT_fake*)
      printf '{"data":{"resolveReviewThread":{"thread":{"id":"PRRT_fake","isResolved":true}}}}\n'
      exit 0
      ;;
  esac
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
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        let resolved = store
            .set_pull_request_review_thread_resolution("berlin", "PRRT_fake", true)
            .unwrap();
        assert!(resolved.resolved);
        assert_eq!(resolved.id.as_deref(), Some("PRRT_fake"));

        let reopened = store
            .set_pull_request_review_thread_resolution("berlin", "PRRT_fake", false)
            .unwrap();
        assert!(!reopened.resolved);
        assert_eq!(reopened.id.as_deref(), Some("PRRT_fake"));

        restore_path(old_path);
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

        let err = store
            .merge_pull_request("berlin", Some("squash"))
            .unwrap_err();

        assert!(err.to_string().contains("open review comment(s) remain"));
    }

    #[test]
    fn merge_pull_request_honors_disabled_todo_and_comment_blockers() {
        let _guard = env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
[customization.merge_rules]
block_on_open_todos = false
block_on_open_comments = false
"#,
        )
        .unwrap();
        let old_path = install_fake_gh(
            temp.path(),
            r#"#!/bin/sh
if [ "$1" = "pr" ] && [ "$2" = "merge" ]; then
  printf 'merged despite local review state\n'
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
        store.add_todo("berlin", "ship anyway").unwrap();
        store
            .add_review_comment("berlin", "src/lib.rs", Some(12), "known follow-up")
            .unwrap();

        let output = store.merge_pull_request("berlin", Some("squash")).unwrap();

        restore_path(old_path);
        assert_eq!(output.trim(), "merged despite local review state");
    }

    #[test]
    fn merge_pull_request_blocks_failed_checks_when_configured() {
        let _guard = env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
[customization.merge_rules]
block_on_failed_checks = true
"#,
        )
        .unwrap();
        let old_path = install_fake_gh(
            temp.path(),
            r#"#!/bin/sh
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  printf '{"reviewDecision":"APPROVED","latestReviews":[],"comments":[],"statusCheckRollup":[{"name":"unit","conclusion":"FAILURE","detailsUrl":"https://example.test/unit"}]}\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "merge" ]; then
  printf 'merge should not run\n' >&2
  exit 1
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

        let err = store
            .merge_pull_request("berlin", Some("squash"))
            .unwrap_err();

        restore_path(old_path);
        assert!(err.to_string().contains("1 failing check(s) remain"));
        assert!(err.to_string().contains("unit"));
    }

    #[test]
    fn merge_pull_request_blocks_pending_checks_when_configured() {
        let _guard = env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
[customization.merge_rules]
block_on_pending_checks = true
"#,
        )
        .unwrap();
        let old_path = install_fake_gh(
            temp.path(),
            r#"#!/bin/sh
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  printf '{"reviewDecision":"APPROVED","latestReviews":[],"comments":[],"statusCheckRollup":[{"name":"e2e","status":"IN_PROGRESS","detailsUrl":"https://example.test/e2e"}]}\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "merge" ]; then
  printf 'merge should not run\n' >&2
  exit 1
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

        let err = store
            .merge_pull_request("berlin", Some("squash"))
            .unwrap_err();

        restore_path(old_path);
        assert!(err.to_string().contains("1 pending check(s) remain"));
        assert!(err.to_string().contains("e2e"));
    }

    #[test]
    fn merge_pull_request_uses_configured_default_merge_method() {
        let _guard = env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
[customization.naming]
default_merge_method = "rebase"
"#,
        )
        .unwrap();
        let old_path = install_fake_gh(
            temp.path(),
            r#"#!/bin/sh
if [ "$1" = "pr" ] && [ "$2" = "merge" ] && [ "$4" = "--rebase" ]; then
  printf 'merged with rebase\n'
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

        let output = store.merge_pull_request("berlin", None).unwrap();

        restore_path(old_path);
        assert_eq!(output.trim(), "merged with rebase");
    }

    #[test]
    fn merge_and_maybe_archive_archives_when_repository_setting_is_enabled() {
        let _guard = env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
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
            .merge_and_maybe_archive_pull_request("berlin", Some("squash"))
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
        let events = store
            .workspace_timeline("berlin", Some("file.reverted"))
            .unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].summary.contains("README.md"));
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
    fn stage_and_unstage_workspace_file_update_the_index() {
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
        store.stage_workspace_file("berlin", "README.md").unwrap();
        let staged_diff = store
            .staged_diff("berlin", Some(Path::new("README.md")))
            .unwrap();
        assert!(staged_diff.contains("+changed"));
        assert_eq!(
            git_output(&workspace.path, ["diff", "--cached", "--name-only"]).trim(),
            "README.md"
        );

        store.unstage_workspace_file("berlin", "README.md").unwrap();
        assert!(store
            .staged_diff("berlin", Some(Path::new("README.md")))
            .unwrap()
            .trim()
            .is_empty());
        assert!(
            git_output(&workspace.path, ["diff", "--cached", "--name-only"])
                .trim()
                .is_empty()
        );
        assert!(store
            .changed_files("berlin")
            .unwrap()
            .contains(&"README.md".to_owned()));

        fs::write(workspace.path.join("notes.txt"), "new\n").unwrap();
        store.stage_all_workspace_files("berlin").unwrap();
        let staged_files = git_output(&workspace.path, ["diff", "--cached", "--name-only"]);
        assert!(staged_files.contains("README.md"));
        assert!(staged_files.contains("notes.txt"));
        assert_eq!(
            store
                .workspace_timeline("berlin", Some("git.staged_all"))
                .unwrap()
                .len(),
            1
        );

        store.unstage_all_workspace_files("berlin").unwrap();
        assert!(
            git_output(&workspace.path, ["diff", "--cached", "--name-only"])
                .trim()
                .is_empty()
        );
        assert_eq!(
            store
                .workspace_timeline("berlin", Some("git.unstaged_all"))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn stage_and_unstage_workspace_hunk_updates_the_index() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        let db_path = temp.path().join("state.db");
        let readme = repo_path.join("README.md");
        fs::write(
            &readme,
            "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\neleven\ntwelve\n",
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", "README.md"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "expand readme",
            ])
            .status()
            .unwrap();

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
        fs::write(
            workspace.path.join("README.md"),
            "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\neleven\nTWELVE\n",
        )
        .unwrap();

        let hunks = store.diff_hunks("berlin", "README.md", false).unwrap();
        assert_eq!(hunks.len(), 2);
        store
            .stage_workspace_hunk("berlin", "README.md", 0)
            .unwrap();
        let staged = store
            .staged_diff("berlin", Some(Path::new("README.md")))
            .unwrap();
        let unstaged = store
            .unified_diff("berlin", Some(Path::new("README.md")))
            .unwrap();
        assert!(staged.contains("+ONE"));
        assert!(!staged.contains("+TWELVE"));
        assert!(unstaged.contains("+TWELVE"));

        let staged_hunks = store.diff_hunks("berlin", "README.md", true).unwrap();
        assert_eq!(staged_hunks.len(), 1);
        store
            .unstage_workspace_hunk("berlin", "README.md", 0)
            .unwrap();
        assert!(store
            .staged_diff("berlin", Some(Path::new("README.md")))
            .unwrap()
            .trim()
            .is_empty());
    }

    #[test]
    fn commit_workspace_changes_commits_staged_files() {
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

        git_dynamic(
            &workspace.path,
            &["config", "user.name", "Linux Archductor"],
        )
        .unwrap();
        git_dynamic(
            &workspace.path,
            &["config", "user.email", "linux-archductor@example.test"],
        )
        .unwrap();
        git_dynamic(&workspace.path, &["config", "commit.gpgsign", "false"]).unwrap();
        fs::write(workspace.path.join("README.md"), "demo\ncommitted\n").unwrap();
        store.stage_workspace_file("berlin", "README.md").unwrap();

        let draft = store.commit_message_draft("berlin").unwrap();
        assert_eq!(draft, "chore: update README.md");
        let generated = store
            .generated_commit_message_from_staged_diff("berlin")
            .unwrap();
        assert_eq!(generated, "chore: update README.md");
        let output = store
            .commit_workspace_changes("berlin", "fix: update readme")
            .unwrap();

        assert!(output.contains("fix: update readme"));
        assert!(!store
            .changed_files("berlin")
            .unwrap()
            .contains(&"README.md".to_owned()));
        assert_eq!(
            git_output(&workspace.path, ["log", "-1", "--pretty=%s"]).trim(),
            "fix: update readme"
        );
        assert!(store
            .workspace_timeline("berlin", Some("commit.created"))
            .unwrap()
            .iter()
            .any(|event| event.summary.contains("fix: update readme")));
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
    fn render_pull_request_template_uses_workspace_variables_and_default_sections() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
[customization.naming]
pr_title_template = "{workspace}: {branch} ({changed_files_count})"
pr_body_sections = ["Summary", "Tests", "Risk"]
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "add pr template settings",
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
        fs::write(workspace.path.join("README.md"), "demo\nchanged\n").unwrap();

        let template = store.render_pull_request_template("berlin").unwrap();

        assert_eq!(template.title, "berlin: lc/berlin (1)");
        assert!(template.body.contains("## Summary"));
        assert!(template.body.contains("## Tests"));
        assert!(template.body.contains("## Risk"));
        assert!(template.body.contains("- README.md"));
        assert!(template
            .body
            .contains("No saved agent session summary yet."));
    }

    #[test]
    fn create_pull_request_returns_existing_remote_pr_instead_of_duplicate() {
        let _guard = env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        let db_path = temp.path().join("state.db");
        let old_path = install_fake_gh(
            temp.path(),
            r#"#!/bin/sh
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  printf '[{"url":"https://github.com/example/demo/pull/42"}]\n'
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
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();

        let output = store
            .create_pull_request("berlin", Some("Title"), Some("Body"), true)
            .unwrap();

        restore_path(old_path);
        assert_eq!(
            output.trim(),
            "Existing PR: https://github.com/example/demo/pull/42"
        );
        assert_eq!(store.pull_request("berlin").unwrap().unwrap().number, 42);
    }

    #[test]
    fn create_pull_request_ignores_closed_cached_pr_before_remote_lookup() {
        let _guard = env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        let db_path = temp.path().join("state.db");
        let old_path = install_fake_gh(
            temp.path(),
            r#"#!/bin/sh
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  printf '[{"url":"https://github.com/example/demo/pull/42"}]\n'
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
        let workspace = store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "berlin".to_owned(),
                branch: "lc/berlin".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        store
            .record_pull_request(workspace.id, "https://github.com/example/demo/pull/41")
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE pull_requests SET state = 'closed' WHERE workspace_id = ?1",
                [workspace.id],
            )
            .unwrap();

        let output = store
            .create_pull_request("berlin", Some("Title"), Some("Body"), true)
            .unwrap();

        restore_path(old_path);
        assert_eq!(
            output.trim(),
            "Existing PR: https://github.com/example/demo/pull/42"
        );
        let pull_request = store.pull_request("berlin").unwrap().unwrap();
        assert_eq!(pull_request.number, 42);
        assert_eq!(pull_request.state, "open");
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
    fn sync_todos_from_context_updates_existing_context_todo_status() {
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
        let todos_path = workspace.path.join(".context/todos.md");

        fs::write(&todos_path, "- [ ] finish parser\n").unwrap();
        assert_eq!(store.sync_todos_from_context("berlin").unwrap(), 1);
        assert_eq!(store.checks_summary("berlin").unwrap().open_todos, 1);

        fs::write(&todos_path, "- [x] finish parser\n").unwrap();
        assert_eq!(store.sync_todos_from_context("berlin").unwrap(), 0);

        let todos = store.list_todos("berlin").unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].status, "done");
        assert_eq!(store.checks_summary("berlin").unwrap().open_todos, 0);
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
    fn rename_updates_name_without_moving_worktree() {
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

        let old_path = workspace.path.clone();
        let renamed = store.rename("berlin", "oslo").unwrap();

        assert_eq!(renamed.name, "oslo");
        assert_eq!(renamed.path, old_path);
        assert!(old_path.exists());
        assert!(renamed.path.join(".context").is_dir());
        let branch = git_output(&renamed.path, ["branch", "--show-current"]);
        assert_eq!(branch.trim(), "lc/berlin");
        let worktrees = Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["worktree", "list", "--porcelain"])
            .output()
            .unwrap();
        let worktrees = String::from_utf8_lossy(&worktrees.stdout);
        assert!(worktrees.contains(renamed.path.to_str().unwrap()));

        // Should appear under new name in list
        let list = store.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "oslo");
    }

    #[test]
    fn spotlight_rename_updates_watch_and_sync_works_with_stale_session_name() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        fs::create_dir(repo_path.join(".archductor")).unwrap();
        fs::write(
            repo_path.join(".archductor/settings.toml"),
            r#"
spotlight_testing = true
"#,
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args(["add", ".archductor/settings.toml"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repo_path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
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
        assert!(cp.git_ref.starts_with("refs/linux-archductor/checkpoints/"));
        assert!(cp.session_id.is_none());

        let list = store.checkpoint_list("berlin").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, cp.id);
    }

    #[test]
    fn checkpoint_create_snapshots_dirty_worktree_and_untracked_files() {
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

        fs::write(workspace.path.join("README.md"), "dirty readme\n").unwrap();
        fs::write(workspace.path.join("new.txt"), "new file\n").unwrap();

        let cp = store
            .checkpoint_create("berlin", "dirty state", None)
            .unwrap();

        let readme = git_output_dynamic(
            &workspace.path,
            &["show", &format!("{}:README.md", cp.git_ref)],
        )
        .unwrap();
        let new_file = git_output_dynamic(
            &workspace.path,
            &["show", &format!("{}:new.txt", cp.git_ref)],
        )
        .unwrap();

        assert_eq!(readme, "dirty readme\n");
        assert_eq!(new_file, "new file\n");
    }

    #[test]
    fn latest_turn_checkpoint_diff_compares_current_worktree_to_turn_start() {
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

        fs::write(workspace.path.join("README.md"), "turn start\n").unwrap();
        store
            .checkpoint_create_turn_start("berlin", 42, None, "user")
            .unwrap();
        fs::write(workspace.path.join("README.md"), "turn end\n").unwrap();
        fs::write(workspace.path.join("created.txt"), "created during turn\n").unwrap();

        let turn = store
            .latest_turn_checkpoint_diff("berlin")
            .unwrap()
            .unwrap();

        assert!(turn
            .checkpoint
            .message
            .starts_with("Turn start: thread #42"));
        assert!(turn.end_checkpoint.is_none());
        assert!(turn.diff.contains("-turn start"));
        assert!(turn.diff.contains("+turn end"));
        assert!(turn.diff.contains("created.txt"));
        assert!(turn.diff.contains("+created during turn"));
    }

    #[test]
    fn turn_checkpoint_diffs_include_recent_turns_with_hard_limit() {
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

        fs::write(workspace.path.join("README.md"), "turn one start\n").unwrap();
        let first = store
            .checkpoint_create_turn_start("berlin", 1, None, "user")
            .unwrap();
        fs::write(workspace.path.join("README.md"), "turn one end\n").unwrap();
        let second = store
            .checkpoint_create_turn_start("berlin", 1, None, "user")
            .unwrap();
        fs::write(workspace.path.join("README.md"), "turn two end\n").unwrap();

        let diffs = store.turn_checkpoint_diffs("berlin", 25).unwrap();

        assert_eq!(diffs.len(), 2);
        assert_eq!(diffs[0].checkpoint.id, second.id);
        assert!(diffs[0].end_checkpoint.is_none());
        assert!(diffs[0].diff.contains("-turn one end"));
        assert!(diffs[0].diff.contains("+turn two end"));
        assert_eq!(diffs[1].checkpoint.id, first.id);
        assert_eq!(
            diffs[1]
                .end_checkpoint
                .as_ref()
                .map(|checkpoint| checkpoint.id),
            Some(second.id)
        );
        assert!(diffs[1].diff.contains("-turn one start"));
        assert!(diffs[1].diff.contains("+turn one end"));

        let limited = store.turn_checkpoint_diffs("berlin", 1).unwrap();
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].checkpoint.id, second.id);
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
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
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
    fn chat_thread_crud_persists_multiple_threads_per_workspace_and_provider() {
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
            .create_chat_thread("berlin", "codex", "Bugfix A", None)
            .unwrap();
        let second = store
            .create_chat_thread("berlin", "codex", "Bugfix B", None)
            .unwrap();
        let third = store
            .create_chat_thread("berlin", "claude", "Review", None)
            .unwrap();

        let threads = store.list_chat_threads("berlin").unwrap();
        assert_eq!(threads.len(), 3);
        assert_eq!(threads[0].id, third.id);
        assert_eq!(threads[1].id, second.id);
        assert_eq!(threads[2].id, first.id);
    }

    #[test]
    fn chat_messages_persist_user_control_and_agent_rows() {
        let (_temp, store) = test_workspace_store();
        let thread = store
            .create_chat_thread("berlin", "codex", "Bugfix A", None)
            .unwrap();

        store
            .append_chat_message(thread.id, "user", "run tests", "user_send")
            .unwrap();
        store
            .append_chat_message(thread.id, "system", "/model gpt-5", "control_command")
            .unwrap();
        store
            .append_chat_message(thread.id, "agent", "Running now.", "agent_screen_parse")
            .unwrap();

        let messages = store.list_chat_messages(thread.id).unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].source, "control_command");
        assert_eq!(messages[2].role, "agent");
    }

    #[test]
    fn chat_messages_skip_exact_adjacent_duplicates_from_same_source() {
        let (_temp, store) = test_workspace_store();
        let thread = store
            .create_chat_thread("berlin", "codex", "Bugfix A", None)
            .unwrap();

        let first = store
            .append_chat_message(thread.id, "user", "run tests", "user_send")
            .unwrap();
        let second = store
            .append_chat_message(thread.id, "user", "run tests", "user_send")
            .unwrap();
        store
            .append_chat_message(thread.id, "agent", "Running.", "agent_screen_parse")
            .unwrap();
        store
            .append_chat_message(thread.id, "user", "run tests", "user_send")
            .unwrap();

        let messages = store.list_chat_messages(thread.id).unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].content, "run tests");
        assert_eq!(messages[1].content, "Running.");
        assert_eq!(messages[2].content, "run tests");
    }

    #[test]
    fn chat_messages_merge_scrolled_agent_repaint_fragments() {
        let (_temp, store) = test_workspace_store();
        let thread = store
            .create_chat_thread("berlin", "codex", "Bugfix A", None)
            .unwrap();

        store
            .append_chat_message(
                thread.id,
                "agent",
                "Repo Status\n\nThis is Linux Archductor.\n\nCurrent Git State",
                "agent_screen_parse",
            )
            .unwrap();
        store
            .append_chat_message(
                thread.id,
                "agent",
                "This is Linux Archductor.\n\nCurrent Git State\n\nYou’re in worktree valencia.",
                "agent_screen_parse",
            )
            .unwrap();
        store
            .append_chat_message(
                thread.id,
                "agent",
                "Current Git State\n\nYou’re in worktree valencia.",
                "agent_screen_parse",
            )
            .unwrap();

        let messages = store.list_chat_messages(thread.id).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].content,
            "Repo Status\n\nThis is Linux Archductor.\n\nCurrent Git State\n\nYou’re in worktree valencia."
        );
    }

    #[test]
    fn chat_messages_only_dedupe_against_the_latest_shared_timeline_item() {
        let (_temp, store) = test_workspace_store();
        let thread = store
            .create_chat_thread("berlin", "codex", "Bugfix A", None)
            .unwrap();
        let process = process_record_for_thread(&store, thread.id);

        let first_message = store
            .append_chat_message(thread.id, "user", "run tests", "user_send")
            .unwrap();
        store
            .append_chat_event(
                thread.id,
                process.id,
                &CodexTranscriptEvent::Tool {
                    title: "cargo test".to_owned(),
                    body: "ok".to_owned(),
                },
            )
            .unwrap();
        let second_message = store
            .append_chat_message(thread.id, "user", "run tests", "user_send")
            .unwrap();

        let messages = store.list_chat_messages(thread.id).unwrap();
        assert_ne!(first_message.id, second_message.id);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "run tests");
        assert_eq!(messages[1].content, "run tests");
    }

    #[test]
    fn chat_messages_backfill_null_timeline_seq_before_event_dedupes() {
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
        let thread = store
            .create_chat_thread("berlin", "codex", "Bugfix A", None)
            .unwrap();
        store
            .append_chat_message(thread.id, "user", "run tests", "user_send")
            .unwrap();
        store
            .append_chat_message(thread.id, "agent", "running tests", "agent_screen_parse")
            .unwrap();
        store
            .append_chat_message(thread.id, "user", "run tests", "user_send")
            .unwrap();

        store
            .conn
            .execute("UPDATE chat_messages SET timeline_seq = NULL", [])
            .unwrap();
        drop(store);

        let store = WorkspaceStore::open_with_logs(&db_path, temp.path().join("logs")).unwrap();
        let messages = store.list_chat_messages(thread.id).unwrap();
        assert!(messages
            .iter()
            .all(|message| message.timeline_seq.is_some()));
        assert_eq!(messages[0].timeline_seq, Some(4));
        assert_eq!(messages[1].timeline_seq, Some(5));
        assert_eq!(messages[2].timeline_seq, Some(6));

        let process = process_record_for_thread(&store, thread.id);
        let event = store
            .append_chat_event(
                thread.id,
                process.id,
                &CodexTranscriptEvent::Tool {
                    title: "cargo test".to_owned(),
                    body: "ok".to_owned(),
                },
            )
            .unwrap();
        let repeated = store
            .append_chat_message(thread.id, "user", "run tests", "user_send")
            .unwrap();

        let messages = store.list_chat_messages(thread.id).unwrap();
        let events = store.list_chat_events(thread.id).unwrap();
        assert_eq!(event.timeline_seq, 7);
        assert_eq!(messages.len(), 4);
        assert_ne!(messages[2].id, repeated.id);
        assert_eq!(messages[3].content, "run tests");
        assert_eq!(messages[3].timeline_seq, Some(8));
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn codex_parser_cursor_and_events_persist_separately_from_messages() {
        let (_temp, store) = test_workspace_store();
        let thread = store
            .create_chat_thread("berlin", "codex", "Bugfix A", None)
            .unwrap();
        let process = process_record_for_thread(&store, thread.id);
        let cursor = CodexParseCursor {
            fingerprint: Some("12:300:last lines".to_owned()),
        };
        let event = CodexTranscriptEvent::Tool {
            title: "cargo test".to_owned(),
            body: "ok".to_owned(),
        };

        store.set_codex_parse_cursor(process.id, &cursor).unwrap();
        store
            .append_chat_event(thread.id, process.id, &event)
            .unwrap();

        assert_eq!(
            store.get_codex_parse_cursor(process.id).unwrap(),
            Some(cursor)
        );
        assert_eq!(store.list_chat_messages(thread.id).unwrap().len(), 0);
        let events = store.list_chat_events(thread.id).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "tool");
        assert_eq!(events[0].title, "cargo test");
    }

    #[test]
    fn chat_messages_and_events_share_a_monotonic_timeline_sequence() {
        let (_temp, store) = test_workspace_store();
        let thread = store
            .create_chat_thread("berlin", "codex", "Bugfix A", None)
            .unwrap();
        let process = process_record_for_thread(&store, thread.id);

        let first_message = store
            .append_chat_message(thread.id, "user", "run tests", "user_send")
            .unwrap();
        let event = store
            .append_chat_event(
                thread.id,
                process.id,
                &CodexTranscriptEvent::Tool {
                    title: "cargo test".to_owned(),
                    body: "ok".to_owned(),
                },
            )
            .unwrap();
        let second_message = store
            .append_chat_message(thread.id, "agent", "running now", "agent_screen_parse")
            .unwrap();

        assert_eq!(first_message.timeline_seq, Some(1));
        assert_eq!(event.timeline_seq, 2);
        assert_eq!(second_message.timeline_seq, Some(3));
    }

    #[test]
    fn chat_events_are_idempotent_without_allocating_new_timeline_sequence() {
        let (_temp, store) = test_workspace_store();
        let thread = store
            .create_chat_thread("berlin", "codex", "Bugfix A", None)
            .unwrap();
        let process = process_record_for_thread(&store, thread.id);
        let event = CodexTranscriptEvent::Tool {
            title: "cargo test".to_owned(),
            body: "ok".to_owned(),
        };

        let first = store
            .append_chat_event(thread.id, process.id, &event)
            .unwrap();
        let second = store
            .append_chat_event(thread.id, process.id, &event)
            .unwrap();
        let events = store.list_chat_events(thread.id).unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(first.timeline_seq, second.timeline_seq);
        assert_eq!(events.len(), 1);

        let message = store
            .append_chat_message(thread.id, "user", "run tests", "user_send")
            .unwrap();
        assert_eq!(message.timeline_seq, Some(2));
    }

    #[test]
    fn streaming_chat_event_updates_latest_matching_event_without_duplicate_chip() {
        let (_temp, store) = test_workspace_store();
        let thread = store
            .create_chat_thread("berlin", "codex", "Bugfix A", None)
            .unwrap();
        let process = process_record_for_thread(&store, thread.id);

        let first = store
            .append_chat_event(
                thread.id,
                process.id,
                &CodexTranscriptEvent::Tool {
                    title: "cargo test".to_owned(),
                    body: "running".to_owned(),
                },
            )
            .unwrap();
        let updated = store
            .append_chat_event(
                thread.id,
                process.id,
                &CodexTranscriptEvent::Tool {
                    title: "cargo test".to_owned(),
                    body: "running\nok".to_owned(),
                },
            )
            .unwrap();
        let events = store.list_chat_events(thread.id).unwrap();

        assert_eq!(first.id, updated.id);
        assert_eq!(updated.timeline_seq, 1);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].body, "running\nok");
        let message = store
            .append_chat_message(thread.id, "agent", "done", "agent_screen_parse")
            .unwrap();
        assert_eq!(message.timeline_seq, Some(2));
    }

    #[test]
    fn same_tool_title_after_later_timeline_item_creates_new_event() {
        let (_temp, store) = test_workspace_store();
        let thread = store
            .create_chat_thread("berlin", "codex", "Bugfix A", None)
            .unwrap();
        let process = process_record_for_thread(&store, thread.id);

        store
            .append_chat_event(
                thread.id,
                process.id,
                &CodexTranscriptEvent::Tool {
                    title: "cargo test".to_owned(),
                    body: "first".to_owned(),
                },
            )
            .unwrap();
        store
            .append_chat_message(thread.id, "agent", "between", "agent_screen_parse")
            .unwrap();
        let second = store
            .append_chat_event(
                thread.id,
                process.id,
                &CodexTranscriptEvent::Tool {
                    title: "cargo test".to_owned(),
                    body: "second".to_owned(),
                },
            )
            .unwrap();
        let events = store.list_chat_events(thread.id).unwrap();

        assert_eq!(events.len(), 2);
        assert_eq!(second.timeline_seq, 3);
    }

    #[test]
    fn exact_same_tool_event_after_later_timeline_item_creates_new_event() {
        let (_temp, store) = test_workspace_store();
        let thread = store
            .create_chat_thread("berlin", "codex", "Bugfix A", None)
            .unwrap();
        let process = process_record_for_thread(&store, thread.id);

        store
            .append_chat_event(
                thread.id,
                process.id,
                &CodexTranscriptEvent::Tool {
                    title: "cargo test".to_owned(),
                    body: "ok".to_owned(),
                },
            )
            .unwrap();
        store
            .append_chat_message(thread.id, "agent", "between", "agent_screen_parse")
            .unwrap();
        let second = store
            .append_chat_event(
                thread.id,
                process.id,
                &CodexTranscriptEvent::Tool {
                    title: "cargo test".to_owned(),
                    body: "ok".to_owned(),
                },
            )
            .unwrap();
        let events = store.list_chat_events(thread.id).unwrap();

        assert_eq!(events.len(), 2);
        assert_eq!(second.timeline_seq, 3);
    }

    #[test]
    fn adjacent_same_tool_title_with_distinct_body_creates_new_event() {
        let (_temp, store) = test_workspace_store();
        let thread = store
            .create_chat_thread("berlin", "codex", "Bugfix A", None)
            .unwrap();
        let process = process_record_for_thread(&store, thread.id);

        store
            .append_chat_event(
                thread.id,
                process.id,
                &CodexTranscriptEvent::Tool {
                    title: "cargo test".to_owned(),
                    body: "first".to_owned(),
                },
            )
            .unwrap();
        let second = store
            .append_chat_event(
                thread.id,
                process.id,
                &CodexTranscriptEvent::Tool {
                    title: "cargo test".to_owned(),
                    body: "second".to_owned(),
                },
            )
            .unwrap();
        let events = store.list_chat_events(thread.id).unwrap();

        assert_eq!(events.len(), 2);
        assert_eq!(second.timeline_seq, 2);
    }

    #[test]
    fn chat_events_round_trip_payload_json_for_file_changes() {
        let (_temp, store) = test_workspace_store();
        let thread = store
            .create_chat_thread("berlin", "codex", "Bugfix A", None)
            .unwrap();
        let process = process_record_for_thread(&store, thread.id);
        let event = CodexTranscriptEvent::FileChange(crate::codex_tui::CodexFileChange {
            action: CodexFileChangeAction::Edited,
            path: "src/lib.rs".to_owned(),
            additions: Some(2),
            deletions: Some(1),
            lines: vec![
                crate::codex_tui::CodexFileChangeLine {
                    kind: crate::codex_tui::CodexFileChangeLineKind::Context,
                    old_line: Some(10),
                    new_line: Some(10),
                    content: "fn keep() {}".to_owned(),
                },
                crate::codex_tui::CodexFileChangeLine {
                    kind: crate::codex_tui::CodexFileChangeLineKind::Added,
                    old_line: None,
                    new_line: Some(11),
                    content: "fn add() {}".to_owned(),
                },
            ],
        });

        let inserted = store
            .append_chat_event(thread.id, process.id, &event)
            .unwrap();
        let stored = store.list_chat_events(thread.id).unwrap();
        let expected_payload = json!({
            "type": "file_change",
            "action": "edited",
            "path": "src/lib.rs",
            "additions": 2,
            "deletions": 1,
            "lines": [
                {
                    "kind": "context",
                    "old_line": 10,
                    "new_line": 10,
                    "content": "fn keep() {}",
                },
                {
                    "kind": "added",
                    "old_line": null,
                    "new_line": 11,
                    "content": "fn add() {}",
                },
            ],
        });

        assert_eq!(inserted.payload_json, expected_payload.to_string());
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].payload_json, expected_payload.to_string());
        assert_eq!(
            serde_json::from_str::<Value>(&stored[0].payload_json).unwrap(),
            expected_payload
        );
    }

    #[test]
    fn chat_thread_title_updates_without_creating_extra_rows() {
        let (_temp, store) = test_workspace_store();
        let thread = store
            .create_chat_thread("berlin", "codex", "New Chat", None)
            .unwrap();

        store
            .update_chat_thread_title(thread.id, "Fix parser failure")
            .unwrap();

        let updated = store.get_chat_thread_record(thread.id).unwrap();
        assert_eq!(updated.title, "Fix parser failure");
        assert!(store.list_chat_messages(thread.id).unwrap().is_empty());
    }

    #[test]
    fn agent_metadata_directive_renames_workspace_branch_and_chat_without_persisting_marker() {
        let (_temp, store) = test_workspace_store();
        let workspace = store.get_by_name("berlin").unwrap();
        let thread = store
            .create_chat_thread("berlin", "codex", "New Chat", None)
            .unwrap();
        store
            .append_chat_message(thread.id, "user", "Fix billing webhook", "user_send")
            .unwrap();

        store
            .append_agent_chat_message_with_metadata(
                thread.id,
                "<archductor_metadata>{\"workspace_name\":\"billing webhook fix\",\"branch_name\":\"lc/billing-webhook-fix\",\"chat_title\":\"Billing Webhook Fix\"}</archductor_metadata>\nI'll inspect the webhook.",
                "agent_screen_parse",
            )
            .unwrap();

        let renamed = store.get_by_name("billing-webhook-fix").unwrap();
        assert_eq!(renamed.path, workspace.path);
        assert_eq!(renamed.branch, "lc/billing-webhook-fix");
        assert_eq!(
            git_output(&renamed.path, ["branch", "--show-current"]).trim(),
            "lc/billing-webhook-fix"
        );
        let updated_thread = store.get_chat_thread_record(thread.id).unwrap();
        assert_eq!(updated_thread.title, "Billing Webhook Fix");
        let messages = store.list_chat_messages(thread.id).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].content, "I'll inspect the webhook.");
        assert!(!messages[1].content.contains("archductor_metadata"));
    }

    #[test]
    fn agent_metadata_directive_uses_configured_prefix_for_lc_placeholder() {
        let (temp, store) = test_workspace_store();
        let settings_dir = temp.path().join("demo/.archductor");
        fs::create_dir_all(&settings_dir).unwrap();
        fs::write(
            settings_dir.join("settings.toml"),
            "[customization.workspace_defaults]\nbranch_prefix = \"team\"\n",
        )
        .unwrap();
        let thread = store
            .create_chat_thread("berlin", "codex", "New Chat", None)
            .unwrap();
        store
            .append_chat_message(thread.id, "user", "Fix billing webhook", "user_send")
            .unwrap();

        store
            .append_agent_chat_message_with_metadata(
                thread.id,
                "<archductor_metadata>{\"branch_name\":\"lc/billing-webhook-fix\"}</archductor_metadata>\nI'll inspect the webhook.",
                "agent_screen_parse",
            )
            .unwrap();

        let workspace = store.get_by_name("berlin").unwrap();
        assert_eq!(workspace.branch, "team/billing-webhook-fix");
    }

    #[test]
    fn incomplete_agent_metadata_marker_preserves_original_content() {
        let (_temp, store) = test_workspace_store();
        let thread = store
            .create_chat_thread("berlin", "codex", "New Chat", None)
            .unwrap();
        store
            .append_chat_message(thread.id, "user", "Fix billing webhook", "user_send")
            .unwrap();

        store
            .append_agent_chat_message_with_metadata(
                thread.id,
                "<archductor_metadata>{\"workspace_name\":\"billing webhook fix\"}\nI'll inspect the webhook.",
                "agent_screen_parse",
            )
            .unwrap();

        assert!(store.get_by_name("billing-webhook-fix").is_err());
        let messages = store.list_chat_messages(thread.id).unwrap();
        assert_eq!(
            messages[1].content,
            "<archductor_metadata>{\"workspace_name\":\"billing webhook fix\"}\nI'll inspect the webhook."
        );
    }

    #[test]
    fn later_agent_metadata_directives_do_not_rename_workspace() {
        let (_temp, store) = test_workspace_store();
        let thread = store
            .create_chat_thread("berlin", "codex", "New Chat", None)
            .unwrap();
        store
            .append_chat_message(thread.id, "user", "Fix billing webhook", "user_send")
            .unwrap();
        store
            .append_agent_chat_message_with_metadata(
                thread.id,
                "First response.",
                "agent_screen_parse",
            )
            .unwrap();

        store
            .append_agent_chat_message_with_metadata(
                thread.id,
                "<archductor_metadata>{\"workspace_name\":\"late rename\",\"branch_name\":\"lc/late-rename\"}</archductor_metadata>\nContinuing.",
                "agent_screen_parse",
            )
            .unwrap();

        assert!(store.get_by_name("late-rename").is_err());
        let workspace = store.get_by_name("berlin").unwrap();
        assert_eq!(workspace.branch, "lc/berlin");
    }

    #[test]
    fn chat_thread_close_and_reopen_preserves_history_and_resume_id() {
        let (_temp, store) = test_workspace_store();
        let thread = store
            .create_chat_thread("berlin", "codex", "New Chat", None)
            .unwrap();
        store
            .update_chat_thread_native_id(thread.id, "codex-thread-1")
            .unwrap();
        store
            .append_chat_message(thread.id, "user", "keep this", "user_send")
            .unwrap();

        store.close_chat_thread(thread.id).unwrap();
        let closed = store.get_chat_thread_record(thread.id).unwrap();
        assert_eq!(closed.status, "closed");
        assert_eq!(closed.native_thread_id.as_deref(), Some("codex-thread-1"));
        assert!(closed.archived_at.is_some());
        assert_eq!(store.list_chat_messages(thread.id).unwrap().len(), 1);

        store.reopen_chat_thread(thread.id).unwrap();
        let reopened = store.get_chat_thread_record(thread.id).unwrap();
        assert_eq!(reopened.status, "active");
        assert_eq!(reopened.native_thread_id.as_deref(), Some("codex-thread-1"));
        assert_eq!(reopened.archived_at, None);
        assert_eq!(store.list_chat_messages(thread.id).unwrap().len(), 1);
    }

    #[test]
    fn persist_codex_screen_delta_persists_structured_agent_messages_for_threads() {
        let (_temp, store) = test_workspace_store();
        let thread = store
            .create_chat_thread("berlin", "codex", "Bugfix A", None)
            .unwrap();
        store
            .append_chat_message(thread.id, "user", "Fix the failing test", "user_send")
            .unwrap();
        let process = store
            .record_session_process_for_thread(
                "berlin",
                thread.id,
                &SessionLaunch {
                    kind: SessionKind::Codex,
                    program: PathBuf::from("codex"),
                    args: vec!["--no-alt-screen".to_owned()],
                    cwd: PathBuf::from("/tmp/berlin"),
                    env: Vec::new(),
                    harness_metadata: Some("harness=codex".to_owned()),
                    session_resume_id: None,
                },
                exited_child_pid(),
            )
            .unwrap();

        store
            .persist_codex_screen_delta(
                thread.id,
                process.id,
                "╭─ You\n│ Fix the failing test\n╰─\n╭─ Codex\n│ Running the test suite now.\n╰─",
            )
            .unwrap();
        store
            .persist_codex_screen_delta(
                thread.id,
                process.id,
                "╭─ You\n│ Fix the failing test\n╰─\n╭─ Codex\n│ Running the test suite now.\n│ The failure is in parser.rs.\n╰─",
            )
            .unwrap();

        let messages = store.list_chat_messages(thread.id).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].role, "agent");
        assert_eq!(
            messages[1].content,
            "Running the test suite now.\nThe failure is in parser.rs."
        );
        assert_eq!(messages[1].source, "agent_screen_parse");
    }

    #[test]
    fn persist_codex_screen_delta_persists_file_reads_as_events_only() {
        let (_temp, store) = test_workspace_store();
        let thread = store
            .create_chat_thread("berlin", "codex", "Bugfix A", None)
            .unwrap();
        let process = process_record_for_thread(&store, thread.id);

        store
            .append_chat_message(thread.id, "user", "Read the README", "user_send")
            .unwrap();
        store
            .persist_codex_screen_delta(
                thread.id,
                process.id,
                "› Read the README\nRead README.md\n# Project\nDetails.\n",
            )
            .unwrap();

        let messages = store.list_chat_messages(thread.id).unwrap();
        let events = store.list_chat_events(thread.id).unwrap();

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "tool");
        assert_eq!(events[0].title, "Read README.md");
        assert_eq!(events[0].body, "# Project\nDetails.");
    }

    #[test]
    fn codex_screen_delta_does_not_replay_old_messages_after_new_user_input() {
        let (_temp, store) = test_workspace_store();
        let thread = store
            .create_chat_thread("berlin", "codex", "Bugfix A", None)
            .unwrap();
        let process = process_record_for_thread(&store, thread.id);
        store
            .append_chat_message(thread.id, "user", "first question", "user_send")
            .unwrap();
        store
            .append_chat_message(thread.id, "agent", "first answer", "agent_screen_parse")
            .unwrap();
        store
            .append_chat_message(thread.id, "user", "second question", "user_send")
            .unwrap();

        let screen = "\
› first question
• first answer
› second question
• second answer
";

        store
            .persist_codex_screen_delta(thread.id, process.id, screen)
            .unwrap();

        let messages = store.list_chat_messages(thread.id).unwrap();
        assert_eq!(
            messages
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>(),
            vec![
                "first question",
                "first answer",
                "second question",
                "second answer"
            ]
        );
    }

    #[test]
    fn chat_messages_codex_repaint_after_new_message_does_not_persist_old_messages_again() {
        let (_temp, store) = test_workspace_store();
        let thread = store
            .create_chat_thread("berlin", "codex", "Bugfix A", None)
            .unwrap();
        let process = process_record_for_thread(&store, thread.id);
        store
            .append_chat_message(thread.id, "user", "first user message", "user_send")
            .unwrap();
        store
            .append_chat_message(
                thread.id,
                "agent",
                "first agent response",
                "agent_screen_parse",
            )
            .unwrap();
        store
            .append_chat_message(thread.id, "user", "second user message", "user_send")
            .unwrap();

        let screen = include_str!("../tests/fixtures/codex_replay_duplicate_screen.txt");
        store
            .persist_codex_screen_delta(thread.id, process.id, screen)
            .unwrap();
        store
            .persist_codex_screen_delta(thread.id, process.id, screen)
            .unwrap();

        let messages = store.list_chat_messages(thread.id).unwrap();
        assert_eq!(messages.len(), 4);
        assert_eq!(
            messages[3].content,
            "second agent response\nwith continuation"
        );
    }

    #[test]
    fn resolve_codex_native_thread_id_uses_rollout_session_meta() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = init_repo(temp.path().join("demo"));
        let db_path = temp.path().join("state.db");
        let fake_home = temp.path().join("home");
        let rollout_dir = fake_home.join(".codex/sessions/2026/06/27");
        fs::create_dir_all(&rollout_dir).unwrap();

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
        let thread = store
            .create_chat_thread("berlin", "codex", "Bugfix A", None)
            .unwrap();
        let process = store
            .record_session_process_for_thread(
                "berlin",
                thread.id,
                &SessionLaunch {
                    kind: SessionKind::Codex,
                    program: PathBuf::from("codex"),
                    args: vec!["--no-alt-screen".to_owned()],
                    cwd: workspace.path.clone(),
                    env: Vec::new(),
                    harness_metadata: Some("harness=codex".to_owned()),
                    session_resume_id: None,
                },
                exited_child_pid(),
            )
            .unwrap();

        fs::write(
            rollout_dir.join("rollout-test.jsonl"),
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"session_id\":\"native-codex-session\",\"cwd\":\"{}\"}}}}\n",
                workspace.path.display()
            ),
        )
        .unwrap();

        temp_env_var("HOME", &fake_home, || {
            let native_thread_id = store
                .resolve_codex_native_thread_id_for_process(process.id)
                .unwrap();
            assert_eq!(native_thread_id.as_deref(), Some("native-codex-session"));
        });

        let updated_thread = store.get_chat_thread_record(thread.id).unwrap();
        assert_eq!(
            updated_thread.native_thread_id.as_deref(),
            Some("native-codex-session")
        );
        let updated_process = store.get_process(process.id).unwrap();
        assert_eq!(
            updated_process.session_resume_id.as_deref(),
            Some("native-codex-session")
        );
    }

    #[test]
    fn local_chat_threads_prefer_structured_thread_summaries() {
        let (_temp, store) = test_workspace_store();
        let thread = store
            .create_chat_thread("berlin", "codex", "Bugfix A", None)
            .unwrap();
        store
            .append_chat_message(thread.id, "user", "run tests", "user_send")
            .unwrap();
        store
            .append_chat_message(thread.id, "agent", "tests failed", "agent_screen_parse")
            .unwrap();

        let summaries = store.list_local_chat_threads(None).unwrap();
        let messages = store.local_chat_thread_messages(thread.id).unwrap();

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].thread_id, thread.id);
        assert_eq!(summaries[0].provider, "codex");
        assert_eq!(summaries[0].title, "Bugfix A");
        assert_eq!(summaries[0].message_count, 2);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].content, "tests failed");
    }

    #[test]
    fn chat_thread_context_summaries_count_utf8_message_and_event_bytes() {
        let (_temp, store) = test_workspace_store();
        let thread = store
            .create_chat_thread("berlin", "codex", "Bugfix A", None)
            .unwrap();
        let message = store
            .append_chat_message(thread.id, "user", "hé", "user_send")
            .unwrap();
        let process = process_record_for_thread(&store, thread.id);
        let event = store
            .append_chat_event(
                thread.id,
                process.id,
                &CodexTranscriptEvent::Tool {
                    title: "cargo test".to_owned(),
                    body: "é".to_owned(),
                },
            )
            .unwrap();

        let summaries = store.chat_thread_context_summaries("berlin").unwrap();
        let summary = summaries
            .iter()
            .find(|summary| summary.title == "Bugfix A")
            .unwrap();
        let expected_bytes =
            message.content.len() + event.title.len() + event.body.len() + event.payload_json.len();

        assert_eq!(summary.message_count, 1);
        assert_eq!(summary.event_count, 1);
        assert_eq!(summary.transcript_bytes, expected_bytes);
    }

    #[test]
    fn concurrent_codex_sessions_in_different_workspaces_keep_persisted_state_separate() {
        let (temp, store) = test_workspace_store();
        store
            .create(CreateWorkspace {
                repository_name: "demo".to_owned(),
                name: "paris".to_owned(),
                branch: "lc/paris".to_owned(),
                base_ref: Some("main".to_owned()),
            })
            .unwrap();
        let berlin_thread = store
            .create_chat_thread("berlin", "codex", "Berlin task", None)
            .unwrap();
        let paris_thread = store
            .create_chat_thread("paris", "codex", "Paris task", None)
            .unwrap();
        let berlin_process = process_record_for_thread(&store, berlin_thread.id);
        let paris_process = store
            .record_session_process_for_thread(
                "paris",
                paris_thread.id,
                &SessionLaunch {
                    kind: SessionKind::Codex,
                    program: PathBuf::from("codex"),
                    args: vec!["--no-alt-screen".to_owned()],
                    cwd: temp.path().join("workspaces/demo/paris"),
                    env: Vec::new(),
                    harness_metadata: Some("harness=codex".to_owned()),
                    session_resume_id: None,
                },
                exited_child_pid(),
            )
            .unwrap();

        store
            .append_pty_chunk(berlin_process.id, "stdout_pty", "berlin raw chunk\n")
            .unwrap();
        store
            .append_pty_chunk(paris_process.id, "stdout_pty", "paris raw chunk\n")
            .unwrap();
        store
            .append_session_events(
                berlin_process.id,
                vec![SessionEvent::new(
                    SessionEventSource::Assistant,
                    Some("berlin raw event".to_owned()),
                    SessionEventPayload::AssistantText {
                        text: "berlin transcript".to_owned(),
                    },
                )],
            )
            .unwrap();
        store
            .append_session_events(
                paris_process.id,
                vec![SessionEvent::new(
                    SessionEventSource::Assistant,
                    Some("paris raw event".to_owned()),
                    SessionEventPayload::AssistantText {
                        text: "paris transcript".to_owned(),
                    },
                )],
            )
            .unwrap();
        store
            .append_chat_message(
                berlin_thread.id,
                "agent",
                "berlin screen snapshot",
                "agent_screen_parse",
            )
            .unwrap();
        store
            .append_chat_message(
                paris_thread.id,
                "agent",
                "paris screen snapshot",
                "agent_screen_parse",
            )
            .unwrap();

        let reopened =
            WorkspaceStore::open_with_logs(temp.path().join("state.db"), temp.path().join("logs"))
                .unwrap();
        let berlin_sessions = reopened.list_sessions("berlin").unwrap();
        let paris_sessions = reopened.list_sessions("paris").unwrap();

        assert_eq!(berlin_sessions.len(), 1);
        assert_eq!(paris_sessions.len(), 1);
        assert_ne!(berlin_sessions[0].pid, paris_sessions[0].pid);
        assert_eq!(berlin_sessions[0].chat_thread_id, Some(berlin_thread.id));
        assert_eq!(paris_sessions[0].chat_thread_id, Some(paris_thread.id));
        assert_eq!(
            reopened.list_pty_chunks(berlin_process.id).unwrap()[0].text,
            "berlin raw chunk\n"
        );
        assert_eq!(
            reopened.list_pty_chunks(paris_process.id).unwrap()[0].text,
            "paris raw chunk\n"
        );
        assert_eq!(
            reopened.list_session_events(berlin_process.id).unwrap()[0].render_text(),
            "berlin transcript"
        );
        assert_eq!(
            reopened.list_session_events(paris_process.id).unwrap()[0].render_text(),
            "paris transcript"
        );
        assert_eq!(
            reopened.list_chat_messages(berlin_thread.id).unwrap()[0].content,
            "berlin screen snapshot"
        );
        assert_eq!(
            reopened.list_chat_messages(paris_thread.id).unwrap()[0].content,
            "paris screen snapshot"
        );
    }

    #[test]
    fn concurrent_codex_sessions_in_same_workspace_keep_thread_transcripts_separate() {
        let (_temp, store) = test_workspace_store();
        let thread_a = store
            .create_chat_thread("berlin", "codex", "Bugfix A", None)
            .unwrap();
        let thread_b = store
            .create_chat_thread("berlin", "codex", "Bugfix B", None)
            .unwrap();
        let process_a = process_record_for_thread(&store, thread_a.id);
        let process_b = process_record_for_thread(&store, thread_b.id);

        store
            .append_pty_chunk(process_a.id, "stdout_pty", "thread A raw\n")
            .unwrap();
        store
            .append_pty_chunk(process_b.id, "stdout_pty", "thread B raw\n")
            .unwrap();
        store
            .append_chat_message(thread_a.id, "agent", "thread A transcript", "agent_reply")
            .unwrap();
        store
            .append_chat_message(thread_b.id, "agent", "thread B transcript", "agent_reply")
            .unwrap();
        store
            .append_session_events(
                process_a.id,
                vec![SessionEvent::new(
                    SessionEventSource::Assistant,
                    None,
                    SessionEventPayload::AssistantText {
                        text: "thread A event".to_owned(),
                    },
                )],
            )
            .unwrap();
        store
            .append_session_events(
                process_b.id,
                vec![SessionEvent::new(
                    SessionEventSource::Assistant,
                    None,
                    SessionEventPayload::AssistantText {
                        text: "thread B event".to_owned(),
                    },
                )],
            )
            .unwrap();

        let sessions = store.list_sessions("berlin").unwrap();
        assert_eq!(sessions.len(), 2);
        assert_ne!(process_a.pid, process_b.pid);
        assert_eq!(
            store.list_thread_processes(thread_a.id).unwrap()[0].id,
            process_a.id
        );
        assert_eq!(
            store.list_thread_processes(thread_b.id).unwrap()[0].id,
            process_b.id
        );
        assert_eq!(
            store.list_chat_messages(thread_a.id).unwrap()[0].content,
            "thread A transcript"
        );
        assert_eq!(
            store.list_chat_messages(thread_b.id).unwrap()[0].content,
            "thread B transcript"
        );
        assert_eq!(
            store.list_pty_chunks(process_a.id).unwrap()[0].text,
            "thread A raw\n"
        );
        assert_eq!(
            store.list_pty_chunks(process_b.id).unwrap()[0].text,
            "thread B raw\n"
        );
        assert_eq!(
            store.list_session_events(process_a.id).unwrap()[0].render_text(),
            "thread A event"
        );
        assert_eq!(
            store.list_session_events(process_b.id).unwrap()[0].render_text(),
            "thread B event"
        );
    }

    #[test]
    fn diff_file_summaries_include_staged_unstaged_and_untracked_changes() {
        let (_temp, store) = test_workspace_store();
        let workspace = store.get_by_name("berlin").unwrap();
        fs::write(workspace.path.join("README.md"), "staged change\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args(["add", "README.md"])
            .status()
            .unwrap();
        fs::write(
            workspace.path.join("README.md"),
            "staged change\nunstaged change\n",
        )
        .unwrap();
        fs::write(workspace.path.join("new.txt"), "untracked\n").unwrap();

        let summaries = store.diff_file_summaries("berlin").unwrap();

        assert!(summaries.iter().any(|summary| {
            summary.path == "README.md" && summary.staged && summary.unstaged && !summary.untracked
        }));
        assert!(summaries.iter().any(|summary| {
            summary.path == "new.txt" && !summary.staged && !summary.unstaged && summary.untracked
        }));
        let changed = store.changed_files("berlin").unwrap();
        assert!(changed.iter().any(|path| path == "README.md"));
        assert!(changed.iter().any(|path| path == "new.txt"));
    }

    #[test]
    fn diff_file_summaries_include_deleted_and_renamed_changes() {
        let (_temp, store) = test_workspace_store();
        let workspace = store.get_by_name("berlin").unwrap();
        fs::write(workspace.path.join("delete-me.txt"), "delete me\n").unwrap();
        fs::write(workspace.path.join("rename-me.txt"), "rename me\n").unwrap();
        let add_status = Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args(["add", "delete-me.txt", "rename-me.txt"])
            .status()
            .unwrap();
        assert!(add_status.success(), "git add failed: {add_status:?}");
        let commit_status = Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args([
                "-c",
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "add tracked files",
            ])
            .status()
            .unwrap();
        assert!(
            commit_status.success(),
            "git commit failed: {commit_status:?}"
        );
        let rm_status = Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args(["rm", "delete-me.txt"])
            .status()
            .unwrap();
        assert!(rm_status.success(), "git rm failed: {rm_status:?}");
        let mv_status = Command::new("git")
            .arg("-C")
            .arg(&workspace.path)
            .args(["mv", "rename-me.txt", "renamed.txt"])
            .status()
            .unwrap();
        assert!(mv_status.success(), "git mv failed: {mv_status:?}");

        let summaries = store.diff_file_summaries("berlin").unwrap();
        assert!(summaries.iter().any(|summary| {
            summary.path == "delete-me.txt" && summary.deletions.unwrap_or_default() > 0
        }));
        assert!(summaries
            .iter()
            .any(|summary| summary.path.contains("rename-me.txt")
                && summary.path.contains("renamed.txt")));
        let changed = store.changed_files("berlin").unwrap();
        assert!(changed.iter().any(|path| path == "delete-me.txt"));
        assert!(changed
            .iter()
            .any(|path| path.contains("rename-me.txt") && path.contains("renamed.txt")));
    }

    #[test]
    fn workspace_timeline_records_lifecycle_branch_session_and_pr_events() {
        let (_temp, store) = test_workspace_store();
        let launch = store.session_launch("berlin", SessionKind::Shell).unwrap();
        let process = store
            .record_session_process("berlin", &launch, exited_child_pid())
            .unwrap();
        store
            .mark_session_process_stopped(process.id, Some(SIGTERM_EXIT_CODE))
            .unwrap();
        store.rename_branch("berlin", "lc/renamed").unwrap();
        let workspace = store.archive("berlin", false).unwrap();
        store
            .record_pull_request(workspace.id, "https://github.com/example/demo/pull/42")
            .unwrap();

        let events = store.workspace_timeline("berlin", None).unwrap();
        let kinds = events
            .iter()
            .map(|event| event.kind.as_str())
            .collect::<Vec<_>>();

        assert!(kinds.contains(&"workspace.created"));
        assert!(kinds.contains(&"session.started"));
        assert!(kinds.contains(&"session.stopped"));
        assert!(kinds.contains(&"branch.renamed"));
        assert!(kinds.contains(&"workspace.archived"));
        assert!(kinds.contains(&"pr.created"));
        assert!(kinds.contains(&"commit.created"));
        assert!(events.windows(2).all(|pair| pair[0].id < pair[1].id));
        assert!(events.iter().all(|event| !event.created_at.is_empty()));

        let commit_count = events
            .iter()
            .filter(|event| event.kind == "commit.created")
            .count();
        let reread_commit_count = store
            .workspace_timeline("berlin", Some("commit.created"))
            .unwrap()
            .len();
        assert_eq!(reread_commit_count, commit_count);
    }

    #[test]
    fn duplicate_workspace_creates_new_worktree_from_selected_branch() {
        let (_temp, store) = test_workspace_store();

        let copy = store
            .duplicate("berlin", "oslo", Some("lc/oslo-copy"))
            .unwrap();

        assert_eq!(copy.name, "oslo");
        assert_eq!(copy.branch, "lc/oslo-copy");
        assert!(copy.path.exists());
        assert!(copy.path.join(".context/brief.md").exists());
        let events = store.workspace_timeline("oslo", None).unwrap();
        assert!(events
            .iter()
            .any(|event| event.kind == "workspace.duplicated"));
    }

    #[test]
    fn branch_actions_are_scoped_to_workspace_and_preserve_metadata() {
        let (_temp, store) = test_workspace_store();

        store.create_branch("berlin", "lc/feature").unwrap();
        store.create_branch("berlin", "lc/other").unwrap();
        store.checkout_branch("berlin", "lc/feature").unwrap();
        let checked_out = store.get_by_name("berlin").unwrap();
        assert_eq!(checked_out.branch, "lc/feature");

        store.rename_branch("berlin", "lc/feature-renamed").unwrap();
        let renamed = store.get_by_name("berlin").unwrap();
        assert_eq!(renamed.branch, "lc/feature-renamed");

        let err = store
            .delete_branch("berlin", "lc/feature-renamed")
            .unwrap_err();
        assert!(err.to_string().contains("current workspace branch"));

        store.checkout_branch("berlin", "lc/other").unwrap();
        store.delete_branch("berlin", "lc/feature-renamed").unwrap();
        let branches = git_output(&renamed.path, ["branch", "--list", "lc/feature-renamed"]);
        assert!(branches.trim().is_empty());
    }

    #[test]
    fn slugify_converts_to_kebab_case() {
        assert_eq!(slugify("Add search feature"), "add-search-feature");
        assert_eq!(slugify("Fix: weird  spaces"), "fix-weird-spaces");
        assert_eq!(slugify("feat/cool-thing"), "feat-cool-thing");
        let long = "a".repeat(50);
        assert!(slugify(&long).len() <= 40);
    }

    fn test_workspace_store() -> (tempfile::TempDir, WorkspaceStore) {
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
        (temp, store)
    }

    fn process_record_for_thread(store: &WorkspaceStore, thread_id: i64) -> ProcessRecord {
        store
            .record_session_process_for_thread(
                "berlin",
                thread_id,
                &SessionLaunch {
                    kind: SessionKind::Codex,
                    program: PathBuf::from("codex"),
                    args: vec!["--no-alt-screen".to_owned()],
                    cwd: PathBuf::from("/tmp/berlin"),
                    env: Vec::new(),
                    harness_metadata: Some("harness=codex".to_owned()),
                    session_resume_id: None,
                },
                exited_child_pid(),
            )
            .unwrap()
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
                "user.name=Linux Archductor",
                "-c",
                "user.email=linux-archductor@example.test",
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
                ProcessKind::Check => store.list_checks(workspace).unwrap(),
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

    #[test]
    fn codex_log_formatters_emit_canonical_markers() {
        assert_eq!(
            format_codex_raw_output("hello"),
            "[codex raw]\nhello\n[/codex raw]\n"
        );
        assert_eq!(
            format_codex_screen_snapshot("hello\n"),
            "[codex screen]\nhello\n[/codex screen]\n"
        );
    }

    #[test]
    fn workspace_store_delegates_schema_migration_to_storage_module() {
        let source = include_str!("workspace.rs");
        let create_table_marker = concat!("CREATE TABLE", " IF NOT EXISTS");

        assert!(source.contains("crate::storage::migrate_workspace_db(&self.conn)"));
        assert!(
            !source.contains(create_table_marker),
            "workspace store should not own schema DDL"
        );
    }

    #[test]
    fn git_review_service_boundary_exists_for_pr_review_flows() {
        let git_review_service =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/git_review_service.rs");
        assert!(
            git_review_service.exists(),
            "git/review flows need a narrow service boundary outside the WorkspaceStore monolith"
        );
    }
}
