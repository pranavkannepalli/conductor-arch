# GitHub Integrations Everywhere Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build full GitHub integration coverage across the GTK app so repository onboarding, workspace creation, diff review, PR review, checks, merge, and status visibility all work directly from the app.

**Architecture:** Reuse the existing `WorkspaceStore` GitHub primitives in `crates/core/src/workspace.rs` and finish the app surface around them instead of inventing a new GitHub layer. Keep the diff small by adding a thin GTK view-model layer inside `workspace_command_center.rs`/`projects.rs`, only extending core where the GTK app is blocked on missing structured data instead of string output.

**Tech Stack:** Rust, GTK4/libadwaita, rusqlite, local `gh` CLI, existing `WorkspaceStore` core APIs, inline Rust unit tests in crate source files.

---

## File Map

- Modify: `crates/core/src/workspace.rs`
  - Keep GitHub operations centralized here.
  - Add any missing structured read helpers needed by GTK.
- Modify: `crates/gtk-app/src/projects.rs`
  - Improve GitHub issue/PR source pickers and readiness feedback in workspace creation.
- Modify: `crates/gtk-app/src/workspace_command_center.rs`
  - Main implementation area for Changes, Checks, and Review tabs.
- Modify: `crates/gtk-app/src/dashboard.rs`
  - Surface GitHub attention state in cards.
- Modify: `crates/gtk-app/src/sidebar.rs`
  - Add GitHub-aware workspace badges.
- Modify: `crates/gtk-app/src/theme.rs`
  - Add only the CSS classes needed for new GitHub states.
- Modify: `README.md`
  - Update product claims to match shipped GTK behavior.
- Modify: `progress.md`
  - Record the completed phase and verification.

## Scope Notes

- Do not add a separate GitHub service abstraction. `WorkspaceStore` already owns `gh` access.
- Do not replace the existing diff viewer. Layer GitHub actions onto the current unified diff/file-summary flow.
- Do not add a background sync daemon. Refresh remains user-driven plus existing page refresh hooks.

### Task 1: Add Structured GitHub Read Models For GTK

**Files:**
- Modify: `crates/core/src/workspace.rs:2093-2335`
- Test: `crates/core/src/workspace.rs`

- [ ] **Step 1: Write the failing core tests for structured GitHub panel data**

```rust
#[test]
fn pull_request_panel_state_collects_pr_readiness_and_review_text() {
    let temp = tempdir().unwrap();
    let repo_path = init_repo(temp.path().join("demo"));
    let store = test_store(temp.path().join("db.sqlite"));
    let workspace = store
        .create(CreateWorkspace {
            repository_name: add_test_repository(&store, &repo_path),
            name: "berlin".to_owned(),
            branch: "lc/berlin".to_owned(),
            base_ref: None,
        })
        .unwrap();
    store
        .record_pull_request(workspace.id, "https://github.com/example/demo/pull/42")
        .unwrap();

    let panel = store.pull_request_panel_state("berlin").unwrap();

    assert_eq!(panel.pull_request.unwrap().number, 42);
    assert!(panel.readiness_text.contains("PR readiness for workspace berlin."));
    assert!(panel.review_text.is_some());
}

#[test]
fn github_source_choices_parse_number_title_and_state() {
    let raw = "\
12\tOPEN\tFix auth loop\n\
18\tDRAFT\tShip checks ui\n";

    let choices = parse_github_numbered_stateful_choices(raw);

    assert_eq!(choices[0].number, 12);
    assert_eq!(choices[0].state, "OPEN");
    assert_eq!(choices[0].title, "Fix auth loop");
}
```

- [ ] **Step 2: Run the focused core test to verify it fails**

Run: `cargo test -p linux-archductor-core pull_request_panel_state_collects_pr_readiness_and_review_text -- --nocapture`

Expected: FAIL with missing `pull_request_panel_state` / parser symbol.

