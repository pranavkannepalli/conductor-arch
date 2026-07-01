# Archcar Harness Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace direct GTK PTY harness management with a reusable `archcar` Unix-socket controller used by both GTK and CLI, with Codex working and Claude stubbed.

**Architecture:** Add a new `archcar` sidecar binary plus shared protocol/client/server modules in `crates/core`. Migrate GTK workspace/session flows to the shared client and route live state through typed controller events and snapshots.

**Tech Stack:** Rust, anyhow, portable-pty, vt100, serde, Unix domain sockets, Tokio mpsc

---

### Task 1: Create the Archcar crate and shared module skeleton

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/core/Cargo.toml`
- Modify: `crates/core/src/lib.rs`
- Create: `crates/archcar/Cargo.toml`
- Create: `crates/archcar/src/main.rs`
- Create: `crates/core/src/archcar/mod.rs`
- Create: `crates/core/src/archcar/protocol.rs`
- Create: `crates/core/src/archcar/client.rs`
- Create: `crates/core/src/archcar/server.rs`
- Create: `crates/core/src/archcar/session.rs`
- Create: `crates/core/src/archcar/harness.rs`
- Test: `cargo test -p linux-archductor-core archcar -- --nocapture`

- [ ] Write the failing module-shape tests
- [ ] Run the focused core test command and confirm it fails
- [ ] Add the new crate, modules, and exported entry points
- [ ] Re-run the focused core test command and confirm it passes

### Task 2: Implement typed protocol and Codex/Claude harness abstraction

**Files:**
- Modify: `crates/core/src/archcar/protocol.rs`
- Modify: `crates/core/src/archcar/harness.rs`
- Test: `cargo test -p linux-archductor-core archcar::protocol -- --nocapture`
- Test: `cargo test -p linux-archductor-core archcar::harness -- --nocapture`

- [ ] Write failing tests for protocol serialization, Codex readiness detection, and Claude stub unsupported responses
- [ ] Run the focused test commands and confirm they fail for the right reasons
- [ ] Implement the minimal protocol types and harness trait/structs to satisfy those tests
- [ ] Re-run the focused test commands and confirm they pass

### Task 3: Implement session worker and server debounce/event flow

**Files:**
- Modify: `crates/core/src/archcar/server.rs`
- Modify: `crates/core/src/archcar/session.rs`
- Modify: `crates/core/src/archcar/client.rs`
- Test: `cargo test -p linux-archductor-core archcar::server -- --nocapture`

- [ ] Write failing tests for immediate spawn ack, workspace debounce, and subscriber event delivery
- [ ] Run the focused server test command and confirm it fails
- [ ] Implement the minimal async server/session flow to satisfy those tests
- [ ] Re-run the focused server test command and confirm it passes

### Task 4: Migrate GTK to use Archcar client

**Files:**
- Modify: `crates/gtk-app/Cargo.toml`
- Modify: `crates/gtk-app/src/session_surface.rs`
- Modify: `crates/gtk-app/src/sidebar.rs`
- Modify: `crates/gtk-app/src/workspace_command_center.rs`
- Test: `cargo test -p linux-archductor-gtk session_surface -- --nocapture`

- [ ] Write a failing GTK regression test covering “workspace selection does not directly spawn PTY on the UI path”
- [ ] Run the focused GTK test command and confirm it fails
- [ ] Replace direct PTY/session calls with `ArchcarClient` calls and event/snapshot refresh logic
- [ ] Re-run the focused GTK test command and confirm it passes

### Task 5: Add CLI reuse path and wire Archcar binary startup

**Files:**
- Modify: `crates/cli/Cargo.toml`
- Modify: `crates/cli/src/main.rs`
- Modify: `crates/archcar/src/main.rs`
- Test: `cargo test -p linux-archductor archcar -- --nocapture`

- [ ] Write failing CLI tests for basic controller-backed status or send flow
- [ ] Run the focused CLI test command and confirm it fails
- [ ] Implement minimal CLI client commands and sidecar bootstrap wiring
- [ ] Re-run the focused CLI test command and confirm it passes

### Task 6: Remove or deprecate old direct harness path and run full verification

**Files:**
- Modify: `crates/core/src/harness.rs`
- Modify: `crates/core/src/workspace.rs`
- Modify: `crates/gtk-app/src/logger.rs`
- Modify: `README.md` if controller startup/use needs doc changes
- Test: `cargo test --workspace -- --nocapture`

- [ ] Remove dead direct GTK harness code or fence it behind explicit deprecated helpers not used by GTK
- [ ] Run the full workspace test command
- [ ] Fix any regressions
- [ ] Re-run the full workspace test command and confirm it passes
