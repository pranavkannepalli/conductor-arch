#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'USAGE'
Usage: scripts/release-readiness.sh [--version VERSION] [--skip-tests] [--package]

Runs the local release-readiness gate for Linux Archductor.

Options:
  --version VERSION  Version used for local package names. Default: 0.1.0
  --skip-tests       Skip fmt, clippy, tests, and release build.
  --skip-doctor      Skip the linux-archductor doctor check.
  --skip-deny        Skip cargo-deny even when it is installed.
  --package          Build local .deb/.rpm/AppImage artifacts on Linux.

The script is intentionally conservative: package mode only runs on Linux and
requires nfpm plus appimagetool, while CI remains the authoritative tag
publisher.
USAGE
}

version="0.1.0"
skip_tests=0
skip_doctor=0
skip_deny=0
package=0

while [ $# -gt 0 ]; do
    case "$1" in
        --version)
            if [ $# -lt 2 ]; then
                echo "error: --version requires a value" >&2
                exit 2
            fi
            version="$2"
            shift 2
            ;;
        --skip-tests)
            skip_tests=1
            shift
            ;;
        --skip-doctor)
            skip_doctor=1
            shift
            ;;
        --skip-deny)
            skip_deny=1
            shift
            ;;
        --package)
            package=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "error: unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
    echo "error: version must look like MAJOR.MINOR.PATCH, got: $version" >&2
    exit 2
fi

run() {
    echo
    echo "==> $*"
    "$@"
}

if [ "$skip_tests" -eq 0 ]; then
    run cargo fmt --all -- --check
    run cargo clippy --workspace --all-targets --locked -- -D warnings
    run cargo test --workspace --locked
    run cargo build --workspace --release --locked
fi

if [ "$skip_doctor" -eq 0 ]; then
    if [ ! -x ./target/release/linux-archductor ]; then
        echo "error: ./target/release/linux-archductor is missing; run without --skip-tests first" >&2
        exit 1
    fi
    run ./target/release/linux-archductor doctor
fi

if [ "$skip_deny" -eq 1 ]; then
    echo
    echo "==> cargo-deny skipped by --skip-deny"
elif command -v cargo-deny >/dev/null 2>&1; then
    run cargo deny check
else
    echo
    echo "==> cargo-deny not found; skipping local dependency policy check"
fi

if [ "$package" -eq 0 ]; then
    echo
    echo "==> Package build skipped. Pass --package to build local artifacts."
    exit 0
fi

if [ "$(uname -s)" != "Linux" ]; then
    echo
    echo "==> Local package build skipped: Linux release artifacts must be built on Linux or by CI."
    exit 0
fi

run mkdir -p dist

run tar -czf "dist/linux-archductor-${version}-linux-x86_64.tar.gz" \
    -C target/release linux-archductor linux-archductor-gtk

if ! command -v nfpm >/dev/null 2>&1; then
    echo "error: nfpm is required for --package on Linux" >&2
    exit 1
fi

if ! command -v appimagetool >/dev/null 2>&1; then
    echo "error: appimagetool is required for --package on Linux" >&2
    exit 1
fi

echo
echo "==> VERSION=${version} nfpm package --packager deb --target dist/"
VERSION="$version" nfpm package --packager deb --target dist/
echo
echo "==> VERSION=${version} nfpm package --packager rpm --target dist/"
VERSION="$version" nfpm package --packager rpm --target dist/

appdir="packaging/appimage/linux-archductor.AppDir"
run install -Dm755 target/release/linux-archductor "$appdir/usr/bin/linux-archductor"
run install -Dm755 target/release/linux-archductor-gtk "$appdir/usr/bin/linux-archductor-gtk"
run appimagetool --appimage-extract-and-run "$appdir" \
    "dist/linux-archductor-${version}-x86_64.AppImage"

if compgen -G "dist/*" >/dev/null; then
    echo
    echo "==> Generating dist/SHA256SUMS"
    if command -v sha256sum >/dev/null 2>&1; then
        (cd dist && sha256sum ./* > SHA256SUMS)
    else
        (cd dist && shasum -a 256 ./* > SHA256SUMS)
    fi
    ls -lh dist
fi
