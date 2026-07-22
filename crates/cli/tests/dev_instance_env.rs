use std::process::Command;

fn dev_env_command(script: &std::path::Path) -> Command {
    #[cfg(windows)]
    {
        let mut command = Command::new(r"C:\Program Files\Git\bin\bash.exe");
        command.arg(script);
        command
    }
    #[cfg(not(windows))]
    {
        Command::new(script)
    }
}

#[test]
fn dev_instance_env_preserves_github_cli_config() {
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temp = tempfile::tempdir().unwrap();
    let original_config = temp.path().join("real-config");
    let dev_home = temp.path().join("dev-home");
    std::fs::create_dir_all(original_config.join("gh")).unwrap();

    let output = dev_env_command(&repo_root.join("scripts/dev-instance-env.sh"))
        .env("HOME", temp.path().join("home"))
        .env("XDG_CONFIG_HOME", &original_config)
        .env("GH_CONFIG_DIR", original_config.join("gh"))
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
        lines.first().map(std::path::PathBuf::from),
        Some(original_config.join("gh"))
    );
    assert_eq!(
        lines.get(1).map(std::path::PathBuf::from),
        Some(dev_home.join("config"))
    );
}

#[cfg(windows)]
#[test]
fn dev_instance_env_uses_the_native_github_cli_config_on_windows() {
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temp = tempfile::tempdir().unwrap();
    let app_data = temp.path().join("AppData").join("Roaming");

    let output = dev_env_command(&repo_root.join("scripts/dev-instance-env.sh"))
        .env("HOME", temp.path().join("msys-home"))
        .env("APPDATA", &app_data)
        .env_remove("GH_CONFIG_DIR")
        .env("ARCHDUCTOR_DEV_HOME", temp.path().join("dev-home"))
        .arg("sh")
        .arg("-c")
        .arg("printf '%s' \"$GH_CONFIG_DIR\"")
        .output()
        .expect("run dev instance env through bash");

    assert!(
        output.status.success(),
        "dev instance env failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::path::PathBuf::from(String::from_utf8(output.stdout).unwrap()),
        app_data.join("GitHub CLI")
    );
}

#[cfg(windows)]
#[test]
fn dev_instance_env_imports_the_registered_windows_path() {
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temp = tempfile::tempdir().unwrap();
    let tool_dir = temp.path().join("newly-installed-tool");
    std::fs::create_dir_all(&tool_dir).unwrap();
    std::fs::copy(
        std::path::Path::new(r"C:\Windows\System32\cmd.exe"),
        tool_dir.join("fresh-tool.exe"),
    )
    .unwrap();

    let output = dev_env_command(&repo_root.join("scripts/dev-instance-env.sh"))
        .env("HOME", temp.path().join("msys-home"))
        .env("APPDATA", temp.path().join("AppData").join("Roaming"))
        .env("UCRT64_ROOT", temp.path().join("missing-ucrt64"))
        .env("ARCHDUCTOR_WINDOWS_REGISTERED_PATH", tool_dir.as_os_str())
        .env("ARCHDUCTOR_DEV_HOME", temp.path().join("dev-home"))
        .arg("fresh-tool")
        .args(["/D", "/C", "exit", "0"])
        .output()
        .expect("run a tool from the registered Windows PATH");

    assert!(
        output.status.success(),
        "dev instance env should import registered Windows PATH entries: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(windows)]
#[test]
fn dev_instance_env_print_continues_without_the_windows_gtk_toolchain() {
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temp = tempfile::tempdir().unwrap();

    let output = dev_env_command(&repo_root.join("scripts/dev-instance-env.sh"))
        .env("HOME", temp.path().join("msys-home"))
        .env("APPDATA", temp.path().join("AppData").join("Roaming"))
        .env("UCRT64_ROOT", temp.path().join("missing-ucrt64"))
        .env("ARCHDUCTOR_DEV_HOME", temp.path().join("dev-home"))
        .env_remove("CARGO_BUILD_TARGET")
        .arg("--print")
        .output()
        .expect("print dev instance env without GTK toolchain");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("ARCHDUCTOR_DEV_INSTANCE="));
    assert!(!stdout.contains("x86_64-pc-windows-gnu"));
}

#[cfg(windows)]
#[test]
fn dev_instance_env_run_dev_requires_the_windows_gtk_toolchain() {
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temp = tempfile::tempdir().unwrap();

    let output = dev_env_command(&repo_root.join("scripts/dev-instance-env.sh"))
        .env("HOME", temp.path().join("msys-home"))
        .env("APPDATA", temp.path().join("AppData").join("Roaming"))
        .env("UCRT64_ROOT", temp.path().join("missing-ucrt64"))
        .env("ARCHDUCTOR_DEV_HOME", temp.path().join("dev-home"))
        .arg("--run-dev")
        .output()
        .expect("run dev instance env without GTK toolchain");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Windows GTK toolchain not found"));
}

#[cfg(windows)]
#[test]
fn dev_instance_env_selects_the_msys2_ucrt_toolchain_on_windows() {
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temp = tempfile::tempdir().unwrap();

    let output = dev_env_command(&repo_root.join("scripts/dev-instance-env.sh"))
        .env("HOME", temp.path().join("home"))
        .env("ARCHDUCTOR_DEV_HOME", temp.path().join("dev-home"))
        .arg("sh")
        .arg("-c")
        .arg("printf '%s\n%s\n%s\n' \"$CARGO_BUILD_TARGET\" \"$PKG_CONFIG\" \"$ARCHDUCTOR_ARCHCAR_BIN\"")
        .output()
        .expect("run dev instance env through bash");

    assert!(
        output.status.success(),
        "dev instance env failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines = stdout.lines().collect::<Vec<_>>();

    assert_eq!(lines.first().copied(), Some("x86_64-pc-windows-gnu"));
    assert!(
        lines
            .get(1)
            .is_some_and(|path| path.ends_with("/ucrt64/bin/pkgconf.exe")),
        "PKG_CONFIG should point at the MSYS2 UCRT64 pkgconf: {stdout}"
    );
    assert!(
        lines
            .get(2)
            .is_some_and(|path| path.contains("target/x86_64-pc-windows-gnu/debug/archcar.exe")),
        "archcar path should use the GNU target directory: {stdout}"
    );
}

#[cfg(windows)]
#[test]
fn dev_instance_env_bootstraps_msys2_commands_without_a_unix_path() {
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new(r"C:\msys64\usr\bin\bash.exe")
        .arg(repo_root.join("scripts/dev-instance-env.sh"))
        .env("PATH", r"C:\Windows\System32")
        .arg("sh")
        .arg("-c")
        .arg("printf '%s' \"$CARGO_BUILD_TARGET\"")
        .output()
        .expect("run dev instance env through MSYS2 bash");

    assert!(
        output.status.success(),
        "dev instance env should bootstrap MSYS2 PATH: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "x86_64-pc-windows-gnu"
    );
}
