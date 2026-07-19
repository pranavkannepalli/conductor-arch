VERSION ?= 0.1.0
DEV_ENV := scripts/dev-instance-env.sh

.PHONY: help dev dev-env archcar gtk cli run build build-release check release tag publish-tag

help:
	@printf '%s\n' \
		'make gtk                      Run GTK app in branch-scoped dev mode' \
		'make archcar                  Run archcar sidecar in branch-scoped dev mode' \
		'make cli                      Run CLI in branch-scoped dev mode' \
		'make run                      Alias for make gtk' \
		'make dev                      Build workspace, then run watched GTK app for this branch' \
		'make dev-env                  Print branch-scoped dev environment' \
		'make build                    Build workspace in dev mode' \
		'make build-release            Build workspace in release mode' \
		'make check                    Run fmt, clippy, and tests' \
		'make release VERSION=x.y.z    Run local release gate and build packages' \
		'make tag VERSION=x.y.z        Create git tag vVERSION' \
		'make publish-tag VERSION=x.y.z Push git tag vVERSION'

dev:
	@cargo build --workspace
	@cleanup_dev() { \
		status=$$?; \
		trap - INT TERM EXIT; \
		if [ -n "$$archcar_pid" ]; then kill "$$archcar_pid" 2>/dev/null || true; fi; \
		if [ -n "$$gtk_pid" ]; then kill "$$gtk_pid" 2>/dev/null || true; fi; \
		wait "$$archcar_pid" "$$gtk_pid" 2>/dev/null || true; \
		exit "$$status"; \
	}; \
	trap cleanup_dev INT TERM EXIT; \
	$(DEV_ENV) cargo run --bin archcar & \
	archcar_pid=$$!; \
	$(DEV_ENV) cargo watch -w crates -w Cargo.toml -w Cargo.lock -x "run --bin archductor-gtk" & \
	gtk_pid=$$!; \
	wait "$$archcar_pid" "$$gtk_pid"

dev-env:
	@$(DEV_ENV) --print

archcar:
	$(DEV_ENV) cargo run --bin archcar

gtk:
	$(DEV_ENV) cargo run --bin archductor-gtk

cli:
	$(DEV_ENV) cargo run --bin archductor --

run: gtk

build:
	cargo build --workspace

build-release:
	cargo build --workspace --release --locked

check:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets --locked -- -D warnings
	cargo test --workspace --locked

release:
	scripts/release-readiness.sh --version $(VERSION) --package

tag:
	git tag -a v$(VERSION) -m "v$(VERSION)"

publish-tag:
	git push origin v$(VERSION)
