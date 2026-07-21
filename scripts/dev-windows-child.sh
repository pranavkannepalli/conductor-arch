#!/usr/bin/env bash
set -euo pipefail

case "${1:-}" in
    archcar)
        exec /usr/bin/bash scripts/dev-instance-env.sh cargo run --bin archcar
        ;;
    gtk-watch)
        exec /usr/bin/bash scripts/dev-instance-env.sh cargo watch \
            -w crates -w Cargo.toml -w Cargo.lock \
            -x "run --bin archductor-gtk"
        ;;
    *)
        printf 'unknown Windows dev child: %s\n' "${1:-}" >&2
        exit 2
        ;;
esac
