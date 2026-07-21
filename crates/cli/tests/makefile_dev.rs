use std::fs;
use std::path::PathBuf;

#[test]
fn make_dev_cleanup_does_not_signal_its_own_process_group() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let makefile = fs::read_to_string(repo_root.join("Makefile")).expect("read root Makefile");

    assert!(
        !makefile.contains("kill 0"),
        "make dev cleanup must not signal the whole process group"
    );
    assert!(
        makefile.contains("cleanup_dev()"),
        "make dev should use an explicit cleanup function"
    );
    assert!(
        makefile.contains("archcar_pid") && makefile.contains("gtk_pid"),
        "make dev should terminate only the child processes it started"
    );
}

#[test]
fn make_dev_watch_avoids_generated_build_state() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let makefile = fs::read_to_string(repo_root.join("Makefile")).expect("read root Makefile");

    assert!(
        makefile.contains("cargo watch -w crates -w Cargo.toml -w Cargo.lock"),
        "make dev should watch source roots explicitly so generated build state is not crawled"
    );
    assert!(
        !makefile.contains("cargo watch -x \"run --bin archductor-gtk\""),
        "make dev should not let cargo-watch default to watching the whole repo"
    );
}

#[test]
fn make_dev_build_uses_the_platform_dev_environment() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let makefile = fs::read_to_string(repo_root.join("Makefile")).expect("read root Makefile");

    assert!(
        makefile.contains("$(DEV_ENV) cargo build --workspace"),
        "make dev must configure the Windows GNU/GTK environment before its initial build"
    );
}

#[test]
fn make_uses_msys2_bash_for_windows_dev_recipes() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let makefile = fs::read_to_string(repo_root.join("Makefile")).expect("read root Makefile");

    assert!(
        makefile.contains("ifeq ($(OS),Windows_NT)")
            && makefile.contains("SHELL := C:/msys64/usr/bin/bash.exe")
            && makefile
                .contains("DEV_ENV := C:/msys64/usr/bin/bash.exe scripts/dev-instance-env.sh"),
        "Windows make targets should use the Bash installed with the required MSYS2 toolchain"
    );
}

#[test]
fn windows_make_dev_uses_a_native_owned_process_tree_runner() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let makefile = fs::read_to_string(repo_root.join("Makefile")).expect("read root Makefile");
    let runner_path = repo_root.join("scripts/dev-windows.ps1");
    let child_launcher_path = repo_root.join("scripts/dev-windows-child.sh");

    assert!(
        makefile.contains(
            "powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/dev-windows.ps1"
        ),
        "Windows make dev should delegate lifecycle ownership to a native PowerShell runner"
    );

    let runner = fs::read_to_string(runner_path).expect("read Windows dev runner");
    assert!(
        runner.contains("finally"),
        "the runner must clean up when interrupted"
    );
    assert!(
        runner.contains("Stop-DevProcessTree -RootProcess $archcar")
            && runner.contains("Stop-DevProcessTree -RootProcess $gtk_watch"),
        "the runner must stop both process trees it started"
    );
    assert!(
        !runner.contains("Get-Process archcar") && !runner.contains("Get-Process archductor-gtk"),
        "cleanup must be scoped by owned process trees, not executable names"
    );

    let child_launcher =
        fs::read_to_string(child_launcher_path).expect("read Windows dev child launcher");
    assert!(
        child_launcher.contains("archcar)")
            && child_launcher.contains("/usr/bin/bash scripts/dev-instance-env.sh cargo run --bin archcar")
            && child_launcher.contains("gtk-watch)")
            && child_launcher.contains("/usr/bin/bash scripts/dev-instance-env.sh cargo watch")
            && child_launcher.contains("-w crates -w Cargo.toml -w Cargo.lock"),
        "the Bash launcher should construct child commands after crossing PowerShell's argument boundary"
    );
}
