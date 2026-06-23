use crate::workspace::{SessionHarnessOptions, SessionKind};
use std::ffi::OsString;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHarnessLaunchPlan {
    pub args: Vec<String>,
    pub env: Vec<(String, OsString)>,
    pub harness_metadata: Option<String>,
    pub bootstrap_payload: Option<String>,
}

pub fn build_session_harness_launch_plan(
    kind: SessionKind,
    cwd: &Path,
    harness: &SessionHarnessOptions,
) -> SessionHarnessLaunchPlan {
    let bootstrap_payload = build_bootstrap_payload(kind, harness);
    let harness_metadata = build_harness_metadata(kind, harness);
    let mut env = Vec::new();
    if let Some(payload) = &bootstrap_payload {
        env.push((
            "CONDUCTOR_SESSION_BOOTSTRAP".to_owned(),
            OsString::from(payload),
        ));
    }

    let args = match kind {
        SessionKind::Shell => Vec::new(),
        SessionKind::Cursor => Vec::new(),
        SessionKind::Codex => build_codex_args(cwd, harness),
        SessionKind::Claude => build_claude_args(harness, bootstrap_payload.as_deref()),
    };

    SessionHarnessLaunchPlan {
        args,
        env,
        harness_metadata,
        bootstrap_payload,
    }
}

pub fn build_harness_metadata(
    kind: SessionKind,
    harness: &SessionHarnessOptions,
) -> Option<String> {
    let mut entries = Vec::new();
    entries.push(format!("harness={}", session_kind_label(kind)));

    if harness.plan_mode {
        entries.push("plan=true".to_owned());
    }
    if harness.fast_mode {
        entries.push("fast=true".to_owned());
    }
    if let Some(value) = sanitize_text(harness.approval_mode.as_deref()) {
        entries.push(format!("approval={value}"));
    }
    if let Some(value) = sanitize_text(harness.reasoning_mode.as_deref()) {
        entries.push(format!("reasoning={value}"));
    }
    if let Some(value) = sanitize_text(harness.effort_mode.as_deref()) {
        entries.push(format!("effort={value}"));
    }
    if let Some(value) = sanitize_text(harness.codex_personality.as_deref()) {
        entries.push(format!("personality={value}"));
    }
    if let Some(value) = sanitize_text(harness.codex_goals.as_deref()) {
        entries.push(format!("goals={}", sanitize_metadata_value(&value)));
    }
    if let Some(value) = sanitize_text(harness.codex_skills.as_deref()) {
        entries.push(format!("skills={}", sanitize_metadata_value(&value)));
    }

    if entries.len() == 1 {
        None
    } else {
        Some(entries.join(";"))
    }
}

