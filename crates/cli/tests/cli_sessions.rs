use archductor_core::archcar::harness::managed_harness_for_kind;
use archductor_core::archcar::harness_contract::{HarnessCapability, SupportMode};
use archductor_core::provider_events::{
    ProviderEventDraft, ProviderEventKind, ProviderEventPhase, ProviderEventStore,
};
use archductor_core::workspace::{SessionKind, WorkspaceStore};
use assert_cmd::Command as AssertCommand;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn cli_starts_and_logs_real_shell_session() {
    let temp = tempfile::tempdir().unwrap();
    let repo_path = init_repo(temp.path().join("demo"));
    let workspace_parent = temp.path().join("workspaces/demo");
    let fake_shell = temp.path().join("fake-shell");
    fs::write(
        &fake_shell,
        "#!/bin/sh\nprintf 'cli-session:%s:%s\\n' \"$ARCHDUCTOR_WORKSPACE_NAME\" \"$ARCHDUCTOR_PORT\"\n",
    )
    .unwrap();
    Command::new("chmod")
        .arg("+x")
        .arg(&fake_shell)
        .status()
        .unwrap();

    app(temp.path())
        .args([
            "repo",
            "add",
            repo_path.to_str().unwrap(),
            "--name",
            "demo",
            "--default-branch",
            "main",
            "--workspace-parent",
            workspace_parent.to_str().unwrap(),
        ])
        .assert()
        .success();
    app(temp.path())
        .args([
            "workspace",
            "create",
            "demo",
            "--name",
            "berlin",
            "--branch",
            "lc/berlin",
            "--base",
            "main",
        ])
        .assert()
        .success();

    app(temp.path())
        .env("SHELL", &fake_shell)
        .args(["session", "start", "berlin", "--kind", "shell"])
        .assert()
        .success()
        .stdout(contains("Started session for berlin"));

    wait_for_session_log(temp.path());
    app(temp.path())
        .args(["session", "attach", "berlin", "--print-pty-path"])
        .assert()
        .failure()
        .stderr(contains("is not attached to a PTY slave"));
    app(temp.path())
        .args(["logs", "berlin", "--session"])
        .assert()
        .success()
        .stdout(contains("cli-session:berlin:"));
}