- [ ] **Step 3: Add the minimal structured core helpers**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubNumberedChoice {
    pub number: u64,
    pub state: String,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct PullRequestPanelState {
    pub pull_request: Option<PullRequest>,
    pub readiness: Option<PullRequestReadiness>,
    pub readiness_text: String,
    pub review_text: Option<String>,
}

pub fn parse_github_numbered_stateful_choices(raw: &str) -> Vec<GitHubNumberedChoice> {
    raw.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let number = parts.next()?.trim().parse().ok()?;
            let state = parts.next()?.trim().to_owned();
            let title = parts.next()?.trim().to_owned();
            Some(GitHubNumberedChoice { number, state, title })
        })
        .collect()
}

pub fn pull_request_panel_state(&self, name: &str) -> Result<PullRequestPanelState> {
    let pull_request = self.pull_request(name)?;
    let readiness = pull_request
        .as_ref()
        .map(|_| self.pull_request_readiness(name))
        .transpose()?;
    let readiness_text = pull_request
        .as_ref()
        .map(|_| self.pull_request_readiness_text(name))
        .transpose()?
        .unwrap_or_else(|| "No pull request yet.".to_owned());
    let review_text = pull_request
        .as_ref()
        .map(|_| self.pull_request_review_state(name))
        .transpose()?;
    Ok(PullRequestPanelState {
        pull_request,
        readiness,
        readiness_text,
        review_text,
    })
}
```

- [ ] **Step 4: Run the focused tests to verify they pass**

Run: `cargo test -p linux-archductor-core pull_request_panel_state_collects_pr_readiness_and_review_text github_source_choices_parse_number_title_and_state -- --nocapture`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/workspace.rs
git commit -m "feat(core): add structured github panel state helpers"
```

### Task 2: Finish GitHub Workspace Source Creation UX

**Files:**
- Modify: `crates/gtk-app/src/projects.rs:840-1215`
- Test: `crates/gtk-app/src/projects.rs`

- [ ] **Step 1: Write the failing GTK tests for richer GitHub source labels and previews**

```rust
#[test]
fn parse_github_numbered_stateful_choices_reads_number_state_and_title() {
    let choices = parse_github_numbered_stateful_choices(
        "12\tOPEN\tFix auth loop\n18\tDRAFT\tShip checks ui\n",
    );
    assert_eq!(choices[0].label, "#12 · OPEN · Fix auth loop");
    assert_eq!(choices[1].label, "#18 · DRAFT · Ship checks ui");
}

#[test]
fn workspace_source_preview_mentions_github_stateful_selection() {
    let preview = github_source_preview_line("github_pr", Some("#18 · DRAFT · Ship checks ui"));
    assert_eq!(preview, "Source: PR #18 · DRAFT · Ship checks ui");
}
```

- [ ] **Step 2: Run the focused GTK tests to verify they fail**

Run: `cargo test -p linux-archductor-gtk parse_github_numbered_stateful_choices_reads_number_state_and_title workspace_source_preview_mentions_github_stateful_selection -- --nocapture`

Expected: FAIL with missing helper functions.

- [ ] **Step 3: Replace number-only GitHub picker rows with stateful labels and stronger readiness copy**

```rust
struct GithubNumberedChoice {
    value: String,
    label: String,
}

fn parse_github_numbered_stateful_choices(raw: &str) -> Vec<GithubNumberedChoice> {
    linux_archductor_core::workspace::parse_github_numbered_stateful_choices(raw)
        .into_iter()
        .map(|choice| GithubNumberedChoice {
            value: format!("#{}", choice.number),
            label: format!("#{} · {} · {}", choice.number, choice.state, choice.title),
        })
        .collect()
}

fn github_source_preview_line(source: &str, selected: Option<&str>) -> String {
    match (source, selected) {
        ("github_issue", Some(label)) => format!("Source: Issue {label}"),
        ("github_pr", Some(label)) => format!("Source: PR {label}"),
        ("github_issue", None) => "Source: Issue missing".to_owned(),
        ("github_pr", None) => "Source: PR missing".to_owned(),
        _ => "Source: none".to_owned(),
    }
}
```

