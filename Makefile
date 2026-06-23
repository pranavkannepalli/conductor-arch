VERSION ?= 0.1.0

.PHONY: help gtk cli run build build-release check release tag publish-tag

help:
	@printf '%s\n' \
		'make gtk                      Run GTK app in dev mode' \
		'make cli                      Run CLI in dev mode' \
		'make run                      Alias for make gtk' \
		'make build                    Build workspace in dev mode' \
		'make build-release            Build workspace in release mode' \
		'make check                    Run fmt, clippy, and tests' \
		'make release VERSION=x.y.z    Run local release gate and build packages' \
		'make tag VERSION=x.y.z        Create git tag vVERSION' \
		'make publish-tag VERSION=x.y.z Push git tag vVERSION'

dev:
	@trap 'kill 0' INT TERM EXIT; \
	cargo watch -x "run --bin linux-conductor-gtk" & \
	cargo watch -x "run --bin linux-conductor --" & \
	wait

gtk:
	cargo run --bin linux-conductor-gtk

cli:
	cargo run --bin linux-conductor --

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