#[test]
fn cli_exports_and_imports_repository_settings() {
    let temp = tempfile::tempdir().unwrap();
    let repo_path = init_repo(temp.path().join("demo"));
    let workspace_parent = temp.path().join("workspaces/demo");
    let conductor_dir = repo_path.join(".archductor");
    fs::create_dir(&conductor_dir).unwrap();
    fs::write(
        conductor_dir.join("settings.toml"),
        r#"
[scripts]
run = "cargo run"

[customization.view]
keybindings = "vim"
colors = {}
notification_rules = []
command_palette_presets = []

[prompts]
general = "Before import"
"#,
    )
    .unwrap();
    let export_path = temp.path().join("settings-export.toml");

    app(temp.path())
        .args([
            "repo",
            "add",
            repo_path.to_str().unwrap(),
            "--name",
            "demo",
            "--default-branch",
            "main",
            "--workspace-parent",
            workspace_parent.to_str().unwrap(),
        ])
        .assert()
        .success();
    app(temp.path())
        .args([
            "workspace",
            "create",
            "demo",
            "--name",
            "berlin",
            "--branch",
            "lc/berlin",
            "--base",
            "main",
        ])
        .assert()
        .success();
    app(temp.path())
        .args([
            "repo",
            "settings",
            "demo",
            "export",
            "--output",
            export_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(contains("Exported shared settings"));

    let exported = fs::read_to_string(&export_path).unwrap();
    assert!(exported.contains("keybindings = \"vim\""));
    fs::write(
        &export_path,
        exported.replace("Before import", "After import"),
    )
    .unwrap();

    app(temp.path())
        .args([
            "repo",
            "settings",
            "demo",
            "import",
            export_path.to_str().unwrap(),
            "--local",
        ])
        .assert()
        .success()
        .stdout(contains("Imported local settings"));

    let local_settings = fs::read_to_string(conductor_dir.join("settings.local.toml")).unwrap();
    assert!(local_settings.contains("keybindings = \"vim\""));
    assert!(local_settings.contains("[customization.view.colors]"));
    assert!(local_settings.contains("notification_rules = []"));
    assert!(local_settings.contains("command_palette_presets = []"));
    assert!(
        fs::read_to_string(workspace_parent.join("berlin/.context/PROMPTS.md"))
            .unwrap()
            .contains("After import")
    );
}

#[test]
fn cli_app_shared_import_export_preserves_explicit_empty_collections() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("shared-input.toml");
    let output = temp.path().join("shared-output.toml");
    fs::write(
        &input,
        r#"
file_include_globs = ""
env_file_refs = ""

[environment_variables]

[customization.view]
colors = {}
notification_rules = []
command_palette_presets = []
"#,
    )
    .unwrap();

    app(temp.path())
        .args(["settings", "import", input.to_str().unwrap()])
        .assert()
        .success();
    app(temp.path())
        .args(["settings", "export", "--output", output.to_str().unwrap()])
        .assert()
        .success();

    let exported = fs::read_to_string(output).unwrap();
    assert!(exported.contains("file_include_globs = \"\""));
    assert!(exported.contains("env_file_refs = \"\""));
    assert!(exported.contains("[environment_variables]"));
    assert!(exported.contains("[customization.view.colors]"));
    assert!(exported.contains("notification_rules = []"));
    assert!(exported.contains("command_palette_presets = []"));
}

#[test]
fn cli_session_open_print_command_uses_explicit_provider_models() {
    let temp = tempfile::tempdir().unwrap();
    let repo_path = init_repo(temp.path().join("demo"));
    let workspace_parent = temp.path().join("workspaces/demo");

    app(temp.path())
        .args([
            "repo",
            "add",
            repo_path.to_str().unwrap(),
            "--name",
            "demo",
            "--default-branch",
            "main",
            "--workspace-parent",
            workspace_parent.to_str().unwrap(),
        ])
        .assert()
        .success();
    app(temp.path())
        .args([
            "workspace",
            "create",
            "demo",
            "--name",
            "berlin",
            "--branch",
            "lc/berlin",
            "--base",
            "main",
        ])
        .assert()
        .success();

    app(temp.path())
        .args([
            "session",
            "open",
            "berlin",
            "--kind",
            "codex",
            "--model",
            "gpt-5.6-sol",
            "--print-command",
        ])
        .assert()
        .success()
        .stdout(contains(
            "codex --no-alt-screen --dangerously-bypass-approvals-and-sandbox",
        ))
        .stdout(contains("--model gpt-5.6-sol"));

    app(temp.path())
        .args([
            "session",
            "open",
            "berlin",
            "--kind",
            "claude",
            "--model",
            "claude-sonnet-5",
            "--print-command",
        ])
        .assert()
        .success()
        .stdout(contains(
            "claude --permission-mode bypassPermissions --dangerously-skip-permissions",
        ))
        .stdout(contains("--model claude-sonnet-5"));
}

#[test]
fn cli_session_open_applies_app_shared_launch_settings() {
    let temp = tempfile::tempdir().unwrap();
    let repo_path = init_repo(temp.path().join("demo"));
    let workspace_parent = temp.path().join("workspaces/demo");
    let shared_settings = temp.path().join("xdg/config/archductor/settings.toml");
    fs::create_dir_all(shared_settings.parent().unwrap()).unwrap();
    fs::write(
        shared_settings,
        r#"
codex_executable_path = "/shared/bin/codex"

[environment_variables]
SHARED_SESSION_VALUE = "from-app-shared"
"#,
    )
    .unwrap();

    app(temp.path())
        .args([
            "repo",
            "add",
            repo_path.to_str().unwrap(),
            "--name",
            "demo",
            "--default-branch",
            "main",
            "--workspace-parent",
            workspace_parent.to_str().unwrap(),
        ])
        .assert()
        .success();
    app(temp.path())
        .args([
            "workspace",
            "create",
            "demo",
            "--name",
            "berlin",
            "--branch",
            "lc/berlin",
            "--base",
            "main",
        ])
        .assert()
        .success();

    app(temp.path())
        .args([
            "session",
            "open",
            "berlin",
            "--kind",
            "codex",
            "--print-command",
        ])
        .assert()
        .success()
        .stdout(contains("SHARED_SESSION_VALUE=from-app-shared"))
        .stdout(contains("exec /shared/bin/codex"));
}

#[test]
fn cli_session_send_hides_general_prompt_while_provider_receives_first_turn_prefix() {
    let temp = tempfile::tempdir().unwrap();
    let repo_path = init_repo(temp.path().join("demo"));
    let workspace_parent = temp.path().join("workspaces/demo");
    let fake_codex = fake_codex_path(temp.path());
    let provider_inputs = temp.path().join("provider-inputs.txt");
    let fake_home = temp.path().join("home");
    write_fake_codex(&fake_codex).unwrap();

    app_with_home(temp.path(), &fake_home)
        .args([
            "repo",
            "add",
            repo_path.to_str().unwrap(),
            "--name",
            "demo",
            "--default-branch",
            "main",
            "--workspace-parent",
            workspace_parent.to_str().unwrap(),
        ])
        .assert()
        .success();
    app_with_home(temp.path(), &fake_home)
        .args([
            "workspace",
            "create",
            "demo",
            "--name",
            "berlin",
            "--branch",
            "lc/berlin",
            "--base",
            "main",
        ])
        .assert()
        .success();
    fs::write(
        repo_path.join(".archductor/settings.local.toml"),
        format!(
            "codex_executable_path = {:?}\n\n[prompts]\ngeneral = \"Keep changes focused.\"\n",
            fake_codex.to_string_lossy()
        ),
    )
    .unwrap();

    app_with_home(temp.path(), &fake_home)
        .env("ARCHDUCTOR_CAPTURE_PATH", &provider_inputs)
        .args([
            "session",
            "send",
            "berlin",
            "--kind",
            "codex",
            "--timeout-ms",
            "5000",
            "Fix auth",
        ])
        .assert()
        .success();
    wait_for_file_lines(&provider_inputs, 1);

    let store = WorkspaceStore::open(app_database_path(temp.path())).unwrap();
    let thread = store
        .list_chat_threads("berlin")
        .unwrap()
        .into_iter()
        .find(|thread| thread.provider == "codex")
        .unwrap();
    let first_session = store.list_sessions("berlin").unwrap()[0].id;
    app_with_home(temp.path(), &fake_home)
        .env("ARCHDUCTOR_CAPTURE_PATH", &provider_inputs)
        .args(["archcar", "kill", &first_session.to_string()])
        .assert()
        .success();
    wait_for_session_exit(temp.path(), first_session);

    app_with_home(temp.path(), &fake_home)
        .env("ARCHDUCTOR_CAPTURE_PATH", &provider_inputs)
        .args([
            "session",
            "send",
            "berlin",
            "--kind",
            "codex",
            "--thread-id",
            &thread.id.to_string(),
            "--timeout-ms",
            "5000",
            "Run tests",
        ])
        .assert()
        .success();
    wait_for_file_lines(&provider_inputs, 2);

    let captured = fs::read_to_string(&provider_inputs).unwrap();
    let captured = captured
        .lines()
        .map(|line| serde_json::from_str::<String>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        captured,
        vec!["Keep changes focused.\n\nFix auth", "Run tests"]
    );

    let visible_messages =
        wait_for_visible_user_messages(temp.path(), thread.id, &["Fix auth", "Run tests"]);
    assert!(visible_messages
        .iter()
        .all(|message| !message.content.contains("Keep changes focused.")));
    let second_session = store
        .list_sessions("berlin")
        .unwrap()
        .into_iter()
        .find(|session| session.id != first_session)
        .unwrap()
        .id;
    app_with_home(temp.path(), &fake_home)
        .env("ARCHDUCTOR_CAPTURE_PATH", &provider_inputs)
        .args(["archcar", "kill", &second_session.to_string()])
        .assert()
        .success();
    wait_for_session_exit(temp.path(), second_session);
}

#[cfg(unix)]
fn fake_codex_path(root: &Path) -> PathBuf {
    root.join("fake-codex")
}

#[cfg(windows)]
fn fake_codex_path(root: &Path) -> PathBuf {
    root.join("fake-codex.cmd")
}

#[cfg(unix)]
fn write_fake_codex(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::write(
        path,
        r#"#!/usr/bin/env python3
import json
import os
import sys

def write_rollout(message):
    home = os.environ.get("HOME")
    if not home:
        return
    cwd = message.get("params", {}).get("cwd", ".")
    rollout_dir = os.path.join(home, ".codex", "sessions", "2026", "07", "19")
    os.makedirs(rollout_dir, exist_ok=True)
    rollout_path = os.path.join(rollout_dir, "rollout-thread-test.jsonl")
    meta = {"type": "session_meta", "payload": {"session_id": "thread-test", "cwd": cwd}}
    with open(rollout_path, "w", encoding="utf-8") as output:
        output.write(json.dumps(meta) + "\n")

capture = os.environ["ARCHDUCTOR_CAPTURE_PATH"]
for raw in sys.stdin:
    message = json.loads(raw)
    method = message.get("method")
    if method == "initialize":
        print(json.dumps({"id": message["id"], "result": {}}), flush=True)
    elif method in ("thread/start", "thread/resume"):
        write_rollout(message)
        print(json.dumps({"id": message["id"], "result": {"thread": {"id": "thread-test"}}}), flush=True)
    elif method == "turn/start":
        text = message["params"]["input"][0]["text"]
        with open(capture, "a", encoding="utf-8") as output:
            output.write(json.dumps(text) + "\n")
        turn_id = "turn-test"
        print(json.dumps({"id": message["id"], "result": {"turn": {"id": turn_id}}}), flush=True)
        print(json.dumps({"method": "turn/completed", "params": {"turn": {"id": turn_id, "status": "completed"}}}), flush=True)
"#,
    )?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

#[cfg(windows)]
fn write_fake_codex(path: &Path) -> std::io::Result<()> {
    let script = path.with_extension("ps1");
    fs::write(
        path,
        format!(
            "@echo off\r\npowershell.exe -NoProfile -ExecutionPolicy Bypass -File \"{}\"\r\n",
            script.display()
        ),
    )?;
    fs::write(
        script,
        r#"
$capture = $env:ARCHDUCTOR_CAPTURE_PATH
function Write-Rollout($message) {
  $homeDir = if ($env:USERPROFILE) { $env:USERPROFILE } else { $env:HOME }
  if (-not $homeDir) { return }
  $cwd = if ($message.params -and $message.params.cwd) { $message.params.cwd } else { "." }
  $rolloutDir = Join-Path $homeDir ".codex/sessions/2026/07/19"
  New-Item -ItemType Directory -Force -Path $rolloutDir | Out-Null
  $meta = @{ type = "session_meta"; payload = @{ session_id = "thread-test"; cwd = $cwd } }
  Set-Content -Path (Join-Path $rolloutDir "rollout-thread-test.jsonl") -Encoding UTF8 -Value ($meta | ConvertTo-Json -Compress -Depth 8)
}
while (($raw = [Console]::In.ReadLine()) -ne $null) {
  $message = $raw | ConvertFrom-Json
  $method = $message.method
  if ($method -eq "initialize") {
    Write-Output (@{ id = $message.id; result = @{} } | ConvertTo-Json -Compress -Depth 8)
  } elseif ($method -eq "thread/start" -or $method -eq "thread/resume") {
    Write-Rollout $message
    Write-Output (@{ id = $message.id; result = @{ thread = @{ id = "thread-test" } } } | ConvertTo-Json -Compress -Depth 8)
  } elseif ($method -eq "turn/start") {
    $text = $message.params.input[0].text
    Add-Content -Path $capture -Encoding UTF8 -Value ($text | ConvertTo-Json -Compress)
    $turnId = "turn-test"
    Write-Output (@{ id = $message.id; result = @{ turn = @{ id = $turnId } } } | ConvertTo-Json -Compress -Depth 8)
    Write-Output (@{ method = "turn/completed"; params = @{ turn = @{ id = $turnId; status = "completed" } } } | ConvertTo-Json -Compress -Depth 8)
  }
  [Console]::Out.Flush()
}
"#,
    )
}

#[test]
fn cli_archcar_messages_renders_projected_provider_events() {
    let temp = tempfile::tempdir().unwrap();
    let repo_path = init_repo(temp.path().join("demo"));
    let workspace_parent = temp.path().join("workspaces/demo");

    app(temp.path())
        .args([
            "repo",
            "add",
            repo_path.to_str().unwrap(),
            "--name",
            "demo",
            "--default-branch",
            "main",
            "--workspace-parent",
            workspace_parent.to_str().unwrap(),
        ])
        .assert()
        .success();
    app(temp.path())
        .args([
            "workspace",
            "create",
            "demo",
            "--name",
            "berlin",
            "--branch",
            "lc/berlin",
            "--base",
            "main",
        ])
        .assert()
        .success();

    let db_path = app_database_path(temp.path());
    let store = WorkspaceStore::open(&db_path).unwrap();
    let thread = store
        .create_chat_thread("berlin", "codex", "Codex", None)
        .unwrap();
    store
        .append_chat_message(thread.id, "user", "Run tests", "cli")
        .unwrap();
    let provider_store = ProviderEventStore::new(&db_path);
    provider_store
        .upsert_event(&provider_event(
            thread.id,
            "assistant-1",
            ProviderEventKind::AssistantOutput,
            ProviderEventPhase::Completed,
            "agent_message",
            "Assistant",
            "<arch\nTests passed",
        ))
        .unwrap();
    provider_store
        .upsert_event(&provider_event(
            thread.id,
            "reasoning-1",
            ProviderEventKind::PlanningReasoning,
            ProviderEventPhase::Progress,
            "reasoning_summary",
            "Reasoning",
            "Checking failure output",
        ))
        .unwrap();
    provider_store
        .upsert_event(&provider_event(
            thread.id,
            "turn-1",
            ProviderEventKind::Turn,
            ProviderEventPhase::Started,
            "turn_started",
            "Turn started",
            "raw lifecycle",
        ))
        .unwrap();

    app(temp.path())
        .args(["archcar", "messages", &thread.id.to_string()])
        .assert()
        .success()
        .stdout(contains("You\nRun tests\n\n"))
        .stdout(contains("Assistant\nTests passed\n\n"))
        .stdout(predicates::str::contains("<arch").not())
        .stdout(contains("Reasoning\nChecking failure output\n\n"))
        .stdout(predicates::str::contains("turn_started").not())
        .stdout(predicates::str::contains("raw lifecycle").not());
}

#[test]
fn cli_archcar_messages_hides_mcp_startup_status_provider_event() {
    let temp = tempfile::tempdir().unwrap();
    let repo_path = init_repo(temp.path().join("demo"));
    let workspace_parent = temp.path().join("workspaces/demo");

    app(temp.path())
        .args([
            "repo",
            "add",
            repo_path.to_str().unwrap(),
            "--name",
            "demo",
            "--default-branch",
            "main",
            "--workspace-parent",
            workspace_parent.to_str().unwrap(),
        ])
        .assert()
        .success();
    app(temp.path())
        .args([
            "workspace",
            "create",
            "demo",
            "--name",
            "berlin",
            "--branch",
            "lc/berlin",
            "--base",
            "main",
        ])
        .assert()
        .success();

    let db_path = app_database_path(temp.path());
    let store = WorkspaceStore::open(&db_path).unwrap();
    let thread = store
        .create_chat_thread("berlin", "codex", "Codex", None)
        .unwrap();
    let provider_store = ProviderEventStore::new(&db_path);
    provider_store
        .upsert_event(&provider_event(
            thread.id,
            "mcp-startup-status",
            ProviderEventKind::Mcp,
            ProviderEventPhase::Progress,
            "mcpServer/startupStatus/updated",
            "MCP loading",
            "",
        ))
        .unwrap();
    provider_store
        .upsert_event(&provider_event(
            thread.id,
            "mcp-startup-status",
            ProviderEventKind::Mcp,
            ProviderEventPhase::Completed,
            "mcpServer/startupStatus/updated",
            "MCP loaded",
            "github: ready",
        ))
        .unwrap();

    app(temp.path())
        .args(["archcar", "messages", &thread.id.to_string()])
        .assert()
        .success()
        .stdout(predicates::str::contains("MCP loaded").not())
        .stdout(predicates::str::contains("github: ready").not())
        .stdout(predicates::str::contains("MCP loading").not())
        .stdout(predicates::str::contains("mcpServer/startupStatus/updated").not());
}

#[test]
fn cli_archcar_messages_renders_claude_projected_provider_events() {
    let temp = tempfile::tempdir().unwrap();
    let repo_path = init_repo(temp.path().join("demo"));
    let workspace_parent = temp.path().join("workspaces/demo");

    app(temp.path())
        .args([
            "repo",
            "add",
            repo_path.to_str().unwrap(),
            "--name",
            "demo",
            "--default-branch",
            "main",
            "--workspace-parent",
            workspace_parent.to_str().unwrap(),
        ])
        .assert()
        .success();
    app(temp.path())
        .args([
            "workspace",
            "create",
            "demo",
            "--name",
            "berlin",
            "--branch",
            "lc/berlin",
            "--base",
            "main",
        ])
        .assert()
        .success();

    let db_path = app_database_path(temp.path());
    let store = WorkspaceStore::open(&db_path).unwrap();
    let thread = store
        .create_chat_thread("berlin", "claude", "Claude", None)
        .unwrap();
    let provider_store = ProviderEventStore::new(&db_path);
    let mut assistant_event = provider_event(
        thread.id,
        "assistant-1",
        ProviderEventKind::AssistantOutput,
        ProviderEventPhase::Completed,
        "assistant_message",
        "Assistant",
        "Claude answer",
    );
    assistant_event.provider = "claude".to_owned();
    provider_store.upsert_event(&assistant_event).unwrap();
    let mut tool_event = provider_event(
        thread.id,
        "tool-1",
        ProviderEventKind::Tool,
        ProviderEventPhase::Completed,
        "tool_result",
        "Tool",
        "Read file",
    );
    tool_event.provider = "claude".to_owned();
    provider_store.upsert_event(&tool_event).unwrap();

    app(temp.path())
        .args(["archcar", "messages", &thread.id.to_string()])
        .assert()
        .success()
        .stdout(contains("Assistant\nClaude answer\n\n"))
        .stdout(contains("Tool\nRead file\n\n"))
        .stdout(predicates::str::contains("\"method\"").not());
}

#[test]
fn claude_thread_session_send_help_exposes_thread_targeting() {
    let temp = tempfile::tempdir().unwrap();
    app(temp.path())
        .args(["session", "send", "--help"])
        .assert()
        .success()
        .stdout(contains("--thread-id"))
        .stdout(contains("claude"));
}

#[test]
fn claude_hook_hidden_command_prints_single_json_object() {
    let temp = tempfile::tempdir().unwrap();
    let output = app(temp.path())
        .args(["--archcar-claude-hook", "42"])
        .write_stdin(
            json!({
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_input": {"command": "cargo test"}
            })
            .to_string(),
        )
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();

    assert_eq!(
        parsed,
        json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "defer"
            }
        })
    );
    assert_eq!(stdout.trim_end_matches('\n').lines().count(), 1);
}

