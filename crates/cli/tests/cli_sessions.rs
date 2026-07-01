use assert_cmd::Command as AssertCommand;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn cli_starts_logs_and_stops_real_shell_session() {
    let temp = tempfile::tempdir().unwrap();
    let repo_path = init_repo(temp.path().join("demo"));
    let workspace_parent = temp.path().join("workspaces/demo");
    let fake_shell = temp.path().join("fake-shell");
    fs::write(
        &fake_shell,
        "#!/bin/sh\nprintf 'cli-session:%s:%s\\n' \"$ARCHDUCTOR_WORKSPACE_NAME\" \"$ARCHDUCTOR_PORT\"\nwhile true; do sleep 1; done\n",
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
        .stdout(contains("cli-session:berlin:3000"));
    app(temp.path())
        .args(["session", "stop", "berlin"])
        .assert()
        .success()
        .stdout(contains("Stopped session for berlin"));
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
fn cli_codex_and_claude_sessions_preserve_harness_launch_details() {
    let temp = tempfile::tempdir().unwrap();
    let repo_path = init_repo(temp.path().join("demo"));
    let workspace_parent = temp.path().join("workspaces/demo");
    let fake_bin = temp.path().join("bin");
    fs::create_dir(&fake_bin).unwrap();
    write_fake_agent_wrapper(fake_bin.join("codex"), "codex");
    write_fake_agent_wrapper(fake_bin.join("claude"), "claude");
    let fake_path = path_with_prepend(&fake_bin);

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
            "workspace",
            "create",
            "demo",
            "--name",
            "oslo",
            "--branch",
            "lc/oslo",
            "--base",
            "main",
        ])
        .assert()
        .success();

    app(temp.path())
        .env("PATH", &fake_path)
        .args([
            "session",
            "start",
            "berlin",
            "--kind",
            "codex",
            "--plan-mode",
            "--fast-mode",
            "--approval-mode",
            "ask",
            "--reasoning-mode",
            "high",
            "--codex-personality",
            "pragmatic",
            "--codex-goals",
            "ship the fix",
        ])
        .assert()
        .success();
    wait_for_session_log_contains(temp.path(), "berlin", "wrapper:codex");
    app(temp.path())
        .args(["logs", "berlin", "--session"])
        .assert()
        .success()
        .stdout(contains("wrapper:codex"))
        .stdout(contains("arg:-c"))
        .stdout(contains("arg:model_reasoning_effort=\"high\""))
        .stdout(contains("arg:personality=\"pragmatic\""))
        .stdout(contains("arg:service_tier=\"fast\""))
        .stdout(contains("arg:--ask-for-approval"))
        .stdout(contains("arg:on-request"))
        .stdout(contains("arg:--enable"))
        .stdout(contains("arg:goals"))
        .stdout(
            predicates::str::is_match("env_bootstrap:.*\\S")
                .unwrap()
                .not(),
        );
    app(temp.path())
        .env("PATH", &fake_path)
        .args(["session", "stop", "berlin"])
        .assert()
        .success();

    app(temp.path())
        .env("PATH", &fake_path)
        .args([
            "session",
            "start",
            "oslo",
            "--kind",
            "claude",
            "--plan-mode",
            "--fast-mode",
            "--approval-mode",
            "never",
            "--reasoning-mode",
            "low",
            "--effort-mode",
            "high",
            "--codex-skills",
            "rust, tests",
        ])
        .assert()
        .success();
    wait_for_session_log_contains(temp.path(), "oslo", "wrapper:claude");
    app(temp.path())
        .args(["logs", "oslo", "--session"])
        .assert()
        .success()
        .stdout(contains("wrapper:claude"))
        .stdout(contains("arg:--permission-mode"))
        .stdout(contains("arg:plan"))
        .stdout(contains("arg:--effort"))
        .stdout(contains("arg:high"))
        .stdout(contains("arg:--session-id"))
        .stdout(contains("arg:--append-system-prompt"))
        .stdout(contains("arg:[archductor bootstrap for claude]"))
        .stdout(contains("skills: rust, tests"));
    app(temp.path())
        .env("PATH", &fake_path)
        .args(["session", "stop", "oslo"])
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
            "--plan-mode",
            "--codex-goals",
            "ship the fix",
        ])
        .assert()
        .success()
        .stdout(contains("exec codex"))
        .stdout(contains("--enable goals"))
        .stdout(
            predicates::str::is_match("exec codex .*'\\[archductor bootstrap for codex]")
                .unwrap()
                .not(),
        )
        .stdout(predicates::str::contains("ARCHDUCTOR_SESSION_BOOTSTRAP=").not());

    app(temp.path())
        .args([
            "session",
            "open",
            "oslo",
            "--kind",
            "claude",
            "--print-command",
            "--plan-mode",
            "--codex-skills",
            "rust, tests",
        ])
        .assert()
        .success()
        .stdout(contains("exec claude --permission-mode plan --session-id"))
        .stdout(contains(
            "--append-system-prompt '[archductor bootstrap for claude]",
        ))
        .stdout(contains("skills: rust, tests'"));
}

fn app(root: &Path) -> AssertCommand {
    let mut command = AssertCommand::cargo_bin("linux-archductor").unwrap();
    command
        .env("XDG_CONFIG_HOME", root.join("xdg/config"))
        .env("XDG_DATA_HOME", root.join("xdg/data"))
        .env("XDG_STATE_HOME", root.join("xdg/state"))
        .env("XDG_CACHE_HOME", root.join("xdg/cache"));
    command
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
            "user.name=Linux Archductor",
            "-c",
            "user.email=linux-archductor@example.test",
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
    wait_for_session_log_contains(root, "berlin", "cli-session:berlin:3000");
}

fn wait_for_session_log_contains(root: &Path, workspace: &str, needle: &str) {
    let log_dir = root.join("xdg/state/linux-archductor/logs").join(workspace);
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

fn write_fake_agent_wrapper(path: PathBuf, name: &str) {
    fs::write(
        &path,
        format!(
            "#!/bin/sh\nprintf 'wrapper:{name}\\n'\nprintf 'env_bootstrap:%s\\n' \"$ARCHDUCTOR_SESSION_BOOTSTRAP\"\nfor arg in \"$@\"; do\n  printf 'arg:%s\\n' \"$arg\"\ndone\nwhile true; do sleep 1; done\n"
        ),
    )
    .unwrap();
    Command::new("chmod").arg("+x").arg(&path).status().unwrap();
}

fn path_with_prepend(dir: &Path) -> std::ffi::OsString {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![dir.to_path_buf()];
    paths.extend(std::env::split_paths(&current));
    std::env::join_paths(paths).unwrap()
}
