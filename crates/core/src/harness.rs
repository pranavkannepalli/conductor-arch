use crate::provider_capabilities::add_bare_when_supported;
use crate::workspace::{SessionHarnessOptions, SessionKind};
use std::ffi::OsString;
use std::path::Path;
use uuid::Uuid;

pub const FORCED_APPROVAL_MODE: &str = "never";

pub fn normalize_agent_harness_options(
    mut options: SessionHarnessOptions,
) -> SessionHarnessOptions {
    options.approval_mode = Some(FORCED_APPROVAL_MODE.to_owned());
    options
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHarnessLaunchPlan {
    pub args: Vec<String>,
    pub env: Vec<(String, OsString)>,
    pub harness_metadata: Option<String>,
    pub bootstrap_payload: Option<String>,
    pub session_resume_id: Option<String>,
}

pub fn build_session_harness_launch_plan(
    kind: SessionKind,
    cwd: &Path,
    harness: &SessionHarnessOptions,
) -> SessionHarnessLaunchPlan {
    let bootstrap_payload = build_bootstrap_payload(kind, harness);
    let harness_metadata = build_harness_metadata(kind, harness);
    let session_resume_id = match kind {
        SessionKind::Claude => Some(Uuid::new_v4().to_string()),
        _ => None,
    };
    let mut env = Vec::new();
    if let Some(payload) = &bootstrap_payload {
        env.push((
            "ARCHDUCTOR_SESSION_BOOTSTRAP".to_owned(),
            OsString::from(payload),
        ));
    }

    let args = match kind {
        SessionKind::Shell => Vec::new(),
        SessionKind::Codex => build_codex_args(cwd, harness),
        SessionKind::Claude => build_claude_args(
            harness,
            bootstrap_payload.as_deref(),
            session_resume_id.as_deref(),
        ),
    };

    SessionHarnessLaunchPlan {
        args,
        env,
        harness_metadata,
        bootstrap_payload,
        session_resume_id,
    }
}

pub fn build_session_resume_launch_plan(
    kind: SessionKind,
    cwd: &Path,
    harness: &SessionHarnessOptions,
    session_resume_id: Option<&str>,
) -> SessionHarnessLaunchPlan {
    let bootstrap_payload = build_bootstrap_payload(kind, harness);
    let harness_metadata = build_harness_metadata(kind, harness);
    let session_resume_id = session_resume_id.map(ToOwned::to_owned);
    let mut env = Vec::new();
    if let Some(payload) = &bootstrap_payload {
        env.push((
            "ARCHDUCTOR_SESSION_BOOTSTRAP".to_owned(),
            OsString::from(payload),
        ));
    }

    let args = match kind {
        SessionKind::Shell => Vec::new(),
        SessionKind::Codex => build_codex_resume_args(cwd, harness, session_resume_id.as_deref()),
        SessionKind::Claude => build_claude_resume_args(
            harness,
            bootstrap_payload.as_deref(),
            session_resume_id.as_deref(),
        ),
    };

    SessionHarnessLaunchPlan {
        args,
        env,
        harness_metadata,
        bootstrap_payload,
        session_resume_id,
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
    if let Some(value) = explicit_model(harness) {
        entries.push(format!("model={}", sanitize_metadata_value(&value)));
    }
    let approval_mode = match kind {
        SessionKind::Codex | SessionKind::Claude => Some(FORCED_APPROVAL_MODE),
        SessionKind::Shell => harness.approval_mode.as_deref(),
    };
    if let Some(value) = sanitize_text(approval_mode) {
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
        "[archductor bootstrap for {}]",
        session_kind_label(kind)
    )];

    let mut push_line = |line: Option<String>| {
        if let Some(line) = line {
            lines.push(line);
        }
    };

    match kind {
        SessionKind::Shell => {
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
        SessionKind::Codex => return None,
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

    let model = explicit_model(harness);
    push_line(optional_kv_line("model", model.as_deref()));
    push_line(optional_kv_line(
        "approval mode",
        match kind {
            SessionKind::Codex | SessionKind::Claude => Some(FORCED_APPROVAL_MODE),
            SessionKind::Shell => harness.approval_mode.as_deref(),
        },
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

pub(crate) fn codex_trust_level_config(cwd: &Path) -> String {
    let escaped = cwd
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!(r#"projects."{escaped}".trust_level="trusted""#)
}

fn build_codex_args(cwd: &Path, harness: &SessionHarnessOptions) -> Vec<String> {
    let mut args = vec![
        "--no-alt-screen".to_owned(),
        "--dangerously-bypass-approvals-and-sandbox".to_owned(),
        "-c".to_owned(),
        "check_for_update_on_startup=false".to_owned(),
        "-c".to_owned(),
        codex_trust_level_config(cwd),
        "-C".to_owned(),
        cwd.to_string_lossy().to_string(),
    ];
    add_bare_when_supported(&mut args, "codex");

    if let Some(value) = explicit_model(harness) {
        args.push("--model".to_owned());
        args.push(value);
    }
    if let Some(value) = sanitize_text(harness.reasoning_mode.as_deref()) {
        args.push("-c".to_owned());
        args.push(format!("model_reasoning_effort=\"{value}\""));
    }
    if let Some(value) = sanitize_text(harness.codex_personality.as_deref()) {
        args.push("-c".to_owned());
        args.push(format!("personality=\"{value}\""));
    }
    if harness.fast_mode {
        args.push("-c".to_owned());
        args.push("service_tier=\"fast\"".to_owned());
    }
    if let Some(value) = sanitize_text(harness.codex_goals.as_deref()) {
        args.push("--enable".to_owned());
        args.push("goals".to_owned());
        args.push(value);
    }

    args
}

fn build_codex_resume_args(
    cwd: &Path,
    harness: &SessionHarnessOptions,
    session_resume_id: Option<&str>,
) -> Vec<String> {
    let mut args = build_codex_args(cwd, harness);
    args.push("resume".to_owned());
    args.push(
        session_resume_id
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| "--last".to_owned()),
    );
    args
}

fn build_claude_args(
    harness: &SessionHarnessOptions,
    bootstrap_payload: Option<&str>,
    session_id: Option<&str>,
) -> Vec<String> {
    let mut args = Vec::new();
    args.push("--permission-mode".to_owned());
    args.push(claude_permission_mode(harness));
    args.push("--dangerously-skip-permissions".to_owned());
    add_bare_when_supported(&mut args, "claude");
    if let Some(value) = explicit_model(harness) {
        args.push("--model".to_owned());
        args.push(value);
    }
    if let Some(value) = claude_effort_mode(harness) {
        args.push("--effort".to_owned());
        args.push(value);
    }
    if let Some(session_id) = session_id {
        args.push("--session-id".to_owned());
        args.push(session_id.to_owned());
    }
    if let Some(payload) = bootstrap_payload {
        args.push("--append-system-prompt".to_owned());
        args.push(payload.to_owned());
    }
    args
}

fn build_claude_resume_args(
    harness: &SessionHarnessOptions,
    bootstrap_payload: Option<&str>,
    session_id: Option<&str>,
) -> Vec<String> {
    let mut args = Vec::new();
    args.push("--permission-mode".to_owned());
    args.push(claude_permission_mode(harness));
    args.push("--dangerously-skip-permissions".to_owned());
    add_bare_when_supported(&mut args, "claude");
    if let Some(value) = explicit_model(harness) {
        args.push("--model".to_owned());
        args.push(value);
    }
    if let Some(value) = claude_effort_mode(harness) {
        args.push("--effort".to_owned());
        args.push(value);
    }
    if let Some(session_id) = session_id {
        args.push("--resume".to_owned());
        args.push(session_id.to_owned());
    } else {
        args.push("--continue".to_owned());
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

fn explicit_model(harness: &SessionHarnessOptions) -> Option<String> {
    sanitize_text(harness.model.as_deref())
}

fn claude_permission_mode(_harness: &SessionHarnessOptions) -> String {
    "bypassPermissions".to_owned()
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn codex_and_claude_force_bypass_for_every_requested_approval_mode() {
        for requested in [
            None,
            Some("ask"),
            Some("on-request"),
            Some("default"),
            Some("never"),
        ] {
            let options = SessionHarnessOptions {
                approval_mode: requested.map(str::to_owned),
                ..SessionHarnessOptions::default()
            };
            let codex = build_codex_args(Path::new("/tmp/workspace"), &options);
            assert!(codex
                .iter()
                .any(|arg| arg == "--dangerously-bypass-approvals-and-sandbox"));
            assert!(!codex.iter().any(|arg| arg == "--ask-for-approval"));

            let claude = build_claude_args(&options, None, None);
            assert!(claude
                .windows(2)
                .any(|args| args == ["--permission-mode", "bypassPermissions"]));
            assert!(claude
                .iter()
                .any(|arg| arg == "--dangerously-skip-permissions"));
        }
    }

    #[test]
    fn codex_launch_plan_uses_documented_flags_without_bootstrap_payload() {
        let harness = SessionHarnessOptions {
            plan_mode: true,
            fast_mode: true,
            model: Some("gpt-5.6-sol".to_owned()),
            approval_mode: Some("ask".to_owned()),
            reasoning_mode: Some("high".to_owned()),
            effort_mode: Some("medium".to_owned()),
            codex_personality: Some("pragmatic".to_owned()),
            codex_goals: Some("ship the fix".to_owned()),
            codex_skills: Some("tests".to_owned()),
        };

        let launch =
            build_session_harness_launch_plan(SessionKind::Codex, Path::new("/tmp/work"), &harness);

        assert_eq!(
            &launch.args,
            &vec![
                "--no-alt-screen",
                "--dangerously-bypass-approvals-and-sandbox",
                "-c",
                "check_for_update_on_startup=false",
                "-c",
                r#"projects."/tmp/work".trust_level="trusted""#,
                "-C",
                "/tmp/work",
                "--model",
                "gpt-5.6-sol",
                "-c",
                r#"model_reasoning_effort="high""#,
                "-c",
                r#"personality="pragmatic""#,
                "-c",
                r#"service_tier="fast""#,
                "--enable",
                "goals",
                "ship the fix",
            ]
        );
        assert_eq!(
            launch.harness_metadata.as_deref(),
            Some(
                "harness=codex;plan=true;fast=true;model=gpt-5.6-sol;approval=never;reasoning=high;effort=medium;personality=pragmatic;goals=ship the fix;skills=tests"
            )
        );
        assert!(launch.session_resume_id.is_none());
        assert!(launch.bootstrap_payload.is_none());
        assert!(launch
            .env
            .iter()
            .all(|(key, _)| key != "ARCHDUCTOR_SESSION_BOOTSTRAP"));
    }

    #[test]
    fn claude_launch_plan_uses_documented_flags_and_bootstrap_payload() {
        let harness = SessionHarnessOptions {
            plan_mode: true,
            fast_mode: true,
            model: Some("claude-fable-5".to_owned()),
            approval_mode: Some("never".to_owned()),
            reasoning_mode: Some("low".to_owned()),
            effort_mode: Some("high".to_owned()),
            codex_personality: Some("friendly".to_owned()),
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
                "bypassPermissions",
                "--dangerously-skip-permissions",
                "--model",
                "claude-fable-5",
                "--effort",
                "high",
                "--session-id",
                launch.session_resume_id.as_deref().unwrap(),
                "--append-system-prompt",
                bootstrap,
            ]
        );
        assert_eq!(
            launch.harness_metadata.as_deref(),
            Some(
                "harness=claude;plan=true;fast=true;model=claude-fable-5;approval=never;reasoning=low;effort=high;personality=friendly;goals=stabilize the fix;skills=rust, tests"
            )
        );
        assert!(launch.session_resume_id.is_some());
        assert!(bootstrap.contains("[archductor bootstrap for claude]"));
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

    #[test]
    fn resume_launch_plan_uses_resume_flags() {
        let harness = SessionHarnessOptions {
            fast_mode: true,
            approval_mode: Some("ask".to_owned()),
            reasoning_mode: Some("high".to_owned()),
            codex_personality: Some("pragmatic".to_owned()),
            codex_goals: Some("ship the fix".to_owned()),
            ..SessionHarnessOptions::default()
        };
        let codex_resume = build_session_resume_launch_plan(
            SessionKind::Codex,
            Path::new("/tmp/work"),
            &harness,
            Some("019ef6b1-8a1b-78f0-ae17-0db46572decf"),
        );
        assert_eq!(
            codex_resume.args,
            vec![
                "--no-alt-screen".to_owned(),
                "--dangerously-bypass-approvals-and-sandbox".to_owned(),
                "-c".to_owned(),
                "check_for_update_on_startup=false".to_owned(),
                "-c".to_owned(),
                r#"projects."/tmp/work".trust_level="trusted""#.to_owned(),
                "-C".to_owned(),
                "/tmp/work".to_owned(),
                "-c".to_owned(),
                r#"model_reasoning_effort="high""#.to_owned(),
                "-c".to_owned(),
                r#"personality="pragmatic""#.to_owned(),
                "-c".to_owned(),
                r#"service_tier="fast""#.to_owned(),
                "--enable".to_owned(),
                "goals".to_owned(),
                "ship the fix".to_owned(),
                "resume".to_owned(),
                "019ef6b1-8a1b-78f0-ae17-0db46572decf".to_owned()
            ]
        );

        let claude_resume = build_session_resume_launch_plan(
            SessionKind::Claude,
            Path::new("/tmp/work"),
            &harness,
            Some("019ef6b1-8a1b-78f0-ae17-0db46572decf"),
        );
        let claude_bootstrap = claude_resume.bootstrap_payload.as_deref().unwrap();
        assert_eq!(
            claude_resume.args,
            vec![
                "--permission-mode".to_owned(),
                "bypassPermissions".to_owned(),
                "--dangerously-skip-permissions".to_owned(),
                "--effort".to_owned(),
                "low".to_owned(),
                "--resume".to_owned(),
                "019ef6b1-8a1b-78f0-ae17-0db46572decf".to_owned(),
                "--append-system-prompt".to_owned(),
                claude_bootstrap.to_owned(),
            ]
        );
        assert_eq!(
            claude_resume.session_resume_id.as_deref(),
            Some("019ef6b1-8a1b-78f0-ae17-0db46572decf")
        );
    }

    #[test]
    fn codex_resume_launch_without_id_uses_last_and_preserves_harness_flags() {
        let harness = SessionHarnessOptions {
            fast_mode: true,
            approval_mode: Some("ask".to_owned()),
            reasoning_mode: Some("high".to_owned()),
            codex_personality: Some("pragmatic".to_owned()),
            codex_goals: Some("ship the fix".to_owned()),
            ..SessionHarnessOptions::default()
        };

        let codex_resume = build_session_resume_launch_plan(
            SessionKind::Codex,
            Path::new("/tmp/work"),
            &harness,
            None,
        );

        assert_eq!(
            codex_resume.args,
            vec![
                "--no-alt-screen".to_owned(),
                "--dangerously-bypass-approvals-and-sandbox".to_owned(),
                "-c".to_owned(),
                "check_for_update_on_startup=false".to_owned(),
                "-c".to_owned(),
                r#"projects."/tmp/work".trust_level="trusted""#.to_owned(),
                "-C".to_owned(),
                "/tmp/work".to_owned(),
                "-c".to_owned(),
                r#"model_reasoning_effort="high""#.to_owned(),
                "-c".to_owned(),
                r#"personality="pragmatic""#.to_owned(),
                "-c".to_owned(),
                r#"service_tier="fast""#.to_owned(),
                "--enable".to_owned(),
                "goals".to_owned(),
                "ship the fix".to_owned(),
                "resume".to_owned(),
                "--last".to_owned()
            ]
        );
        assert!(codex_resume.session_resume_id.is_none());
    }

    #[test]
    fn codex_launch_plan_adds_clean_startup_flags() {
        let harness = SessionHarnessOptions::default();
        let launch =
            build_session_harness_launch_plan(SessionKind::Codex, Path::new("/tmp/work"), &harness);

        assert_eq!(
            launch.args,
            vec![
                "--no-alt-screen".to_owned(),
                "--dangerously-bypass-approvals-and-sandbox".to_owned(),
                "-c".to_owned(),
                "check_for_update_on_startup=false".to_owned(),
                "-c".to_owned(),
                r#"projects."/tmp/work".trust_level="trusted""#.to_owned(),
                "-C".to_owned(),
                "/tmp/work".to_owned(),
            ]
        );
    }
}