Use `gh issue list --json number,state,title --template ...` and `gh pr list --json number,state,title,isDraft --template ...` instead of number/title-only shell formatting.

- [ ] **Step 4: Run GTK source-form tests to verify they pass**

Run: `cargo test -p linux-archductor-gtk projects::tests -- --nocapture`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/gtk-app/src/projects.rs
git commit -m "feat(gtk): improve github workspace source selection"
```

### Task 3: Make Dashboard And Sidebar GitHub-Aware

**Files:**
- Modify: `crates/gtk-app/src/dashboard.rs:72-214`
- Modify: `crates/gtk-app/src/sidebar.rs:554-577`
- Modify: `crates/gtk-app/src/theme.rs`
- Test: `crates/gtk-app/src/sidebar.rs`

- [ ] **Step 1: Write the failing sidebar/dashboard badge tests**

```rust
#[test]
fn workspace_badge_prefers_failed_github_state_over_pr_number() {
    let badge = workspace_badge_text(
        Some(42),
        Some("checks failed"),
        false,
        0,
        0,
        0,
    );
    assert_eq!(badge.as_deref(), Some("checks failed"));
}

#[test]
fn dashboard_meta_includes_pr_attention_state() {
    let meta = dashboard_pr_meta("demo", 42, "checks failed");
    assert_eq!(meta, "demo · PR #42 · checks failed");
}
```

- [ ] **Step 2: Run the focused GTK tests to verify they fail**

Run: `cargo test -p linux-archductor-gtk workspace_badge_prefers_failed_github_state_over_pr_number dashboard_meta_includes_pr_attention_state -- --nocapture`

Expected: FAIL with missing helpers.

- [ ] **Step 3: Add small badge/meta helpers and thread them into existing cards**

```rust
fn workspace_badge_text(
    pr_number: Option<i64>,
    pr_attention: Option<&str>,
    run_active: bool,
    active_sessions: usize,
    ahead: usize,
    open_todos: usize,
) -> Option<String> {
    if let Some(attention) = pr_attention.filter(|value| !value.is_empty()) {
        return Some(attention.to_owned());
    }
    if let Some(pr) = pr_number {
        return Some(format!("PR #{pr}"));
    }
    if run_active {
        return Some("preview".to_owned());
    }
    if active_sessions > 0 {
        return Some("active".to_owned());
    }
    if ahead > 0 {
        return Some(format!("+{ahead}"));
    }
    if open_todos > 0 {
        return Some(format!("{open_todos} todo"));
    }
    None
}
```

Use `pull_request_state_summary(...)` output when a PR exists so the dashboard/sidebar can show `ready to merge`, `checks failed`, or `merged` instead of always plain `PR #n`.

- [ ] **Step 4: Run the GTK tests to verify they pass**

Run: `cargo test -p linux-archductor-gtk sidebar -- --nocapture`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/gtk-app/src/dashboard.rs crates/gtk-app/src/sidebar.rs crates/gtk-app/src/theme.rs
git commit -m "feat(gtk): surface github status in workspace lists"
```

### Task 4: Build Real PR Controls In The Checks Tab

**Files:**
- Modify: `crates/gtk-app/src/workspace_command_center.rs:3209-3487`
- Test: `crates/gtk-app/src/workspace_command_center.rs`

- [ ] **Step 1: Write the failing checks-tab tests for direct GitHub actions**

```rust
#[test]
fn pull_request_create_feedback_extracts_created_url() {
    let message = pull_request_create_feedback(Ok(
        "Creating pull request for lc/berlin into main\nhttps://github.com/example/demo/pull/42\n"
            .to_owned(),
    ));
    assert_eq!(message, "Created PR: https://github.com/example/demo/pull/42");
}

