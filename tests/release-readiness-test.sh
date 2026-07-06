#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
script="$repo_root/scripts/release-readiness.sh"
aur_script="$repo_root/scripts/update-aur-checksum.sh"
required_docs=(
    docs/conductor-gui-mvp-handoff.md
    docs/mvp-scope.md
    docs/manual-testing-checklist.md
    docs/archductor-docs-parity-map.md
    README.md
)

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

for doc in "${required_docs[@]}"; do
    [ -s "$repo_root/$doc" ] || fail "required repository guidance doc missing or empty: $doc"
done

output="$("$script" --help)"
[[ "$output" == *"Usage: scripts/release-readiness.sh"* ]] \
    || fail "help output did not include usage"

set +e
output="$("$script" --version main --skip-tests 2>&1)"
status=$?
set -e
[[ "$status" -eq 2 ]] || fail "invalid version exited $status, expected 2"
[[ "$output" == *"version must look like MAJOR.MINOR.PATCH"* ]] \
    || fail "invalid version output did not explain version format"

if [ "$(uname -s)" != "Linux" ]; then
    rm -rf "$repo_root/dist"
    output="$("$script" --version 0.1.0 --skip-tests --skip-doctor --skip-deny --package)"
    [[ "$output" == *"Linux release artifacts must be built on Linux or by CI"* ]] \
        || fail "non-Linux package mode did not explain package skip"
    [ ! -e "$repo_root/dist" ] || fail "non-Linux package mode created dist"
fi

output="$("$aur_script" --help)"
[[ "$output" == *"Usage: scripts/update-aur-checksum.sh"* ]] \
    || fail "AUR helper help output did not include usage"

set +e
output="$("$aur_script" 0.1.0 not-a-checksum 2>&1)"
status=$?
set -e
[[ "$status" -eq 2 ]] || fail "invalid AUR checksum exited $status, expected 2"
[[ "$output" == *"64-character SHA-256"* ]] \
    || fail "invalid AUR checksum output did not explain checksum format"

echo "release-readiness-test: ok"
