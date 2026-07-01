# Workspace Open Chat Loading And Archcar Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make workspace open render immediately with honest inline Codex loading/error state, and make archcar clear stale managed sessions on startup so destroyed sidecars cannot poison later workspace opens.

**Architecture:** Keep workspace navigation unchanged and localize the new UX to `session_surface.rs` with a small Codex startup state model and inline status card renderer. Add a server-boot reconciliation step in archcar that sweeps persisted managed session records before accepting requests, then continue using the existing archcar event/response flow for readiness and error transitions.

**Tech Stack:** Rust, GTK4/libadwaita, tracing, SQLite-backed `WorkspaceStore`, archcar Unix socket server/client

---

## File Map

- Modify: `crates/gtk-app/src/session_surface.rs`
  - Add local Codex startup UI state helpers, inline status card rendering, and event/response-driven state transitions.
- Modify: `crates/core/src/archcar/server.rs`
  - Add archcar boot-time reconciliation for stale managed sessions and focused tests for the sweep behavior.
- Modify: `crates/archcar/src/main.rs`
  - Run the new reconciliation step before `serve()`.
- Possibly modify: `crates/core/src/workspace.rs`
  - Only if the existing store API needs a small helper for marking stale session records exited/killed cleanly.
- Test through existing unit targets in `linux-archductor-core`; add GTK-local state transition tests in `session_surface.rs`.

### Task 1: Add Archcar Boot-Time Stale Session Sweep

**Files:**
- Modify: `crates/core/src/archcar/server.rs`
- Modify: `crates/archcar/src/main.rs`
- Possibly modify: `crates/core/src/workspace.rs`
- Test: `cargo test -p linux-archductor-core archcar::server -- --nocapture`

- [ ] **Step 1: Write the failing reconciliation tests**

Add tests in `crates/core/src/archcar/server.rs` for:

```rust
#[test]
fn reconcile_startup_marks_stale_codex_sessions_exited() {
    // create temp db + workspace + codex process record marked running
    // run reconcile function
    // reload process record
    // assert status is Exited and not Running
}

#[test]
fn reconcile_startup_leaves_non_managed_sessions_untouched() {
    // create temp db + shell process record marked running
    // run reconcile function
    // assert record is still Running
}
```

- [ ] **Step 2: Run server tests to verify the new cases fail**

Run: `cargo test -p linux-archductor-core archcar::server -- --nocapture`

Expected:
- New reconciliation tests fail because the sweep does not exist yet.
- Existing `archcar::server` tests still compile and run.

- [ ] **Step 3: Implement the minimal reconciliation path**

Implement a small startup helper in `crates/core/src/archcar/server.rs` that:

```rust
pub fn reconcile_managed_sessions_on_startup(paths: &AppPaths) -> Result<()> {
    let store = WorkspaceStore::open(&paths.database_path)?;
    for workspace in store.list_workspaces()? {
        for record in store.list_sessions(&workspace.name)? {
            if record.status != ProcessStatus::Running {
                continue;
            }
            if !session_kind_matches_command(&record.command, SessionKind::Codex) {
                continue;
            }
            store.mark_session_process_exited(record.id, None)?;
        }
    }
    Ok(())
}
```

If `WorkspaceStore` lacks a convenient workspace iterator, add the smallest store helper needed rather than duplicating SQL in the server.

- [ ] **Step 4: Call reconciliation during archcar boot before serving**

Update `crates/archcar/src/main.rs` so startup runs:

```rust
let paths = AppPaths::from_env();
let _log_guard = init_logger(&paths)?;
linux_archductor_core::archcar::server::reconcile_managed_sessions_on_startup(&paths)?;
let server = ArchcarServer::bind(paths)?;
server.serve()
```

- [ ] **Step 5: Re-run the server test target and verify it passes**

Run: `cargo test -p linux-archductor-core archcar::server -- --nocapture`

Expected:
- All `archcar::server` tests pass.
- Reconciliation tests prove stale managed sessions are cleared at startup.

- [ ] **Step 6: Commit the reconciliation slice**

