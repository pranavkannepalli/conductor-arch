#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"

raw_instance="${ARCHDUCTOR_DEV_INSTANCE:-}"
if [[ -z "$raw_instance" ]]; then
    raw_instance="$(git -C "$repo_root" branch --show-current 2>/dev/null || true)"
fi
if [[ -z "$raw_instance" ]]; then
    raw_instance="$(basename "$repo_root")"
fi

slug="$(
    printf '%s' "$raw_instance" \
        | tr '[:upper:]' '[:lower:]' \
        | sed -E 's/[^a-z0-9]+/-/g; s/^-+//; s/-+$//; s/-+/-/g'
)"
if [[ -z "$slug" ]]; then
    slug="default"
fi

base="${ARCHDUCTOR_DEV_HOME:-$repo_root/.archductor/dev-instances/$slug}"
mkdir -p "$base/config" "$base/data" "$base/state" "$base/cache"

export ARCHDUCTOR_DEV_INSTANCE="$slug"
export ARCHDUCTOR_ARCHCAR_BIN="${ARCHDUCTOR_ARCHCAR_BIN:-$repo_root/target/debug/archcar}"
export XDG_CONFIG_HOME="$base/config"
export XDG_DATA_HOME="$base/data"
export XDG_STATE_HOME="$base/state"
export XDG_CACHE_HOME="$base/cache"

if [[ "${1:-}" == "--print" ]]; then
    printf 'ARCHDUCTOR_DEV_INSTANCE=%s\n' "$ARCHDUCTOR_DEV_INSTANCE"
    printf 'ARCHDUCTOR_ARCHCAR_BIN=%s\n' "$ARCHDUCTOR_ARCHCAR_BIN"
    printf 'XDG_CONFIG_HOME=%s\n' "$XDG_CONFIG_HOME"
    printf 'XDG_DATA_HOME=%s\n' "$XDG_DATA_HOME"
    printf 'XDG_STATE_HOME=%s\n' "$XDG_STATE_HOME"
    printf 'XDG_CACHE_HOME=%s\n' "$XDG_CACHE_HOME"
    exit 0
fi

exec "$@"
