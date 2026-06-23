# Agent Harnesses Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add real Codex and Claude harnesses with shared launch logic, session controls, and structured transcript rendering.

**Architecture:** Put harness-specific launch logic in `crates/core` so CLI and GTK share one source of truth. Keep PTY-backed sessions, but make the launch payload, session metadata, and transcript labels agent-aware. GTK remains the control surface for session selection, parameter changes, and checkpointing.

**Tech Stack:** Rust, GTK4/libadwaita, portable-pty, SQLite, existing Codex and Claude CLIs.

---

### Task 1: Add a shared agent harness module in core

**Files:**
- Create: `crates/core/src/harness.rs`
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/core/src/workspace.rs`
- Test: `crates/core/src/harness.rs`
- Test: `crates/core/src/workspace.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn codex_launch_includes_workspace_and_harness_metadata() {
    // build a launch plan for Codex with plan, fast, goal, and skills
    // assert the command, cwd, env, and metadata are all populated
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p linux-conductor-core harness -- --nocapture`
Expected: fail because `crates/core/src/harness.rs` does not exist yet.

- [ ] **Step 3: Write minimal implementation**

```rust
pub enum AgentHarnessKind {
    Codex,
    Claude,
}

pub fn build_session_launch(...) -> Result<SessionLaunch> {
    // choose command, args, startup payload, env, and metadata
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p linux-conductor-core harness -- --nocapture`
Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/harness.rs crates/core/src/lib.rs crates/core/src/workspace.rs
git commit -m "feat(core): add shared agent harness builder"
```

### Task 2: Route workspace launches through the harness module

**Files:**
- Modify: `crates/core/src/workspace.rs`
- Modify: `crates/cli/src/main.rs`
- Test: `crates/core/src/workspace.rs`
- Test: `crates/cli/tests/cli_sessions.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn session_launch_uses_harness_builder_for_codex_and_claude() {
    // assert the workspace store returns the same launch data the harness module builds
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p linux-conductor-core session_launch -- --nocapture`
Expected: fail until `WorkspaceStore` delegates to the harness module.

- [ ] **Step 3: Write minimal implementation**

```rust
let launch = harness::build_session_launch(&workspace, kind, harness, &settings, &repository)?;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p linux-conductor-core session_launch -- --nocapture`
Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/workspace.rs crates/cli/src/main.rs crates/core/src/harness.rs
git commit -m "feat(core): route session launches through harness"
```

### Task 3: Make the GTK session panel render harness-aware state

**Files:**
- Modify: `crates/gtk-app/src/session_surface.rs`
- Modify: `crates/gtk-app/src/terminal.rs`
- Test: `crates/gtk-app/src/session_surface.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn selected_session_surface_shows_harness_event_labels() {
    // assert tool use, skill, and checkpoint markers are rendered as readable events
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p linux-conductor-gtk session_surface -- --nocapture`
Expected: fail on the new labels.

- [ ] **Step 3: Write minimal implementation**

```rust
fn is_system_event_marker(line: &str) -> bool {
    line.starts_with("[session ")
        || line.starts_with("[checkpoint")
        || line.starts_with("[tool ")
        || line.starts_with("[skill ")
        || line.starts_with("[harness ")
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p linux-conductor-gtk session_surface -- --nocapture`
Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add crates/gtk-app/src/session_surface.rs crates/gtk-app/src/terminal.rs
git commit -m "feat(gtk): render harness-aware session transcripts"
```

### Task 4: Prove Codex and Claude startup behavior with real wrappers

**Files:**
- Modify: `crates/cli/tests/cli_sessions.rs`
- Modify: `crates/core/src/workspace.rs`
- Test: `crates/cli/tests/cli_sessions.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn fake_codex_and_claude_wrappers_receive_expected_startup_payload() {
    // use shell wrapper scripts to capture argv and stdin for both harnesses
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p linux-conductor --test cli_sessions -- --nocapture`
Expected: fail until the wrapper test captures the new payload.

- [ ] **Step 3: Write minimal implementation**

```rust
fs::write(&fake_codex, "#!/bin/sh\nprintf '%s\\n' \"$*\" > \"$CAPTURE\"\n")?;
fs::write(&fake_claude, "#!/bin/sh\nprintf '%s\\n' \"$*\" > \"$CAPTURE\"\n")?;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p linux-conductor --test cli_sessions -- --nocapture`
Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add crates/cli/tests/cli_sessions.rs crates/core/src/workspace.rs
git commit -m "test(cli): cover Codex and Claude harness startup"
```
