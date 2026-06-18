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
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PromptSettings {
    pub general: Option<String>,
    pub code_review: Option<String>,
    pub create_pr: Option<String>,
    pub fix_errors: Option<String>,
    pub resolve_merge_conflicts: Option<String>,
    pub rename_branch: Option<String>,
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

pub fn load_repository_settings(repo_path: &Path) -> Result<RepositorySettings> {
    let shared = load_optional_settings(&repo_path.join(".conductor/settings.toml"))?;
    let local = load_optional_settings(&repo_path.join(".conductor/settings.local.toml"))?;
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
    let shared_settings_path = repo_path.join(".conductor/settings.toml");
    let local_settings_path = repo_path.join(".conductor/settings.local.toml");
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
    let conductor_dir = repo_path.join(".conductor");
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
            }),
            providers: self.providers.into_settings(),
            git: self.git.unwrap_or_default().into_settings(),
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
            }),
            providers: RawProviderSettings::from_settings(&settings.providers),
            git: Some(RawGitSettings::from_settings(&settings.git)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn loads_shared_settings_file() {
        let temp = tempfile::tempdir().unwrap();
        let conductor_dir = temp.path().join(".conductor");
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
run = "pnpm dev --port $CONDUCTOR_PORT"
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
            Some("pnpm dev --port $CONDUCTOR_PORT".to_owned())
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
        let conductor_dir = temp.path().join(".conductor");
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
                run: Some("pnpm dev --port $CONDUCTOR_PORT".to_owned()),
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
        };

        save_repository_settings(temp.path(), SettingsLayer::RepositoryShared, &settings).unwrap();
        let loaded = load_repository_settings(temp.path()).unwrap();

        assert_eq!(loaded, settings);
        assert!(temp.path().join(".conductor/settings.toml").exists());
        assert!(!temp.path().join(".conductor/settings.local.toml").exists());
    }

    #[test]
    fn inspect_repository_settings_reports_worktreeinclude_precedence() {
        let temp = tempfile::tempdir().unwrap();
        let conductor_dir = temp.path().join(".conductor");
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
}
