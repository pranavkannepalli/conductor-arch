use std::fs;
use std::path::PathBuf;

#[test]
fn windows_publish_build_uses_path_resolved_msys_tools() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflow = fs::read_to_string(repo_root.join(".github/workflows/publish.yml")).unwrap();

    assert!(
        workflow.contains("Verify Windows GTK pkg-config"),
        "Windows release should smoke-test pkgconf before cargo build"
    );
    assert!(
        workflow.contains("PKG_CONFIG: pkgconf"),
        "Windows release should use the PATH-resolved pkgconf executable"
    );
    assert!(
        workflow.contains("CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER: gcc"),
        "Windows release should use the PATH-resolved UCRT64 gcc executable"
    );
    assert!(
        !workflow.contains("PKG_CONFIG: C:\\msys64\\ucrt64\\bin\\pkgconf.exe"),
        "absolute MSYS pkgconf paths failed to spawn in GitHub Actions"
    );
}
