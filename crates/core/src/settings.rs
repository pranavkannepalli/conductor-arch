use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepositorySettings {
    pub file_include_globs: Vec<String>,
    pub env_file_refs: Vec<String>,
    pub spotlight_testing: Option<bool>,
    pub enterprise_data_privacy: Option<bool>,
    pub scripts: ScriptSettings,
    pub environment_variables: Vec<(String, String)>,
    pub prompt_pack: PromptPackSettings,
    pub prompts: Option<PromptSettings>,
    pub providers: ProviderSettings,
    pub git: GitSettings,
    pub customization: CustomizationSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepositoryConfigBootstrap {
    pub conductor_dir_created: bool,
    pub shared_settings_created: bool,
    pub prompt_pack_dir_created: bool,
    pub default_prompt_pack_created: bool,
    pub active_prompt_pack_created: bool,
    pub context_gitignore_updated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositorySettingsLoadReport {
    pub settings: RepositorySettings,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PromptSettings {
    pub new_workspace: Option<String>,
    pub general: Option<String>,
    pub continue_work: Option<String>,
    pub summarize_session: Option<String>,
    pub handoff: Option<String>,
    pub code_review: Option<String>,
    pub create_pr: Option<String>,
    pub fix_errors: Option<String>,
    pub resolve_merge_conflicts: Option<String>,
    pub rename_branch: Option<String>,
    pub commit_generation: Option<String>,
    pub test_fixing: Option<String>,
    pub refactor_style: Option<String>,
    pub setup_script: Option<String>,
    pub run_script: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    NewWorkspace,
    General,
    ContinueWork,
    SummarizeSession,
    Handoff,
    CodeReview,
    CreatePr,
    FixErrors,
    ResolveMergeConflicts,
    RenameBranch,
    CommitGeneration,
    TestFixing,
    RefactorStyle,
    SetupScript,
    RunScript,
}

impl PromptKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NewWorkspace => "new_workspace",
            Self::General => "general",
            Self::ContinueWork => "continue_work",
            Self::SummarizeSession => "summarize_session",
            Self::Handoff => "handoff",
            Self::CodeReview => "code_review",
            Self::CreatePr => "create_pr",
            Self::FixErrors => "fix_errors",
            Self::ResolveMergeConflicts => "resolve_merge_conflicts",
            Self::RenameBranch => "rename_branch",
            Self::CommitGeneration => "commit_generation",
            Self::TestFixing => "test_fixing",
            Self::RefactorStyle => "refactor_style",
            Self::SetupScript => "setup_script",
            Self::RunScript => "run_script",
        }
    }
}

