use assert_cmd::Command as AssertCommand;
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
        "#!/bin/sh\nprintf 'cli-session:%s:%s\\n' \"$CONDUCTOR_WORKSPACE_NAME\" \"$CONDUCTOR_PORT\"\nwhile true; do sleep 1; done\n",
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

fn app(root: &Path) -> AssertCommand {
    let mut command = AssertCommand::cargo_bin("linux-conductor").unwrap();
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
            "user.name=Linux Conductor",
            "-c",
            "user.email=linux-conductor@example.test",
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
    let log_dir = root.join("xdg/state/linux-conductor/logs/berlin");
    for _ in 0..100 {
        if fs::read_dir(&log_dir)
            .ok()
            .into_iter()
            .flat_map(|entries| entries.flatten())
            .any(|entry| {
                fs::read_to_string(entry.path())
                    .map(|contents| contents.contains("cli-session:berlin:3000"))
                    .unwrap_or(false)
            })
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("timed out waiting for session log in {}", log_dir.display());
}
