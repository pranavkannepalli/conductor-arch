use std::fs;
use std::path::PathBuf;

#[test]
fn publish_build_uses_ci_verified_release_packaging() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let publish = fs::read_to_string(repo_root.join(".github/workflows/publish.yml")).unwrap();
    let ci = fs::read_to_string(repo_root.join(".github/workflows/ci.yml")).unwrap();
    let nfpm = fs::read_to_string(repo_root.join("nfpm.yaml")).unwrap();
    let app_run =
        fs::read_to_string(repo_root.join("packaging/appimage/archductor.AppDir/AppRun")).unwrap();
    let flatpak =
        fs::read_to_string(repo_root.join("packaging/flatpak/ai.perceo.Archductor.yml")).unwrap();
    let aur = fs::read_to_string(repo_root.join("packaging/aur/PKGBUILD")).unwrap();
    let nix = fs::read_to_string(repo_root.join("flake.nix")).unwrap();
    let homebrew =
        fs::read_to_string(repo_root.join("packaging/homebrew/Formula/archductor.rb")).unwrap();

    assert!(
        publish.contains("Verify Windows GTK pkg-config"),
        "Windows release should smoke-test pkgconf before cargo build"
    );
    assert!(
        !publish.contains("continue-on-error: true"),
        "publish should stay strict; CI should catch package failures before release"
    );
    assert!(
        ci.contains("linux-release-packages:"),
        "CI should build Linux release artifacts before publish"
    );
    assert!(
        ci.contains("windows-release-package:"),
        "CI should build the Windows portable ZIP before publish"
    );
    assert!(
        ci.contains("      - linux-release-packages")
            && ci.contains("      - windows-release-package")
            && ci.contains("linux-release-packages=${{ needs.linux-release-packages.result }}")
            && ci.contains("windows-release-package=${{ needs.windows-release-package.result }}"),
        "release package preview jobs should be required by ci-gate"
    );
    assert!(
        ci.contains("Build .deb") && ci.contains("Build .rpm") && ci.contains("Build AppImage"),
        "CI should exercise Linux package creation before publish"
    );
    assert!(
        ci.contains("Assemble portable Windows bundle") && ci.contains("Compress-Archive"),
        "CI should exercise Windows ZIP assembly before publish"
    );
    assert!(
        ci.contains("Scan release artifacts with Trivy"),
        "CI should scan generated release artifacts before publish"
    );
    assert!(
        publish.contains("\"PKG_CONFIG=$pkgconf\"") && ci.contains("\"PKG_CONFIG=$pkgconf\""),
        "Windows workflows should use pkgconf from the actual MSYS2 install"
    );
    assert!(
        publish.contains("steps.msys2.outputs.msys2-location")
            && ci.contains("steps.msys2.outputs.msys2-location"),
        "Windows workflows should derive MSYS2 paths from setup-msys2 output"
    );
    assert!(
        publish.contains("CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER: gcc"),
        "Windows release should use the PATH-resolved UCRT64 gcc executable"
    );
    assert!(
        publish.contains("Validate AUR package")
            && publish.contains("makepkg --noconfirm")
            && publish.contains("pacman -U --noconfirm /pkg/archductor-*.pkg.tar.*")
            && publish.contains("xvfb-run -a timeout 15s archductor-gtk --page dashboard")
            && publish.contains("[ \"$gtk_status\" -ne 0 ] && [ \"$gtk_status\" -ne 124 ]"),
        "publish should build, install, and smoke-test the AUR package before publishing"
    );
    assert!(
        publish.contains("Validate Homebrew formula")
            && publish.contains("brew audit --strict --online --formula")
            && publish.contains("brew install --build-from-source packaging/homebrew/Formula/archductor.rb")
            && publish.contains("brew test archductor")
            && publish.contains("xvfb-run -a timeout 15s archductor-gtk --page dashboard")
            && publish.contains("[ \"$gtk_status\" -ne 0 ] && [ \"$gtk_status\" -ne 124 ]"),
        "publish should audit, install, test, and smoke-test the Homebrew formula before publishing"
    );
    assert!(
        aur.contains("export LIBSQLITE3_SYS_USE_PKG_CONFIG=1")
            && homebrew.contains("ENV[\"LIBSQLITE3_SYS_USE_PKG_CONFIG\"] = \"1\"")
            && nix.contains("LIBSQLITE3_SYS_USE_PKG_CONFIG = \"1\";"),
        "source-built package managers should use distro SQLite through pkg-config"
    );
    assert!(
        !publish.contains("PKG_CONFIG: C:\\msys64\\ucrt64\\bin\\pkgconf.exe")
            && !ci.contains("PKG_CONFIG: C:\\msys64\\ucrt64\\bin\\pkgconf.exe"),
        "absolute MSYS pkgconf paths failed to spawn in GitHub Actions"
    );
    assert!(
        !publish.contains("C:\\msys64\\ucrt64") && !ci.contains("C:\\msys64\\ucrt64"),
        "Windows workflows should not assume setup-msys2 installs under C:\\msys64"
    );
    for (name, manifest, archductor_tokens, gtk_tokens) in [
        (
            "nfpm",
            &nfpm,
            vec!["src: target/release/archductor\n    dst: /usr/bin/archductor"],
            vec!["src: target/release/archductor-gtk\n    dst: /usr/bin/archductor-gtk"],
        ),
        (
            "AppRun",
            &app_run,
            vec!["exec \"$SELF_DIR/usr/bin/archductor\" \"$@\""],
            vec!["exec \"$SELF_DIR/usr/bin/archductor-gtk\""],
        ),
        (
            "flatpak",
            &flatpak,
            vec!["install -Dm755 target/release/archductor /app/bin/archductor"],
            vec!["install -Dm755 target/release/archductor-gtk /app/bin/archductor-gtk"],
        ),
        (
            "nix",
            &nix,
            vec!["install -Dm755 target/release/archductor \"$out/bin/archductor\""],
            vec!["install -Dm755 target/release/archductor-gtk \"$out/bin/archductor-gtk\""],
        ),
        (
            "homebrew",
            &homebrew,
            vec!["bin.install \"target/release/archductor\""],
            vec!["bin.install \"target/release/archductor-gtk\""],
        ),
        (
            "publish",
            &publish,
            vec![
                "-C target/release archductor archductor-gtk",
                "install -Dm755 target/release/archductor \"$APPDIR/usr/bin/archductor\"",
                "Copy-Item target\\x86_64-pc-windows-gnu\\release\\archductor.exe $bundle",
            ],
            vec![
                "install -Dm755 target/release/archductor-gtk \"$APPDIR/usr/bin/archductor-gtk\"",
                "Copy-Item target\\x86_64-pc-windows-gnu\\release\\archductor-gtk.exe $bundle",
            ],
        ),
        (
            "ci",
            &ci,
            vec![
                "cp target/debug/archductor ci-artifacts/bin/",
                "-C target/release archductor archductor-gtk",
                "install -Dm755 target/release/archductor \"$APPDIR/usr/bin/archductor\"",
                "Copy-Item target\\x86_64-pc-windows-gnu\\release\\archductor.exe $bundle",
            ],
            vec![
                "cp target/debug/archductor-gtk ci-artifacts/bin/",
                "install -Dm755 target/release/archductor-gtk \"$APPDIR/usr/bin/archductor-gtk\"",
                "Copy-Item target\\x86_64-pc-windows-gnu\\release\\archductor-gtk.exe $bundle",
            ],
        ),
    ] {
        assert!(
            archductor_tokens
                .iter()
                .all(|token| manifest.contains(token)),
            "{name} should ship the plain archductor binary through exact package paths or tokens"
        );
        assert!(
            gtk_tokens.iter().all(|token| manifest.contains(token)),
            "{name} should ship the archductor-gtk binary"
        );
        assert!(
            !manifest.contains("archductor-cli"),
            "{name} packaged CLI binary should remain archductor, not archductor-cli"
        );
    }
}
