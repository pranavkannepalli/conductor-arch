use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyCheck {
    pub name: &'static str,
    pub required: bool,
    pub installed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorReport {
    pub distro_id: Option<String>,
    pub distro_like: Vec<String>,
    pub install_command: Option<&'static str>,
    pub dependencies: Vec<DependencyCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupReadiness {
    pub gh: SetupCheck,
    pub codex: SetupCheck,
    pub claude: SetupCheck,
    pub opencode: SetupCheck,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupBlocker {
    GithubUnavailable,
    MissingAgent,
    SelectedProviderUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupCheck {
    pub installed: bool,
    pub ready: bool,
    pub detail: String,
}

impl SetupCheck {
    pub fn missing(detail: impl Into<String>) -> Self {
        Self {
            installed: false,
            ready: false,
            detail: detail.into(),
        }
    }

    pub fn ready(detail: impl Into<String>) -> Self {
        Self {
            installed: true,
            ready: true,
            detail: detail.into(),
        }
    }

    pub fn blocked(detail: impl Into<String>) -> Self {
        Self {
            installed: true,
            ready: false,
            detail: detail.into(),
        }
    }
}

impl DoctorReport {
    pub fn missing_required(&self) -> Vec<&'static str> {
        self.dependencies
            .iter()
            .filter(|dep| dep.required && !dep.installed)
            .map(|dep| dep.name)
            .collect()
    }
}

impl SetupReadiness {
    pub fn from_host() -> Self {
        Self {
            gh: gh_readiness(),
            codex: codex_readiness(),
            claude: claude_readiness(),
            opencode: opencode_readiness(),
        }
    }

    pub fn any_agent_ready(&self) -> bool {
        self.codex.ready || self.claude.ready || self.opencode.ready
    }

    pub fn first_ready_launchable_provider(&self) -> Option<&'static str> {
        if self.codex.ready {
            Some("codex")
        } else if self.claude.ready {
            Some("claude")
        } else {
            None
        }
    }

    pub fn provider_ready(&self, provider: &str) -> bool {
        match normalize_provider(provider).as_str() {
            "codex" => self.codex.ready,
            "claude" | "claudecode" => self.claude.ready,
            "opencode" => self.opencode.ready,
            _ => false,
        }
    }
}

pub fn setup_blockers(readiness: &SetupReadiness) -> Vec<SetupBlocker> {
    let mut blockers = Vec::new();
    if !readiness.gh.ready {
        blockers.push(SetupBlocker::GithubUnavailable);
    }
    if !readiness.any_agent_ready() {
        blockers.push(SetupBlocker::MissingAgent);
    }
    blockers
}

pub fn setup_blockers_for_provider(
    readiness: &SetupReadiness,
    provider: Option<&str>,
) -> Vec<SetupBlocker> {
    let mut blockers = setup_blockers(readiness);
    if let Some(provider) = provider {
        if !provider.trim().is_empty() && !readiness.provider_ready(provider) {
            blockers.push(SetupBlocker::SelectedProviderUnavailable);
        }
    }
    blockers
}

pub fn report_from_os_release(os_release: &str) -> DoctorReport {
    let parsed = parse_os_release(os_release);
    let distro_id = parsed.get("ID").cloned();
    let distro_like: Vec<String> = parsed
        .get("ID_LIKE")
        .map(|value| value.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default();

    DoctorReport {
        install_command: install_command(distro_id.as_deref(), &distro_like),
        distro_id,
        distro_like,
        dependencies: dependency_checks(),
    }
}

pub fn report_from_host() -> DoctorReport {
    let os_release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    report_from_os_release(&os_release)
}

fn parse_os_release(input: &str) -> HashMap<String, String> {
    input
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_owned(), value.trim_matches('"').to_owned()))
        .collect()
}

fn install_command(id: Option<&str>, like: &[String]) -> Option<&'static str> {
    let matches = |needle: &str| id == Some(needle) || like.iter().any(|item| item == needle);

    if matches("ubuntu") || matches("debian") {
        Some("sudo apt update && sudo apt install git gh sqlite3 openssh-client")
    } else if matches("fedora") {
        Some("sudo dnf install git gh sqlite openssh-clients")
    } else if matches("arch") {
        Some("sudo pacman -S git github-cli sqlite openssh")
    } else if matches("opensuse") || matches("suse") {
        Some("sudo zypper install git gh sqlite3 openssh")
    } else {
        None
    }
}

fn dependency_checks() -> Vec<DependencyCheck> {
    [
        ("git", true),
        ("gh", true),
        ("sqlite3", true),
        ("ssh", true),
        ("codex", false),
        ("claude", false),
        ("opencode", false),
        ("code", false),
        ("cursor", false),
    ]
    .into_iter()
    .map(|(name, required)| DependencyCheck {
        name,
        required,
        installed: command_exists(name),
    })
    .collect()
}

