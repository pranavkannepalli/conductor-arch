use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepositorySettings {
    pub file_include_globs: Vec<String>,
    pub scripts: ScriptSettings,
    pub environment_variables: Vec<(String, String)>,
    pub prompts: Option<PromptSettings>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PromptSettings {
    pub general: Option<String>,
    pub code_review: Option<String>,
    pub create_pr: Option<String>,
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

#[derive(Debug, Clone, Default, Deserialize)]
struct RawRepositorySettings {
    file_include_globs: Option<String>,
    scripts: Option<RawScriptSettings>,
    environment_variables: Option<BTreeMap<String, String>>,
    prompts: Option<RawPromptSettings>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawPromptSettings {
    general: Option<String>,
    code_review: Option<String>,
    create_pr: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawScriptSettings {
    setup: Option<String>,
    run: Option<String>,
    archive: Option<String>,
    run_mode: Option<String>,
}

impl RawRepositorySettings {
    fn merge(self, local: Self) -> Self {
        Self {
            file_include_globs: local.file_include_globs.or(self.file_include_globs),
            scripts: Some(
                self.scripts
                    .unwrap_or_default()
                    .merge(local.scripts.unwrap_or_default()),
            ),
            environment_variables: Some(merge_maps(
                self.environment_variables.unwrap_or_default(),
                local.environment_variables.unwrap_or_default(),
            )),
            prompts: local.prompts.or(self.prompts),
        }
    }

    fn into_settings(self) -> RepositorySettings {
        let scripts = self.scripts.unwrap_or_default();
        RepositorySettings {
            file_include_globs: split_patterns(self.file_include_globs),
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
            }),
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
}
