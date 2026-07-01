# CI/CD Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add mature GitHub Actions validation, security scanning, release artifact checks, and dependency automation for the Rust Linux Archductor workspace.

**Architecture:** Keep fast Rust build/test validation in the existing `Test` workflow. Put supply-chain, SARIF-producing scans, and GitHub Actions analysis in a separate `Security` workflow with least-privilege permissions. Harden release publishing by scanning generated artifacts, producing checksums, and creating provenance attestations.

**Tech Stack:** GitHub Actions, Rust stable, cargo/clippy/rustfmt, cargo-deny, Trivy, CodeQL, zizmor, actionlint, Dependabot, nfpm, AppImage.

---

### Task 1: Rust PR Validation

**Files:**
- Modify: `.github/workflows/test.yml`

- [x] Add `clippy` to the installed Rust components.
- [x] Add `cargo clippy --workspace --all-targets --locked -- -D warnings`.
- [x] Keep formatting, build, and test commands aligned with `docs/deploy-and-local-test.md`.

### Task 2: Supply-Chain And Workflow Security

**Files:**
- Create: `.github/workflows/security.yml`
- Create: `deny.toml`

- [x] Add cargo-deny checks for advisories, bans, licenses, and sources.
- [x] Add PR dependency review.
- [x] Add Trivy filesystem scan with SARIF upload.
- [x] Add actionlint and zizmor checks for GitHub Actions hardening.

### Task 3: Code Scanning

**Files:**
- Create: `.github/workflows/codeql.yml`

- [x] Add CodeQL Rust scanning on pushes, pull requests, weekly schedule, and manual dispatch.
- [x] Install GTK/libadwaita build dependencies before CodeQL's manual Rust build.

### Task 4: Release Hardening

**Files:**
- Modify: `.github/workflows/publish.yml`

- [x] Add OIDC and attestations permissions.
- [x] Generate `SHA256SUMS` for release artifacts.
- [x] Run Trivy against packaged release artifacts.
- [x] Upload Trivy SARIF to code scanning.
- [x] Generate build provenance attestations for distributable files.
- [x] Attach checksums to GitHub releases.

### Task 5: Dependency Automation

**Files:**
- Create: `.github/dependabot.yml`

- [x] Enable weekly updates for Cargo dependencies and GitHub Actions.
- [x] Group Rust and Actions updates separately.

### Task 6: Verification

**Files:**
- Validate: `.github/workflows/*.yml`
- Validate: `deny.toml`

- [x] Run local YAML parsing.
- [x] Run `cargo fmt --all -- --check`.
- [x] Run `cargo clippy --workspace --all-targets --locked -- -D warnings`.
- [x] Run `cargo test --workspace --locked`.
- [x] Run `cargo deny check` if installable in the local environment.
