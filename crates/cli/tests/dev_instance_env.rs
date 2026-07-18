use std::process::Command;

#[test]
fn dev_instance_env_preserves_github_cli_config() {
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temp = tempfile::tempdir().unwrap();
    let original_config = temp.path().join("real-config");
    let dev_home = temp.path().join("dev-home");
    std::fs::create_dir_all(original_config.join("gh")).unwrap();

    let output = Command::new(repo_root.join("scripts/dev-instance-env.sh"))
        .env("HOME", temp.path().join("home"))
        .env("XDG_CONFIG_HOME", &original_config)
        .env("ARCHDUCTOR_DEV_HOME", &dev_home)
        .arg("sh")
        .arg("-c")
        .arg("printf '%s\n%s\n' \"$GH_CONFIG_DIR\" \"$XDG_CONFIG_HOME\"")
        .output()
        .expect("run dev instance env");

    assert!(
        output.status.success(),
        "dev instance env failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines = stdout.lines().collect::<Vec<_>>();

    assert_eq!(
        lines.first().copied(),
        Some(original_config.join("gh").to_str().unwrap())
    );
    assert_eq!(
        lines.get(1).copied(),
        Some(dev_home.join("config").to_str().unwrap())
    );
}