pub fn build_bootstrap_payload(
    kind: SessionKind,
    harness: &SessionHarnessOptions,
) -> Option<String> {
    let mut lines = vec![format!(
        "[conductor bootstrap for {}]",
        session_kind_label(kind)
    )];

    let mut push_line = |line: Option<String>| {
        if let Some(line) = line {
            lines.push(line);
        }
    };

    match kind {
        SessionKind::Shell | SessionKind::Cursor => {
            push_line(if harness.plan_mode {
                Some("plan mode: enabled".to_owned())
            } else {
                None
            });
            push_line(if harness.fast_mode {
                Some("fast mode: enabled".to_owned())
            } else {
                None
            });
        }
        SessionKind::Codex => {
            push_line(if harness.plan_mode {
                Some("/plan".to_owned())
            } else {
                None
            });
            push_line(if harness.fast_mode {
                Some("/fast".to_owned())
            } else {
                None
            });
            push_line(codex_goal_line(harness.codex_goals.as_deref()));
        }
        SessionKind::Claude => {
            push_line(if harness.plan_mode {
                Some("plan mode: enabled".to_owned())
            } else {
                None
            });
            push_line(if harness.fast_mode {
                Some("fast mode: enabled".to_owned())
            } else {
                None
            });
        }
    }

    push_line(optional_kv_line(
        "approval mode",
        harness.approval_mode.as_deref(),
    ));
    push_line(optional_kv_line(
        "reasoning mode",
        harness.reasoning_mode.as_deref(),
    ));
    push_line(optional_kv_line(
        "effort mode",
        harness.effort_mode.as_deref(),
    ));
    push_line(optional_kv_line(
        "personality",
        harness.codex_personality.as_deref(),
    ));
    push_line(optional_kv_line("goals", harness.codex_goals.as_deref()));
    push_line(optional_kv_line("skills", harness.codex_skills.as_deref()));

    if lines.len() == 1 {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn build_codex_args(cwd: &Path, harness: &SessionHarnessOptions) -> Vec<String> {
    let mut args = Vec::new();
    args.push("-C".to_owned());
    args.push(cwd.to_string_lossy().to_string());
    if let Some(policy) = codex_approval_policy(harness.approval_mode.as_deref()) {
        args.push("--ask-for-approval".to_owned());
        args.push(policy.to_owned());
    }
    if sanitize_text(harness.codex_goals.as_deref()).is_some() {
        args.push("--enable".to_owned());
        args.push("goals".to_owned());
    }
    args
}

fn build_claude_args(
    harness: &SessionHarnessOptions,
    bootstrap_payload: Option<&str>,
) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(value) = claude_permission_mode(harness) {
        args.push("--permission-mode".to_owned());
        args.push(value);
    }
    if let Some(value) = claude_effort_mode(harness) {
        args.push("--effort".to_owned());
        args.push(value);
    }
    if let Some(payload) = bootstrap_payload {
        args.push("--append-system-prompt".to_owned());
        args.push(payload.to_owned());
    }
    args
}

fn optional_kv_line(label: &str, value: Option<&str>) -> Option<String> {
    sanitize_text(value).map(|value| format!("{label}: {value}"))
}

fn codex_goal_line(value: Option<&str>) -> Option<String> {
    sanitize_text(value).map(|value| format!("/goal {}", sanitize_metadata_value(&value)))
}

fn codex_approval_policy(value: Option<&str>) -> Option<&'static str> {
    match sanitize_text(value).as_deref() {
        Some("ask") | Some("on-request") => Some("on-request"),
        Some("never") => Some("never"),
        Some("untrusted") => Some("untrusted"),
        Some("default") | None => None,
        Some(_) => Some("on-request"),
    }
}

fn claude_permission_mode(harness: &SessionHarnessOptions) -> Option<String> {
    if harness.plan_mode {
        return Some("plan".to_owned());
    }
    match sanitize_text(harness.approval_mode.as_deref()).as_deref() {
        Some("never") => Some("bypassPermissions".to_owned()),
        Some("ask") | Some("default") => Some("default".to_owned()),
        Some("auto") => Some("auto".to_owned()),
        Some("acceptEdits") => Some("acceptEdits".to_owned()),
        Some("dontAsk") => Some("dontAsk".to_owned()),
        Some("bypassPermissions") => Some("bypassPermissions".to_owned()),
        Some("plan") => Some("plan".to_owned()),
        Some(other) => Some(other.to_owned()),
        None => None,
    }
}

fn claude_effort_mode(harness: &SessionHarnessOptions) -> Option<String> {
    if let Some(value) = sanitize_text(harness.effort_mode.as_deref()) {
        return Some(value);
    }
    if harness.fast_mode {
        return Some("low".to_owned());
    }
    None
}

fn sanitize_metadata_value(value: &str) -> String {
    value.replace(['\n', '\r', ';'], " ")
}

