use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepositorySettings {
    pub file_include_globs: Vec<String>,
    pub spotlight_testing: Option<bool>,
    pub enterprise_data_privacy: Option<bool>,
    pub scripts: ScriptSettings,
    pub environment_variables: Vec<(String, String)>,
    pub prompts: Option<PromptSettings>,
    pub providers: ProviderSettings,
    pub git: GitSettings,
    pub customization: CustomizationSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PromptSettings {
    pub general: Option<String>,
    pub code_review: Option<String>,
    pub create_pr: Option<String>,
    pub fix_errors: Option<String>,
    pub resolve_merge_conflicts: Option<String>,
    pub rename_branch: Option<String>,
    pub commit_generation: Option<String>,
    pub test_fixing: Option<String>,
    pub refactor_style: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderSettings {
    pub claude_code_executable_path: Option<String>,
    pub codex_executable_path: Option<String>,
    pub claude_provider: Option<String>,
    pub codex_provider: Option<String>,
    pub bedrock_region: Option<String>,
    pub vertex_project_id: Option<String>,
    pub ssh_key_path: Option<String>,
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
    let shared = load_optional_settings(&repo_path.join(".archductor/settings.toml"))?;
    let local = load_optional_settings(&repo_path.join(".archductor/settings.local.toml"))?;
    Ok(shared.merge(local).into_settings())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsLayer {
    RepositoryShared,
    LocalOverride,
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
    let settings = load_repository_settings(repo_path)?;
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
    validate_repository_settings(settings)?;
    let conductor_dir = repo_path.join(".archductor");
    std::fs::create_dir_all(&conductor_dir)
        .with_context(|| format!("create {}", conductor_dir.display()))?;
    let path = match layer {
        SettingsLayer::RepositoryShared => conductor_dir.join("settings.toml"),
        SettingsLayer::LocalOverride => conductor_dir.join("settings.local.toml"),
    };
    let raw = RawRepositorySettings::from_settings(settings);
    let contents = toml::to_string_pretty(&raw).context("serialize repository settings")?;
    std::fs::write(&path, contents).with_context(|| format!("write {}", path.display()))
}

pub fn customization_settings_to_toml(settings: &CustomizationSettings) -> Result<String> {
    let raw = RawRepositorySettings {
        customization: Some(RawCustomizationSettings::from_settings(settings)),
        ..RawRepositorySettings::default()
    };
    toml::to_string_pretty(&raw).context("serialize customization settings")
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

pub fn repository_settings_from_toml(contents: &str) -> Result<RepositorySettings> {
    let raw: RawRepositorySettings =
        toml::from_str(contents).context("parse repository settings")?;
    let settings = raw.into_settings();
    validate_repository_settings(&settings)?;
    Ok(settings)
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
    ] {
        if let Some(command) = command {
            anyhow::ensure!(!command.contains('\0'), "{label} cannot contain NUL bytes");
        }
    }
    for (key, _) in &settings.environment_variables {
        anyhow::ensure!(
            is_valid_environment_key(key),
            "environment variable key {key:?} is invalid"
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
    Ok(())
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct RawRepositorySettings {
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    schema: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_include_globs: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    spotlight_testing: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    enterprise_data_privacy: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scripts: Option<RawScriptSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    environment_variables: Option<BTreeMap<String, String>>,
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
    general: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    ssh_key_path: Option<String>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pr_body_sections: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_merge_method: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct RawAutomationSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    auto_setup: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auto_start_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    required_local_files: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    test_command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lint_command: Option<String>,
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
    approval_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    personality: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    mcp_servers: Vec<String>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    dashboard_columns: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    notification_rules: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    keybindings: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    command_palette_presets: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    settings_import_export: Option<String>,
}

impl RawRepositorySettings {
    fn merge(self, local: Self) -> Self {
        Self {
            file_include_globs: local.file_include_globs.or(self.file_include_globs),
            spotlight_testing: local.spotlight_testing.or(self.spotlight_testing),
            enterprise_data_privacy: local
                .enterprise_data_privacy
                .or(self.enterprise_data_privacy),
            scripts: Some(
                self.scripts
                    .unwrap_or_default()
                    .merge(local.scripts.unwrap_or_default()),
            ),
            environment_variables: Some(merge_maps(
                self.environment_variables.unwrap_or_default(),
                local.environment_variables.unwrap_or_default(),
            )),
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
            spotlight_testing: self.spotlight_testing,
            enterprise_data_privacy: self.enterprise_data_privacy,
            scripts: ScriptSettings {
                setup: scripts.setup,
                run: scripts.run,
                archive: scripts.archive,
                run_mode: scripts.run_mode,
            },
            environment_variables: self
                .environment_variables
                .unwrap_or_default()
                .into_iter()
                .collect(),
            prompts: self.prompts.map(|p| PromptSettings {
                general: p.general,
                code_review: p.code_review,
                create_pr: p.create_pr,
                fix_errors: p.fix_errors,
                resolve_merge_conflicts: p.resolve_merge_conflicts,
                rename_branch: p.rename_branch,
                commit_generation: p.commit_generation,
                test_fixing: p.test_fixing,
                refactor_style: p.refactor_style,
            }),
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
            spotlight_testing: settings.spotlight_testing,
            enterprise_data_privacy: settings.enterprise_data_privacy,
            scripts: Some(RawScriptSettings {
                setup: settings.scripts.setup.clone(),
                run: settings.scripts.run.clone(),
                archive: settings.scripts.archive.clone(),
                run_mode: settings.scripts.run_mode.clone(),
            }),
            environment_variables: (!settings.environment_variables.is_empty()).then(|| {
                settings
                    .environment_variables
                    .iter()
                    .cloned()
                    .collect::<BTreeMap<_, _>>()
            }),
            prompts: settings.prompts.as_ref().map(|p| RawPromptSettings {
                general: p.general.clone(),
                code_review: p.code_review.clone(),
                create_pr: p.create_pr.clone(),
                fix_errors: p.fix_errors.clone(),
                resolve_merge_conflicts: p.resolve_merge_conflicts.clone(),
                rename_branch: p.rename_branch.clone(),
                commit_generation: p.commit_generation.clone(),
                test_fixing: p.test_fixing.clone(),
                refactor_style: p.refactor_style.clone(),
            }),
            providers: RawProviderSettings::from_settings(&settings.providers),
            git: Some(RawGitSettings::from_settings(&settings.git)),
            customization: Some(RawCustomizationSettings::from_settings(
                &settings.customization,
            )),
        }
    }
}

impl RawScriptSettings {
    fn merge(self, local: Self) -> Self {
        Self {
            setup: local.setup.or(self.setup),
            run: local.run.or(self.run),
            archive: local.archive.or(self.archive),
            run_mode: local.run_mode.or(self.run_mode),
        }
    }
}

impl RawPromptSettings {
    fn merge(self, local: Self) -> Self {
        Self {
            general: local.general.or(self.general),
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
            ssh_key_path: local.ssh_key_path.or(self.ssh_key_path),
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
            ssh_key_path: self.ssh_key_path,
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
            ssh_key_path: settings.ssh_key_path.clone(),
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
            agent_profiles: Some(merge_profile_maps(
                self.agent_profiles.unwrap_or_default(),
                local.agent_profiles.unwrap_or_default(),
            )),
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
            pr_body_sections: if local.pr_body_sections.is_empty() {
                self.pr_body_sections
            } else {
                local.pr_body_sections
            },
            default_merge_method: local.default_merge_method.or(self.default_merge_method),
        }
    }

    fn into_settings(self) -> NamingSettings {
        NamingSettings {
            branch_template: self.branch_template,
            workspace_name_style: self.workspace_name_style,
            commit_style: self.commit_style,
            pr_title_template: self.pr_title_template,
            pr_body_sections: self.pr_body_sections,
            default_merge_method: self.default_merge_method,
        }
    }

    fn from_settings(settings: &NamingSettings) -> Self {
        Self {
            branch_template: settings.branch_template.clone(),
            workspace_name_style: settings.workspace_name_style.clone(),
            commit_style: settings.commit_style.clone(),
            pr_title_template: settings.pr_title_template.clone(),
            pr_body_sections: settings.pr_body_sections.clone(),
            default_merge_method: settings.default_merge_method.clone(),
        }
    }
}

impl RawAutomationSettings {
    fn merge(self, local: Self) -> Self {
        Self {
            auto_setup: local.auto_setup.or(self.auto_setup),
            auto_start_agent: local.auto_start_agent.or(self.auto_start_agent),
            required_local_files: if local.required_local_files.is_empty() {
                self.required_local_files
            } else {
                local.required_local_files
            },
            test_command: local.test_command.or(self.test_command),
            lint_command: local.lint_command.or(self.lint_command),
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
            required_local_files: self.required_local_files,
            test_command: self.test_command,
            lint_command: self.lint_command,
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
            required_local_files: settings.required_local_files.clone(),
            test_command: settings.test_command.clone(),
            lint_command: settings.lint_command.clone(),
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
            approval_mode: local.approval_mode.or(self.approval_mode),
            reasoning_mode: local.reasoning_mode.or(self.reasoning_mode),
            personality: local.personality.or(self.personality),
            mcp_servers: if local.mcp_servers.is_empty() {
                self.mcp_servers
            } else {
                local.mcp_servers
            },
        }
    }

    fn into_settings(self) -> AgentProfileSettings {
        AgentProfileSettings {
            agent: self.agent,
            approval_mode: self.approval_mode,
            reasoning_mode: self.reasoning_mode,
            personality: self.personality,
            mcp_servers: self.mcp_servers,
        }
    }

    fn from_settings(settings: &AgentProfileSettings) -> Self {
        Self {
            agent: settings.agent.clone(),
            approval_mode: settings.approval_mode.clone(),
            reasoning_mode: settings.reasoning_mode.clone(),
            personality: settings.personality.clone(),
            mcp_servers: settings.mcp_servers.clone(),
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
            density: local.density.or(self.density),
            sidebar_layout: local.sidebar_layout.or(self.sidebar_layout),
            diff_preference: local.diff_preference.or(self.diff_preference),
            terminal_font: local.terminal_font.or(self.terminal_font),
            terminal_scrollback: local.terminal_scrollback.or(self.terminal_scrollback),
            transcript_display: local.transcript_display.or(self.transcript_display),
            dashboard_columns: if local.dashboard_columns.is_empty() {
                self.dashboard_columns
            } else {
                local.dashboard_columns
            },
            notification_rules: if local.notification_rules.is_empty() {
                self.notification_rules
            } else {
                local.notification_rules
            },
            keybindings: local.keybindings.or(self.keybindings),
            command_palette_presets: if local.command_palette_presets.is_empty() {
                self.command_palette_presets
            } else {
                local.command_palette_presets
            },
            settings_import_export: local.settings_import_export.or(self.settings_import_export),
        }
    }

    fn into_settings(self) -> ViewSettings {
        ViewSettings {
            theme: self.theme,
            accent_color: self.accent_color,
            density: self.density,
            sidebar_layout: self.sidebar_layout,
            diff_preference: self.diff_preference,
            terminal_font: self.terminal_font,
            terminal_scrollback: self.terminal_scrollback,
            transcript_display: self.transcript_display,
            dashboard_columns: self.dashboard_columns,
            notification_rules: self.notification_rules,
            keybindings: self.keybindings,
            command_palette_presets: self.command_palette_presets,
            settings_import_export: self.settings_import_export,
        }
    }

    fn from_settings(settings: &ViewSettings) -> Self {
        Self {
            theme: settings.theme.clone(),
            accent_color: settings.accent_color.clone(),
            density: settings.density.clone(),
            sidebar_layout: settings.sidebar_layout.clone(),
            diff_preference: settings.diff_preference.clone(),
            terminal_font: settings.terminal_font.clone(),
            terminal_scrollback: settings.terminal_scrollback,
            transcript_display: settings.transcript_display.clone(),
            dashboard_columns: settings.dashboard_columns.clone(),
            notification_rules: settings.notification_rules.clone(),
            keybindings: settings.keybindings.clone(),
            command_palette_presets: settings.command_palette_presets.clone(),
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

fn load_optional_settings(path: &Path) -> Result<RawRepositorySettings> {
    if !path.exists() {
        return Ok(RawRepositorySettings::default());
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

fn is_valid_environment_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(first) if first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
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
            spotlight_testing: Some(true),
            enterprise_data_privacy: Some(false),
            scripts: ScriptSettings {
                setup: Some("pnpm install".to_owned()),
                run: Some("pnpm dev --port $ARCHDUCTOR_PORT".to_owned()),
                archive: Some("./script/archive.sh".to_owned()),
                run_mode: Some("nonconcurrent".to_owned()),
            },
            environment_variables: vec![(
                "API_BASE_URL".to_owned(),
                "http://localhost:3000".to_owned(),
            )],
            prompts: Some(PromptSettings {
                general: Some("Ship small changes.".to_owned()),
                code_review: Some("Find correctness issues.".to_owned()),
                create_pr: Some("Include test evidence.".to_owned()),
                fix_errors: Some("Focus on failing checks.".to_owned()),
                resolve_merge_conflicts: Some("Preserve user changes.".to_owned()),
                rename_branch: Some("Use short feature names.".to_owned()),
                commit_generation: None,
                test_fixing: None,
                refactor_style: None,
            }),
            providers: ProviderSettings {
                claude_code_executable_path: Some("/usr/local/bin/claude".to_owned()),
                codex_executable_path: Some("/usr/local/bin/codex".to_owned()),
                claude_provider: Some("anthropic".to_owned()),
                codex_provider: Some("openai".to_owned()),
                bedrock_region: None,
                vertex_project_id: None,
                ssh_key_path: None,
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
        assert!(temp.path().join(".archductor/settings.toml").exists());
        assert!(!temp.path().join(".archductor/settings.local.toml").exists());
    }

    #[test]
    fn loads_merges_and_saves_customization_settings() {
        let temp = tempfile::tempdir().unwrap();
        let conductor_dir = temp.path().join(".archductor");
        fs::create_dir(&conductor_dir).unwrap();
        fs::write(
            conductor_dir.join("settings.toml"),
            r#"
[prompts]
test_fixing = "Fix the failing test first."
refactor_style = "Keep refactors behavior-preserving."

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
build_command = "cargo build --workspace"
pre_workspace_create = "just pre-workspace"
post_workspace_create = "just post-workspace"
pre_pr_create = "just pre-pr"
post_merge = "just post-merge"

[customization.agent_profiles.default]
agent = "codex"
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
accent_color = "green"
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
"#,
        )
        .unwrap();
        fs::write(
            conductor_dir.join("settings.local.toml"),
            r#"
[customization.automation]
auto_start_agent = "claude"

[customization.agent_profiles.default]
reasoning_mode = "high"

[customization.view]
density = "comfortable"
"#,
        )
        .unwrap();

        let settings = load_repository_settings(temp.path()).unwrap();

        assert_eq!(
            settings.prompts.as_ref().unwrap().test_fixing,
            Some("Fix the failing test first.".to_owned())
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
                .reasoning_mode,
            Some("high".to_owned())
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