#[test]
fn pull_request_actions_include_summary_and_review_controls_for_open_prs() {
    let actions = pull_request_action_labels(PullRequestStateKind::Open);
    assert_eq!(actions, vec!["PR summary", "Reviews", "Refresh"]);
}
```

- [ ] **Step 2: Run the focused GTK tests to verify they fail**

Run: `cargo test -p linux-archductor-gtk pull_request_actions_include_summary_and_review_controls_for_open_prs -- --nocapture`

Expected: FAIL with missing helper function / mismatched action set.

- [ ] **Step 3: Replace the current minimal checks header with a full PR action strip**

```rust
fn pull_request_action_labels(state: PullRequestStateKind) -> Vec<&'static str> {
    match state {
        PullRequestStateKind::Open => vec!["PR summary", "Reviews", "Refresh"],
        PullRequestStateKind::Ready => vec!["Merge", "PR summary", "Refresh"],
        PullRequestStateKind::Failed => vec!["Fix checks", "PR summary", "Refresh"],
        PullRequestStateKind::Merged => vec!["Continue", "Archive"],
    }
}
```

In `workspace_checks_panel(...)`:

```rust
let create_btn = text_button("Create PR");
let title_entry = Entry::new();
title_entry.set_placeholder_text(Some("PR title"));
let draft_check = CheckButton::with_label("Draft");
let body_view = TextView::new();
body_view.buffer().set_text(
    &WorkspaceStore::open(db_path.to_path_buf())
        .and_then(|store| store.read_context_brief(name))
        .ok()
        .flatten()
        .unwrap_or_default(),
);
```

Wire buttons to:

- `store.push_branch(...)`
- `store.create_pull_request(...)`
- `store.refresh_pull_request_state(...)`
- `store.pull_request_readiness_agent_prompt(...)`
- `store.pull_request_review_agent_prompt(...)`
- `store.pull_request_checks_agent_prompt(...)`
- `store.merge_and_maybe_archive_pull_request(...)`

Render three sections below the header:

- `Checks Summary` from `workspace_checks_text(...)`
- `PR Readiness` from `pull_request_panel_state(...).readiness_text`
- `PR Comments / Reviews` from `pull_request_panel_state(...).review_text`

- [ ] **Step 4: Run the GTK checks-panel tests to verify they pass**

Run: `cargo test -p linux-archductor-gtk workspace_command_center::tests -- --nocapture`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/gtk-app/src/workspace_command_center.rs
git commit -m "feat(gtk): expand github controls in checks tab"
```

### Task 5: Add GitHub Review Thread Sync To The Review Tab

**Files:**
- Modify: `crates/gtk-app/src/workspace_command_center.rs:3883-4094`
- Test: `crates/gtk-app/src/workspace_command_center.rs`

- [ ] **Step 1: Write the failing review-thread tests**

```rust
#[test]
fn review_thread_action_feedback_reports_thread_location() {
    let thread = PullRequestReviewThread {
        id: Some("PRRT_fake".to_owned()),
        resolved: true,
        path: Some("src/lib.rs".to_owned()),
        line: Some(42),
        start_line: None,
        comments: Vec::new(),
    };
    let message = pull_request_review_thread_action_feedback("Resolved", Ok(thread));
    assert_eq!(message, "Resolved review thread PRRT_fake: resolved at src/lib.rs:42.");
}

#[test]
fn review_tab_sections_include_github_threads_before_local_comments() {
    let sections = review_tab_section_titles(true);
    assert_eq!(sections, vec!["GitHub review threads", "Local review comments"]);
}
```

- [ ] **Step 2: Run the focused GTK tests to verify they fail**

Run: `cargo test -p linux-archductor-gtk review_thread_action_feedback_reports_thread_location review_tab_sections_include_github_threads_before_local_comments -- --nocapture`

Expected: FAIL with missing helper.

- [ ] **Step 3: Split the review tab into GitHub threads and local comments**

```rust
fn review_tab_section_titles(has_threads: bool) -> Vec<&'static str> {
    if has_threads {
        vec!["GitHub review threads", "Local review comments"]
    } else {
        vec!["Local review comments"]
    }
}
```

Add a GitHub thread list above local comments:

```rust
if let Ok(readiness) = WorkspaceStore::open(db_path.to_path_buf())
    .and_then(|store| store.pull_request_readiness(name))
{
    for thread in readiness.review_threads {
        let title = format_review_thread_row(&thread);
        let resolve_btn = if thread.resolved { "Reopen" } else { "Resolve" };
        // call set_pull_request_review_thread_resolution(...)
    }
}
```

Use one formatter for rows:

```rust
fn format_review_thread_row(thread: &PullRequestReviewThread) -> String {
    let state = if thread.resolved { "resolved" } else { "open" };
    let location = match (thread.path.as_deref(), thread.line) {
        (Some(path), Some(line)) => format!("{path}:{line}"),
        (Some(path), None) => path.to_owned(),
        _ => "unknown location".to_owned(),
    };
    format!(
        "{} · {} · {}",
        thread.id.as_deref().unwrap_or("thread"),
        state,
        location
    )
}
```

Keep existing local add/resolve review comment form intact below the GitHub area.

- [ ] **Step 4: Run the GTK review-tab tests to verify they pass**

Run: `cargo test -p linux-archductor-gtk workspace_command_center::tests -- --nocapture`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/gtk-app/src/workspace_command_center.rs
git commit -m "feat(gtk): add github review thread controls"
```

### Task 6: Add GitHub Actions To The Changes Tab

**Files:**
- Modify: `crates/gtk-app/src/workspace_command_center.rs:2513-3208`
- Test: `crates/gtk-app/src/workspace_command_center.rs`

- [ ] **Step 1: Write the failing changes-tab tests for file-level GitHub actions**

```rust
#[test]
fn diff_summary_label_shows_add_delete_counts() {
    let summary = DiffFileSummary {
        path: "src/lib.rs".to_owned(),
        additions: Some(12),
        deletions: Some(3),
    };
    assert_eq!(diff_summary_label(&summary), "src/lib.rs +12 -3");
}

#[test]
fn file_inline_comments_text_filters_to_selected_path() {
    let comments = vec![ReviewComment {
        id: 7,
        workspace_id: 1,
        file_path: "src/lib.rs".to_owned(),
        line_number: Some(42),
        body: "Fix this".to_owned(),
        status: "open".to_owned(),
        source: "local".to_owned(),
        created_at: "now".to_owned(),
        updated_at: "now".to_owned(),
    }];
    let text = file_inline_comments_text(&comments, "src/lib.rs");
    assert!(text.contains("#7 [open] src/lib.rs:42 - Fix this"));
}
```

- [ ] **Step 2: Run the focused GTK tests to verify they fail or cover the old behavior**

Run: `cargo test -p linux-archductor-gtk diff_summary_label_shows_add_delete_counts file_inline_comments_text_filters_to_selected_path -- --nocapture`

Expected: PASS or FAIL; if PASS, keep them and add one new failing test below:

```rust
#[test]
fn github_diff_action_labels_include_stage_buttons() {
    assert_eq!(
        diff_action_labels(true),
        vec!["Stage file diff", "Stage file comments", "Copy path"]
    );
}
```

- [ ] **Step 3: Add file-level GitHub staging controls beside the diff preview**

```rust
fn diff_action_labels(has_pr: bool) -> Vec<&'static str> {
    if has_pr {
        vec!["Stage file diff", "Stage file comments", "Copy path"]
    } else {
        vec!["Stage file diff", "Copy path"]
    }
}
```

Inside the selected-file panel:

- `Stage file diff` loads `workspace_diff_text_for_path(...)` into `app_state.set_staged_review_prompt(...)`
- `Stage file comments` loads `workspace_file_comments_text(...)`
- `Copy path` just mirrors the selected path into a small feedback label

Keep the existing unified diff renderer. Do not build inline review widgets yet.

- [ ] **Step 4: Run the GTK changes-tab tests to verify they pass**

Run: `cargo test -p linux-archductor-gtk workspace_command_center::tests -- --nocapture`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/gtk-app/src/workspace_command_center.rs
git commit -m "feat(gtk): connect changes tab to github review flow"
```