```bash
git add crates/core/src/archcar/server.rs crates/archcar/src/main.rs crates/core/src/workspace.rs
git commit -m "fix(archcar): sweep stale managed sessions on startup"
```

### Task 2: Add Local Codex Startup State Model For Chat Panel

**Files:**
- Modify: `crates/gtk-app/src/session_surface.rs`
- Test: `cargo test -p linux-archductor-gtk session_surface -- --nocapture`

- [ ] **Step 1: Write failing local state tests**

Add focused tests near the existing `session_surface.rs` test module for helpers like:

```rust
#[test]
fn codex_startup_state_defaults_to_loading_without_ready_session() {
    let state = derive_codex_startup_state(false, false, None);
    assert_eq!(state, CodexStartupState::Loading {
        message: "Starting Codex...".to_owned(),
    });
}

#[test]
fn codex_startup_state_switches_to_error_when_archcar_reports_failure() {
    let state = codex_startup_state_from_error("spawn failed");
    assert_eq!(state, CodexStartupState::Error {
        message: "spawn failed".to_owned(),
    });
}
```

- [ ] **Step 2: Run the GTK session surface tests to verify they fail**

Run: `cargo test -p linux-archductor-gtk session_surface -- --nocapture`

Expected:
- New startup-state tests fail because the enum/helpers are not defined yet.

- [ ] **Step 3: Implement the minimal startup state helpers**

Add a small local model in `crates/gtk-app/src/session_surface.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
enum CodexStartupState {
    Idle,
    Loading { message: String },
    Error { message: String },
    Ready,
}

fn default_codex_loading_state() -> CodexStartupState {
    CodexStartupState::Loading {
        message: "Starting Codex...".to_owned(),
    }
}
```

Also add tiny pure helpers for:

- deriving initial state from ready/error inputs
- switching to `Ready`
- switching to `Error`

- [ ] **Step 4: Re-run the GTK session surface tests and verify the helper tests pass**

Run: `cargo test -p linux-archductor-gtk session_surface -- --nocapture`

Expected:
- Startup-state helper tests pass.
- No existing session surface tests regress.

- [ ] **Step 5: Commit the local state-model slice**

```bash
git add crates/gtk-app/src/session_surface.rs
git commit -m "feat(chat): add codex startup state model"
```

### Task 3: Render Inline Loading/Error Card In Chat Panel

**Files:**
- Modify: `crates/gtk-app/src/session_surface.rs`
- Test: `cargo test -p linux-archductor-gtk session_surface -- --nocapture`

- [ ] **Step 1: Write failing rendering/state-selection tests**

Add tests for a small helper that decides whether the chat panel should render a startup card:

```rust
#[test]
fn startup_card_is_rendered_for_loading_state() {
    assert!(should_render_codex_startup_card(&CodexStartupState::Loading {
        message: "Starting Codex...".to_owned(),
    }));
}

#[test]
fn startup_card_is_not_rendered_for_ready_state() {
    assert!(!should_render_codex_startup_card(&CodexStartupState::Ready));
}
```

- [ ] **Step 2: Run GTK tests to verify the rendering-helper cases fail**

Run: `cargo test -p linux-archductor-gtk session_surface -- --nocapture`

Expected:
- New startup-card tests fail because the helper does not exist yet.

- [ ] **Step 3: Implement the minimal inline card renderer**

Add a focused helper in `crates/gtk-app/src/session_surface.rs`:

```rust
fn codex_startup_status_widget(state: &CodexStartupState) -> Option<Widget> {
    match state {
        CodexStartupState::Loading { message } => Some(build_loading_card(message).upcast()),
        CodexStartupState::Error { message } => Some(build_error_card(message).upcast()),
        CodexStartupState::Idle | CodexStartupState::Ready => None,
    }
}
```

Render it above transcript content in the existing messages column without blocking transcript/history rendering.

- [ ] **Step 4: Re-run GTK tests and verify the helper/render selection cases pass**

Run: `cargo test -p linux-archductor-gtk session_surface -- --nocapture`

