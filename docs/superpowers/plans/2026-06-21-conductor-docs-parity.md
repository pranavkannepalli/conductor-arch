# Conductor Docs Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restyle the GTK app so the shell feels much closer to `conductor.build/docs` while keeping workspace tool panes denser and Linux-native.

**Architecture:** Keep the current GTK layout and state model. Extract the large inline CSS into a dedicated theme module, introduce a small set of visual tokens, then restyle shared shell primitives first and dense workspace panes second so the whole app reads as one system without a product rewrite.

**Tech Stack:** Rust, GTK4, libadwaita, inline CSS loaded through `CssProvider`, existing GTK page modules in `crates/gtk-app/src`

---

## File Map

- Create: `crates/gtk-app/src/theme.rs`
  - Own the app CSS string and the one entrypoint used by `main.rs` to load it.
- Modify: `crates/gtk-app/src/main.rs`
  - Remove the giant inline `APP_CSS`, load theme CSS from `theme.rs`, and keep view-preference class application intact.
- Modify: `crates/gtk-app/src/sidebar.rs`
  - Add any small structural classes needed for a calmer docs-style sidebar and workspace list.
- Modify: `crates/gtk-app/src/dashboard.rs`
  - Add minimal wrappers/classes needed so the dashboard can use the new shell system cleanly.
- Modify: `crates/gtk-app/src/projects.rs`
  - Tighten page hierarchy and panel grouping with the new shell classes.
- Modify: `crates/gtk-app/src/history.rs`
  - Align history framing and typography with the restyled shell.
- Modify: `crates/gtk-app/src/workspace_command_center.rs`
  - Keep structure, add small class hooks where needed, and improve top-level workspace framing.
- Modify: `crates/gtk-app/src/session_surface.rs`
  - Improve transcript/panel framing so it fits the new visual language.
- Modify: `crates/gtk-app/src/terminal.rs`
  - Keep dense tool panes dark, but align them to the new shell tokens and contrast model.

### Task 1: Extract Theme Ownership

**Files:**
- Create: `crates/gtk-app/src/theme.rs`
- Modify: `crates/gtk-app/src/main.rs`
- Test: `cargo test -p linux-conductor-gtk`

- [ ] **Step 1: Add a focused theme module**

Create `crates/gtk-app/src/theme.rs` with a single public CSS export and a small helper so `main.rs` stops owning a thousand-line style blob.

```rust
pub(crate) const APP_CSS: &str = r#"
/* theme tokens and component classes live here */
"#;

pub(crate) fn app_css() -> &'static str {
    APP_CSS
}
```

- [ ] **Step 2: Wire `main.rs` to use the theme module**

Update imports near the top of `crates/gtk-app/src/main.rs`:

```rust
mod theme;
```

Replace CSS loading:

```rust
css.load_from_data(theme::app_css());
```

Delete the old inline:

```rust
const APP_CSS: &str = r#"... "#;
```

- [ ] **Step 3: Run GTK tests to catch broken compile paths**

Run:

```bash
cargo test -p linux-conductor-gtk
```

Expected:

```text
test result: ok
```

- [ ] **Step 4: Commit**

```bash
git add crates/gtk-app/src/main.rs crates/gtk-app/src/theme.rs
git commit -m "refactor(gtk): extract app theme module"
```

### Task 2: Replace the Visual Token System

**Files:**
- Modify: `crates/gtk-app/src/theme.rs`
- Test: `cargo test -p linux-conductor-gtk`

- [ ] **Step 1: Write the new token block inside `theme.rs`**

Replace the old dark-default palette with a smaller shell/tool split:

```css
window {
    background-color: #f3f4ef;
    color: #111827;
}

.sidebar {
    background-color: #f7f7f2;
    border-right: 1px solid #d9dcd1;
}

.dashboard,
.history-view {
    background-color: #f3f4ef;
    color: #111827;
}

.workspace-card,
.command-panel,
.metric-card,
.detail-row {
    background-color: #fbfbf8;
    border: 1px solid #d9dcd1;
    border-radius: 12px;
}

.diff-view,
.checks-view,
.terminal-panel .history-view,
.terminal-transcript-dark {
    background-color: #12161a;
    color: #d8dee7;
}
```

- [ ] **Step 2: Normalize typography and spacing tokens**

Add consistent classes for hierarchy:

```css
.dashboard-title {
    color: #101828;
    font-size: 22px;
    font-weight: 700;
}

.section-title,
.sidebar-header,
.repo-section-header {
    color: #667085;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
}

.card-meta,
.workspace-meta,
.detail-label {
    color: #667085;
}
```

- [ ] **Step 3: Keep dense panes intentionally dark**

Preserve Linux utility contrast with a dedicated group instead of mixing many unrelated dark tones:

```css
.terminal-panel,
.session-tool-surface,
.checks-view,
.diff-view {
    background-color: #101418;
    border-color: #27303a;
    color: #d0d5dd;
}
```

- [ ] **Step 4: Run tests**

Run:

```bash
cargo test -p linux-conductor-gtk
```

Expected:

```text
test result: ok
```