fn gh_readiness() -> SetupCheck {
    if !command_exists("gh") {
        return SetupCheck::missing("Install GitHub CLI.");
    }
    if command_succeeds("gh", &["auth", "status"]) {
        SetupCheck::ready("Authenticated with GitHub.")
    } else {
        SetupCheck::blocked("Run `gh auth login`.")
    }
}

fn codex_readiness() -> SetupCheck {
    if !command_exists("codex") {
        return SetupCheck::missing("Install Codex CLI.");
    }
    if command_succeeds("codex", &["login", "status"]) {
        SetupCheck::ready("Signed in to Codex.")
    } else {
        SetupCheck::blocked("Run `codex login`.")
    }
}

fn claude_readiness() -> SetupCheck {
    if !command_exists("claude") {
        return SetupCheck::missing("Install Claude Code.");
    }
    if command_succeeds("claude", &["auth", "status"]) {
        SetupCheck::ready("Signed in to Claude Code.")
    } else {
        SetupCheck::blocked("Run `claude auth login`.")
    }
}

fn opencode_readiness() -> SetupCheck {
    if !command_exists("opencode") {
        return SetupCheck::missing("Install OpenCode.");
    }
    if command_succeeds("opencode", &["--version"]) {
        SetupCheck::ready("OpenCode CLI responds to version probe.")
    } else {
        SetupCheck::blocked("OpenCode CLI is installed but did not pass a version probe.")
    }
}

fn command_succeeds(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn command_exists(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|path| {
                let candidate = path.join(name);
                is_executable(&candidate)
            })
        })
        .unwrap_or(false)
}

fn normalize_provider(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.metadata()
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
        || std::process::Command::new(path)
            .arg("--version")
            .output()
            .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_apt_guidance_for_ubuntu() {
        let report = report_from_os_release(
            r#"
ID=ubuntu
ID_LIKE=debian
"#,
        );

        assert_eq!(
            report.install_command,
            Some("sudo apt update && sudo apt install git gh sqlite3 openssh-client")
        );
    }

    #[test]
    fn selects_pacman_guidance_for_arch_derivatives() {
        let report = report_from_os_release(
            r#"
ID=endeavouros
ID_LIKE=arch
"#,
        );

        assert_eq!(
            report.install_command,
            Some("sudo pacman -S git github-cli sqlite openssh")
        );
    }

    #[test]
    fn setup_blockers_require_github_cli() {
        let readiness = SetupReadiness {
            gh: SetupCheck::missing("missing"),
            codex: SetupCheck::ready("ready"),
            claude: SetupCheck::missing("missing"),
            opencode: SetupCheck::missing("missing"),
        };

        assert_eq!(
            setup_blockers(&readiness),
            vec![SetupBlocker::GithubUnavailable]
        );
    }

    #[test]
    fn setup_blockers_require_at_least_one_agent() {
        let readiness = SetupReadiness {
            gh: SetupCheck::ready("ready"),
            codex: SetupCheck::missing("missing"),
            claude: SetupCheck::missing("missing"),
            opencode: SetupCheck::missing("missing"),
        };

        assert_eq!(setup_blockers(&readiness), vec![SetupBlocker::MissingAgent]);
    }

    #[test]
    fn setup_blockers_accept_opencode_as_agent() {
        let readiness = SetupReadiness {
            gh: SetupCheck::ready("ready"),
            codex: SetupCheck::missing("missing"),
            claude: SetupCheck::missing("missing"),
            opencode: SetupCheck::ready("ready"),
        };

        assert!(setup_blockers(&readiness).is_empty());
    }

    #[test]
    fn setup_blockers_include_selected_provider_readiness() {
        let readiness = SetupReadiness {
            gh: SetupCheck::ready("ready"),
            codex: SetupCheck::missing("missing"),
            claude: SetupCheck::ready("ready"),
            opencode: SetupCheck::missing("missing"),
        };

        assert_eq!(
            setup_blockers_for_provider(&readiness, Some("codex")),
            vec![SetupBlocker::SelectedProviderUnavailable]
        );
        assert!(setup_blockers_for_provider(&readiness, Some("claude")).is_empty());
    }

    #[test]
    fn first_ready_launchable_provider_prefers_codex_then_claude() {
        let readiness = SetupReadiness {
            gh: SetupCheck::ready("ready"),
            codex: SetupCheck::missing("missing"),
            claude: SetupCheck::ready("ready"),
            opencode: SetupCheck::ready("ready"),
        };

        assert_eq!(readiness.first_ready_launchable_provider(), Some("claude"));
    }

    #[test]
    fn dependency_checks_include_opencode() {
        let names = dependency_checks()
            .into_iter()
            .map(|dep| dep.name)
            .collect::<Vec<_>>();

        assert!(names.contains(&"opencode"));
    }
}