#[test]
fn provider_interactions_help_lists_cli_actions() {
    let temp = tempfile::tempdir().unwrap();
    app(temp.path())
        .args(["archcar", "interactions", "--help"])
        .assert()
        .success()
        .stdout(contains("list"))
        .stdout(contains("allow"))
        .stdout(contains("deny"))
        .stdout(contains("answer"));
}

#[test]
fn harness_capabilities_gate_goals_to_codex_descriptor() {
    let codex = managed_harness_for_kind(SessionKind::Codex).unwrap();
    let claude = managed_harness_for_kind(SessionKind::Claude).unwrap();

    assert_eq!(
        codex.descriptor().optional(HarnessCapability::Goals),
        SupportMode::Native
    );
    assert!(matches!(
        claude.descriptor().optional(HarnessCapability::Goals),
        SupportMode::Unsupported { reason } if !reason.is_empty()
    ));
    for harness in [codex, claude] {
        assert!(harness
            .descriptor()
            .required_features
            .iter()
            .any(|feature| feature.as_str() == "session_controls"));
    }
}

fn app(root: &Path) -> AssertCommand {
    let mut command = AssertCommand::cargo_bin("archductor").unwrap();
    command
        .env("XDG_CONFIG_HOME", root.join("xdg/config"))
        .env("XDG_DATA_HOME", root.join("xdg/data"))
        .env("XDG_STATE_HOME", root.join("xdg/state"))
        .env("XDG_CACHE_HOME", root.join("xdg/cache"))
        .env("APPDATA", root.join("xdg/config"))
        .env("LOCALAPPDATA", root.join("xdg/data"));
    command
}