- [ ] **Step 5: Commit**

```bash
git add crates/gtk-app/src/theme.rs
git commit -m "feat(gtk): define docs-inspired shell theme"
```

### Task 3: Restyle Shared Shell Primitives

**Files:**
- Modify: `crates/gtk-app/src/sidebar.rs`
- Modify: `crates/gtk-app/src/main.rs`
- Modify: `crates/gtk-app/src/theme.rs`
- Test: `cargo test -p linux-conductor-gtk`

- [ ] **Step 1: Add structural classes for the top nav group**

In `crates/gtk-app/src/sidebar.rs`, wrap the three top buttons:

```rust
let nav_group = GBox::new(Orientation::Vertical, 4);
nav_group.add_css_class("sidebar-nav-group");
nav_group.append(&dashboard_btn);
nav_group.append(&history_btn);
nav_group.append(&projects_btn);
sidebar_box.append(&nav_group);
```

Remove direct top-level appends for those buttons.

- [ ] **Step 2: Add classes for calmer workspace row anatomy**

In `build_workspace_row`:

```rust
text_box.add_css_class("workspace-row-text");
row_box.add_css_class("workspace-row-shell");
```

This gives CSS a stable hook to style row spacing without changing logic.

- [ ] **Step 3: Add a class to the header bar**

In `crates/gtk-app/src/main.rs`:

```rust
header.add_css_class("app-header");
toggle_btn.add_css_class("chrome-button");
refresh_btn.add_css_class("chrome-button");
palette_btn.add_css_class("chrome-button");
```

- [ ] **Step 4: Define the new shared shell classes in `theme.rs`**

Add:

```css
.sidebar-nav-group {
    padding: 8px 10px 6px 10px;
}

.nav-button,
.nav-button-active {
    margin: 0;
    padding: 10px 12px;
    border-radius: 10px;
    text-align: left;
}

.chrome-button {
    border-radius: 10px;
    background: transparent;
    border: 1px solid transparent;
}

.chrome-button:hover {
    background-color: #eceee6;
    border-color: #d9dcd1;
}
```

- [ ] **Step 5: Run tests**

Run:

```bash
cargo test -p linux-conductor-gtk
```

Expected:

```text
test result: ok
```

- [ ] **Step 6: Commit**

```bash
git add crates/gtk-app/src/sidebar.rs crates/gtk-app/src/main.rs crates/gtk-app/src/theme.rs
git commit -m "feat(gtk): restyle shared shell primitives"
```

### Task 4: Restyle Dashboard, Projects, and History

**Files:**
- Modify: `crates/gtk-app/src/dashboard.rs`
- Modify: `crates/gtk-app/src/projects.rs`
- Modify: `crates/gtk-app/src/history.rs`
- Modify: `crates/gtk-app/src/theme.rs`
- Test: `cargo test -p linux-conductor-gtk`

- [ ] **Step 1: Add shell container classes where missing**

In each page module, add calm shell framing hooks:

```rust
root.add_css_class("page-shell");
header.add_css_class("page-header");
```

Use the existing `dashboard-header` where already present, but also add the shared class:

```rust
header.add_css_class("page-header");
```

- [ ] **Step 2: Tighten dashboard card hierarchy**

Keep `dashboard.rs` structure but add a board class and stronger content split:

```rust
board.add_css_class("page-board");
card.add_css_class("shell-card");
```

- [ ] **Step 3: Make Projects and History read like docs pages**

Add class hooks in `projects.rs` and `history.rs`:

```rust
body.add_css_class("page-body");
list.add_css_class("shell-list");
```

- [ ] **Step 4: Style those shared classes in `theme.rs`**

Add:

```css
.page-shell {
    background-color: #f3f4ef;
}

.page-header {
    padding: 24px 30px 10px 30px;
    border-bottom: 1px solid #dde1d7;
}

.page-body,
.detail-body,
.page-board {
    padding: 24px 30px;
}

.shell-card,
.workspace-card {
    background-color: #fbfbf8;
    border: 1px solid #d9dcd1;
    border-radius: 14px;
}
```

- [ ] **Step 5: Run tests**

Run:

```bash
cargo test -p linux-conductor-gtk
```

Expected:

```text
test result: ok
```

- [ ] **Step 6: Commit**

```bash
git add crates/gtk-app/src/dashboard.rs crates/gtk-app/src/projects.rs crates/gtk-app/src/history.rs crates/gtk-app/src/theme.rs
git commit -m "feat(gtk): restyle top-level app pages"
```

### Task 5: Bring the Workspace Command Center into the Same System

**Files:**
- Modify: `crates/gtk-app/src/workspace_command_center.rs`
- Modify: `crates/gtk-app/src/session_surface.rs`
- Modify: `crates/gtk-app/src/theme.rs`
- Test: `cargo test -p linux-conductor-gtk`

- [ ] **Step 1: Add shared page-shell classes to the workspace root**

In `crates/gtk-app/src/workspace_command_center.rs`:

```rust
root.add_css_class("page-shell");
header.add_css_class("page-header");
body.add_css_class("page-body");
```