fn sanitize_text(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn session_kind_label(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Shell => "shell",
        SessionKind::Codex => "codex",
        SessionKind::Claude => "claude",
        SessionKind::Cursor => "cursor",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn codex_launch_plan_uses_cd_flag_and_bootstrap_payload() {
        let harness = SessionHarnessOptions {
            plan_mode: true,
            fast_mode: true,
            approval_mode: Some("ask".to_owned()),
            reasoning_mode: Some("high".to_owned()),
            effort_mode: Some("medium".to_owned()),
            codex_personality: Some("careful".to_owned()),
            codex_goals: Some("ship the fix".to_owned()),
            codex_skills: Some("tests".to_owned()),
        };

        let launch =
            build_session_harness_launch_plan(SessionKind::Codex, Path::new("/tmp/work"), &harness);
        let bootstrap = launch.bootstrap_payload.as_deref().unwrap();

        assert_eq!(
            &launch.args,
            &vec![
                "-C",
                "/tmp/work",
                "--ask-for-approval",
                "on-request",
                "--enable",
                "goals"
            ]
        );
        assert_eq!(
            launch.harness_metadata.as_deref(),
            Some(
                "harness=codex;plan=true;fast=true;approval=ask;reasoning=high;effort=medium;personality=careful;goals=ship the fix;skills=tests"
            )
        );
        assert!(bootstrap.contains("[conductor bootstrap for codex]"));
        assert!(bootstrap.contains("/plan"));
        assert!(bootstrap.contains("/fast"));
        assert!(bootstrap.contains("/goal ship the fix"));
        assert!(bootstrap.contains("approval mode: ask"));
        assert!(bootstrap.contains("goals: ship the fix"));
        assert_eq!(
            launch
                .env
                .iter()
                .find(|(key, _)| key == "CONDUCTOR_SESSION_BOOTSTRAP")
                .and_then(|(_, value)| value.to_str()),
            Some(bootstrap)
        );
    }

    #[test]
    fn claude_launch_plan_uses_documented_flags_and_bootstrap_payload() {
        let harness = SessionHarnessOptions {
            plan_mode: true,
            fast_mode: true,
            approval_mode: Some("never".to_owned()),
            reasoning_mode: Some("low".to_owned()),
            effort_mode: Some("high".to_owned()),
            codex_personality: Some("thorough".to_owned()),
            codex_goals: Some("stabilize the fix".to_owned()),
            codex_skills: Some("rust, tests".to_owned()),
        };

        let launch = build_session_harness_launch_plan(
            SessionKind::Claude,
            Path::new("/tmp/work"),
            &harness,
        );
        let bootstrap = launch.bootstrap_payload.as_deref().unwrap();

        assert_eq!(
            &launch.args,
            &vec![
                "--permission-mode",
                "plan",
                "--effort",
                "high",
                "--append-system-prompt",
                bootstrap,
            ]
        );
        assert_eq!(
            launch.harness_metadata.as_deref(),
            Some(
                "harness=claude;plan=true;fast=true;approval=never;reasoning=low;effort=high;personality=thorough;goals=stabilize the fix;skills=rust, tests"
            )
        );
        assert!(bootstrap.contains("[conductor bootstrap for claude]"));
        assert!(bootstrap.contains("plan mode: enabled"));
        assert!(bootstrap.contains("fast mode: enabled"));
        assert!(bootstrap.contains("reasoning mode: low"));
        assert!(bootstrap.contains("skills: rust, tests"));
    }

    #[test]
    fn claude_fast_mode_defaults_effort_to_low_when_not_set() {
        let harness = SessionHarnessOptions {
            fast_mode: true,
            ..SessionHarnessOptions::default()
        };

        let launch = build_session_harness_launch_plan(
            SessionKind::Claude,
            Path::new("/tmp/work"),
            &harness,
        );

        assert!(launch
            .args
            .windows(2)
            .any(|window| window == ["--effort", "low"]));
        assert!(launch
            .bootstrap_payload
            .as_deref()
            .unwrap()
            .contains("fast mode: enabled"));
    }
}
