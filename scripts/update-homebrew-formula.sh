#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'USAGE'
Usage: scripts/update-homebrew-formula.sh VERSION SHA256

Updates packaging/homebrew/Formula/archductor.rb with a release tag and source checksum.

Example:
  scripts/update-homebrew-formula.sh 0.1.0 \
    0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
USAGE
}

if [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ]; then
    usage
    exit 0
fi

if [ $# -ne 2 ]; then
    usage >&2
    exit 2
fi

version="$1"
checksum="$2"
formula="packaging/homebrew/Formula/archductor.rb"

if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
    echo "error: version must look like MAJOR.MINOR.PATCH, got: $version" >&2
    exit 2
fi

if [[ ! "$checksum" =~ ^[0-9a-fA-F]{64}$ ]]; then
    echo "error: checksum must be a 64-character SHA-256 hex digest" >&2
    exit 2
fi

tmp="$(mktemp)"
sed -E \
    -e "s#archive/refs/tags/v[^.]+\\.[^.]+\\.[^/]+\\.tar\\.gz#archive/refs/tags/v${version}.tar.gz#" \
    -e "s/^  sha256 \".*\"/  sha256 \"${checksum}\"/" \
    "$formula" > "$tmp"
mv "$tmp" "$formula"
chmod 0644 "$formula"

echo "Updated $formula to v$version"