- [ ] **Step 2: Add clear hooks for top-level workspace framing**

Add:

```rust
strip.add_css_class("workspace-summary-strip");
```

Where session/transcript containers are created, add:

```rust
chat_box.add_css_class("session-tool-surface");
terminal_box.add_css_class("session-tool-surface");
```

- [ ] **Step 3: Align the session panel to the shell/tool split**

In `crates/gtk-app/src/session_surface.rs`:

```rust
root.add_css_class("session-surface");
transcript.add_css_class("session-transcript");
```

- [ ] **Step 4: Style workspace shell vs tool surfaces in `theme.rs`**

Add:

```css
.workspace-summary-strip {
    margin-bottom: 8px;
}

.session-surface,
.session-tool-surface {
    border-radius: 14px;
}

.session-transcript,
.session-tool-surface .history-view {
    background-color: #101418;
    color: #d0d5dd;
    border: 1px solid #27303a;
}
```

- [ ] **Step 5: Run tests**

Run:

```bash
cargo test -p linux-conductor-gtk
```

Expected:

```text
test result: ok
```

- [ ] **Step 6: Commit**

```bash
git add crates/gtk-app/src/workspace_command_center.rs crates/gtk-app/src/session_surface.rs crates/gtk-app/src/theme.rs
git commit -m "feat(gtk): align workspace command center with shell theme"
```

### Task 6: Polish Terminal, Checks, and Diff Surfaces

**Files:**
- Modify: `crates/gtk-app/src/terminal.rs`
- Modify: `crates/gtk-app/src/theme.rs`
- Test: `cargo test -p linux-conductor-gtk`

- [ ] **Step 1: Add stable classes for dense tool panes**

In `crates/gtk-app/src/terminal.rs`, add:

```rust
root.add_css_class("session-tool-surface");
transcript.add_css_class("terminal-transcript-dark");
```

Keep existing behavior intact.

- [ ] **Step 2: Unify dark-pane styling in `theme.rs`**

Add or consolidate:

```css
.terminal-panel,
.terminal-transcript-dark,
.checks-view,
.diff-view,
.status-container {
    background-color: #101418;
    color: #d0d5dd;
    border: 1px solid #27303a;
    border-radius: 14px;
}
```

- [ ] **Step 3: Improve utility control readability**

Update related classes in `theme.rs`:

```css
.panel-switcher button:checked {
    background-color: #eef2e8;
    color: #101828;
}

.composer-bar entry,
.sidebar-search {
    background-color: #ffffff;
    color: #101828;
    border: 1px solid #d0d5dd;
}
```

- [ ] **Step 4: Run tests**

Run:

```bash
cargo test -p linux-conductor-gtk
```

Expected:

```text
test result: ok
```

- [ ] **Step 5: Commit**

```bash
git add crates/gtk-app/src/terminal.rs crates/gtk-app/src/theme.rs
git commit -m "feat(gtk): polish dense workspace tool panes"
```

### Task 7: Verification and Cleanup

**Files:**
- Modify: `crates/gtk-app/src/theme.rs` (only if needed after visual verification)
- Test: `cargo fmt --all -- --check`
- Test: `cargo test -p linux-conductor-gtk`
- Test: `cargo build -p linux-conductor-gtk`

- [ ] **Step 1: Run formatting check**

Run:

```bash
cargo fmt --all -- --check
```

Expected:

```text
no output and exit code 0
```

- [ ] **Step 2: Run GTK tests**

Run:

```bash
cargo test -p linux-conductor-gtk
```

Expected:

```text
test result: ok
```

- [ ] **Step 3: Build the GTK app**

Run:

```bash
cargo build -p linux-conductor-gtk
```

Expected:

```text
Finished
```

- [ ] **Step 4: Launch for manual review**

Run:

```bash
cargo run -p linux-conductor-gtk
```

Review:

```text
Dashboard, sidebar, projects, history, workspace, terminal, checks
```

- [ ] **Step 5: Final commit**

```bash
git add crates/gtk-app/src/theme.rs crates/gtk-app/src/main.rs crates/gtk-app/src/sidebar.rs crates/gtk-app/src/dashboard.rs crates/gtk-app/src/projects.rs crates/gtk-app/src/history.rs crates/gtk-app/src/workspace_command_center.rs crates/gtk-app/src/session_surface.rs crates/gtk-app/src/terminal.rs
git commit -m "feat(gtk): bring app shell closer to conductor docs"
```

## Self-Review

Spec coverage:

- shell parity: Tasks 2-4
- workspace shell vs dense tools split: Tasks 5-6
- no rewrite / small diff: all tasks keep layout and logic intact
- verification: Task 7

Placeholder scan:

- no `TODO` or `TBD`
- every task includes exact files and commands

Type consistency:

- shared class names are defined once and reused consistently:
  `page-shell`, `page-header`, `page-body`, `shell-card`,
  `session-tool-surface`, `session-transcript`, `terminal-transcript-dark`,
  `workspace-summary-strip`, `chrome-button`

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-06-21-conductor-docs-parity.md`.

Execution choice already supplied by user intent: continue with inline execution in this session.