### Task 7: Polish Theme, Copy, And Product Docs

**Files:**
- Modify: `crates/gtk-app/src/theme.rs`
- Modify: `README.md:16-33`
- Modify: `progress.md`

- [ ] **Step 1: Add the failing lightweight assertions for new CSS class names if the file already has theme tests**

```rust
#[test]
fn github_status_classes_are_listed_in_theme_source() {
    let css = app_theme_css();
    assert!(css.contains(".ws-pr-status-ready"));
    assert!(css.contains(".ws-pr-status-failed"));
    assert!(css.contains(".workspace-badge-attention"));
}
```

- [ ] **Step 2: Run the smallest relevant test/build command**

Run: `cargo test -p linux-archductor-gtk github_status_classes_are_listed_in_theme_source -- --nocapture`

Expected: FAIL if helper is missing, otherwise skip and move to the next step.

- [ ] **Step 3: Add the minimal CSS and doc updates**

```css
.workspace-badge-attention { color: @error_color; }
.workspace-badge-ready { color: @success_color; }
.ws-pr-status-ready { color: @success_color; }
.ws-pr-status-failed { color: @error_color; }
.ws-pr-status-muted { opacity: 0.8; }
```

Update `README.md` bullet list to explicitly say the GTK workspace view can:

- create PRs
- inspect checks/readiness/reviews
- resolve/reopen GitHub review threads
- merge/archive from the app

Add one short phase entry to `progress.md` with the exact verification commands used in Task 8.

- [ ] **Step 4: Run formatting to verify docs/CSS/source stay clean**

Run: `cargo fmt --all -- --check`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/gtk-app/src/theme.rs README.md progress.md
git commit -m "docs: update github integration coverage"
```

### Task 8: Full Verification

**Files:**
- Modify: none unless a failure requires a fix
- Test: `crates/core/src/workspace.rs`
- Test: `crates/gtk-app/src/projects.rs`
- Test: `crates/gtk-app/src/workspace_command_center.rs`
- Test: `crates/gtk-app/src/sidebar.rs`

- [ ] **Step 1: Run focused core verification**

Run: `cargo test -p linux-archductor-core workspace::tests -- --nocapture`

Expected: PASS

- [ ] **Step 2: Run focused GTK verification**

Run: `cargo test -p linux-archductor-gtk projects::tests workspace_command_center::tests sidebar::tests -- --nocapture`

Expected: PASS

- [ ] **Step 3: Run the crate-level GTK suite**

Run: `cargo test -p linux-archductor-gtk -- --nocapture`

Expected: PASS

- [ ] **Step 4: Run formatting and one full workspace check**

Run: `cargo fmt --all -- --check && cargo test --workspace`

Expected: PASS

- [ ] **Step 5: Commit the final verified state**

```bash
git add -A
git commit -m "feat(gtk): ship full github workspace integration"
```

## Self-Review

- Spec coverage:
  - GitHub issue/PR workspace creation in GTK: covered by Task 2.
  - GitHub visibility in dashboard/sidebar: covered by Task 3.
  - Checks/PR lifecycle in workspace view: covered by Task 4.
  - GitHub review-thread sync and actions: covered by Task 5.
  - Diff-to-review workflow from the Changes tab: covered by Task 6.
  - Styling/docs/progress/verifications: covered by Tasks 7 and 8.
- Placeholder scan:
  - No `TODO`/`TBD` markers remain.
  - Each task has concrete files, commands, and code snippets.
- Type consistency:
  - New helper names are reused consistently: `pull_request_panel_state`, `parse_github_numbered_stateful_choices`, `workspace_badge_text`, `pull_request_action_labels`, `review_tab_section_titles`, `diff_action_labels`.

Plan complete and saved to `docs/superpowers/plans/2026-06-23-github-integrations-everywhere.md`. Two execution options:

1. Subagent-Driven (recommended) - I dispatch a fresh subagent per task, review between tasks, fast iteration
2. Inline Execution - Execute tasks in this session using executing-plans, batch execution with checkpoints

Which approach?
