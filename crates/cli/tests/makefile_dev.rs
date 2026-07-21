use std::fs;
use std::path::PathBuf;

fn makefile() -> String {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::read_to_string(repo_root.join("Makefile")).expect("read root Makefile")
}

#[test]
fn make_dev_uses_the_shared_interactive_runner() {
    let makefile = makefile();
    assert!(
        makefile.contains("$(DEV_ENV) cargo build --workspace")
            && makefile.contains("$(DEV_ENV) --run-dev"),
        "make dev should use the shared interactive runner"
    );
}

#[test]
fn make_dev_does_not_automatically_watch_sources() {
    assert!(
        !makefile().contains("cargo watch"),
        "make dev should reload only when the user presses r"
    );
}

#[test]
fn make_dev_uses_the_platform_dev_environment() {
    assert!(
        makefile().contains("$(DEV_ENV) --run-dev"),
        "make dev must configure the platform GTK environment"
    );
}

#[test]
fn make_uses_msys2_bash_for_windows_dev_recipes() {
    let makefile = makefile();
    assert!(
        makefile.contains("ifeq ($(OS),Windows_NT)")
            && makefile.contains("SHELL := C:/msys64/usr/bin/bash.exe")
            && makefile
                .contains("DEV_ENV := C:/msys64/usr/bin/bash.exe scripts/dev-instance-env.sh"),
        "Windows make targets should use the required MSYS2 toolchain"
    );
}

#[test]
fn make_dev_advertises_flutter_style_controls() {
    let makefile = makefile();
    assert!(
        makefile.contains("r Reload GTK") && makefile.contains("q Quit"),
        "make help should advertise the interactive controls"
    );
    assert!(
        !makefile.contains("dev-windows.ps1") && !makefile.contains("dev-windows-child.sh"),
        "Windows and Linux should use the same Rust runner"
    );
}
