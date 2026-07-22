#!/usr/bin/env bash
set -euo pipefail

case "${OSTYPE:-}" in
    msys*|cygwin*)
        windows_host=1
        export PATH="/usr/bin:$PATH"
        ;;
esac

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"

case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*)
        ucrt_root="${UCRT64_ROOT:-C:/msys64/ucrt64}"
        if command -v cygpath >/dev/null 2>&1; then
            ucrt_root="$(cygpath -u "$ucrt_root")"
        fi
        pkgconf="$ucrt_root/bin/pkgconf.exe"
        if [[ ! -x "$pkgconf" ]]; then
            if [[ "${1:-}" == "--run-dev" ]]; then
                printf 'Windows GTK toolchain not found at %s. Install MSYS2 UCRT64 pkgconf, GTK4, and libadwaita first.\n' "$ucrt_root" >&2
                exit 1
            fi
            pkgconf=""
        fi

        registered_path="${ARCHDUCTOR_WINDOWS_REGISTERED_PATH:-}"
        if [[ -z "$registered_path" ]]; then
            powershell_exe="/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe"
            registered_path="$("$powershell_exe" -NoProfile -NonInteractive -Command "[Console]::Out.Write(([Environment]::GetEnvironmentVariable('Path','Machine') + ';' + [Environment]::GetEnvironmentVariable('Path','User')))" 2>/dev/null || true)"
        fi
        registered_msys_path=""
        old_ifs="$IFS"
        IFS=';'
        for entry in $registered_path; do
            [[ -n "$entry" ]] || continue
            converted="$(cygpath -u "$entry" 2>/dev/null || true)"
            [[ -n "$converted" ]] || continue
            registered_msys_path="${registered_msys_path:+$registered_msys_path:}$converted"
        done
        IFS="$old_ifs"

        export PATH="${pkgconf:+$ucrt_root/bin:}${registered_msys_path:+$registered_msys_path:}$PATH"
        if [[ -n "$pkgconf" ]]; then
            export CARGO_BUILD_TARGET="${CARGO_BUILD_TARGET:-x86_64-pc-windows-gnu}"
            export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="${CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER:-gcc}"
            export CC_x86_64_pc_windows_gnu="${CC_x86_64_pc_windows_gnu:-gcc}"
            export AR_x86_64_pc_windows_gnu="${AR_x86_64_pc_windows_gnu:-ar}"
            export PKG_CONFIG="${PKG_CONFIG:-$pkgconf}"
            export PKG_CONFIG_x86_64_pc_windows_gnu="${PKG_CONFIG_x86_64_pc_windows_gnu:-$pkgconf}"
            export PKG_CONFIG_PATH="${PKG_CONFIG_PATH:-$ucrt_root/lib/pkgconfig}"
            export PKG_CONFIG_PATH_x86_64_pc_windows_gnu="${PKG_CONFIG_PATH_x86_64_pc_windows_gnu:-$ucrt_root/lib/pkgconfig}"
            export PKG_CONFIG_ALLOW_CROSS="${PKG_CONFIG_ALLOW_CROSS:-1}"
        fi
        ;;
esac

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

user_config_home="${XDG_CONFIG_HOME:-$HOME/.config}"
if [[ "${windows_host:-0}" == "1" && -n "${APPDATA:-}" ]]; then
    default_gh_config_dir="$APPDATA/GitHub CLI"
else
    default_gh_config_dir="$user_config_home/gh"
fi

export ARCHDUCTOR_DEV_INSTANCE="$slug"
if [[ "${CARGO_BUILD_TARGET:-}" == "x86_64-pc-windows-gnu" ]]; then
    default_archcar_bin="$repo_root/target/$CARGO_BUILD_TARGET/debug/archcar.exe"
    default_gtk_bin="$repo_root/target/$CARGO_BUILD_TARGET/debug/archductor-gtk.exe"
    default_dev_runner_bin="$repo_root/target/$CARGO_BUILD_TARGET/debug/archductor-dev.exe"
else
    default_archcar_bin="$repo_root/target/debug/archcar"
    default_gtk_bin="$repo_root/target/debug/archductor-gtk"
    default_dev_runner_bin="$repo_root/target/debug/archductor-dev"
fi
export ARCHDUCTOR_ARCHCAR_BIN="${ARCHDUCTOR_ARCHCAR_BIN:-$default_archcar_bin}"
export ARCHDUCTOR_GTK_BIN="${ARCHDUCTOR_GTK_BIN:-$default_gtk_bin}"
export ARCHDUCTOR_DEV_RUNNER_BIN="${ARCHDUCTOR_DEV_RUNNER_BIN:-$default_dev_runner_bin}"
export GH_CONFIG_DIR="${GH_CONFIG_DIR:-$default_gh_config_dir}"
export XDG_CONFIG_HOME="$base/config"
export XDG_DATA_HOME="$base/data"
export XDG_STATE_HOME="$base/state"
export XDG_CACHE_HOME="$base/cache"

if [[ "${1:-}" == "--print" ]]; then
    printf 'ARCHDUCTOR_DEV_INSTANCE=%s\n' "$ARCHDUCTOR_DEV_INSTANCE"
    printf 'ARCHDUCTOR_ARCHCAR_BIN=%s\n' "$ARCHDUCTOR_ARCHCAR_BIN"
    printf 'GH_CONFIG_DIR=%s\n' "$GH_CONFIG_DIR"
    printf 'XDG_CONFIG_HOME=%s\n' "$XDG_CONFIG_HOME"
    printf 'XDG_DATA_HOME=%s\n' "$XDG_DATA_HOME"
    printf 'XDG_STATE_HOME=%s\n' "$XDG_STATE_HOME"
    printf 'XDG_CACHE_HOME=%s\n' "$XDG_CACHE_HOME"
    exit 0
fi

if [[ "${1:-}" == "--run-dev" ]]; then
    exec "$ARCHDUCTOR_DEV_RUNNER_BIN"
fi

exec "$@"