impl PromptSettings {
    pub fn get(&self, kind: PromptKind) -> Option<&str> {
        match kind {
            PromptKind::NewWorkspace => self.new_workspace.as_deref(),
            PromptKind::General => self.general.as_deref(),
            PromptKind::ContinueWork => self.continue_work.as_deref(),
            PromptKind::SummarizeSession => self.summarize_session.as_deref(),
            PromptKind::Handoff => self.handoff.as_deref(),
            PromptKind::CodeReview => self.code_review.as_deref(),
            PromptKind::CreatePr => self.create_pr.as_deref(),
            PromptKind::FixErrors => self.fix_errors.as_deref(),
            PromptKind::ResolveMergeConflicts => self.resolve_merge_conflicts.as_deref(),
            PromptKind::RenameBranch => self.rename_branch.as_deref(),
            PromptKind::CommitGeneration => self.commit_generation.as_deref(),
            PromptKind::TestFixing => self.test_fixing.as_deref(),
            PromptKind::RefactorStyle => self.refactor_style.as_deref(),
            PromptKind::SetupScript => self.setup_script.as_deref(),
            PromptKind::RunScript => self.run_script.as_deref(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PromptPackSettings {
    pub active: Option<String>,
    pub version: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderSettings {
    pub claude_code_executable_path: Option<String>,
    pub codex_executable_path: Option<String>,
    pub claude_provider: Option<String>,
    pub codex_provider: Option<String>,
    pub bedrock_region: Option<String>,
    pub vertex_project_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitSettings {
    pub delete_branch_on_archive: Option<bool>,
    pub archive_on_merge: Option<bool>,
    pub worktree_push_auto_setup_remote: Option<bool>,
    pub branch_prefix_type: Option<String>,
    pub branch_prefix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScriptSettings {
    pub setup: Option<String>,
    pub run: Option<String>,
    pub archive: Option<String>,
    pub test: Option<String>,
    pub lint: Option<String>,
    pub typecheck: Option<String>,
    pub build: Option<String>,
    pub run_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CustomizationSettings {
    pub naming: NamingSettings,
    pub automation: AutomationSettings,
    pub agent_profiles: BTreeMap<String, AgentProfileSettings>,
    pub merge_rules: MergeRuleSettings,
    pub workspace_defaults: WorkspaceDefaultSettings,
    pub view: ViewSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NamingSettings {
    pub branch_template: Option<String>,
    pub workspace_name_style: Option<String>,
    pub commit_style: Option<String>,
    pub pr_title_template: Option<String>,
    pub pr_body_sections: Vec<String>,
    pub default_merge_method: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AutomationSettings {
    pub auto_setup: Option<bool>,
    pub auto_start_agent: Option<String>,
    pub required_local_files: Vec<String>,
    pub test_command: Option<String>,
    pub lint_command: Option<String>,
    pub typecheck_command: Option<String>,
    pub build_command: Option<String>,
    pub pre_clone: Option<String>,
    pub post_clone: Option<String>,
    pub pre_workspace_create: Option<String>,
    pub post_workspace_create: Option<String>,
    pub pre_setup: Option<String>,
    pub post_setup: Option<String>,
    pub pre_pr_create: Option<String>,
    pub post_pr_create: Option<String>,
    pub pre_merge: Option<String>,
    pub post_merge: Option<String>,
    pub pre_archive: Option<String>,
    pub post_archive: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AgentProfileSettings {
    pub agent: Option<String>,
    pub model: Option<String>,
    pub approval_mode: Option<String>,
    pub reasoning_mode: Option<String>,
    pub personality: Option<String>,
    pub mcp_servers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MergeRuleSettings {
    pub block_on_open_todos: Option<bool>,
    pub block_on_open_comments: Option<bool>,
    pub block_on_failed_checks: Option<bool>,
    pub block_on_pending_checks: Option<bool>,
    pub definition_of_done: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkspaceDefaultSettings {
    pub base_branch: Option<String>,
    pub workspace_parent: Option<String>,
    pub branch_prefix: Option<String>,
    pub working_directory: Option<String>,
    pub port_block_size: Option<u16>,
    pub auto_open: Option<bool>,
    pub checkpoint_timing: Option<String>,
    pub default_visible_tab: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ViewSettings {
    pub theme: Option<String>,
    pub accent_color: Option<String>,
    pub colors: BTreeMap<String, String>,
    pub density: Option<String>,
    pub sidebar_layout: Option<String>,
    pub diff_preference: Option<String>,
    pub terminal_font: Option<String>,
    pub terminal_scrollback: Option<u32>,
    pub transcript_display: Option<String>,
    pub dashboard_columns: Vec<String>,
    pub notification_rules: Vec<String>,
    pub keybindings: Option<String>,
    pub command_palette_presets: Vec<String>,
    pub settings_import_export: Option<String>,
}

pub fn load_repository_settings(repo_path: &Path) -> Result<RepositorySettings> {
    match load_repository_settings_strict(repo_path) {
        Ok(settings) => Ok(settings),
        Err(err) if is_recoverable_settings_load_error(&err) => {
            Ok(load_repository_settings_recovering(repo_path).settings)
        }
        Err(err) => Err(err),
    }
}

pub fn load_app_shared_settings(path: &Path) -> Result<RepositorySettings> {
    let settings = load_optional_settings(path)?.into_settings();
    validate_repository_settings(&settings)?;
    Ok(settings)
}

pub fn save_app_shared_settings(path: &Path, settings: &RepositorySettings) -> Result<()> {
    save_app_shared_settings_with_collection_intent(path, settings, &[], &[])
}

pub fn save_app_shared_settings_with_explicit_empty_collections(
    path: &Path,
    settings: &RepositorySettings,
    explicit_empty_collections: &[SettingsCollectionField],
) -> Result<()> {
    save_app_shared_settings_with_collection_intent(path, settings, explicit_empty_collections, &[])
}

pub fn save_app_shared_settings_with_collection_intent(
    path: &Path,
    settings: &RepositorySettings,
    explicit_empty_collections: &[SettingsCollectionField],
    unset_collections: &[SettingsCollectionField],
) -> Result<()> {
    validate_repository_settings(settings)?;
    let parent = path
        .parent()
        .with_context(|| format!("resolve parent for {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    ensure_real_directory(parent)?;
    reject_symlink_file(path)?;
    let original = load_optional_settings(path)?;
    let contents = repository_settings_to_toml_with_collection_intent(
        settings,
        &original,
        explicit_empty_collections,
        unset_collections,
    )?;
    atomic_write_no_symlink(path, contents.as_bytes())
}

pub fn save_app_shared_settings_from_toml(path: &Path, contents: &str) -> Result<()> {
    let raw = validated_raw_repository_settings_from_toml(contents)?;
    let settings = raw.clone().into_settings();
    let parent = path
        .parent()
        .with_context(|| format!("resolve parent for {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    ensure_real_directory(parent)?;
    reject_symlink_file(path)?;
    let contents = repository_settings_to_toml_preserving_presence(&settings, &raw)?;
    atomic_write_no_symlink(path, contents.as_bytes())
}

pub fn app_shared_settings_to_toml(path: &Path) -> Result<String> {
    let raw = load_optional_settings(path)?;
    let settings = raw.clone().into_settings();
    validate_repository_settings(&settings)?;
    repository_settings_to_toml_preserving_presence(&settings, &raw)
}

pub fn load_effective_repository_settings(
    repo_path: &Path,
    app_settings_path: &Path,
) -> Result<RepositorySettings> {
    let app_base = RawRepositorySettings::from_settings(&default_app_shared_settings())
        .merge(load_optional_settings(app_settings_path)?);
    let mut repository = load_raw_repository_settings(repo_path)?;
    let repository_settings = repository_settings_from_raw(repo_path, repository.clone())?;
    repository.prompts = repository_settings
        .prompts
        .as_ref()
        .map(RawPromptSettings::from_settings);
    let effective = app_base.merge(repository).into_settings();
    validate_repository_settings(&effective)?;
    Ok(effective)
}

fn default_app_shared_settings() -> RepositorySettings {
    RepositorySettings {
        prompts: Some(default_prompt_settings()),
        ..RepositorySettings::default()
    }
}

fn load_repository_settings_strict(repo_path: &Path) -> Result<RepositorySettings> {
    repository_settings_from_raw(repo_path, load_raw_repository_settings(repo_path)?)
}

fn load_raw_repository_settings(repo_path: &Path) -> Result<RawRepositorySettings> {
    let shared = load_optional_settings(&repo_path.join(".archductor/settings.toml"))?;
    let local = load_optional_settings(&repo_path.join(".archductor/settings.local.toml"))?;
    Ok(shared.merge(local))
}

fn repository_settings_from_raw(
    repo_path: &Path,
    raw: RawRepositorySettings,
) -> Result<RepositorySettings> {
    let mut settings = raw.into_settings();
    validate_repository_settings(&settings)?;
    apply_prompt_pack_prompts(repo_path, &mut settings)?;
    Ok(settings)
}

pub fn load_repository_settings_for_layer(
    repo_path: &Path,
    layer: SettingsLayer,
) -> Result<RepositorySettings> {
    let path = match layer {
        SettingsLayer::RepositoryShared => repo_path.join(".archductor/settings.toml"),
        SettingsLayer::LocalOverride => repo_path.join(".archductor/settings.local.toml"),
    };
    load_optional_settings(&path).map(|settings| settings.into_settings())
}

pub fn load_repository_settings_recovering(repo_path: &Path) -> RepositorySettingsLoadReport {
    let shared_path = repo_path.join(".archductor/settings.toml");
    let local_path = repo_path.join(".archductor/settings.local.toml");
    let mut errors = Vec::new();
    let shared = match load_optional_settings(&shared_path) {
        Ok(settings) => settings,
        Err(err) => {
            errors.push(err.to_string());
            RawRepositorySettings::default()
        }
    };
    let local = match load_optional_settings(&local_path) {
        Ok(settings) => settings,
        Err(err) => {
            errors.push(err.to_string());
            RawRepositorySettings::default()
        }
    };
    let mut settings = shared.merge(local).into_settings();
    match validate_repository_settings(&settings) {
        Ok(()) => {
            if let Err(err) = apply_prompt_pack_prompts(repo_path, &mut settings) {
                errors.push(err.to_string());
            }
            RepositorySettingsLoadReport { settings, errors }
        }
        Err(err) => {
            errors.push(err.to_string());
            RepositorySettingsLoadReport {
                settings: RepositorySettings::default(),
                errors,
            }
        }
    }
}

fn apply_prompt_pack_prompts(repo_path: &Path, settings: &mut RepositorySettings) -> Result<()> {
    let Some(pack_prompts) = load_prompt_pack_prompts(repo_path, &settings.prompt_pack)? else {
        return Ok(());
    };
    settings.prompts = Some(match settings.prompts.take() {
        Some(prompts) => merge_prompt_settings(pack_prompts, prompts),
        None => pack_prompts,
    });
    Ok(())
}

fn load_prompt_pack_prompts(
    repo_path: &Path,
    prompt_pack: &PromptPackSettings,
) -> Result<Option<PromptSettings>> {
    let Some(relative_path) = prompt_pack
        .path
        .as_deref()
        .and_then(active_prompt_pack_path)
    else {
        return Ok(None);
    };
    let path = repo_path.join(relative_path);
    if !prompt_pack_path_is_real(&repo_path.join(".archductor/prompt-packs"), true)? {
        return Ok(None);
    }
    if !prompt_pack_path_is_real(&path, false)? {
        return Ok(None);
    }
    let contents = fs::read_to_string(&path)
        .with_context(|| format!("read prompt pack {}", path.display()))?;
    let raw: RawPromptPackFile = toml::from_str(&contents)
        .with_context(|| format!("parse prompt pack {}", path.display()))?;
    Ok(Some(raw.prompts.into_settings()))
}

fn merge_prompt_settings(base: PromptSettings, overrides: PromptSettings) -> PromptSettings {
    RawPromptSettings::from_settings(&base)
        .merge(RawPromptSettings::from_settings(&overrides))
        .into_settings()
}

fn is_recoverable_settings_load_error(err: &anyhow::Error) -> bool {
    let message = err.to_string();
    !(message.contains("must not be a symlink")
        || message.contains("read settings")
        || message.contains("inspect "))
}

pub fn ensure_repository_config(repo_path: &Path) -> Result<RepositoryConfigBootstrap> {
    let (conductor_dir, conductor_dir_created) = ensure_settings_dir(repo_path)?;
    let shared_path = conductor_dir.join("settings.toml");
    reject_symlink_file(&shared_path)?;

    let mut report = RepositoryConfigBootstrap {
        conductor_dir_created,
        shared_settings_created: false,
        prompt_pack_dir_created: false,
        default_prompt_pack_created: false,
        active_prompt_pack_created: false,
        context_gitignore_updated: false,
    };

    let settings = match fs::read_to_string(&shared_path) {
        Ok(contents) => {
            let settings = repository_settings_from_toml(&contents)
                .with_context(|| format!("validate {}", shared_path.display()))?;
            validate_repository_settings(&settings)?;
            settings
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {
            let contents = default_repository_settings_toml()?;
            atomic_write_no_symlink(&shared_path, contents.as_bytes())?;
            report.shared_settings_created = true;
            repository_settings_from_toml(&contents)
                .with_context(|| format!("validate {}", shared_path.display()))?
        }
        Err(err) => return Err(err).with_context(|| format!("read {}", shared_path.display())),
    };
    let prompt_pack_report =
        ensure_default_prompt_pack_files(repo_path, &conductor_dir, &settings)?;
    report.prompt_pack_dir_created = prompt_pack_report.prompt_pack_dir_created;
    report.default_prompt_pack_created = prompt_pack_report.default_prompt_pack_created;
    report.active_prompt_pack_created = prompt_pack_report.active_prompt_pack_created;
    report.context_gitignore_updated = ensure_context_gitignored(repo_path)?;

    Ok(report)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsLayer {
    RepositoryShared,
    LocalOverride,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsCollectionField {
    FileIncludeGlobs,
    EnvFileRefs,
    EnvironmentVariables,
    AgentProfiles,
    PrBodySections,
    RequiredLocalFiles,
    AgentProfileMcpServers(String),
    ViewColors,
    DashboardColumns,
    NotificationRules,
    CommandPalettePresets,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositorySettingsInspection {
    pub shared_settings_exists: bool,
    pub local_settings_exists: bool,
    pub worktreeinclude_exists: bool,
    pub worktreeinclude_patterns: Vec<String>,
    pub active_file_patterns_source: FilePatternSource,
    pub active_file_patterns: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilePatternSource {
    Worktreeinclude,
    RepositorySettings,
    BuiltInDefault,
}

pub fn inspect_repository_settings(repo_path: &Path) -> Result<RepositorySettingsInspection> {
    let shared_settings_path = repo_path.join(".archductor/settings.toml");
    let local_settings_path = repo_path.join(".archductor/settings.local.toml");
    let worktreeinclude_path = repo_path.join(".worktreeinclude");
    let worktreeinclude_patterns = if worktreeinclude_path.exists() {
        split_patterns(Some(
            std::fs::read_to_string(&worktreeinclude_path)
                .with_context(|| format!("read {}", worktreeinclude_path.display()))?,
        ))
    } else {
        Vec::new()
    };
    let settings = load_repository_settings_recovering(repo_path).settings;
    let (active_file_patterns_source, active_file_patterns) =
        if !worktreeinclude_patterns.is_empty() {
            (
                FilePatternSource::Worktreeinclude,
                worktreeinclude_patterns.clone(),
            )
        } else if !settings.file_include_globs.is_empty() {
            (
                FilePatternSource::RepositorySettings,
                settings.file_include_globs,
            )
        } else {
            (FilePatternSource::BuiltInDefault, vec![".env*".to_owned()])
        };

    Ok(RepositorySettingsInspection {
        shared_settings_exists: shared_settings_path.exists(),
        local_settings_exists: local_settings_path.exists(),
        worktreeinclude_exists: worktreeinclude_path.exists(),
        worktreeinclude_patterns,
        active_file_patterns_source,
        active_file_patterns,
    })
}

pub fn save_repository_settings(
    repo_path: &Path,
    layer: SettingsLayer,
    settings: &RepositorySettings,
) -> Result<()> {
    save_repository_settings_with_collection_intent(repo_path, layer, settings, &[], &[])
}

pub fn save_repository_settings_with_explicit_empty_collections(
    repo_path: &Path,
    layer: SettingsLayer,
    settings: &RepositorySettings,
    explicit_empty_collections: &[SettingsCollectionField],
) -> Result<()> {
    save_repository_settings_with_collection_intent(
        repo_path,
        layer,
        settings,
        explicit_empty_collections,
        &[],
    )
}

pub fn save_repository_settings_with_collection_intent(
    repo_path: &Path,
    layer: SettingsLayer,
    settings: &RepositorySettings,
    explicit_empty_collections: &[SettingsCollectionField],
    unset_collections: &[SettingsCollectionField],
) -> Result<()> {
    validate_repository_settings(settings)?;
    let (conductor_dir, _) = ensure_settings_dir(repo_path)?;
    let path = match layer {
        SettingsLayer::RepositoryShared => conductor_dir.join("settings.toml"),
        SettingsLayer::LocalOverride => conductor_dir.join("settings.local.toml"),
    };
    reject_symlink_file(&path)?;
    let original = load_optional_settings(&path)?;
    backup_settings_file(&path)?;
    let contents = repository_settings_to_toml_with_collection_intent(
        settings,
        &original,
        explicit_empty_collections,
        unset_collections,
    )?;
    atomic_write_no_symlink(&path, contents.as_bytes())
}

pub fn save_repository_settings_replacing(
    repo_path: &Path,
    layer: SettingsLayer,
    settings: &RepositorySettings,
) -> Result<()> {
    validate_repository_settings(settings)?;
    let (conductor_dir, _) = ensure_settings_dir(repo_path)?;
    let path = match layer {
        SettingsLayer::RepositoryShared => conductor_dir.join("settings.toml"),
        SettingsLayer::LocalOverride => conductor_dir.join("settings.local.toml"),
    };
    reject_symlink_file(&path)?;
    backup_settings_file(&path)?;
    let contents = repository_settings_to_toml(settings)?;
    atomic_write_no_symlink(&path, contents.as_bytes())
}

pub fn save_repository_settings_from_toml(
    repo_path: &Path,
    layer: SettingsLayer,
    contents: &str,
) -> Result<()> {
    let raw = validated_raw_repository_settings_from_toml(contents)?;
    let settings = raw.clone().into_settings();
    let (conductor_dir, _) = ensure_settings_dir(repo_path)?;
    let path = match layer {
        SettingsLayer::RepositoryShared => conductor_dir.join("settings.toml"),
        SettingsLayer::LocalOverride => conductor_dir.join("settings.local.toml"),
    };
    reject_symlink_file(&path)?;
    backup_settings_file(&path)?;
    let contents = repository_settings_to_toml_preserving_presence(&settings, &raw)?;
    atomic_write_no_symlink(&path, contents.as_bytes())
}

pub fn save_local_default_agent_provider(repo_path: &Path, provider: &str) -> Result<()> {
    validate_agent_provider(provider)?;
    let conductor_dir = ensure_local_settings_dir(repo_path)?;
    let path = conductor_dir.join("settings.local.toml");
    reject_symlink_file(&path)?;
    let mut value = match fs::read_to_string(&path) {
        Ok(contents) if contents.trim().is_empty() => toml::Value::Table(toml::map::Map::new()),
        Ok(contents) => toml::from_str::<toml::Value>(&contents)
            .with_context(|| format!("parse {}", path.display()))?,
        Err(err) if err.kind() == ErrorKind::NotFound => toml::Value::Table(toml::map::Map::new()),
        Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
    };
    set_local_default_agent_provider(&mut value, provider)?;
    let contents = toml::to_string_pretty(&value).context("serialize local settings")?;
    atomic_write_no_symlink(&path, contents.as_bytes())
}

pub fn customization_settings_to_toml(settings: &CustomizationSettings) -> Result<String> {
    let raw = RawRepositorySettings {
        customization: Some(RawCustomizationSettings::from_settings(settings)),
        ..RawRepositorySettings::default()
    };
    toml::to_string_pretty(&raw).context("serialize customization settings")
}

pub fn app_shared_customization_settings_toml(path: &Path) -> Result<String> {
    raw_customization_settings_to_toml(load_optional_settings(path)?)
}

pub fn repository_customization_settings_toml(
    repo_path: &Path,
    layer: SettingsLayer,
) -> Result<String> {
    let path = match layer {
        SettingsLayer::RepositoryShared => repo_path.join(".archductor/settings.toml"),
        SettingsLayer::LocalOverride => repo_path.join(".archductor/settings.local.toml"),
    };
    raw_customization_settings_to_toml(load_optional_settings(&path)?)
}

fn raw_customization_settings_to_toml(raw: RawRepositorySettings) -> Result<String> {
    let customization = RawRepositorySettings {
        customization: raw.customization,
        ..RawRepositorySettings::default()
    };
    toml::to_string_pretty(&customization).context("serialize customization settings")
}

pub fn customization_settings_from_toml(contents: &str) -> Result<CustomizationSettings> {
    let raw: RawRepositorySettings =
        toml::from_str(contents).context("parse customization settings")?;
    Ok(raw.customization.unwrap_or_default().into_settings())
}

pub fn repository_settings_to_toml(settings: &RepositorySettings) -> Result<String> {
    let raw = RawRepositorySettings::from_settings(settings);
    toml::to_string_pretty(&raw).context("serialize repository settings")
}

fn repository_settings_to_toml_preserving_presence(
    settings: &RepositorySettings,
    original: &RawRepositorySettings,
) -> Result<String> {
    let raw = RawRepositorySettings::from_settings_preserving_empty_collections(settings, original);
    toml::to_string_pretty(&raw).context("serialize repository settings")
}

fn repository_settings_to_toml_with_collection_intent(
    settings: &RepositorySettings,
    original: &RawRepositorySettings,
    explicit_empty_collections: &[SettingsCollectionField],
    unset_collections: &[SettingsCollectionField],
) -> Result<String> {
    let mut raw =
        RawRepositorySettings::from_settings_preserving_empty_collections(settings, original);
    raw.unset_collections(unset_collections);
    raw.apply_explicit_empty_collections(settings, explicit_empty_collections);
    toml::to_string_pretty(&raw).context("serialize repository settings")
}

pub fn default_repository_settings_toml() -> Result<String> {
    let settings = RepositorySettings {
        file_include_globs: vec![".env*".to_owned()],
        scripts: ScriptSettings {
            run_mode: Some("concurrent".to_owned()),
            ..ScriptSettings::default()
        },
        prompts: None,
        prompt_pack: PromptPackSettings {
            active: Some("default".to_owned()),
            version: Some("v1".to_owned()),
            path: Some(".archductor/prompt-packs/default.toml".to_owned()),
        },
        customization: CustomizationSettings {
            automation: AutomationSettings {
                auto_setup: Some(false),
                ..AutomationSettings::default()
            },
            workspace_defaults: WorkspaceDefaultSettings {
                base_branch: Some("main".to_owned()),
                branch_prefix: Some("lc".to_owned()),
                port_block_size: Some(10),
                default_visible_tab: Some("changes".to_owned()),
                ..WorkspaceDefaultSettings::default()
            },
            view: ViewSettings {
                theme: Some("system".to_owned()),
                accent_color: Some("blue".to_owned()),
                colors: default_view_colors(),
                density: Some("compact".to_owned()),
                diff_preference: Some("unified".to_owned()),
                transcript_display: Some("structured".to_owned()),
                ..ViewSettings::default()
            },
            ..CustomizationSettings::default()
        },
        ..RepositorySettings::default()
    };
    repository_settings_to_toml(&settings)
}

pub fn default_prompt_pack_toml() -> Result<String> {
    let raw = RawPromptPackFile {
        name: Some("default".to_owned()),
        version: Some("v1".to_owned()),
        prompts: RawPromptSettings::from_settings(&default_prompt_settings()),
    };
    toml::to_string_pretty(&raw).context("serialize default prompt pack")
}

fn default_prompt_settings() -> PromptSettings {
    PromptSettings {
        new_workspace: Some(
            "Create a small, reviewable workspace plan before changing code.".to_owned(),
        ),
        general: Some("Prefer small, reviewable changes. Explain verification clearly.".to_owned()),
        continue_work: Some(
            "Continue from the current state. Inspect recent changes before editing.".to_owned(),
        ),
        summarize_session: Some(
            "Summarize the work completed, verification run, and remaining risk.".to_owned(),
        ),
        handoff: Some(
            "Write a concise handoff with context, changed files, tests, and next steps."
                .to_owned(),
        ),
        code_review: Some(
            "Focus on correctness, behavior changes, missing tests, and regressions.".to_owned(),
        ),
        create_pr: Some("Write a concise PR body with summary, tests, and risk.".to_owned()),
        fix_errors: Some("Reproduce the failure, then make the smallest safe fix.".to_owned()),
        resolve_merge_conflicts: Some(
            "Preserve user changes and explain any conflict resolution choices.".to_owned(),
        ),
        rename_branch: Some("Use a short descriptive branch name.".to_owned()),
        commit_generation: Some(
            "Write a conventional commit message that matches the actual diff.".to_owned(),
        ),
        test_fixing: Some(
            "Run the failing test first, fix the root cause, then rerun focused tests.".to_owned(),
        ),
        refactor_style: Some(
            "Keep behavior-preserving refactors separate from feature changes.".to_owned(),
        ),
        setup_script: Some(
            "Infer the repository setup command from existing package and build files.".to_owned(),
        ),
        run_script: Some(
            "Infer the local development run command and include port/env requirements.".to_owned(),
        ),
    }
}

fn default_view_colors() -> BTreeMap<String, String> {
    [
        ("background", "#191919"),
        ("surface", "#1e1e1e"),
        ("surface_raised", "#202020"),
        ("surface_muted", "#181818"),
        ("hover", "#2a2a2a"),
        ("hover_soft", "#242424"),
        ("border", "#2a2a2a"),
        ("border_strong", "#3a3a3a"),
        ("text", "#e4e4e4"),
        ("text_strong", "#f8fafc"),
        ("text_muted", "#8a8a8a"),
        ("accent", "#5b9dff"),
        ("accent_fg", "#071a33"),
        ("success", "#d0d0d0"),
        ("warning", "#f59e0b"),
        ("danger", "#ff8a8a"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect()
}

pub fn repository_settings_from_toml(contents: &str) -> Result<RepositorySettings> {
    let raw = validated_raw_repository_settings_from_toml(contents)?;
    let settings = raw.into_settings();
    Ok(settings)
}

pub fn explicit_empty_collection_fields_from_toml(
    contents: &str,
) -> Result<Vec<SettingsCollectionField>> {
    let raw = validated_raw_repository_settings_from_toml(contents)?;
    Ok(raw.explicit_empty_collection_fields())
}

pub fn present_collection_fields_from_toml(contents: &str) -> Result<Vec<SettingsCollectionField>> {
    let raw = validated_raw_repository_settings_from_toml(contents)?;
    Ok(raw.present_collection_fields())
}

fn validated_raw_repository_settings_from_toml(contents: &str) -> Result<RawRepositorySettings> {
    let raw: RawRepositorySettings =
        toml::from_str(contents).context("parse repository settings")?;
    validate_repository_settings(&raw.clone().into_settings())?;
    Ok(raw)
}

pub fn validate_repository_settings(settings: &RepositorySettings) -> Result<()> {
    if let Some(run_mode) = settings.scripts.run_mode.as_deref() {
        anyhow::ensure!(
            matches!(run_mode, "concurrent" | "nonconcurrent"),
            "scripts.run_mode must be concurrent or nonconcurrent"
        );
    }
    for (label, command) in [
        ("scripts.setup", settings.scripts.setup.as_deref()),
        ("scripts.run", settings.scripts.run.as_deref()),
        ("scripts.archive", settings.scripts.archive.as_deref()),
        ("scripts.test", settings.scripts.test.as_deref()),
        ("scripts.lint", settings.scripts.lint.as_deref()),
        ("scripts.typecheck", settings.scripts.typecheck.as_deref()),
        ("scripts.build", settings.scripts.build.as_deref()),
        (
            "customization.automation.test_command",
            settings.customization.automation.test_command.as_deref(),
        ),
        (
            "customization.automation.lint_command",
            settings.customization.automation.lint_command.as_deref(),
        ),
        (
            "customization.automation.typecheck_command",
            settings
                .customization
                .automation
                .typecheck_command
                .as_deref(),
        ),
        (
            "customization.automation.build_command",
            settings.customization.automation.build_command.as_deref(),
        ),
    ] {
        if let Some(command) = command {
            anyhow::ensure!(!command.contains('\0'), "{label} cannot contain NUL bytes");
        }
    }
    if let Some(active) = settings.prompt_pack.active.as_deref() {
        anyhow::ensure!(
            !active.trim().is_empty() && !active.contains('\0'),
            "prompt_pack.active must not be empty or contain NUL bytes"
        );
    }
    if let Some(path) = settings.prompt_pack.path.as_deref() {
        anyhow::ensure!(
            active_prompt_pack_path(path).is_some(),
            "prompt_pack.path must name a file directly under .archductor/prompt-packs"
        );
    }
    for (key, _) in &settings.environment_variables {
        anyhow::ensure!(
            is_valid_environment_key(key),
            "environment variable key {key:?} is invalid"
        );
    }
    for path in &settings.env_file_refs {
        anyhow::ensure!(
            is_safe_relative_path(path),
            "env_file_refs entry must be a safe relative path: {path}"
        );
    }
    if let Some(port_block_size) = settings.customization.workspace_defaults.port_block_size {
        anyhow::ensure!(
            port_block_size > 0,
            "workspace default port_block_size must be greater than 0"
        );
    }
    if let Some(working_directory) = settings
        .customization
        .workspace_defaults
        .working_directory
        .as_deref()
    {
        anyhow::ensure!(
            is_safe_relative_path(working_directory),
            "workspace default working_directory must be a safe relative path"
        );
    }
    if let Some(tab) = settings
        .customization
        .workspace_defaults
        .default_visible_tab
        .as_deref()
    {
        anyhow::ensure!(
            is_valid_workspace_tab(tab),
            "workspace default_visible_tab must be chats, changes, review, checks, todos, processes, terminal, or checkpoints"
        );
    }
    if let Some(method) = settings
        .customization
        .naming
        .default_merge_method
        .as_deref()
    {
        anyhow::ensure!(
            matches!(method, "squash" | "merge" | "rebase"),
            "customization.naming.default_merge_method must be squash, merge, or rebase"
        );
    }
    for (key, value) in &settings.customization.view.colors {
        anyhow::ensure!(
            is_valid_view_color_key(key),
            "customization.view.colors.{key} is not a supported color key"
        );
        anyhow::ensure!(
            is_valid_hex_color(value),
            "customization.view.colors.{key} must be a hex color like #5b9dff"
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct RawRepositorySettings {
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    schema: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_include_globs: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    env_file_refs: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    spotlight_testing: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    enterprise_data_privacy: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scripts: Option<RawScriptSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    environment_variables: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_pack: Option<RawPromptPackSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompts: Option<RawPromptSettings>,
    #[serde(flatten)]
    providers: RawProviderSettings,
    #[serde(skip_serializing_if = "Option::is_none")]
    git: Option<RawGitSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    customization: Option<RawCustomizationSettings>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct RawPromptSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    new_workspace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    general: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    continue_work: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summarize_session: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    handoff: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code_review: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    create_pr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fix_errors: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolve_merge_conflicts: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rename_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    commit_generation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    test_fixing: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    refactor_style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    setup_script: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    run_script: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct RawPromptPackFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    prompts: RawPromptSettings,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct RawPromptPackSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    active: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct RawScriptSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    setup: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    run: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    archive: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    test: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    typecheck: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    build: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    run_mode: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct RawProviderSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    claude_code_executable_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    codex_executable_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    claude_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    codex_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bedrock_region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vertex_project_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct RawGitSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    delete_branch_on_archive: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    archive_on_merge: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    worktree_push_auto_setup_remote: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch_prefix_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch_prefix: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct RawCustomizationSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    naming: Option<RawNamingSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    automation: Option<RawAutomationSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_profiles: Option<BTreeMap<String, RawAgentProfileSettings>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    merge_rules: Option<RawMergeRuleSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_defaults: Option<RawWorkspaceDefaultSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    view: Option<RawViewSettings>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct RawNamingSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    branch_template: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_name_style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    commit_style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pr_title_template: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pr_body_sections: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_merge_method: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct RawAutomationSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    auto_setup: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auto_start_agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    required_local_files: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    test_command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lint_command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    typecheck_command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    build_command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pre_clone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    post_clone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pre_workspace_create: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    post_workspace_create: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pre_setup: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    post_setup: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pre_pr_create: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    post_pr_create: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pre_merge: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    post_merge: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pre_archive: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    post_archive: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct RawAgentProfileSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    approval_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    personality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mcp_servers: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct RawMergeRuleSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    block_on_open_todos: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    block_on_open_comments: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    block_on_failed_checks: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    block_on_pending_checks: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    definition_of_done: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct RawWorkspaceDefaultSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    base_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_parent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    working_directory: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    port_block_size: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auto_open: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    checkpoint_timing: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_visible_tab: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct RawViewSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    accent_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    colors: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    density: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sidebar_layout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    diff_preference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    terminal_font: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    terminal_scrollback: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transcript_display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dashboard_columns: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    notification_rules: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    keybindings: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    command_palette_presets: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    settings_import_export: Option<String>,
}

impl RawRepositorySettings {
    fn merge(self, local: Self) -> Self {
        Self {
            file_include_globs: local.file_include_globs.or(self.file_include_globs),
            env_file_refs: local.env_file_refs.or(self.env_file_refs),
            spotlight_testing: local.spotlight_testing.or(self.spotlight_testing),
            enterprise_data_privacy: local
                .enterprise_data_privacy
                .or(self.enterprise_data_privacy),
            scripts: Some(
                self.scripts
                    .unwrap_or_default()
                    .merge(local.scripts.unwrap_or_default()),
            ),
            environment_variables: merge_optional_maps(
                self.environment_variables,
                local.environment_variables,
            ),
            prompt_pack: Some(
                self.prompt_pack
                    .unwrap_or_default()
                    .merge(local.prompt_pack.unwrap_or_default()),
            ),
            prompts: Some(
                self.prompts
                    .unwrap_or_default()
                    .merge(local.prompts.unwrap_or_default()),
            ),
            providers: self.providers.merge(local.providers),
            git: Some(
                self.git
                    .unwrap_or_default()
                    .merge(local.git.unwrap_or_default()),
            ),
            customization: Some(
                self.customization
                    .unwrap_or_default()
                    .merge(local.customization.unwrap_or_default()),
            ),
            schema: local.schema.or(self.schema),
        }
    }

    fn into_settings(self) -> RepositorySettings {
        let scripts = self.scripts.unwrap_or_default();
        RepositorySettings {
            file_include_globs: split_patterns(self.file_include_globs),
            env_file_refs: split_patterns(self.env_file_refs),
            spotlight_testing: self.spotlight_testing,
            enterprise_data_privacy: self.enterprise_data_privacy,
            scripts: ScriptSettings {
                setup: scripts.setup,
                run: scripts.run,
                archive: scripts.archive,
                test: scripts.test,
                lint: scripts.lint,
                typecheck: scripts.typecheck,
                build: scripts.build,
                run_mode: scripts.run_mode,
            },
            environment_variables: self
                .environment_variables
                .unwrap_or_default()
                .into_iter()
                .collect(),
            prompt_pack: self.prompt_pack.unwrap_or_default().into_settings(),
            prompts: self.prompts.map(RawPromptSettings::into_settings),
            providers: self.providers.into_settings(),
            git: self.git.unwrap_or_default().into_settings(),
            customization: self.customization.unwrap_or_default().into_settings(),
        }
    }

    fn from_settings(settings: &RepositorySettings) -> Self {
        Self {
            schema: Some("https://conductor.build/schemas/settings.repo.schema.json".to_owned()),
            file_include_globs: (!settings.file_include_globs.is_empty())
                .then(|| settings.file_include_globs.join("\n")),
            env_file_refs: (!settings.env_file_refs.is_empty())
                .then(|| settings.env_file_refs.join("\n")),
            spotlight_testing: settings.spotlight_testing,
            enterprise_data_privacy: settings.enterprise_data_privacy,
            scripts: Some(RawScriptSettings {
                setup: settings.scripts.setup.clone(),
                run: settings.scripts.run.clone(),
                archive: settings.scripts.archive.clone(),
                test: settings.scripts.test.clone(),
                lint: settings.scripts.lint.clone(),
                typecheck: settings.scripts.typecheck.clone(),
                build: settings.scripts.build.clone(),
                run_mode: settings.scripts.run_mode.clone(),
            }),
            environment_variables: (!settings.environment_variables.is_empty()).then(|| {
                settings
                    .environment_variables
                    .iter()
                    .cloned()
                    .collect::<BTreeMap<_, _>>()
            }),
            prompt_pack: Some(RawPromptPackSettings::from_settings(&settings.prompt_pack)),
            prompts: settings
                .prompts
                .as_ref()
                .map(RawPromptSettings::from_settings),
            providers: RawProviderSettings::from_settings(&settings.providers),
            git: Some(RawGitSettings::from_settings(&settings.git)),
            customization: Some(RawCustomizationSettings::from_settings(
                &settings.customization,
            )),
        }
    }

    fn from_settings_preserving_empty_collections(
        settings: &RepositorySettings,
        original: &Self,
    ) -> Self {
        let mut raw = Self::from_settings(settings);
        if settings.file_include_globs.is_empty() && original.file_include_globs.is_some() {
            raw.file_include_globs = Some(String::new());
        }
        if settings.env_file_refs.is_empty() && original.env_file_refs.is_some() {
            raw.env_file_refs = Some(String::new());
        }
        if settings.environment_variables.is_empty() && original.environment_variables.is_some() {
            raw.environment_variables = Some(BTreeMap::new());
        }

        let original_customization = original.customization.as_ref();
        let raw_customization = raw
            .customization
            .get_or_insert_with(RawCustomizationSettings::default);
        if settings.customization.agent_profiles.is_empty()
            && original_customization
                .and_then(|customization| customization.agent_profiles.as_ref())
                .is_some()
        {
            raw_customization.agent_profiles = Some(BTreeMap::new());
        } else if let (Some(raw_profiles), Some(original_profiles)) = (
            raw_customization.agent_profiles.as_mut(),
            original_customization.and_then(|customization| customization.agent_profiles.as_ref()),
        ) {
            for (name, profile) in &settings.customization.agent_profiles {
                if profile.mcp_servers.is_empty()
                    && original_profiles
                        .get(name)
                        .and_then(|profile| profile.mcp_servers.as_ref())
                        .is_some()
                {
                    raw_profiles.entry(name.clone()).or_default().mcp_servers = Some(Vec::new());
                }
            }
        }

        let original_naming =
            original_customization.and_then(|customization| customization.naming.as_ref());
        if settings.customization.naming.pr_body_sections.is_empty()
            && original_naming
                .and_then(|naming| naming.pr_body_sections.as_ref())
                .is_some()
        {
            raw_customization
                .naming
                .get_or_insert_with(RawNamingSettings::default)
                .pr_body_sections = Some(Vec::new());
        }

        let original_automation =
            original_customization.and_then(|customization| customization.automation.as_ref());
        if settings
            .customization
            .automation
            .required_local_files
            .is_empty()
            && original_automation
                .and_then(|automation| automation.required_local_files.as_ref())
                .is_some()
        {
            raw_customization
                .automation
                .get_or_insert_with(RawAutomationSettings::default)
                .required_local_files = Some(Vec::new());
        }

        let original_view =
            original_customization.and_then(|customization| customization.view.as_ref());
        let raw_view = raw_customization
            .view
            .get_or_insert_with(RawViewSettings::default);
        if settings.customization.view.colors.is_empty()
            && original_view
                .and_then(|view| view.colors.as_ref())
                .is_some()
        {
            raw_view.colors = Some(BTreeMap::new());
        }
        if settings.customization.view.dashboard_columns.is_empty()
            && original_view
                .and_then(|view| view.dashboard_columns.as_ref())
                .is_some()
        {
            raw_view.dashboard_columns = Some(Vec::new());
        }
        if settings.customization.view.notification_rules.is_empty()
            && original_view
                .and_then(|view| view.notification_rules.as_ref())
                .is_some()
        {
            raw_view.notification_rules = Some(Vec::new());
        }
        if settings
            .customization
            .view
            .command_palette_presets
            .is_empty()
            && original_view
                .and_then(|view| view.command_palette_presets.as_ref())
                .is_some()
        {
            raw_view.command_palette_presets = Some(Vec::new());
        }
        raw
    }

    fn apply_explicit_empty_collections(
        &mut self,
        settings: &RepositorySettings,
        fields: &[SettingsCollectionField],
    ) {
        if settings.file_include_globs.is_empty()
            && fields.contains(&SettingsCollectionField::FileIncludeGlobs)
        {
            self.file_include_globs = Some(String::new());
        }
        if settings.env_file_refs.is_empty()
            && fields.contains(&SettingsCollectionField::EnvFileRefs)
        {
            self.env_file_refs = Some(String::new());
        }
        if settings.environment_variables.is_empty()
            && fields.contains(&SettingsCollectionField::EnvironmentVariables)
        {
            self.environment_variables = Some(BTreeMap::new());
        }

        let customization = self
            .customization
            .get_or_insert_with(RawCustomizationSettings::default);
        if settings.customization.agent_profiles.is_empty()
            && fields.contains(&SettingsCollectionField::AgentProfiles)
        {
            customization.agent_profiles = Some(BTreeMap::new());
        } else {
            for field in fields {
                let SettingsCollectionField::AgentProfileMcpServers(name) = field else {
                    continue;
                };
                let Some(profile) = settings.customization.agent_profiles.get(name) else {
                    continue;
                };
                if profile.mcp_servers.is_empty() {
                    customization
                        .agent_profiles
                        .get_or_insert_with(BTreeMap::new)
                        .entry(name.clone())
                        .or_default()
                        .mcp_servers = Some(Vec::new());
                }
            }
        }
        if settings.customization.naming.pr_body_sections.is_empty()
            && fields.contains(&SettingsCollectionField::PrBodySections)
        {
            customization
                .naming
                .get_or_insert_with(RawNamingSettings::default)
                .pr_body_sections = Some(Vec::new());
        }
        if settings
            .customization
            .automation
            .required_local_files
            .is_empty()
            && fields.contains(&SettingsCollectionField::RequiredLocalFiles)
        {
            customization
                .automation
                .get_or_insert_with(RawAutomationSettings::default)
                .required_local_files = Some(Vec::new());
        }

        let view = customization
            .view
            .get_or_insert_with(RawViewSettings::default);
        if settings.customization.view.colors.is_empty()
            && fields.contains(&SettingsCollectionField::ViewColors)
        {
            view.colors = Some(BTreeMap::new());
        }
        if settings.customization.view.dashboard_columns.is_empty()
            && fields.contains(&SettingsCollectionField::DashboardColumns)
        {
            view.dashboard_columns = Some(Vec::new());
        }
        if settings.customization.view.notification_rules.is_empty()
            && fields.contains(&SettingsCollectionField::NotificationRules)
        {
            view.notification_rules = Some(Vec::new());
        }
        if settings
            .customization
            .view
            .command_palette_presets
            .is_empty()
            && fields.contains(&SettingsCollectionField::CommandPalettePresets)
        {
            view.command_palette_presets = Some(Vec::new());
        }
    }

    fn unset_collections(&mut self, fields: &[SettingsCollectionField]) {
        for field in fields {
            match field {
                SettingsCollectionField::FileIncludeGlobs => self.file_include_globs = None,
                SettingsCollectionField::EnvFileRefs => self.env_file_refs = None,
                SettingsCollectionField::EnvironmentVariables => {
                    self.environment_variables = None;
                }
                SettingsCollectionField::AgentProfiles => {
                    if let Some(customization) = self.customization.as_mut() {
                        customization.agent_profiles = None;
                    }
                }
                SettingsCollectionField::AgentProfileMcpServers(name) => {
                    if let Some(profile) = self
                        .customization
                        .as_mut()
                        .and_then(|customization| customization.agent_profiles.as_mut())
                        .and_then(|profiles| profiles.get_mut(name))
                    {
                        profile.mcp_servers = None;
                    }
                }
                SettingsCollectionField::PrBodySections => {
                    if let Some(naming) = self
                        .customization
                        .as_mut()
                        .and_then(|customization| customization.naming.as_mut())
                    {
                        naming.pr_body_sections = None;
                    }
                }
                SettingsCollectionField::RequiredLocalFiles => {
                    if let Some(automation) = self
                        .customization
                        .as_mut()
                        .and_then(|customization| customization.automation.as_mut())
                    {
                        automation.required_local_files = None;
                    }
                }
                SettingsCollectionField::ViewColors => {
                    if let Some(view) = self
                        .customization
                        .as_mut()
                        .and_then(|customization| customization.view.as_mut())
                    {
                        view.colors = None;
                    }
                }
                SettingsCollectionField::DashboardColumns => {
                    if let Some(view) = self
                        .customization
                        .as_mut()
                        .and_then(|customization| customization.view.as_mut())
                    {
                        view.dashboard_columns = None;
                    }
                }
                SettingsCollectionField::NotificationRules => {
                    if let Some(view) = self
                        .customization
                        .as_mut()
                        .and_then(|customization| customization.view.as_mut())
                    {
                        view.notification_rules = None;
                    }
                }
                SettingsCollectionField::CommandPalettePresets => {
                    if let Some(view) = self
                        .customization
                        .as_mut()
                        .and_then(|customization| customization.view.as_mut())
                    {
                        view.command_palette_presets = None;
                    }
                }
            }
        }
    }

    fn explicit_empty_collection_fields(&self) -> Vec<SettingsCollectionField> {
        let mut fields = Vec::new();
        if self
            .file_include_globs
            .as_ref()
            .is_some_and(|value| split_patterns(Some(value.clone())).is_empty())
        {
            fields.push(SettingsCollectionField::FileIncludeGlobs);
        }
        if self
            .env_file_refs
            .as_ref()
            .is_some_and(|value| split_patterns(Some(value.clone())).is_empty())
        {
            fields.push(SettingsCollectionField::EnvFileRefs);
        }
        if self
            .environment_variables
            .as_ref()
            .is_some_and(BTreeMap::is_empty)
        {
            fields.push(SettingsCollectionField::EnvironmentVariables);
        }
        let Some(customization) = self.customization.as_ref() else {
            return fields;
        };
        if customization
            .agent_profiles
            .as_ref()
            .is_some_and(BTreeMap::is_empty)
        {
            fields.push(SettingsCollectionField::AgentProfiles);
        } else if let Some(profiles) = customization.agent_profiles.as_ref() {
            for (name, profile) in profiles {
                if profile.mcp_servers.as_ref().is_some_and(Vec::is_empty) {
                    fields.push(SettingsCollectionField::AgentProfileMcpServers(
                        name.clone(),
                    ));
                }
            }
        }
        if customization
            .naming
            .as_ref()
            .and_then(|naming| naming.pr_body_sections.as_ref())
            .is_some_and(Vec::is_empty)
        {
            fields.push(SettingsCollectionField::PrBodySections);
        }
        if customization
            .automation
            .as_ref()
            .and_then(|automation| automation.required_local_files.as_ref())
            .is_some_and(Vec::is_empty)
        {
            fields.push(SettingsCollectionField::RequiredLocalFiles);
        }
        let Some(view) = customization.view.as_ref() else {
            return fields;
        };
        if view.colors.as_ref().is_some_and(BTreeMap::is_empty) {
            fields.push(SettingsCollectionField::ViewColors);
        }
        if view.dashboard_columns.as_ref().is_some_and(Vec::is_empty) {
            fields.push(SettingsCollectionField::DashboardColumns);
        }
        if view.notification_rules.as_ref().is_some_and(Vec::is_empty) {
            fields.push(SettingsCollectionField::NotificationRules);
        }
        if view
            .command_palette_presets
            .as_ref()
            .is_some_and(Vec::is_empty)
        {
            fields.push(SettingsCollectionField::CommandPalettePresets);
        }
        fields
    }

    fn present_collection_fields(&self) -> Vec<SettingsCollectionField> {
        let mut fields = Vec::new();
        if self.file_include_globs.is_some() {
            fields.push(SettingsCollectionField::FileIncludeGlobs);
        }
        if self.env_file_refs.is_some() {
            fields.push(SettingsCollectionField::EnvFileRefs);
        }
        if self.environment_variables.is_some() {
            fields.push(SettingsCollectionField::EnvironmentVariables);
        }
        let Some(customization) = self.customization.as_ref() else {
            return fields;
        };
        if let Some(profiles) = customization.agent_profiles.as_ref() {
            fields.push(SettingsCollectionField::AgentProfiles);
            for (name, profile) in profiles {
                if profile.mcp_servers.is_some() {
                    fields.push(SettingsCollectionField::AgentProfileMcpServers(
                        name.clone(),
                    ));
                }
            }
        }
        if customization
            .naming
            .as_ref()
            .is_some_and(|naming| naming.pr_body_sections.is_some())
        {
            fields.push(SettingsCollectionField::PrBodySections);
        }
        if customization
            .automation
            .as_ref()
            .is_some_and(|automation| automation.required_local_files.is_some())
        {
            fields.push(SettingsCollectionField::RequiredLocalFiles);
        }
        let Some(view) = customization.view.as_ref() else {
            return fields;
        };
        if view.colors.is_some() {
            fields.push(SettingsCollectionField::ViewColors);
        }
        if view.dashboard_columns.is_some() {
            fields.push(SettingsCollectionField::DashboardColumns);
        }
        if view.notification_rules.is_some() {
            fields.push(SettingsCollectionField::NotificationRules);
        }
        if view.command_palette_presets.is_some() {
            fields.push(SettingsCollectionField::CommandPalettePresets);
        }
        fields
    }
}

impl RawScriptSettings {
    fn merge(self, local: Self) -> Self {
        Self {
            setup: local.setup.or(self.setup),
            run: local.run.or(self.run),
            archive: local.archive.or(self.archive),
            test: local.test.or(self.test),
            lint: local.lint.or(self.lint),
            typecheck: local.typecheck.or(self.typecheck),
            build: local.build.or(self.build),
            run_mode: local.run_mode.or(self.run_mode),
        }
    }
}

impl RawPromptSettings {
    fn merge(self, local: Self) -> Self {
        Self {
            new_workspace: local.new_workspace.or(self.new_workspace),
            general: local.general.or(self.general),
            continue_work: local.continue_work.or(self.continue_work),
            summarize_session: local.summarize_session.or(self.summarize_session),
            handoff: local.handoff.or(self.handoff),
            code_review: local.code_review.or(self.code_review),
            create_pr: local.create_pr.or(self.create_pr),
            fix_errors: local.fix_errors.or(self.fix_errors),
            resolve_merge_conflicts: local
                .resolve_merge_conflicts
                .or(self.resolve_merge_conflicts),
            rename_branch: local.rename_branch.or(self.rename_branch),
            commit_generation: local.commit_generation.or(self.commit_generation),
            test_fixing: local.test_fixing.or(self.test_fixing),
            refactor_style: local.refactor_style.or(self.refactor_style),
            setup_script: local.setup_script.or(self.setup_script),
            run_script: local.run_script.or(self.run_script),
        }
    }

    fn into_settings(self) -> PromptSettings {
        PromptSettings {
            new_workspace: self.new_workspace,
            general: self.general,
            continue_work: self.continue_work,
            summarize_session: self.summarize_session,
            handoff: self.handoff,
            code_review: self.code_review,
            create_pr: self.create_pr,
            fix_errors: self.fix_errors,
            resolve_merge_conflicts: self.resolve_merge_conflicts,
            rename_branch: self.rename_branch,
            commit_generation: self.commit_generation,
            test_fixing: self.test_fixing,
            refactor_style: self.refactor_style,
            setup_script: self.setup_script,
            run_script: self.run_script,
        }
    }

    fn from_settings(settings: &PromptSettings) -> Self {
        Self {
            new_workspace: settings.new_workspace.clone(),
            general: settings.general.clone(),
            continue_work: settings.continue_work.clone(),
            summarize_session: settings.summarize_session.clone(),
            handoff: settings.handoff.clone(),
            code_review: settings.code_review.clone(),
            create_pr: settings.create_pr.clone(),
            fix_errors: settings.fix_errors.clone(),
            resolve_merge_conflicts: settings.resolve_merge_conflicts.clone(),
            rename_branch: settings.rename_branch.clone(),
            commit_generation: settings.commit_generation.clone(),
            test_fixing: settings.test_fixing.clone(),
            refactor_style: settings.refactor_style.clone(),
            setup_script: settings.setup_script.clone(),
            run_script: settings.run_script.clone(),
        }
    }
}

impl RawPromptPackSettings {
    fn merge(self, local: Self) -> Self {
        Self {
            active: local.active.or(self.active),
            version: local.version.or(self.version),
            path: local.path.or(self.path),
        }
    }

    fn into_settings(self) -> PromptPackSettings {
        PromptPackSettings {
            active: self.active,
            version: self.version,
            path: self.path,
        }
    }

    fn from_settings(settings: &PromptPackSettings) -> Self {
        Self {
            active: settings.active.clone(),
            version: settings.version.clone(),
            path: settings.path.clone(),
        }
    }
}

impl RawProviderSettings {
    fn merge(self, local: Self) -> Self {
        Self {
            claude_code_executable_path: local
                .claude_code_executable_path
                .or(self.claude_code_executable_path),
            codex_executable_path: local.codex_executable_path.or(self.codex_executable_path),
            claude_provider: local.claude_provider.or(self.claude_provider),
            codex_provider: local.codex_provider.or(self.codex_provider),
            bedrock_region: local.bedrock_region.or(self.bedrock_region),
            vertex_project_id: local.vertex_project_id.or(self.vertex_project_id),
        }
    }

    fn into_settings(self) -> ProviderSettings {
        ProviderSettings {
            claude_code_executable_path: self.claude_code_executable_path,
            codex_executable_path: self.codex_executable_path,
            claude_provider: self.claude_provider,
            codex_provider: self.codex_provider,
            bedrock_region: self.bedrock_region,
            vertex_project_id: self.vertex_project_id,
        }
    }

    fn from_settings(settings: &ProviderSettings) -> Self {
        Self {
            claude_code_executable_path: settings.claude_code_executable_path.clone(),
            codex_executable_path: settings.codex_executable_path.clone(),
            claude_provider: settings.claude_provider.clone(),
            codex_provider: settings.codex_provider.clone(),
            bedrock_region: settings.bedrock_region.clone(),
            vertex_project_id: settings.vertex_project_id.clone(),
        }
    }
}

impl RawGitSettings {
    fn merge(self, local: Self) -> Self {
        Self {
            delete_branch_on_archive: local
                .delete_branch_on_archive
                .or(self.delete_branch_on_archive),
            archive_on_merge: local.archive_on_merge.or(self.archive_on_merge),
            worktree_push_auto_setup_remote: local
                .worktree_push_auto_setup_remote
                .or(self.worktree_push_auto_setup_remote),
            branch_prefix_type: local.branch_prefix_type.or(self.branch_prefix_type),
            branch_prefix: local.branch_prefix.or(self.branch_prefix),
        }
    }

    fn into_settings(self) -> GitSettings {
        GitSettings {
            delete_branch_on_archive: self.delete_branch_on_archive,
            archive_on_merge: self.archive_on_merge,
            worktree_push_auto_setup_remote: self.worktree_push_auto_setup_remote,
            branch_prefix_type: self.branch_prefix_type,
            branch_prefix: self.branch_prefix,
        }
    }

    fn from_settings(settings: &GitSettings) -> Self {
        Self {
            delete_branch_on_archive: settings.delete_branch_on_archive,
            archive_on_merge: settings.archive_on_merge,
            worktree_push_auto_setup_remote: settings.worktree_push_auto_setup_remote,
            branch_prefix_type: settings.branch_prefix_type.clone(),
            branch_prefix: settings.branch_prefix.clone(),
        }
    }
}

impl RawCustomizationSettings {
    fn merge(self, local: Self) -> Self {
        Self {
            naming: Some(
                self.naming
                    .unwrap_or_default()
                    .merge(local.naming.unwrap_or_default()),
            ),
            automation: Some(
                self.automation
                    .unwrap_or_default()
                    .merge(local.automation.unwrap_or_default()),
            ),
            agent_profiles: merge_optional_profile_maps(self.agent_profiles, local.agent_profiles),
            merge_rules: Some(
                self.merge_rules
                    .unwrap_or_default()
                    .merge(local.merge_rules.unwrap_or_default()),
            ),
            workspace_defaults: Some(
                self.workspace_defaults
                    .unwrap_or_default()
                    .merge(local.workspace_defaults.unwrap_or_default()),
            ),
            view: Some(
                self.view
                    .unwrap_or_default()
                    .merge(local.view.unwrap_or_default()),
            ),
        }
    }

    fn into_settings(self) -> CustomizationSettings {
        CustomizationSettings {
            naming: self.naming.unwrap_or_default().into_settings(),
            automation: self.automation.unwrap_or_default().into_settings(),
            agent_profiles: self
                .agent_profiles
                .unwrap_or_default()
                .into_iter()
                .map(|(name, profile)| (name, profile.into_settings()))
                .collect(),
            merge_rules: self.merge_rules.unwrap_or_default().into_settings(),
            workspace_defaults: self.workspace_defaults.unwrap_or_default().into_settings(),
            view: self.view.unwrap_or_default().into_settings(),
        }
    }

    fn from_settings(settings: &CustomizationSettings) -> Self {
        Self {
            naming: Some(RawNamingSettings::from_settings(&settings.naming)),
            automation: Some(RawAutomationSettings::from_settings(&settings.automation)),
            agent_profiles: (!settings.agent_profiles.is_empty()).then(|| {
                settings
                    .agent_profiles
                    .iter()
                    .map(|(name, profile)| {
                        (
                            name.clone(),
                            RawAgentProfileSettings::from_settings(profile),
                        )
                    })
                    .collect()
            }),
            merge_rules: Some(RawMergeRuleSettings::from_settings(&settings.merge_rules)),
            workspace_defaults: Some(RawWorkspaceDefaultSettings::from_settings(
                &settings.workspace_defaults,
            )),
            view: Some(RawViewSettings::from_settings(&settings.view)),
        }
    }
}

impl RawNamingSettings {
    fn merge(self, local: Self) -> Self {
        Self {
            branch_template: local.branch_template.or(self.branch_template),
            workspace_name_style: local.workspace_name_style.or(self.workspace_name_style),
            commit_style: local.commit_style.or(self.commit_style),
            pr_title_template: local.pr_title_template.or(self.pr_title_template),
            pr_body_sections: local.pr_body_sections.or(self.pr_body_sections),
            default_merge_method: local.default_merge_method.or(self.default_merge_method),
        }
    }

    fn into_settings(self) -> NamingSettings {
        NamingSettings {
            branch_template: self.branch_template,
            workspace_name_style: self.workspace_name_style,
            commit_style: self.commit_style,
            pr_title_template: self.pr_title_template,
            pr_body_sections: self.pr_body_sections.unwrap_or_default(),
            default_merge_method: self.default_merge_method,
        }
    }

    fn from_settings(settings: &NamingSettings) -> Self {
        Self {
            branch_template: settings.branch_template.clone(),
            workspace_name_style: settings.workspace_name_style.clone(),
            commit_style: settings.commit_style.clone(),
            pr_title_template: settings.pr_title_template.clone(),
            pr_body_sections: (!settings.pr_body_sections.is_empty())
                .then(|| settings.pr_body_sections.clone()),
            default_merge_method: settings.default_merge_method.clone(),
        }
    }
}

impl RawAutomationSettings {
    fn merge(self, local: Self) -> Self {
        Self {
            auto_setup: local.auto_setup.or(self.auto_setup),
            auto_start_agent: local.auto_start_agent.or(self.auto_start_agent),
            required_local_files: local.required_local_files.or(self.required_local_files),
            test_command: local.test_command.or(self.test_command),
            lint_command: local.lint_command.or(self.lint_command),
            typecheck_command: local.typecheck_command.or(self.typecheck_command),
            build_command: local.build_command.or(self.build_command),
            pre_clone: local.pre_clone.or(self.pre_clone),
            post_clone: local.post_clone.or(self.post_clone),
            pre_workspace_create: local.pre_workspace_create.or(self.pre_workspace_create),
            post_workspace_create: local.post_workspace_create.or(self.post_workspace_create),
            pre_setup: local.pre_setup.or(self.pre_setup),
            post_setup: local.post_setup.or(self.post_setup),
            pre_pr_create: local.pre_pr_create.or(self.pre_pr_create),
            post_pr_create: local.post_pr_create.or(self.post_pr_create),
            pre_merge: local.pre_merge.or(self.pre_merge),
            post_merge: local.post_merge.or(self.post_merge),
            pre_archive: local.pre_archive.or(self.pre_archive),
            post_archive: local.post_archive.or(self.post_archive),
        }
    }

    fn into_settings(self) -> AutomationSettings {
        AutomationSettings {
            auto_setup: self.auto_setup,
            auto_start_agent: self.auto_start_agent,
            required_local_files: self.required_local_files.unwrap_or_default(),
            test_command: self.test_command,
            lint_command: self.lint_command,
            typecheck_command: self.typecheck_command,
            build_command: self.build_command,
            pre_clone: self.pre_clone,
            post_clone: self.post_clone,
            pre_workspace_create: self.pre_workspace_create,
            post_workspace_create: self.post_workspace_create,
            pre_setup: self.pre_setup,
            post_setup: self.post_setup,
            pre_pr_create: self.pre_pr_create,
            post_pr_create: self.post_pr_create,
            pre_merge: self.pre_merge,
            post_merge: self.post_merge,
            pre_archive: self.pre_archive,
            post_archive: self.post_archive,
        }
    }

    fn from_settings(settings: &AutomationSettings) -> Self {
        Self {
            auto_setup: settings.auto_setup,
            auto_start_agent: settings.auto_start_agent.clone(),
            required_local_files: (!settings.required_local_files.is_empty())
                .then(|| settings.required_local_files.clone()),
            test_command: settings.test_command.clone(),
            lint_command: settings.lint_command.clone(),
            typecheck_command: settings.typecheck_command.clone(),
            build_command: settings.build_command.clone(),
            pre_clone: settings.pre_clone.clone(),
            post_clone: settings.post_clone.clone(),
            pre_workspace_create: settings.pre_workspace_create.clone(),
            post_workspace_create: settings.post_workspace_create.clone(),
            pre_setup: settings.pre_setup.clone(),
            post_setup: settings.post_setup.clone(),
            pre_pr_create: settings.pre_pr_create.clone(),
            post_pr_create: settings.post_pr_create.clone(),
            pre_merge: settings.pre_merge.clone(),
            post_merge: settings.post_merge.clone(),
            pre_archive: settings.pre_archive.clone(),
            post_archive: settings.post_archive.clone(),
        }
    }
}

impl RawAgentProfileSettings {
    fn merge(self, local: Self) -> Self {
        Self {
            agent: local.agent.or(self.agent),
            model: local.model.or(self.model),
            approval_mode: local.approval_mode.or(self.approval_mode),
            reasoning_mode: local.reasoning_mode.or(self.reasoning_mode),
            personality: local.personality.or(self.personality),
            mcp_servers: local.mcp_servers.or(self.mcp_servers),
        }
    }

    fn into_settings(self) -> AgentProfileSettings {
        AgentProfileSettings {
            agent: self.agent,
            model: self.model,
            approval_mode: self.approval_mode,
            reasoning_mode: self.reasoning_mode,
            personality: self.personality,
            mcp_servers: self.mcp_servers.unwrap_or_default(),
        }
    }

    fn from_settings(settings: &AgentProfileSettings) -> Self {
        Self {
            agent: settings.agent.clone(),
            model: settings.model.clone(),
            approval_mode: settings.approval_mode.clone(),
            reasoning_mode: settings.reasoning_mode.clone(),
            personality: settings.personality.clone(),
            mcp_servers: (!settings.mcp_servers.is_empty()).then(|| settings.mcp_servers.clone()),
        }
    }
}

impl RawMergeRuleSettings {
    fn merge(self, local: Self) -> Self {
        Self {
            block_on_open_todos: local.block_on_open_todos.or(self.block_on_open_todos),
            block_on_open_comments: local.block_on_open_comments.or(self.block_on_open_comments),
            block_on_failed_checks: local.block_on_failed_checks.or(self.block_on_failed_checks),
            block_on_pending_checks: local
                .block_on_pending_checks
                .or(self.block_on_pending_checks),
            definition_of_done: local.definition_of_done.or(self.definition_of_done),
        }
    }

    fn into_settings(self) -> MergeRuleSettings {
        MergeRuleSettings {
            block_on_open_todos: self.block_on_open_todos,
            block_on_open_comments: self.block_on_open_comments,
            block_on_failed_checks: self.block_on_failed_checks,
            block_on_pending_checks: self.block_on_pending_checks,
            definition_of_done: self.definition_of_done,
        }
    }

    fn from_settings(settings: &MergeRuleSettings) -> Self {
        Self {
            block_on_open_todos: settings.block_on_open_todos,
            block_on_open_comments: settings.block_on_open_comments,
            block_on_failed_checks: settings.block_on_failed_checks,
            block_on_pending_checks: settings.block_on_pending_checks,
            definition_of_done: settings.definition_of_done.clone(),
        }
    }
}

impl RawWorkspaceDefaultSettings {
    fn merge(self, local: Self) -> Self {
        Self {
            base_branch: local.base_branch.or(self.base_branch),
            workspace_parent: local.workspace_parent.or(self.workspace_parent),
            branch_prefix: local.branch_prefix.or(self.branch_prefix),
            working_directory: local.working_directory.or(self.working_directory),
            port_block_size: local.port_block_size.or(self.port_block_size),
            auto_open: local.auto_open.or(self.auto_open),
            checkpoint_timing: local.checkpoint_timing.or(self.checkpoint_timing),
            default_visible_tab: local.default_visible_tab.or(self.default_visible_tab),
        }
    }

    fn into_settings(self) -> WorkspaceDefaultSettings {
        WorkspaceDefaultSettings {
            base_branch: self.base_branch,
            workspace_parent: self.workspace_parent,
            branch_prefix: self.branch_prefix,
            working_directory: self.working_directory,
            port_block_size: self.port_block_size,
            auto_open: self.auto_open,
            checkpoint_timing: self.checkpoint_timing,
            default_visible_tab: self.default_visible_tab,
        }
    }

    fn from_settings(settings: &WorkspaceDefaultSettings) -> Self {
        Self {
            base_branch: settings.base_branch.clone(),
            workspace_parent: settings.workspace_parent.clone(),
            branch_prefix: settings.branch_prefix.clone(),
            working_directory: settings.working_directory.clone(),
            port_block_size: settings.port_block_size,
            auto_open: settings.auto_open,
            checkpoint_timing: settings.checkpoint_timing.clone(),
            default_visible_tab: settings.default_visible_tab.clone(),
        }
    }
}

impl RawViewSettings {
    fn merge(self, local: Self) -> Self {
        Self {
            theme: local.theme.or(self.theme),
            accent_color: local.accent_color.or(self.accent_color),
            colors: merge_optional_maps(self.colors, local.colors),
            density: local.density.or(self.density),
            sidebar_layout: local.sidebar_layout.or(self.sidebar_layout),
            diff_preference: local.diff_preference.or(self.diff_preference),
            terminal_font: local.terminal_font.or(self.terminal_font),
            terminal_scrollback: local.terminal_scrollback.or(self.terminal_scrollback),
            transcript_display: local.transcript_display.or(self.transcript_display),
            dashboard_columns: local.dashboard_columns.or(self.dashboard_columns),
            notification_rules: local.notification_rules.or(self.notification_rules),
            keybindings: local.keybindings.or(self.keybindings),
            command_palette_presets: local
                .command_palette_presets
                .or(self.command_palette_presets),
            settings_import_export: local.settings_import_export.or(self.settings_import_export),
        }
    }

    fn into_settings(self) -> ViewSettings {
        ViewSettings {
            theme: self.theme,
            accent_color: self.accent_color,
            colors: self.colors.unwrap_or_default(),
            density: self.density,
            sidebar_layout: self.sidebar_layout,
            diff_preference: self.diff_preference,
            terminal_font: self.terminal_font,
            terminal_scrollback: self.terminal_scrollback,
            transcript_display: self.transcript_display,
            dashboard_columns: self.dashboard_columns.unwrap_or_default(),
            notification_rules: self.notification_rules.unwrap_or_default(),
            keybindings: self.keybindings,
            command_palette_presets: self.command_palette_presets.unwrap_or_default(),
            settings_import_export: self.settings_import_export,
        }
    }

    fn from_settings(settings: &ViewSettings) -> Self {
        Self {
            theme: settings.theme.clone(),
            accent_color: settings.accent_color.clone(),
            colors: (!settings.colors.is_empty()).then(|| settings.colors.clone()),
            density: settings.density.clone(),
            sidebar_layout: settings.sidebar_layout.clone(),
            diff_preference: settings.diff_preference.clone(),
            terminal_font: settings.terminal_font.clone(),
            terminal_scrollback: settings.terminal_scrollback,
            transcript_display: settings.transcript_display.clone(),
            dashboard_columns: (!settings.dashboard_columns.is_empty())
                .then(|| settings.dashboard_columns.clone()),
            notification_rules: (!settings.notification_rules.is_empty())
                .then(|| settings.notification_rules.clone()),
            keybindings: settings.keybindings.clone(),
            command_palette_presets: (!settings.command_palette_presets.is_empty())
                .then(|| settings.command_palette_presets.clone()),
            settings_import_export: settings.settings_import_export.clone(),
        }
    }
}

fn merge_profile_maps(
    mut shared: BTreeMap<String, RawAgentProfileSettings>,
    local: BTreeMap<String, RawAgentProfileSettings>,
) -> BTreeMap<String, RawAgentProfileSettings> {
    for (name, local_profile) in local {
        let profile = shared
            .remove(&name)
            .unwrap_or_default()
            .merge(local_profile);
        shared.insert(name, profile);
    }
    shared
}

fn merge_optional_profile_maps(
    shared: Option<BTreeMap<String, RawAgentProfileSettings>>,
    local: Option<BTreeMap<String, RawAgentProfileSettings>>,
) -> Option<BTreeMap<String, RawAgentProfileSettings>> {
    match local {
        None => shared,
        Some(local) if local.is_empty() => Some(local),
        Some(local) => Some(merge_profile_maps(shared.unwrap_or_default(), local)),
    }
}

fn load_optional_settings(path: &Path) -> Result<RawRepositorySettings> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            anyhow::ensure!(
                !metadata.file_type().is_symlink(),
                "{} must not be a symlink",
                path.display()
            );
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {
            return Ok(RawRepositorySettings::default());
        }
        Err(err) => return Err(err).with_context(|| format!("inspect {}", path.display())),
    }

    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("read settings {}", path.display()))?;
    toml::from_str(&contents).with_context(|| format!("parse settings {}", path.display()))
}

fn split_patterns(patterns: Option<String>) -> Vec<String> {
    patterns
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

fn merge_maps(
    mut shared: BTreeMap<String, String>,
    local: BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    shared.extend(local);
    shared
}

fn merge_optional_maps(
    shared: Option<BTreeMap<String, String>>,
    local: Option<BTreeMap<String, String>>,
) -> Option<BTreeMap<String, String>> {
    match local {
        None => shared,
        Some(local) if local.is_empty() => Some(local),
        Some(local) => Some(merge_maps(shared.unwrap_or_default(), local)),
    }
}

pub(crate) fn is_valid_environment_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(first) if first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn is_valid_view_color_key(key: &str) -> bool {
    matches!(
        key,
        "background"
            | "surface"
            | "surface_raised"
            | "surface_muted"
            | "hover"
            | "hover_soft"
            | "border"
            | "border_strong"
            | "text"
            | "text_strong"
            | "text_muted"
            | "accent"
            | "accent_fg"
            | "success"
            | "warning"
            | "danger"
    )
}

fn is_valid_hex_color(value: &str) -> bool {
    let Some(hex) = value.strip_prefix('#') else {
        return false;
    };
    matches!(hex.len(), 3 | 6) && hex.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn is_safe_relative_path(path: &str) -> bool {
    let path = Path::new(path);
    !path.as_os_str().is_empty()
        && !path.as_os_str().as_encoded_bytes().contains(&0)
        && path.is_relative()
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

fn is_valid_workspace_tab(value: &str) -> bool {
    matches!(
        normalize_workspace_tab(value).as_str(),
        "chats"
            | "chat"
            | "chatterminal"
            | "session"
            | "sessions"
            | "changes"
            | "diff"
            | "files"
            | "review"
            | "comments"
            | "checks"
            | "ci"
            | "pr"
            | "pullrequest"
            | "todos"
            | "tasks"
            | "processes"
            | "runs"
            | "terminal"
            | "term"
            | "shell"
            | "bigterminal"
            | "checkpoints"
            | "restore"
    )
}

fn validate_agent_provider(provider: &str) -> Result<()> {
    anyhow::ensure!(
        crate::agent_tools::supported_agent_provider_key(provider).is_some(),
        "default agent provider must be codex, claude, or opencode"
    );
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct PromptPackBootstrap {
    prompt_pack_dir_created: bool,
    default_prompt_pack_created: bool,
    active_prompt_pack_created: bool,
}

fn ensure_default_prompt_pack_files(
    repo_path: &Path,
    conductor_dir: &Path,
    settings: &RepositorySettings,
) -> Result<PromptPackBootstrap> {
    let prompt_pack_dir = conductor_dir.join("prompt-packs");
    let prompt_pack_dir_created = ensure_real_directory(&prompt_pack_dir)?;
    let default_path = prompt_pack_dir.join("default.toml");
    let default_prompt_pack_created =
        ensure_prompt_pack_file(&default_path, "default", Some("v1"))?;

    let active_prompt_pack_created = settings
        .prompt_pack
        .path
        .as_deref()
        .and_then(active_prompt_pack_path)
        .map(|relative_path| repo_path.join(relative_path))
        .filter(|active_path| active_path != &default_path)
        .map(|active_path| {
            let active = settings.prompt_pack.active.as_deref().unwrap_or("default");
            ensure_prompt_pack_file(
                &active_path,
                active,
                settings.prompt_pack.version.as_deref(),
            )
        })
        .transpose()?
        .unwrap_or(false);

    Ok(PromptPackBootstrap {
        prompt_pack_dir_created,
        default_prompt_pack_created,
        active_prompt_pack_created,
    })
}

fn ensure_context_gitignored(repo_path: &Path) -> Result<bool> {
    let gitignore_path = repo_path.join(".gitignore");
    let existing = match fs::read_to_string(&gitignore_path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err).with_context(|| format!("read {}", gitignore_path.display())),
    };

    let mut lines = existing.lines().map(str::to_owned).collect::<Vec<_>>();
    let had_context = lines
        .iter()
        .any(|line| gitignore_pattern_key(line).as_deref() == Some(".context"));
    let had_local_settings = lines.iter().any(|line| {
        matches!(
            gitignore_pattern_key(line).as_deref(),
            Some(".archductor/settings.local.toml" | ".archductor/settings.local.toml*")
        )
    });
    let original_len = lines.len();
    lines.retain(|line| !is_managed_archductor_ignore_pattern(line));
    let mut changed = lines.len() != original_len;

    if !had_context {
        lines.push(".context/".to_owned());
        changed = true;
    }
    if lines
        .iter()
        .any(|line| is_archductor_directory_ignore_pattern(line))
    {
        let unignore_rules = [
            "!.archductor/",
            "!.archductor/settings.toml",
            "!.archductor/prompt-packs/",
            "!.archductor/prompt-packs/*.toml",
        ];
        for rule in unignore_rules {
            if !lines.iter().any(|line| line.trim() == rule) {
                lines.push(rule.to_owned());
                changed = true;
            }
        }
    }
    if !had_local_settings {
        lines.push(".archductor/settings.local.toml*".to_owned());
        changed = true;
    }

    if !changed {
        return Ok(false);
    }

    let mut contents = lines.join("\n");
    contents.push('\n');
    atomic_write_no_symlink(&gitignore_path, contents.as_bytes())
        .with_context(|| format!("write {}", gitignore_path.display()))?;
    Ok(true)
}

pub(crate) fn gitignore_pattern_key(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('!') {
        return None;
    }
    let trimmed = trimmed.trim_start_matches('/');
    Some(trimmed.trim_end_matches('/').to_owned())
}

fn is_managed_archductor_ignore_pattern(line: &str) -> bool {
    matches!(gitignore_pattern_key(line).as_deref(), Some(".archductor"))
}

fn is_archductor_directory_ignore_pattern(line: &str) -> bool {
    matches!(
        gitignore_pattern_key(line).as_deref(),
        Some(".archductor/*" | ".archductor/**")
    )
}

fn active_prompt_pack_path(path: &str) -> Option<&Path> {
    let path = Path::new(path);
    let prefix = Path::new(".archductor").join("prompt-packs");
    (is_safe_relative_path(path.to_str()?) && path.parent() == Some(prefix.as_path()))
        .then_some(path)
}

fn ensure_prompt_pack_file(path: &Path, name: &str, version: Option<&str>) -> Result<bool> {
    reject_symlink_file(path)?;
    match fs::read(path) {
        Ok(_) => Ok(false),
        Err(err) if err.kind() == ErrorKind::NotFound => {
            let parent = path
                .parent()
                .with_context(|| format!("resolve parent for {}", path.display()))?;
            ensure_real_directory(parent)?;
            let contents = prompt_pack_toml(name, version.unwrap_or("v1"))?;
            atomic_write_no_symlink(path, contents.as_bytes())?;
            Ok(true)
        }
        Err(err) => Err(err).with_context(|| format!("read {}", path.display())),
    }
}

fn prompt_pack_toml(name: &str, version: &str) -> Result<String> {
    let mut raw: toml::Value =
        toml::from_str(&default_prompt_pack_toml()?).context("parse default prompt pack")?;
    let table = raw
        .as_table_mut()
        .context("default prompt pack must be a TOML table")?;
    table.insert("name".to_owned(), toml::Value::String(name.to_owned()));
    table.insert(
        "version".to_owned(),
        toml::Value::String(version.to_owned()),
    );
    toml::to_string_pretty(&raw).context("serialize prompt pack")
}

fn ensure_real_directory(path: &Path) -> Result<bool> {
    let mut created = false;
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            let file_type = metadata.file_type();
            anyhow::ensure!(
                !file_type.is_symlink() && file_type.is_dir(),
                "{} must be a real directory",
                path.display()
            );
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {
            fs::create_dir(path).with_context(|| format!("create {}", path.display()))?;
            created = true;
        }
        Err(err) => return Err(err).with_context(|| format!("inspect {}", path.display())),
    }
    Ok(created)
}

fn ensure_local_settings_dir(repo_path: &Path) -> Result<PathBuf> {
    ensure_settings_dir(repo_path).map(|(path, _)| path)
}

fn ensure_settings_dir(repo_path: &Path) -> Result<(PathBuf, bool)> {
    let conductor_dir = repo_path.join(".archductor");
    let mut created = false;
    match fs::symlink_metadata(&conductor_dir) {
        Ok(metadata) => {
            let file_type = metadata.file_type();
            anyhow::ensure!(
                !file_type.is_symlink() && file_type.is_dir(),
                "{} must be a real directory",
                conductor_dir.display()
            );
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {
            fs::create_dir(&conductor_dir)
                .with_context(|| format!("create {}", conductor_dir.display()))?;
            created = true;
        }
        Err(err) => {
            return Err(err).with_context(|| format!("inspect {}", conductor_dir.display()))
        }
    }
    let metadata = fs::symlink_metadata(&conductor_dir)
        .with_context(|| format!("inspect {}", conductor_dir.display()))?;
    let file_type = metadata.file_type();
    anyhow::ensure!(
        !file_type.is_symlink() && file_type.is_dir(),
        "{} must be a real directory",
        conductor_dir.display()
    );
    Ok((conductor_dir, created))
}

fn reject_symlink_file(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            anyhow::ensure!(
                !metadata.file_type().is_symlink(),
                "{} must not be a symlink",
                path.display()
            );
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => return Err(err).with_context(|| format!("inspect {}", path.display())),
    }
    Ok(())
}

fn prompt_pack_path_is_real(path: &Path, directory: bool) -> Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err).with_context(|| format!("inspect {}", path.display())),
    };
    let file_type = metadata.file_type();
    anyhow::ensure!(
        !file_type.is_symlink(),
        "{} must not be a symlink",
        path.display()
    );
    anyhow::ensure!(
        if directory {
            file_type.is_dir()
        } else {
            file_type.is_file()
        },
        "{} must be a real {}",
        path.display(),
        if directory { "directory" } else { "file" }
    );
    Ok(true)
}

fn set_local_default_agent_provider(value: &mut toml::Value, provider: &str) -> Result<()> {
    let root = value
        .as_table_mut()
        .context("local settings root must be a TOML table")?;
    let customization = root
        .entry("customization".to_owned())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let customization = customization
        .as_table_mut()
        .context("customization settings must be a TOML table")?;
    let automation = customization
        .entry("automation".to_owned())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let automation = automation
        .as_table_mut()
        .context("customization.automation settings must be a TOML table")?;
    automation.insert(
        "auto_start_agent".to_owned(),
        toml::Value::String(provider.to_owned()),
    );
    Ok(())
}

fn atomic_write_no_symlink(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("resolve parent for {}", path.display()))?;
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("settings.toml");
    let tmp_path = parent.join(format!(".{filename}.{}.tmp", Uuid::new_v4()));
    let write_result = (|| -> Result<()> {
        let permissions = settings_write_permissions(path)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)
            .with_context(|| format!("create {}", tmp_path.display()))?;
        set_permissions_if_supported(&tmp_path, permissions)?;
        file.write_all(contents)
            .with_context(|| format!("write {}", tmp_path.display()))?;
        file.sync_all()
            .with_context(|| format!("sync {}", tmp_path.display()))?;
        reject_symlink_file(path)?;
        fs::rename(&tmp_path, path).with_context(|| format!("replace {}", path.display()))?;
        #[cfg(unix)]
        fs::File::open(parent)
            .with_context(|| format!("open {}", parent.display()))?
            .sync_all()
            .with_context(|| format!("sync {}", parent.display()))?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    write_result
}

struct SettingsWritePermissions {
    #[cfg(unix)]
    value: fs::Permissions,
}

#[cfg(unix)]
fn settings_write_permissions(path: &Path) -> Result<SettingsWritePermissions> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(SettingsWritePermissions {
            value: metadata.permissions(),
        }),
        Err(err) if err.kind() == ErrorKind::NotFound => {
            let value =
                if path.file_name().and_then(|name| name.to_str()) == Some("settings.local.toml") {
                    fs::Permissions::from_mode(0o600)
                } else {
                    fs::Permissions::from_mode(0o644)
                };
            Ok(SettingsWritePermissions { value })
        }
        Err(err) => Err(err).with_context(|| format!("inspect {}", path.display())),
    }
}

#[cfg(not(unix))]
fn settings_write_permissions(_path: &Path) -> Result<SettingsWritePermissions> {
    Ok(SettingsWritePermissions {})
}

#[cfg(unix)]
fn set_permissions_if_supported(path: &Path, permissions: SettingsWritePermissions) -> Result<()> {
    fs::set_permissions(path, permissions.value)
        .with_context(|| format!("set permissions {}", path.display()))
}

#[cfg(not(unix))]
fn set_permissions_if_supported(
    _path: &Path,
    _permissions: SettingsWritePermissions,
) -> Result<()> {
    Ok(())
}

fn backup_settings_file(path: &Path) -> Result<Option<PathBuf>> {
    reject_symlink_file(path)?;
    if !path.exists() {
        return Ok(None);
    }
    let backup_path = path.with_extension("toml.bak");
    reject_symlink_file(&backup_path)?;
    fs::copy(path, &backup_path)
        .with_context(|| format!("backup {} to {}", path.display(), backup_path.display()))?;
    Ok(Some(backup_path))
}

fn normalize_workspace_tab(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn assert_all_settings_collections_empty(settings: &RepositorySettings) {
        assert!(settings.file_include_globs.is_empty());
        assert!(settings.env_file_refs.is_empty());
        assert!(settings.environment_variables.is_empty());
        assert!(settings.customization.naming.pr_body_sections.is_empty());
        assert!(settings
            .customization
            .automation
            .required_local_files
            .is_empty());
        assert!(settings.customization.agent_profiles.is_empty());
        assert!(settings.customization.view.colors.is_empty());
        assert!(settings.customization.view.dashboard_columns.is_empty());
        assert!(settings.customization.view.notification_rules.is_empty());
        assert!(settings
            .customization
            .view
            .command_palette_presets
            .is_empty());
    }

    #[test]
    fn prompt_kind_returns_matching_value() {
        let prompts = PromptSettings {
            continue_work: Some("Inspect current changes.".to_owned()),
            create_pr: Some("Write a concise PR.".to_owned()),
            ..PromptSettings::default()
        };

        assert_eq!(
            prompts.get(PromptKind::ContinueWork),
            Some("Inspect current changes.")
        );
        assert_eq!(
            prompts.get(PromptKind::CreatePr),
            Some("Write a concise PR.")
        );
    }

    #[test]
    fn effective_settings_merge_shared_repository_and_local_in_order() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let app = temp.path().join("config/settings.toml");
        fs::create_dir_all(repo.join(".archductor/prompt-packs")).unwrap();
        fs::create_dir_all(app.parent().unwrap()).unwrap();
        fs::write(
            &app,
            "[prompts]\ncontinue_work = \"shared continue\"\ncode_review = \"shared review\"\n",
        )
        .unwrap();
        fs::write(
            repo.join(".archductor/settings.toml"),
            "[prompt_pack]\npath = \".archductor/prompt-packs/team.toml\"\n\n[prompts]\ngeneral = \"repository general\"\n",
        )
        .unwrap();
        fs::write(
            repo.join(".archductor/settings.local.toml"),
            "[prompts]\nhandoff = \"local handoff\"\n",
        )
        .unwrap();
        fs::write(
            repo.join(".archductor/prompt-packs/team.toml"),
            "[prompts]\ngeneral = \"pack general\"\ncode_review = \"pack review\"\n",
        )
        .unwrap();

        let settings = load_effective_repository_settings(&repo, &app).unwrap();
        let prompts = settings.prompts.unwrap();
        assert_eq!(
            prompts.new_workspace,
            default_prompt_settings().new_workspace,
            "built-in prompt should survive when no later layer overrides it"
        );
        assert_eq!(prompts.continue_work.as_deref(), Some("shared continue"));
        assert_eq!(prompts.code_review.as_deref(), Some("pack review"));
        assert_eq!(prompts.general.as_deref(), Some("repository general"));
        assert_eq!(prompts.handoff.as_deref(), Some("local handoff"));
    }

    #[test]
    fn effective_settings_local_empty_file_globs_clear_app_shared_patterns() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let app = temp.path().join("config/settings.toml");
        fs::create_dir_all(repo.join(".archductor")).unwrap();
        fs::create_dir_all(app.parent().unwrap()).unwrap();
        fs::write(&app, "file_include_globs = \"shared.cache\"\n").unwrap();
        fs::write(
            repo.join(".archductor/settings.toml"),
            "[scripts]\nrun_mode = \"concurrent\"\n",
        )
        .unwrap();
        fs::write(
            repo.join(".archductor/settings.local.toml"),
            "file_include_globs = \"\"\n",
        )
        .unwrap();

        let settings = load_effective_repository_settings(&repo, &app).unwrap();

        assert!(settings.file_include_globs.is_empty());
    }

    #[test]
    fn raw_shared_empty_collections_clear_builtin_collections() {
        let built_in: RawRepositorySettings = toml::from_str(
            r##"
[environment_variables]
BUILT_IN = "1"

[customization]

[customization.naming]
pr_body_sections = ["Summary"]

[customization.automation]
required_local_files = [".env"]

[customization.agent_profiles.default]
mcp_servers = ["github"]

[customization.view]
dashboard_columns = ["ready"]
notification_rules = ["checks_failed"]
command_palette_presets = ["test"]

[customization.view.colors]
accent = "#5b9dff"
"##,
        )
        .unwrap();
        let shared: RawRepositorySettings = toml::from_str(
            r#"
[environment_variables]

[customization]
agent_profiles = {}

[customization.naming]
pr_body_sections = []

[customization.automation]
required_local_files = []

[customization.view]
colors = {}
dashboard_columns = []
notification_rules = []
command_palette_presets = []
"#,
        )
        .unwrap();

        let settings = built_in.merge(shared).into_settings();

        assert!(settings.environment_variables.is_empty());
        assert!(settings.customization.naming.pr_body_sections.is_empty());
        assert!(settings
            .customization
            .automation
            .required_local_files
            .is_empty());
        assert!(settings.customization.agent_profiles.is_empty());
        assert!(settings.customization.view.colors.is_empty());
        assert!(settings.customization.view.dashboard_columns.is_empty());
        assert!(settings.customization.view.notification_rules.is_empty());
        assert!(settings
            .customization
            .view
            .command_palette_presets
            .is_empty());

        let built_in: RawRepositorySettings =
            toml::from_str("[customization.agent_profiles.default]\nmcp_servers = [\"github\"]\n")
                .unwrap();
        let shared: RawRepositorySettings =
            toml::from_str("[customization.agent_profiles.default]\nmcp_servers = []\n").unwrap();
        let settings = built_in.merge(shared).into_settings();
        assert!(settings.customization.agent_profiles["default"]
            .mcp_servers
            .is_empty());
    }

    #[test]
    fn effective_settings_repository_empty_collections_clear_shared_collections() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let app = temp.path().join("config/settings.toml");
        fs::create_dir_all(repo.join(".archductor")).unwrap();
        fs::create_dir_all(app.parent().unwrap()).unwrap();
        fs::write(
            &app,
            r##"
[environment_variables]
SHARED = "1"

[customization.naming]
pr_body_sections = ["Summary"]

[customization.automation]
required_local_files = [".env"]

[customization.agent_profiles.default]
mcp_servers = ["github"]

[customization.view]
dashboard_columns = ["ready"]
notification_rules = ["checks_failed"]
command_palette_presets = ["test"]

[customization.view.colors]
accent = "#5b9dff"
"##,
        )
        .unwrap();
        fs::write(
            repo.join(".archductor/settings.toml"),
            r#"
[environment_variables]

[customization.naming]
pr_body_sections = []

[customization.automation]
required_local_files = []

[customization.agent_profiles.default]
mcp_servers = []

[customization.view]
colors = {}
dashboard_columns = []
notification_rules = []
command_palette_presets = []
"#,
        )
        .unwrap();

        let settings = load_effective_repository_settings(&repo, &app).unwrap();

        assert!(settings.environment_variables.is_empty());
        assert!(settings.customization.naming.pr_body_sections.is_empty());
        assert!(settings
            .customization
            .automation
            .required_local_files
            .is_empty());
        assert!(settings.customization.agent_profiles["default"]
            .mcp_servers
            .is_empty());
        assert!(settings.customization.view.colors.is_empty());
        assert!(settings.customization.view.dashboard_columns.is_empty());
        assert!(settings.customization.view.notification_rules.is_empty());
        assert!(settings
            .customization
            .view
            .command_palette_presets
            .is_empty());
    }

    #[test]
    fn effective_settings_local_empty_collections_clear_repository_collections() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let app = temp.path().join("config/settings.toml");
        fs::create_dir_all(repo.join(".archductor")).unwrap();
        fs::create_dir_all(app.parent().unwrap()).unwrap();
        fs::write(&app, "[scripts]\nrun = \"shared\"\n").unwrap();
        fs::write(
            repo.join(".archductor/settings.toml"),
            r##"
[environment_variables]
REPOSITORY = "1"

[customization.naming]
pr_body_sections = ["Summary"]

[customization.automation]
required_local_files = [".env"]

[customization.agent_profiles.default]
mcp_servers = ["github"]

[customization.view]
dashboard_columns = ["ready"]
notification_rules = ["checks_failed"]
command_palette_presets = ["test"]

[customization.view.colors]
accent = "#5b9dff"
"##,
        )
        .unwrap();
        fs::write(
            repo.join(".archductor/settings.local.toml"),
            r#"
[environment_variables]

[customization]
agent_profiles = {}

[customization.naming]
pr_body_sections = []

[customization.automation]
required_local_files = []

[customization.view]
colors = {}
dashboard_columns = []
notification_rules = []
command_palette_presets = []
"#,
        )
        .unwrap();

        let settings = load_effective_repository_settings(&repo, &app).unwrap();

        assert!(settings.environment_variables.is_empty());
        assert!(settings.customization.naming.pr_body_sections.is_empty());
        assert!(settings
            .customization
            .automation
            .required_local_files
            .is_empty());
        assert!(settings.customization.agent_profiles.is_empty());
        assert!(settings.customization.view.colors.is_empty());
        assert!(settings.customization.view.dashboard_columns.is_empty());
        assert!(settings.customization.view.notification_rules.is_empty());
        assert!(settings
            .customization
            .view
            .command_palette_presets
            .is_empty());
    }

    #[test]
    fn effective_settings_absent_collections_inherit_and_empty_strings_override() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let app = temp.path().join("config/settings.toml");
        fs::create_dir_all(repo.join(".archductor")).unwrap();
        fs::create_dir_all(app.parent().unwrap()).unwrap();
        fs::write(
            &app,
            r##"
[scripts]
run = "shared"

[environment_variables]
SHARED = "1"

[prompts]
general = "shared"

[customization.naming]
pr_body_sections = ["Summary"]

[customization.automation]
required_local_files = [".env"]

[customization.agent_profiles.default]
mcp_servers = ["github"]

[customization.view]
dashboard_columns = ["ready"]
notification_rules = ["checks_failed"]
command_palette_presets = ["test"]

[customization.view.colors]
accent = "#5b9dff"
"##,
        )
        .unwrap();
        fs::write(
            repo.join(".archductor/settings.toml"),
            "[scripts]\nrun = \"\"\n",
        )
        .unwrap();
        fs::write(
            repo.join(".archductor/settings.local.toml"),
            "[prompts]\ngeneral = \"\"\n",
        )
        .unwrap();

        let settings = load_effective_repository_settings(&repo, &app).unwrap();

        assert_eq!(settings.scripts.run.as_deref(), Some(""));
        assert_eq!(settings.prompts.unwrap().general.as_deref(), Some(""));
        assert_eq!(
            settings.environment_variables,
            [("SHARED".into(), "1".into())]
        );
        assert_eq!(settings.customization.naming.pr_body_sections, ["Summary"]);
        assert_eq!(
            settings.customization.automation.required_local_files,
            [".env"]
        );
        assert_eq!(
            settings.customization.agent_profiles["default"].mcp_servers,
            ["github"]
        );
        assert_eq!(
            settings
                .customization
                .view
                .colors
                .get("accent")
                .map(String::as_str),
            Some("#5b9dff")
        );
        assert_eq!(settings.customization.view.dashboard_columns, ["ready"]);
        assert_eq!(
            settings.customization.view.notification_rules,
            ["checks_failed"]
        );
        assert_eq!(
            settings.customization.view.command_palette_presets,
            ["test"]
        );
    }

    #[test]
    fn shared_settings_round_trip_uses_atomic_toml_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config/settings.toml");
        let settings = RepositorySettings {
            prompts: Some(PromptSettings {
                general: Some("Keep changes focused.".to_owned()),
                ..PromptSettings::default()
            }),
            ..RepositorySettings::default()
        };
        save_app_shared_settings(&path, &settings).unwrap();
        assert_eq!(load_app_shared_settings(&path).unwrap(), settings);
    }

    #[test]
    fn app_shared_load_save_preserves_explicit_empty_collection_presence() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config/settings.toml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"
file_include_globs = ""
env_file_refs = ""

[scripts]
run = ""

[environment_variables]

[customization]
agent_profiles = {}

[customization.naming]
pr_body_sections = []

[customization.automation]
required_local_files = []

[customization.view]
colors = {}
dashboard_columns = []
notification_rules = []
command_palette_presets = []
"#,
        )
        .unwrap();

        let settings = load_app_shared_settings(&path).unwrap();
        save_app_shared_settings(&path, &settings).unwrap();

        let built_in: RawRepositorySettings = toml::from_str(
            r##"
file_include_globs = ".env"
env_file_refs = ".env.local"

[environment_variables]
BUILT_IN = "1"

[customization.naming]
pr_body_sections = ["Summary"]

[customization.automation]
required_local_files = [".env"]

[customization.agent_profiles.default]
mcp_servers = ["github"]

[customization.view]
dashboard_columns = ["ready"]
notification_rules = ["checks_failed"]
command_palette_presets = ["test"]

[customization.view.colors]
accent = "#5b9dff"
"##,
        )
        .unwrap();
        let saved = load_optional_settings(&path).unwrap();
        let effective = built_in.merge(saved).into_settings();

        assert_all_settings_collections_empty(&effective);
        assert_eq!(effective.scripts.run.as_deref(), Some(""));
    }

    #[test]
    fn repository_load_save_preserves_explicit_empty_collection_presence() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let app = temp.path().join("config/settings.toml");
        fs::create_dir_all(repo.join(".archductor")).unwrap();
        fs::create_dir_all(app.parent().unwrap()).unwrap();
        fs::write(
            &app,
            r##"
file_include_globs = ".env"
env_file_refs = ".env.local"

[environment_variables]
SHARED = "1"

[customization]
agent_profiles = { default = { mcp_servers = ["github"] } }

[customization.naming]
pr_body_sections = ["Summary"]

[customization.automation]
required_local_files = [".env"]

[customization.view]
colors = { accent = "#5b9dff" }
dashboard_columns = ["ready"]
notification_rules = ["checks_failed"]
command_palette_presets = ["test"]
"##,
        )
        .unwrap();
        let path = repo.join(".archductor/settings.toml");
        fs::write(
            &path,
            r#"
file_include_globs = ""
env_file_refs = ""

[environment_variables]

[customization]
agent_profiles = {}

[customization.naming]
pr_body_sections = []

[customization.automation]
required_local_files = []

[customization.view]
colors = {}
dashboard_columns = []
notification_rules = []
command_palette_presets = []
"#,
        )
        .unwrap();

        let settings =
            load_repository_settings_for_layer(&repo, SettingsLayer::RepositoryShared).unwrap();
        save_repository_settings(&repo, SettingsLayer::RepositoryShared, &settings).unwrap();
        let effective = load_effective_repository_settings(&repo, &app).unwrap();

        assert_all_settings_collections_empty(&effective);
    }

    #[test]
    fn local_load_save_preserves_explicit_empty_collection_presence() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let app = temp.path().join("config/settings.toml");
        fs::create_dir_all(repo.join(".archductor")).unwrap();
        fs::create_dir_all(app.parent().unwrap()).unwrap();
        fs::write(&app, "[scripts]\nrun = \"shared\"\n").unwrap();
        fs::write(
            repo.join(".archductor/settings.toml"),
            r##"
file_include_globs = ".env"
env_file_refs = ".env.local"

[environment_variables]
REPOSITORY = "1"

[customization]
agent_profiles = { default = { mcp_servers = ["github"] } }

[customization.naming]
pr_body_sections = ["Summary"]

[customization.automation]
required_local_files = [".env"]

[customization.view]
colors = { accent = "#5b9dff" }
dashboard_columns = ["ready"]
notification_rules = ["checks_failed"]
command_palette_presets = ["test"]
"##,
        )
        .unwrap();
        let path = repo.join(".archductor/settings.local.toml");
        fs::write(
            &path,
            r#"
file_include_globs = ""
env_file_refs = ""

[environment_variables]

[customization]
agent_profiles = {}

[customization.naming]
pr_body_sections = []

[customization.automation]
required_local_files = []

[customization.view]
colors = {}
dashboard_columns = []
notification_rules = []
command_palette_presets = []
"#,
        )
        .unwrap();

        let settings =
            load_repository_settings_for_layer(&repo, SettingsLayer::LocalOverride).unwrap();
        save_repository_settings(&repo, SettingsLayer::LocalOverride, &settings).unwrap();
        let effective = load_effective_repository_settings(&repo, &app).unwrap();

        assert_all_settings_collections_empty(&effective);
    }

    #[test]
    fn app_shared_toml_import_export_preserves_explicit_empty_collection_presence() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config/settings.toml");
        let input = r#"
file_include_globs = ""
env_file_refs = ""

[environment_variables]

[customization.view]
colors = {}
notification_rules = []
command_palette_presets = []
"#;

        save_app_shared_settings_from_toml(&path, input).unwrap();
        let exported = app_shared_settings_to_toml(&path).unwrap();
        let saved: RawRepositorySettings = toml::from_str(&exported).unwrap();

        assert_eq!(saved.file_include_globs.as_deref(), Some(""));
        assert_eq!(saved.env_file_refs.as_deref(), Some(""));
        assert!(saved.environment_variables.unwrap().is_empty());
        let view = saved.customization.unwrap().view.unwrap();
        assert!(view.colors.unwrap().is_empty());
        assert!(view.notification_rules.unwrap().is_empty());
        assert!(view.command_palette_presets.unwrap().is_empty());
    }

    #[test]
    fn repository_toml_import_preserves_explicit_empty_collection_presence() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let app = temp.path().join("config/settings.toml");
        fs::create_dir_all(repo.join(".archductor")).unwrap();
        fs::create_dir_all(app.parent().unwrap()).unwrap();
        fs::write(
            &app,
            r##"
file_include_globs = ".env"
env_file_refs = ".env.local"

[environment_variables]
SHARED = "1"

[customization.view]
colors = { accent = "#5b9dff" }
notification_rules = ["checks_failed"]
command_palette_presets = ["test"]
"##,
        )
        .unwrap();
        let input = r#"
file_include_globs = ""
env_file_refs = ""

[environment_variables]

[customization.view]
colors = {}
notification_rules = []
command_palette_presets = []
"#;

        save_repository_settings_from_toml(&repo, SettingsLayer::RepositoryShared, input).unwrap();
        let effective = load_effective_repository_settings(&repo, &app).unwrap();

        assert!(effective.file_include_globs.is_empty());
        assert!(effective.env_file_refs.is_empty());
        assert!(effective.environment_variables.is_empty());
        assert!(effective.customization.view.colors.is_empty());
        assert!(effective.customization.view.notification_rules.is_empty());
        assert!(effective
            .customization
            .view
            .command_palette_presets
            .is_empty());
    }

    #[test]
    fn repository_save_can_mark_edited_absent_collections_as_explicitly_empty() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let app = temp.path().join("config/settings.toml");
        fs::create_dir_all(repo.join(".archductor")).unwrap();
        fs::create_dir_all(app.parent().unwrap()).unwrap();
        fs::write(
            repo.join(".archductor/settings.toml"),
            r##"
file_include_globs = ".env"
env_file_refs = ".env.local"

[environment_variables]
REPOSITORY = "1"

[customization]
agent_profiles = { default = { mcp_servers = ["github"] } }

[customization.naming]
pr_body_sections = ["Summary"]

[customization.automation]
required_local_files = [".env"]

[customization.view]
colors = { accent = "#5b9dff" }
dashboard_columns = ["ready"]
notification_rules = ["checks_failed"]
command_palette_presets = ["test"]
"##,
        )
        .unwrap();
        let settings = RepositorySettings::default();

        save_repository_settings_with_explicit_empty_collections(
            &repo,
            SettingsLayer::LocalOverride,
            &settings,
            &[
                SettingsCollectionField::FileIncludeGlobs,
                SettingsCollectionField::EnvFileRefs,
                SettingsCollectionField::EnvironmentVariables,
                SettingsCollectionField::AgentProfiles,
                SettingsCollectionField::PrBodySections,
                SettingsCollectionField::RequiredLocalFiles,
                SettingsCollectionField::ViewColors,
                SettingsCollectionField::DashboardColumns,
                SettingsCollectionField::NotificationRules,
                SettingsCollectionField::CommandPalettePresets,
            ],
        )
        .unwrap();
        let effective = load_effective_repository_settings(&repo, &app).unwrap();

        assert_all_settings_collections_empty(&effective);
    }

    #[test]
    fn repository_replacement_save_removes_collection_overrides() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(repo.join(".archductor")).unwrap();
        fs::write(
            repo.join(".archductor/settings.toml"),
            r##"
file_include_globs = ".env"
env_file_refs = ".env.local"

[environment_variables]
REPOSITORY = "1"

[customization]
agent_profiles = { default = { mcp_servers = ["github"] } }

[customization.naming]
pr_body_sections = ["Summary"]

[customization.automation]
required_local_files = [".env"]

[customization.view]
colors = { accent = "#5b9dff" }
dashboard_columns = ["ready"]
notification_rules = ["checks_failed"]
command_palette_presets = ["test"]
"##,
        )
        .unwrap();
        fs::write(
            repo.join(".archductor/settings.local.toml"),
            r##"
file_include_globs = ""
env_file_refs = "local.env"

[environment_variables]

[customization.naming]
pr_body_sections = ["Local"]

[customization.automation]
required_local_files = []

[customization.agent_profiles.default]
mcp_servers = []

[customization.view]
colors = { accent = "#0ea5e9" }
dashboard_columns = []
notification_rules = ["local"]
command_palette_presets = []
"##,
        )
        .unwrap();

        save_repository_settings_replacing(
            &repo,
            SettingsLayer::LocalOverride,
            &RepositorySettings::default(),
        )
        .unwrap();

        let raw = load_optional_settings(&repo.join(".archductor/settings.local.toml")).unwrap();
        assert!(raw.file_include_globs.is_none());
        assert!(raw.env_file_refs.is_none());
        assert!(raw.environment_variables.is_none());
        let customization = raw.customization.unwrap();
        assert!(customization.agent_profiles.is_none());
        assert!(customization.naming.unwrap().pr_body_sections.is_none());
        assert!(customization
            .automation
            .unwrap()
            .required_local_files
            .is_none());
        let view = customization.view.unwrap();
        assert!(view.colors.is_none());
        assert!(view.dashboard_columns.is_none());
        assert!(view.notification_rules.is_none());
        assert!(view.command_palette_presets.is_none());

        let effective = load_repository_settings(&repo).unwrap();
        assert_eq!(effective.file_include_globs, [".env"]);
        assert_eq!(effective.env_file_refs, [".env.local"]);
        assert_eq!(
            effective.environment_variables,
            [("REPOSITORY".into(), "1".into())]
        );
        assert_eq!(effective.customization.naming.pr_body_sections, ["Summary"]);
        assert_eq!(
            effective.customization.automation.required_local_files,
            [".env"]
        );
        assert_eq!(
            effective.customization.agent_profiles["default"].mcp_servers,
            ["github"]
        );
        assert_eq!(effective.customization.view.colors["accent"], "#5b9dff");
        assert_eq!(effective.customization.view.dashboard_columns, ["ready"]);
        assert_eq!(
            effective.customization.view.notification_rules,
            ["checks_failed"]
        );
        assert_eq!(
            effective.customization.view.command_palette_presets,
            ["test"]
        );
    }

    #[test]
    fn repository_save_can_unset_existing_explicit_empty_collection_markers() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(repo.join(".archductor")).unwrap();
        fs::write(
            repo.join(".archductor/settings.local.toml"),
            r#"
file_include_globs = ""

[environment_variables]

[customization.view]
colors = {}
notification_rules = []
"#,
        )
        .unwrap();
        let settings =
            load_repository_settings_for_layer(&repo, SettingsLayer::LocalOverride).unwrap();

        save_repository_settings_with_collection_intent(
            &repo,
            SettingsLayer::LocalOverride,
            &settings,
            &[],
            &[
                SettingsCollectionField::FileIncludeGlobs,
                SettingsCollectionField::EnvironmentVariables,
                SettingsCollectionField::ViewColors,
                SettingsCollectionField::NotificationRules,
            ],
        )
        .unwrap();

        let raw = load_optional_settings(&repo.join(".archductor/settings.local.toml")).unwrap();
        assert!(raw.file_include_globs.is_none());
        assert!(raw.environment_variables.is_none());
        let view = raw.customization.unwrap().view.unwrap();
        assert!(view.colors.is_none());
        assert!(view.notification_rules.is_none());
    }

    #[test]
    fn explicit_empty_collection_fields_parse_from_advanced_toml() {
        let fields = explicit_empty_collection_fields_from_toml(
            r#"
[customization]
agent_profiles = {}

[customization.naming]
pr_body_sections = []

[customization.automation]
required_local_files = []

[customization.view]
colors = {}
dashboard_columns = []
notification_rules = []
command_palette_presets = []
"#,
        )
        .unwrap();

        assert!(fields.contains(&SettingsCollectionField::AgentProfiles));
        assert!(fields.contains(&SettingsCollectionField::PrBodySections));
        assert!(fields.contains(&SettingsCollectionField::RequiredLocalFiles));
        assert!(fields.contains(&SettingsCollectionField::ViewColors));
        assert!(fields.contains(&SettingsCollectionField::DashboardColumns));
        assert!(fields.contains(&SettingsCollectionField::NotificationRules));
        assert!(fields.contains(&SettingsCollectionField::CommandPalettePresets));
    }

    #[test]
    fn present_collection_fields_parse_from_advanced_toml() {
        let fields = present_collection_fields_from_toml(
            r#"
[customization.naming]
pr_body_sections = ["Summary"]

[customization.view]
colors = {}
"#,
        )
        .unwrap();

        assert!(fields.contains(&SettingsCollectionField::PrBodySections));
        assert!(fields.contains(&SettingsCollectionField::ViewColors));
        assert!(!fields.contains(&SettingsCollectionField::NotificationRules));
    }

    #[test]
    fn shared_settings_save_creates_missing_parent_tree() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("fresh/config/archductor/settings.toml");
        let settings = RepositorySettings {
            prompts: Some(PromptSettings {
                general: Some("Keep changes focused.".to_owned()),
                ..PromptSettings::default()
            }),
            ..RepositorySettings::default()
        };

        save_app_shared_settings(&path, &settings).unwrap();

        assert_eq!(load_app_shared_settings(&path).unwrap(), settings);
    }

    #[test]
    fn loads_shared_settings_file() {
        let temp = tempfile::tempdir().unwrap();
        let conductor_dir = temp.path().join(".archductor");
        fs::create_dir(&conductor_dir).unwrap();
        fs::write(
            conductor_dir.join("settings.toml"),
            r#"
file_include_globs = """
.env*
config/*.local.json
"""
env_file_refs = """
.env
.env.local
"""

[scripts]
setup = "pnpm install"
run = "pnpm dev --port $ARCHDUCTOR_PORT"
run_mode = "concurrent"

[environment_variables]
API_BASE_URL = "http://localhost:3000"
"#,
        )
        .unwrap();

        let settings = load_repository_settings(temp.path()).unwrap();

        assert_eq!(
            settings.file_include_globs,
            [".env*", "config/*.local.json"]
        );
        assert_eq!(settings.env_file_refs, [".env", ".env.local"]);
        assert_eq!(settings.scripts.setup, Some("pnpm install".to_owned()));
        assert_eq!(
            settings.scripts.run,
            Some("pnpm dev --port $ARCHDUCTOR_PORT".to_owned())
        );
        assert_eq!(settings.scripts.run_mode, Some("concurrent".to_owned()));
        assert_eq!(
            settings.environment_variables,
            [(
                "API_BASE_URL".to_owned(),
                "http://localhost:3000".to_owned()
            )]
        );
    }

    #[test]
    fn local_settings_override_shared_settings() {
        let temp = tempfile::tempdir().unwrap();
        let conductor_dir = temp.path().join(".archductor");
        fs::create_dir(&conductor_dir).unwrap();
        fs::write(
            conductor_dir.join("settings.toml"),
            r#"
file_include_globs = ".env"

[scripts]
setup = "pnpm install"
run = "pnpm dev"

[environment_variables]
API_BASE_URL = "https://shared.example"
"#,
        )
        .unwrap();
        fs::write(
            conductor_dir.join("settings.local.toml"),
            r#"
file_include_globs = """
.env.local
secrets/*.json
"""

[scripts]
setup = "just bootstrap"

[environment_variables]
API_BASE_URL = "http://localhost:4000"
LOCAL_ONLY = "1"
"#,
        )
        .unwrap();

        let settings = load_repository_settings(temp.path()).unwrap();

        assert_eq!(
            settings.file_include_globs,
            [".env.local", "secrets/*.json"]
        );
        assert_eq!(settings.scripts.setup, Some("just bootstrap".to_owned()));
        assert_eq!(settings.scripts.run, Some("pnpm dev".to_owned()));
        assert_eq!(
            settings.environment_variables,
            [
                (
                    "API_BASE_URL".to_owned(),
                    "http://localhost:4000".to_owned()
                ),
                ("LOCAL_ONLY".to_owned(), "1".to_owned()),
            ]
        );
    }

    #[test]
    fn saves_repository_settings_to_requested_layer() {
        let temp = tempfile::tempdir().unwrap();
        let settings = RepositorySettings {
            file_include_globs: vec![".env*".to_owned(), "config/*.local.json".to_owned()],
            env_file_refs: vec![".env.local".to_owned()],
            spotlight_testing: Some(true),
            enterprise_data_privacy: Some(false),
            scripts: ScriptSettings {
                setup: Some("pnpm install".to_owned()),
                run: Some("pnpm dev --port $ARCHDUCTOR_PORT".to_owned()),
                archive: Some("./script/archive.sh".to_owned()),
                test: Some("pnpm test".to_owned()),
                lint: Some("pnpm lint".to_owned()),
                typecheck: Some("pnpm typecheck".to_owned()),
                build: Some("pnpm build".to_owned()),
                run_mode: Some("nonconcurrent".to_owned()),
            },
            environment_variables: vec![(
                "API_BASE_URL".to_owned(),
                "http://localhost:3000".to_owned(),
            )],
            prompt_pack: PromptPackSettings {
                active: Some("startup".to_owned()),
                version: Some("v2".to_owned()),
                path: Some(".archductor/prompt-packs/startup.toml".to_owned()),
            },
            prompts: Some(PromptSettings {
                new_workspace: Some("Plan before editing.".to_owned()),
                general: Some("Ship small changes.".to_owned()),
                continue_work: Some("Resume from existing context.".to_owned()),
                summarize_session: Some("Summarize tests and risk.".to_owned()),
                handoff: Some("Leave a concise handoff.".to_owned()),
                code_review: Some("Find correctness issues.".to_owned()),
                create_pr: Some("Include test evidence.".to_owned()),
                fix_errors: Some("Focus on failing checks.".to_owned()),
                resolve_merge_conflicts: Some("Preserve user changes.".to_owned()),
                rename_branch: Some("Use short feature names.".to_owned()),
                commit_generation: None,
                test_fixing: None,
                refactor_style: None,
                setup_script: Some("Use the configured setup script.".to_owned()),
                run_script: Some("Use the configured run script.".to_owned()),
            }),
            providers: ProviderSettings {
                claude_code_executable_path: Some("/usr/local/bin/claude".to_owned()),
                codex_executable_path: Some("/usr/local/bin/codex".to_owned()),
                claude_provider: Some("anthropic".to_owned()),
                codex_provider: Some("openai".to_owned()),
                bedrock_region: None,
                vertex_project_id: None,
            },
            git: GitSettings {
                delete_branch_on_archive: Some(false),
                archive_on_merge: Some(true),
                worktree_push_auto_setup_remote: Some(true),
                branch_prefix_type: Some("custom".to_owned()),
                branch_prefix: Some("feat".to_owned()),
            },
            customization: CustomizationSettings::default(),
        };

        save_repository_settings(temp.path(), SettingsLayer::RepositoryShared, &settings).unwrap();
        let loaded = load_repository_settings(temp.path()).unwrap();

        assert_eq!(loaded, settings);
        let saved = fs::read_to_string(temp.path().join(".archductor/settings.toml")).unwrap();
        assert!(saved.contains("[prompt_pack]"));
        assert!(saved.contains("typecheck = \"pnpm typecheck\""));
        assert!(saved.contains("summarize_session = \"Summarize tests and risk.\""));
        assert!(temp.path().join(".archductor/settings.toml").exists());
        assert!(!temp.path().join(".archductor/settings.local.toml").exists());
    }

    #[test]
    fn ensure_repository_config_creates_shared_defaults() {
        let temp = tempfile::tempdir().unwrap();

        let report = ensure_repository_config(temp.path()).unwrap();

        assert!(report.conductor_dir_created);
        assert!(report.shared_settings_created);
        assert!(report.prompt_pack_dir_created);
        assert!(report.default_prompt_pack_created);
        assert!(report.context_gitignore_updated);
        let shared_path = temp.path().join(".archductor/settings.toml");
        assert!(shared_path.exists());
        let gitignore = fs::read_to_string(temp.path().join(".gitignore")).unwrap();
        assert!(gitignore.contains(".context/"));
        assert!(gitignore.contains(".archductor/settings.local.toml*"));
        assert!(!gitignore.lines().any(|line| line.trim() == ".archductor/"));
        let prompt_pack_path = temp.path().join(".archductor/prompt-packs/default.toml");
        assert!(prompt_pack_path.exists());
        assert!(fs::read_to_string(prompt_pack_path)
            .unwrap()
            .contains("[prompts]"));
        let settings = load_repository_settings(temp.path()).unwrap();
        assert_eq!(settings.file_include_globs, [".env*"]);
        assert_eq!(settings.scripts.run_mode.as_deref(), Some("concurrent"));
        assert_eq!(
            settings.customization.workspace_defaults.port_block_size,
            Some(10)
        );
    }

    #[test]
    fn load_repository_settings_missing_config_uses_safe_defaults() {
        let temp = tempfile::tempdir().unwrap();

        let settings = load_repository_settings(temp.path()).unwrap();

        assert!(settings.file_include_globs.is_empty());
        assert!(settings.env_file_refs.is_empty());
        assert!(settings.scripts.run.is_none());
        assert!(settings.environment_variables.is_empty());
        assert_eq!(settings.customization, CustomizationSettings::default());
    }

    #[test]
    fn ensure_repository_config_keeps_existing_valid_settings() {
        let temp = tempfile::tempdir().unwrap();
        let conductor_dir = temp.path().join(".archductor");
        fs::create_dir(&conductor_dir).unwrap();
        fs::write(
            conductor_dir.join("settings.toml"),
            "[scripts]\nrun = \"cargo run\"\n",
        )
        .unwrap();
        fs::write(temp.path().join(".gitignore"), "target/\n.archductor/\n").unwrap();

        let report = ensure_repository_config(temp.path()).unwrap();

        assert!(!report.conductor_dir_created);
        assert!(!report.shared_settings_created);
        assert!(report.prompt_pack_dir_created);
        assert!(report.default_prompt_pack_created);
        assert!(report.context_gitignore_updated);
        let contents = fs::read_to_string(conductor_dir.join("settings.toml")).unwrap();
        assert!(contents.contains("cargo run"));
        assert!(conductor_dir.join("prompt-packs/default.toml").exists());
        let gitignore = fs::read_to_string(temp.path().join(".gitignore")).unwrap();
        assert!(gitignore.contains("target/"));
        assert!(gitignore.contains(".context/"));
        assert!(gitignore.contains(".archductor/settings.local.toml*"));
        assert!(!gitignore.lines().any(|line| line.trim() == ".archductor/"));
    }

    #[test]
    fn ensure_repository_config_seeds_active_prompt_pack_when_missing() {
        let temp = tempfile::tempdir().unwrap();
        let conductor_dir = temp.path().join(".archductor");
        fs::create_dir(&conductor_dir).unwrap();
        fs::write(
            conductor_dir.join("settings.toml"),
            r#"
[prompt_pack]
active = "review"
version = "v1"
path = ".archductor/prompt-packs/review.toml"
"#,
        )
        .unwrap();

        let report = ensure_repository_config(temp.path()).unwrap();

        assert!(report.prompt_pack_dir_created);
        assert!(report.default_prompt_pack_created);
        assert!(report.active_prompt_pack_created);
        assert!(conductor_dir.join("prompt-packs/default.toml").exists());
        let review_pack =
            fs::read_to_string(conductor_dir.join("prompt-packs/review.toml")).unwrap();
        assert!(review_pack.contains("name = \"review\""));
        assert!(review_pack.contains("version = \"v1\""));
    }

    #[test]
    fn load_repository_settings_uses_configured_prompt_pack_prompts() {
        let temp = tempfile::tempdir().unwrap();
        let conductor_dir = temp.path().join(".archductor");
        let prompt_pack_dir = conductor_dir.join("prompt-packs");
        fs::create_dir_all(&prompt_pack_dir).unwrap();
        fs::write(
            conductor_dir.join("settings.toml"),
            r#"
[prompt_pack]
active = "review"
version = "v7"
path = ".archductor/prompt-packs/review.toml"

[prompts]
create_pr = "Use repository override."
"#,
        )
        .unwrap();
        fs::write(
            prompt_pack_dir.join("review.toml"),
            r#"
name = "review"
version = "v7"

[prompts]
general = "Use review pack."
create_pr = "Use pack PR prompt."
"#,
        )
        .unwrap();

        let settings = load_repository_settings(temp.path()).unwrap();
        let prompts = settings.prompts.unwrap();

        assert_eq!(prompts.general.as_deref(), Some("Use review pack."));
        assert_eq!(
            prompts.create_pr.as_deref(),
            Some("Use repository override.")
        );
    }

    #[cfg(unix)]
    #[test]
    fn ensure_repository_config_rejects_gitignore_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let external = temp.path().join("outside-gitignore");
        fs::write(&external, "target/\n").unwrap();
        std::os::unix::fs::symlink(&external, temp.path().join(".gitignore")).unwrap();

        let err = ensure_repository_config(temp.path()).unwrap_err();

        assert!(format!("{err:#}").contains("symlink"));
    }

    #[cfg(unix)]
    #[test]
    fn ensure_repository_config_rejects_archductor_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let external = temp.path().join("outside");
        fs::create_dir(&external).unwrap();
        std::os::unix::fs::symlink(&external, temp.path().join(".archductor")).unwrap();

        let err = ensure_repository_config(temp.path()).unwrap_err();

        assert!(err.to_string().contains("must be a real directory"));
    }

    #[cfg(unix)]
    #[test]
    fn ensure_repository_config_rejects_shared_settings_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let conductor_dir = temp.path().join(".archductor");
        fs::create_dir(&conductor_dir).unwrap();
        let external = temp.path().join("outside.toml");
        fs::write(&external, "outside = true\n").unwrap();
        std::os::unix::fs::symlink(&external, conductor_dir.join("settings.toml")).unwrap();

        let err = ensure_repository_config(temp.path()).unwrap_err();

        assert!(err.to_string().contains("must not be a symlink"));
    }

    #[cfg(unix)]
    #[test]
    fn ensure_repository_config_rejects_prompt_pack_dir_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let conductor_dir = temp.path().join(".archductor");
        fs::create_dir(&conductor_dir).unwrap();
        fs::write(
            conductor_dir.join("settings.toml"),
            "[scripts]\nrun = \"cargo run\"\n",
        )
        .unwrap();
        let external = temp.path().join("outside-packs");
        fs::create_dir(&external).unwrap();
        std::os::unix::fs::symlink(&external, conductor_dir.join("prompt-packs")).unwrap();

        let err = ensure_repository_config(temp.path()).unwrap_err();

        assert!(err.to_string().contains("must be a real directory"));
    }

    #[test]
    fn load_repository_settings_recovering_reports_invalid_local_settings() {
        let temp = tempfile::tempdir().unwrap();
        let conductor_dir = temp.path().join(".archductor");
        fs::create_dir(&conductor_dir).unwrap();
        fs::write(
            conductor_dir.join("settings.toml"),
            "[scripts]\nrun = \"cargo run\"\n",
        )
        .unwrap();
        fs::write(conductor_dir.join("settings.local.toml"), "[scripts\n").unwrap();

        let report = load_repository_settings_recovering(temp.path());

        assert_eq!(report.settings.scripts.run.as_deref(), Some("cargo run"));
        assert_eq!(report.errors.len(), 1);
        assert!(report.errors[0].contains("parse settings"));
    }

    #[test]
    fn load_repository_settings_recovering_uses_defaults_for_invalid_merged_settings() {
        let temp = tempfile::tempdir().unwrap();
        let conductor_dir = temp.path().join(".archductor");
        fs::create_dir(&conductor_dir).unwrap();
        fs::write(
            conductor_dir.join("settings.toml"),
            "[scripts]\nrun_mode = \"parallel\"\n",
        )
        .unwrap();

        let report = load_repository_settings_recovering(temp.path());

        assert_eq!(report.settings, RepositorySettings::default());
        assert_eq!(report.errors.len(), 1);
        assert!(report.errors[0].contains("scripts.run_mode"));
        assert_eq!(
            load_repository_settings(temp.path()).unwrap(),
            RepositorySettings::default()
        );
    }

    #[cfg(unix)]
    #[test]
    fn load_repository_settings_rejects_settings_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let conductor_dir = temp.path().join(".archductor");
        fs::create_dir(&conductor_dir).unwrap();
        let external = temp.path().join("outside.toml");
        fs::write(&external, "[scripts]\nrun = \"outside\"\n").unwrap();
        std::os::unix::fs::symlink(&external, conductor_dir.join("settings.toml")).unwrap();

        let err = load_repository_settings(temp.path()).unwrap_err();

        assert!(err.to_string().contains("must not be a symlink"));
    }

    #[test]
    fn save_repository_settings_backs_up_existing_file() {
        let temp = tempfile::tempdir().unwrap();
        let conductor_dir = temp.path().join(".archductor");
        fs::create_dir(&conductor_dir).unwrap();
        let shared_path = conductor_dir.join("settings.toml");
        fs::write(&shared_path, "[scripts]\nrun = \"old\"\n").unwrap();

        let settings = RepositorySettings {
            scripts: ScriptSettings {
                run: Some("new".to_owned()),
                ..ScriptSettings::default()
            },
            ..RepositorySettings::default()
        };
        save_repository_settings(temp.path(), SettingsLayer::RepositoryShared, &settings).unwrap();

        assert!(fs::read_to_string(&shared_path).unwrap().contains("new"));
        assert!(fs::read_to_string(conductor_dir.join("settings.toml.bak"))
            .unwrap()
            .contains("old"));
    }

    #[cfg(unix)]
    #[test]
    fn save_repository_settings_rejects_symlink_target() {
        let temp = tempfile::tempdir().unwrap();
        let conductor_dir = temp.path().join(".archductor");
        fs::create_dir(&conductor_dir).unwrap();
        let external = temp.path().join("outside.toml");
        fs::write(&external, "outside = true\n").unwrap();
        std::os::unix::fs::symlink(&external, conductor_dir.join("settings.toml")).unwrap();

        let err = save_repository_settings(
            temp.path(),
            SettingsLayer::RepositoryShared,
            &RepositorySettings::default(),
        )
        .unwrap_err();

        assert!(err.to_string().contains("must not be a symlink"));
        assert_eq!(fs::read_to_string(external).unwrap(), "outside = true\n");
    }

    #[test]
    fn save_local_default_agent_provider_updates_only_local_override() {
        let temp = tempfile::tempdir().unwrap();
        let conductor_dir = temp.path().join(".archductor");
        fs::create_dir(&conductor_dir).unwrap();
        fs::write(
            conductor_dir.join("settings.toml"),
            r#"
[scripts]
setup = "pnpm install"
"#,
        )
        .unwrap();
        fs::write(
            conductor_dir.join("settings.local.toml"),
            r#"
unknown_root = "keep"

[future.provider]
experimental = true

[customization.view]
theme = "dark"
"#,
        )
        .unwrap();

        save_local_default_agent_provider(temp.path(), "claude").unwrap();

        let local = fs::read_to_string(conductor_dir.join("settings.local.toml")).unwrap();
        assert!(local.contains("auto_start_agent = \"claude\""));
        assert!(local.contains("theme = \"dark\""));
        assert!(local.contains("unknown_root = \"keep\""));
        assert!(local.contains("[future.provider]"));
        assert!(local.contains("experimental = true"));
        assert!(!local.contains("pnpm install"));
    }

    #[cfg(unix)]
    #[test]
    fn save_local_default_agent_provider_rejects_symlink_target() {
        let temp = tempfile::tempdir().unwrap();
        let conductor_dir = temp.path().join(".archductor");
        fs::create_dir(&conductor_dir).unwrap();
        let external = temp.path().join("outside.toml");
        fs::write(&external, "outside = true\n").unwrap();
        std::os::unix::fs::symlink(&external, conductor_dir.join("settings.local.toml")).unwrap();

        let err = save_local_default_agent_provider(temp.path(), "codex").unwrap_err();

        assert!(err.to_string().contains("must not be a symlink"));
        assert_eq!(fs::read_to_string(external).unwrap(), "outside = true\n");
    }

    #[cfg(unix)]
    #[test]
    fn save_local_default_agent_provider_preserves_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let conductor_dir = temp.path().join(".archductor");
        fs::create_dir(&conductor_dir).unwrap();
        let path = conductor_dir.join("settings.local.toml");
        fs::write(&path, "[customization.view]\ntheme = \"dark\"\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        save_local_default_agent_provider(temp.path(), "claude").unwrap();

        let mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn loads_merges_and_saves_customization_settings() {
        let temp = tempfile::tempdir().unwrap();
        let conductor_dir = temp.path().join(".archductor");
        fs::create_dir(&conductor_dir).unwrap();
        fs::write(
            conductor_dir.join("settings.toml"),
            r##"
[prompts]
new_workspace = "Plan the workspace."
continue_work = "Continue carefully."
summarize_session = "Summarize verification."
handoff = "Handoff next steps."
test_fixing = "Fix the failing test first."
refactor_style = "Keep refactors behavior-preserving."
setup_script = "Prepare dependencies."
run_script = "Start the app."

[prompt_pack]
active = "core"
version = "v3"
path = ".archductor/prompt-packs/core.toml"

[scripts]
test = "cargo test --workspace"
lint = "cargo clippy --workspace"
typecheck = "cargo check --workspace"
build = "cargo build --workspace"

[customization.naming]
branch_template = "{prefix}/{type}-{slug}"
workspace_name_style = "city"
commit_style = "conventional"
pr_title_template = "{type}: {summary}"
pr_body_sections = ["Summary", "Tests", "Risk"]
default_merge_method = "squash"

[customization.automation]
auto_setup = true
auto_start_agent = "codex"
required_local_files = [".env", "certs/local.pem"]
test_command = "cargo test --workspace"
lint_command = "cargo clippy --workspace"
typecheck_command = "cargo check --workspace"
build_command = "cargo build --workspace"
pre_workspace_create = "just pre-workspace"
post_workspace_create = "just post-workspace"
pre_pr_create = "just pre-pr"
post_merge = "just post-merge"

[customization.agent_profiles.default]
agent = "codex"
model = "gpt-5.6-sol"
approval_mode = "on-request"
reasoning_mode = "medium"
personality = "direct"
mcp_servers = ["github", "figma"]

[customization.merge_rules]
block_on_open_todos = true
block_on_open_comments = true
block_on_failed_checks = true
block_on_pending_checks = false
definition_of_done = "Tests run and comments resolved."

[customization.workspace_defaults]
base_branch = "main"
workspace_parent = "/tmp/workspaces"
branch_prefix = "lc"
port_block_size = 20
auto_open = true
checkpoint_timing = "manual"
default_visible_tab = "changes"

[customization.view]
theme = "system"
accent_color = "blue"
density = "compact"
sidebar_layout = "grouped"
diff_preference = "unified"
terminal_font = "JetBrains Mono 12"
terminal_scrollback = 5000
transcript_display = "structured"
dashboard_columns = ["repo", "workspace", "status"]
notification_rules = ["checks_failed", "review_requested"]
keybindings = "vim"
command_palette_presets = ["ci", "review"]
settings_import_export = "toml"

[customization.view.colors]
accent = "#5b9dff"
accent_fg = "#071a33"
background = "#191919"
text = "#e4e4e4"
"##,
        )
        .unwrap();
        fs::write(
            conductor_dir.join("settings.local.toml"),
            r##"
[customization.automation]
auto_start_agent = "claude"

[customization.agent_profiles.default]
reasoning_mode = "high"

[customization.view]
density = "comfortable"

[customization.view.colors]
accent = "#0ea5e9"
surface = "#102030"
"##,
        )
        .unwrap();

        let settings = load_repository_settings(temp.path()).unwrap();

        assert_eq!(
            settings.prompts.as_ref().unwrap().test_fixing,
            Some("Fix the failing test first.".to_owned())
        );
        assert_eq!(
            settings.prompts.as_ref().unwrap().new_workspace,
            Some("Plan the workspace.".to_owned())
        );
        assert_eq!(
            settings.prompt_pack.path,
            Some(".archductor/prompt-packs/core.toml".to_owned())
        );
        assert_eq!(
            settings.scripts.typecheck,
            Some("cargo check --workspace".to_owned())
        );
        assert_eq!(
            settings.customization.naming.branch_template,
            Some("{prefix}/{type}-{slug}".to_owned())
        );
        assert_eq!(
            settings.customization.automation.auto_start_agent,
            Some("claude".to_owned())
        );
        assert_eq!(
            settings
                .customization
                .agent_profiles
                .get("default")
                .unwrap()
                .model,
            Some("gpt-5.6-sol".to_owned())
        );
        assert_eq!(
            settings
                .customization
                .agent_profiles
                .get("default")
                .unwrap()
                .reasoning_mode,
            Some("high".to_owned())
        );
        assert_eq!(
            settings.customization.automation.typecheck_command,
            Some("cargo check --workspace".to_owned())
        );
        assert_eq!(
            settings.customization.merge_rules.definition_of_done,
            Some("Tests run and comments resolved.".to_owned())
        );
        assert_eq!(
            settings.customization.workspace_defaults.port_block_size,
            Some(20)
        );
        assert_eq!(
            settings.customization.view.density,
            Some("comfortable".to_owned())
        );
        assert_eq!(
            settings.customization.view.colors.get("accent"),
            Some(&"#0ea5e9".to_owned())
        );
        assert_eq!(
            settings.customization.view.colors.get("surface"),
            Some(&"#102030".to_owned())
        );
        assert_eq!(
            settings.customization.view.colors.get("background"),
            Some(&"#191919".to_owned())
        );

        save_repository_settings(temp.path(), SettingsLayer::RepositoryShared, &settings).unwrap();
        assert_eq!(load_repository_settings(temp.path()).unwrap(), settings);
    }

    #[test]
    fn customization_toml_helpers_round_trip_customization_block() {
        let settings = CustomizationSettings {
            naming: NamingSettings {
                branch_template: Some("lc/{slug}".to_owned()),
                pr_body_sections: vec!["Summary".to_owned(), "Tests".to_owned()],
                ..NamingSettings::default()
            },
            automation: AutomationSettings {
                auto_setup: Some(true),
                required_local_files: vec![".env".to_owned()],
                ..AutomationSettings::default()
            },
            view: ViewSettings {
                theme: Some("dark".to_owned()),
                colors: BTreeMap::from([("accent".to_owned(), "#0ea5e9".to_owned())]),
                density: Some("compact".to_owned()),
                ..ViewSettings::default()
            },
            ..CustomizationSettings::default()
        };

        let text = customization_settings_to_toml(&settings).unwrap();

        assert!(text.contains("[customization.naming]"));
        assert_eq!(customization_settings_from_toml(&text).unwrap(), settings);
    }

    #[test]
    fn customization_source_toml_preserves_collection_presence() {
        let temp = tempfile::tempdir().unwrap();
        let shared = temp.path().join("config/settings.toml");
        fs::create_dir_all(shared.parent().unwrap()).unwrap();
        fs::write(
            &shared,
            r#"
[customization.view]
theme = "dark"
notification_rules = []
"#,
        )
        .unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(repo.join(".archductor")).unwrap();
        fs::write(
            repo.join(".archductor/settings.local.toml"),
            r#"
[customization.view]
theme = "light"
colors = {}
"#,
        )
        .unwrap();

        let shared_toml = app_shared_customization_settings_toml(&shared).unwrap();
        let local_toml =
            repository_customization_settings_toml(&repo, SettingsLayer::LocalOverride).unwrap();

        assert!(shared_toml.contains("notification_rules = []"));
        assert!(shared_toml.contains("theme = \"dark\""));
        assert!(local_toml.contains("[customization.view.colors]"));
        assert!(local_toml.contains("theme = \"light\""));
    }

    #[test]
    fn rejects_invalid_view_colors() {
        let invalid_color = repository_settings_from_toml(
            r##"
[customization.view.colors]
accent = "alert(1)"
"##,
        )
        .unwrap_err();
        assert!(invalid_color
            .to_string()
            .contains("customization.view.colors.accent must be a hex color"));

        let invalid_key = repository_settings_from_toml(
            r##"
[customization.view.colors]
totally_custom = "#ffffff"
"##,
        )
        .unwrap_err();
        assert!(invalid_key
            .to_string()
            .contains("customization.view.colors.totally_custom is not a supported color key"));
    }

    #[test]
    fn repository_toml_helpers_parse_validate_and_serialize_settings() {
        let settings = repository_settings_from_toml(
            r#"
[scripts]
run = "cargo run"

[customization.workspace_defaults]
default_visible_tab = "checks"

[customization.view]
keybindings = "palette=ctrl+p,refresh=ctrl+shift+r"
"#,
        )
        .unwrap();

        assert_eq!(settings.scripts.run.as_deref(), Some("cargo run"));
        assert_eq!(
            settings
                .customization
                .workspace_defaults
                .default_visible_tab
                .as_deref(),
            Some("checks")
        );
        assert_eq!(
            settings.customization.view.keybindings.as_deref(),
            Some("palette=ctrl+p,refresh=ctrl+shift+r")
        );
        let text = repository_settings_to_toml(&settings).unwrap();
        assert!(text.contains("[customization.view]"));
        assert!(text.contains("keybindings"));
    }

    #[test]
    fn inspect_repository_settings_reports_worktreeinclude_precedence() {
        let temp = tempfile::tempdir().unwrap();
        let conductor_dir = temp.path().join(".archductor");
        fs::create_dir(&conductor_dir).unwrap();
        fs::write(
            conductor_dir.join("settings.toml"),
            r#"
file_include_globs = ".env.local"
"#,
        )
        .unwrap();
        fs::write(temp.path().join(".worktreeinclude"), ".env*\ncerts/**\n").unwrap();

        let inspection = inspect_repository_settings(temp.path()).unwrap();

        assert!(inspection.shared_settings_exists);
        assert!(inspection.worktreeinclude_exists);
        assert_eq!(
            inspection.active_file_patterns_source,
            FilePatternSource::Worktreeinclude
        );
        assert_eq!(inspection.active_file_patterns, [".env*", "certs/**"]);
    }

    #[test]
    fn rejects_invalid_repository_settings() {
        let settings = RepositorySettings {
            scripts: ScriptSettings {
                run_mode: Some("parallel".to_owned()),
                ..ScriptSettings::default()
            },
            environment_variables: vec![("1_BAD".to_owned(), "value".to_owned())],
            ..RepositorySettings::default()
        };

        assert!(validate_repository_settings(&settings).is_err());
    }

    #[test]
    fn rejects_unsafe_workspace_working_directory_settings() {
        for working_directory in ["/tmp/app", "../outside", "apps/../other", "apps\0web"] {
            let settings = RepositorySettings {
                customization: CustomizationSettings {
                    workspace_defaults: WorkspaceDefaultSettings {
                        working_directory: Some(working_directory.to_owned()),
                        ..WorkspaceDefaultSettings::default()
                    },
                    ..CustomizationSettings::default()
                },
                ..RepositorySettings::default()
            };

            assert!(
                validate_repository_settings(&settings).is_err(),
                "{working_directory:?} should be rejected"
            );
        }
    }

    #[test]
    fn rejects_invalid_default_merge_method_setting() {
        let settings = RepositorySettings {
            customization: CustomizationSettings {
                naming: NamingSettings {
                    default_merge_method: Some("octopus".to_owned()),
                    ..NamingSettings::default()
                },
                ..CustomizationSettings::default()
            },
            ..RepositorySettings::default()
        };

        assert!(validate_repository_settings(&settings).is_err());
    }

    #[test]
    fn rejects_invalid_default_visible_tab_setting() {
        let settings = RepositorySettings {
            customization: CustomizationSettings {
                workspace_defaults: WorkspaceDefaultSettings {
                    default_visible_tab: Some("calendar".to_owned()),
                    ..WorkspaceDefaultSettings::default()
                },
                ..CustomizationSettings::default()
            },
            ..RepositorySettings::default()
        };

        let err = validate_repository_settings(&settings).unwrap_err();
        assert!(err.to_string().contains("default_visible_tab"));
    }
}
