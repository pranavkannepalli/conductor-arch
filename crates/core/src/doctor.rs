use crate::agent_tools::{
    all_tools, launchable_agent_tools, launchable_provider_key, tool_by_provider, ToolSpec,
};
use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

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
        let gh = thread::spawn(gh_readiness);
        let codex = thread::spawn(|| provider_readiness("codex"));
        let claude = thread::spawn(|| provider_readiness("claude"));
        let opencode = thread::spawn(|| provider_readiness("opencode"));

        Self {
            gh: gh
                .join()
                .unwrap_or_else(|_| SetupCheck::blocked("GitHub CLI check failed.")),
            codex: codex
                .join()
                .unwrap_or_else(|_| SetupCheck::blocked("Codex check failed.")),
            claude: claude
                .join()
                .unwrap_or_else(|_| SetupCheck::blocked("Claude check failed.")),
            opencode: opencode
                .join()
                .unwrap_or_else(|_| SetupCheck::blocked("OpenCode check failed.")),
        }
    }

    pub fn any_agent_ready(&self) -> bool {
        launchable_agent_tools().any(|tool| {
            self.provider_check(tool.provider_key)
                .is_some_and(|check| check.ready)
        })
    }

    pub fn first_ready_launchable_provider(&self) -> Option<&'static str> {
        launchable_agent_tools()
            .find(|tool| {
                self.provider_check(tool.provider_key)
                    .is_some_and(|check| check.ready)
            })
            .map(|tool| tool.provider_key)
    }

    pub fn provider_ready(&self, provider: &str) -> bool {
        tool_by_provider(provider)
            .and_then(|tool| self.provider_check(tool.provider_key))
            .is_some_and(|check| check.ready)
    }

    pub fn launchable_provider_ready(&self, provider: &str) -> bool {
        launchable_provider_key(provider)
            .and_then(|provider| self.provider_check(provider))
            .is_some_and(|check| check.ready)
    }

    fn provider_check(&self, provider: &str) -> Option<SetupCheck> {
        match provider {
            "codex" => Some(self.codex.clone()),
            "claude" => Some(self.claude.clone()),
            "opencode" => Some(self.opencode.clone()),
            _ if tool_by_provider(provider).is_some() => Some(provider_readiness(provider)),
            _ => None,
        }
    }
}

pub fn setup_blockers(readiness: &SetupReadiness) -> Vec<SetupBlocker> {
    let mut blockers = Vec::new();
    if !readiness.gh.ready {
        blockers.push(SetupBlocker::GithubUnavailable);
    }
    if readiness.first_ready_launchable_provider().is_none() {
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
        if !provider.trim().is_empty() && !readiness.launchable_provider_ready(provider) {
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
    let mut checks = [
        ("git", true),
        ("gh", true),
        ("sqlite3", true),
        ("ssh", true),
    ]
    .into_iter()
    .map(|(name, required)| DependencyCheck {
        name,
        required,
        installed: command_exists(name),
    })
    .collect::<Vec<_>>();
    checks.extend(all_tools().iter().map(|tool| DependencyCheck {
        name: tool.default_command,
        required: false,
        installed: command_exists(tool.default_command),
    }));
    checks
}

fn gh_readiness() -> SetupCheck {
    if !command_exists("gh") {
        return SetupCheck::missing("Install GitHub CLI.");
    }
    if gh_active_account_ready() {
        SetupCheck::ready("Authenticated with GitHub.")
    } else {
        SetupCheck::blocked(
            "Run `gh auth login --hostname github.com` or `gh auth switch --hostname github.com`.",
        )
    }
}

fn gh_active_account_ready() -> bool {
    let Some(output) = command_output("gh", &["auth", "status", "--json", "hosts"]) else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    gh_status_has_active_github_account(&output.stdout)
}

fn gh_status_has_active_github_account(stdout: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(stdout) else {
        return false;
    };
    value
        .get("hosts")
        .and_then(|hosts| hosts.get("github.com"))
        .and_then(serde_json::Value::as_array)
        .map(|accounts| {
            accounts.iter().any(|account| {
                account.get("active").and_then(serde_json::Value::as_bool) == Some(true)
                    && account.get("state").and_then(serde_json::Value::as_str) == Some("success")
            })
        })
        .unwrap_or(false)
}

fn provider_readiness(provider: &str) -> SetupCheck {
    let Some(tool) = tool_by_provider(provider) else {
        return SetupCheck::missing(format!("Install {provider}."));
    };
    readiness_for_tool(tool)
}

fn readiness_for_tool(tool: &ToolSpec) -> SetupCheck {
    let program = tool
        .readiness_probe
        .first()
        .copied()
        .unwrap_or(tool.default_command);
    if !command_exists(program) {
        return SetupCheck::missing(format!("Install {}.", tool.display_name));
    }
    let readiness_args = tool.readiness_probe.get(1..).unwrap_or_default();
    if command_succeeds(program, readiness_args) {
        SetupCheck::ready(format!("{} is ready.", tool.display_name))
    } else {
        SetupCheck::blocked(tool.auth_guidance)
    }
}

fn command_succeeds(program: &str, args: &[&str]) -> bool {
    command_status(program, args)
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn command_output(program: &str, args: &[&str]) -> Option<Output> {
    run_command_with_timeout(program, args, false)
}

fn command_status(program: &str, args: &[&str]) -> Option<Output> {
    run_command_with_timeout(program, args, true)
}

fn run_command_with_timeout(program: &str, args: &[&str], discard_output: bool) -> Option<Output> {
    let mut command = Command::new(program);
    command.args(args);
    if discard_output {
        command.stdout(Stdio::null()).stderr(Stdio::null());
    } else {
        command.stdout(Stdio::piped()).stderr(Stdio::null());
    }
    let mut child = command.spawn().ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) if started.elapsed() >= PROBE_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(not(unix))]
fn path_version_probe_succeeds(path: &Path) -> bool {
    let mut child = match Command::new(path)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if started.elapsed() >= PROBE_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
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
    path.is_file() || path_version_probe_succeeds(path)
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
    fn setup_blockers_require_launchable_agent_even_when_opencode_ready() {
        let readiness = SetupReadiness {
            gh: SetupCheck::ready("ready"),
            codex: SetupCheck::missing("missing"),
            claude: SetupCheck::missing("missing"),
            opencode: SetupCheck::ready("ready"),
        };

        assert_eq!(setup_blockers(&readiness), vec![SetupBlocker::MissingAgent]);
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
        assert_eq!(
            setup_blockers_for_provider(&readiness, Some("opencode")),
            vec![SetupBlocker::SelectedProviderUnavailable]
        );
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
    fn gh_status_requires_active_successful_github_account() {
        let status = br#"
{
  "hosts": {
    "github.com": [
      {"state": "success", "active": false, "host": "github.com", "login": "old"},
      {"state": "success", "active": true, "host": "github.com", "login": "current"}
    ],
    "github.example.com": [
      {"state": "success", "active": true, "host": "github.example.com", "login": "enterprise"}
    ]
  }
}
"#;

        assert!(gh_status_has_active_github_account(status));

        let stale = br#"
{
  "hosts": {
    "github.com": [
      {"state": "failure", "active": true, "host": "github.com", "login": "current"}
    ]
  }
}
"#;

        assert!(!gh_status_has_active_github_account(stale));
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