fn app_with_home(root: &Path, home: &Path) -> AssertCommand {
    let mut command = app(root);
    command.env("HOME", home).env("USERPROFILE", home);
    command
}

fn app_database_path(root: &Path) -> PathBuf {
    root.join("xdg/data/archductor/archductor.db")
}

fn provider_event(
    thread_id: i64,
    item_id: &str,
    kind: ProviderEventKind,
    phase: ProviderEventPhase,
    subtype: &str,
    title: &str,
    body: &str,
) -> ProviderEventDraft {
    ProviderEventDraft {
        provider: "codex".to_owned(),
        provider_event_id: Some(format!("evt-{item_id}")),
        provider_item_id: Some(item_id.to_owned()),
        provider_thread_id: Some("thread-1".to_owned()),
        provider_turn_id: Some("turn-1".to_owned()),
        parent_provider_item_id: None,
        parent_provider_thread_id: None,
        workspace_id: None,
        chat_thread_id: Some(thread_id),
        process_id: None,
        phase,
        kind,
        provider_subtype: Some(subtype.to_owned()),
        provider_sequence: Some(1),
        occurred_at_ms: 42,
        normalized_payload: json!({"title": title, "body": body}),
        raw_json: json!({"method": subtype, "params": {"body": body}}),
        schema_version: 1,
        adapter_version: "test".to_owned(),
    }
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
            "user.name=Archductor",
            "-c",
            "user.email=archductor@example.test",
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

fn wait_for_session_log(root: &Path) {
    wait_for_session_log_contains(root, "berlin", "cli-session:berlin:");
}

fn wait_for_session_log_contains(root: &Path, workspace: &str, needle: &str) {
    let log_dir = root.join("xdg/state/archductor/logs").join(workspace);
    for _ in 0..100 {
        if fs::read_dir(&log_dir)
            .ok()
            .into_iter()
            .flat_map(|entries| entries.flatten())
            .any(|entry| {
                fs::read_to_string(entry.path())
                    .map(|contents| contents.contains(needle))
                    .unwrap_or(false)
            })
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("timed out waiting for session log in {}", log_dir.display());
}

fn wait_for_file_lines(path: &Path, expected: usize) {
    for _ in 0..100 {
        if fs::read_to_string(path)
            .map(|contents| contents.lines().count() >= expected)
            .unwrap_or(false)
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!(
        "timed out waiting for {expected} line(s) in {}",
        path.display()
    );
}

fn wait_for_visible_user_messages(
    root: &Path,
    thread_id: i64,
    expected: &[&str],
) -> Vec<archductor_core::workspace::ChatMessageRecord> {
    for _ in 0..100 {
        let visible_messages = WorkspaceStore::open(app_database_path(root))
            .and_then(|store| store.list_chat_messages(thread_id))
            .unwrap_or_default();
        let visible_user_messages = visible_messages
            .iter()
            .filter(|message| message.role == "user")
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>();
        if visible_user_messages == expected {
            return visible_messages;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("timed out waiting for visible user messages {expected:?}");
}

fn wait_for_session_exit(root: &Path, session_id: i64) {
    for _ in 0..100 {
        if WorkspaceStore::open(app_database_path(root))
            .and_then(|store| store.get_process_record(session_id))
            .map(|process| process.status != archductor_core::workspace::ProcessStatus::Running)
            .unwrap_or(false)
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("timed out waiting for session {session_id} to exit");
}
