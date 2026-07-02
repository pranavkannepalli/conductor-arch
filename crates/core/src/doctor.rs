use std::collections::HashMap;
use std::path::Path;

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
    pub gh_installed: bool,
    pub codex_installed: bool,
    pub claude_installed: bool,
    pub opencode_installed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupBlocker {
    MissingGithubCli,
    MissingAgent,
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
            gh_installed: command_exists("gh"),
            codex_installed: command_exists("codex"),
            claude_installed: command_exists("claude"),
            opencode_installed: command_exists("opencode"),
        }
    }

    pub fn any_agent_installed(&self) -> bool {
        self.codex_installed || self.claude_installed || self.opencode_installed
    }
}

pub fn setup_blockers(readiness: &SetupReadiness) -> Vec<SetupBlocker> {
    let mut blockers = Vec::new();
    if !readiness.gh_installed {
        blockers.push(SetupBlocker::MissingGithubCli);
    }
    if !readiness.any_agent_installed() {
        blockers.push(SetupBlocker::MissingAgent);
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
            gh_installed: false,
            codex_installed: true,
            claude_installed: false,
            opencode_installed: false,
        };

        assert_eq!(
            setup_blockers(&readiness),
            vec![SetupBlocker::MissingGithubCli]
        );
    }

    #[test]
    fn setup_blockers_require_at_least_one_agent() {
        let readiness = SetupReadiness {
            gh_installed: true,
            codex_installed: false,
            claude_installed: false,
            opencode_installed: false,
        };

        assert_eq!(setup_blockers(&readiness), vec![SetupBlocker::MissingAgent]);
    }

    #[test]
    fn setup_blockers_accept_opencode_as_agent() {
        let readiness = SetupReadiness {
            gh_installed: true,
            codex_installed: false,
            claude_installed: false,
            opencode_installed: true,
        };

        assert!(setup_blockers(&readiness).is_empty());
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
