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