Expected:
- Startup-card helper tests pass.
- Existing tests remain green.

- [ ] **Step 5: Commit the inline card renderer slice**

```bash
git add crates/gtk-app/src/session_surface.rs
git commit -m "feat(chat): render inline codex startup status card"
```

### Task 4: Wire Archcar Events And Responses Into Startup UI State

**Files:**
- Modify: `crates/gtk-app/src/session_surface.rs`
- Test: `cargo test -p linux-archductor-gtk session_surface -- --nocapture`

- [ ] **Step 1: Write failing transition tests for events and responses**

Add pure tests around a transition helper such as:

```rust
#[test]
fn session_ready_event_clears_loading_state() {
    let next = reduce_codex_startup_state(
        CodexStartupState::Loading {
            message: "Starting Codex...".to_owned(),
        },
        CodexStartupSignal::Ready,
    );
    assert_eq!(next, CodexStartupState::Ready);
}

#[test]
fn session_error_event_surfaces_error_message() {
    let next = reduce_codex_startup_state(
        default_codex_loading_state(),
        CodexStartupSignal::Error("spawn failed".to_owned()),
    );
    assert_eq!(next, CodexStartupState::Error {
        message: "spawn failed".to_owned(),
    });
}
```

- [ ] **Step 2: Run GTK tests to verify the transition tests fail**

Run: `cargo test -p linux-archductor-gtk session_surface -- --nocapture`

Expected:
- New reducer tests fail because the signal/reducer logic is not implemented yet.

- [ ] **Step 3: Implement the minimal reducer and integrate it into the poll loop**

Add a local signal/reducer layer and call it from:

- initial panel setup
- `handle_archcar_event`
- `handle_archcar_response`
- status-probe reconciliation path

Pseudo-shape:

```rust
enum CodexStartupSignal {
    Begin,
    Ready,
    Error(String),
}

fn reduce_codex_startup_state(
    current: CodexStartupState,
    signal: CodexStartupSignal,
) -> CodexStartupState {
    match signal {
        CodexStartupSignal::Begin => default_codex_loading_state(),
        CodexStartupSignal::Ready => CodexStartupState::Ready,
        CodexStartupSignal::Error(message) => CodexStartupState::Error { message },
    }
}
```

- [ ] **Step 4: Re-run GTK tests and verify the transition tests pass**

Run: `cargo test -p linux-archductor-gtk session_surface -- --nocapture`

Expected:
- New reducer tests pass.
- Existing session surface tests remain green.

- [ ] **Step 5: Commit the event/response integration slice**

```bash
git add crates/gtk-app/src/session_surface.rs
git commit -m "fix(chat): surface archcar startup state in panel"
```

### Task 5: Full Verification For The Recovery And Loading Story

**Files:**
- Verify only

- [ ] **Step 1: Run focused core verification**

Run: `cargo test -p linux-archductor-core archcar::server -- --nocapture`

Expected:
- All server tests pass, including stale-session sweep coverage.

- [ ] **Step 2: Run focused GTK verification**

Run: `cargo test -p linux-archductor-gtk session_surface -- --nocapture`

Expected:
- All session surface tests pass, including startup loading/error state coverage.

- [ ] **Step 3: Run formatting**

Run: `cargo fmt --all`

Expected:
- Exit code 0 with no formatting errors.

- [ ] **Step 4: Manual smoke-check the full story**

Run the GTK app, then verify:

```text
1. Select a workspace.
2. Confirm the workspace view opens immediately.
3. Confirm the chat panel shows an inline spinner card while Codex starts.
4. Confirm existing transcript stays visible if present.
5. Kill archcar, restart the app/sidecar, and confirm stale managed session records do not block a fresh startup attempt.
6. Force a startup failure and confirm the inline error card surfaces the failure message.
```

- [ ] **Step 5: Commit the verified final slice**

```bash
git add crates/archcar/src/main.rs crates/core/src/archcar/server.rs crates/core/src/workspace.rs crates/gtk-app/src/session_surface.rs
git commit -m "fix(chat): make workspace open loading honest and recover archcar state"
```
