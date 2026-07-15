use archductor_core::provider_events::{
    ProviderEventDraft, ProviderEventKind, ProviderEventPhase, ProviderEventStore,
};
use archductor_core::workspace::WorkspaceStore;
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
    let conductor_dir = repo_path.join(".archductor");
    fs::create_dir(&conductor_dir).unwrap();
    fs::write(
        conductor_dir.join("settings.toml"),
        r#"
[scripts]
run = "cargo run"

[customization.view]
keybindings = "vim"
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
            "exec codex --no-alt-screen --dangerously-bypass-approvals-and-sandbox",
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
        .stdout(contains("exec claude --model claude-sonnet-5"));
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
fn cli_archcar_messages_updates_mcp_startup_status_provider_event() {
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
        .stdout(contains("Tool\nMCP loaded\ngithub: ready\n\n"))
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

fn app(root: &Path) -> AssertCommand {
    let mut command = AssertCommand::cargo_bin("archductor").unwrap();
    command
        .env("XDG_CONFIG_HOME", root.join("xdg/config"))
        .env("XDG_DATA_HOME", root.join("xdg/data"))
        .env("XDG_STATE_HOME", root.join("xdg/state"))
        .env("XDG_CACHE_HOME", root.join("xdg/cache"));
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
