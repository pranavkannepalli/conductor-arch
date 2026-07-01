# Chat Threads And Codex Resume Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add first-class per-workspace chat threads with structured message persistence, exact stored Codex resume ids, and real functional chat controls in GTK and CLI.

**Architecture:** Introduce `chat_threads` and `chat_messages` as the conversation model, keep `processes` as PTY lifecycle records linked to threads, and migrate Codex transcript parsing from a render-time-only reconstruction into structured persisted messages. GTK and CLI continue using PTY transport for Codex, but they operate against thread/message records instead of implicit session logs.

**Tech Stack:** Rust, rusqlite, GTK4/libadwaita, portable-pty, vt100, existing `linux-archductor-core` workspace/session model

---

### Task 1: Add Thread And Message Persistence To Core

**Files:**
- Modify: `crates/core/src/workspace.rs`
- Test: `crates/core/src/workspace.rs`

- [ ] **Step 1: Write the failing schema and CRUD tests**

```rust
#[test]
fn chat_thread_crud_persists_multiple_threads_per_workspace_and_provider() {
    let temp = tempfile::tempdir().unwrap();
    let repo_path = init_repo(temp.path().join("demo"));
    let db_path = temp.path().join("state.db");
    RepositoryStore::open(&db_path)
        .unwrap()
        .add(AddRepository {
            name: Some("demo".to_owned()),
            root_path: repo_path,
            default_branch: Some("main".to_owned()),
            remote_name: "origin".to_owned(),
            workspace_parent_path: Some(temp.path().join("workspaces/demo")),
        })
        .unwrap();
    let store = WorkspaceStore::open_with_logs(&db_path, temp.path().join("logs")).unwrap();
    store.create(CreateWorkspace {
        repository_name: "demo".to_owned(),
        name: "berlin".to_owned(),
        branch: "lc/berlin".to_owned(),
        base_ref: Some("main".to_owned()),
    }).unwrap();

    let first = store.create_chat_thread("berlin", "codex", "Bugfix A", None).unwrap();
    let second = store.create_chat_thread("berlin", "codex", "Bugfix B", None).unwrap();
    let third = store.create_chat_thread("berlin", "claude", "Review", None).unwrap();

    let threads = store.list_chat_threads("berlin").unwrap();
    assert_eq!(threads.len(), 3);
    assert_eq!(threads[0].id, third.id);
    assert_eq!(threads[1].id, second.id);
    assert_eq!(threads[2].id, first.id);
}

#[test]
fn chat_messages_persist_user_control_and_agent_rows() {
    let store = test_workspace_store();
    let thread = store.create_chat_thread("berlin", "codex", "Bugfix A", None).unwrap();

    store.append_chat_message(thread.id, "user", "run tests", "user_send").unwrap();
    store.append_chat_message(thread.id, "system", "/model gpt-5", "control_command").unwrap();
    store.append_chat_message(thread.id, "agent", "Running now.", "agent_screen_parse").unwrap();

    let messages = store.list_chat_messages(thread.id).unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[1].source, "control_command");
    assert_eq!(messages[2].role, "agent");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p linux-archductor-core chat_thread_crud_persists_multiple_threads_per_workspace_and_provider chat_messages_persist_user_control_and_agent_rows -- --nocapture`

Expected: FAIL with missing `chat_threads`/`chat_messages` schema or missing methods on `WorkspaceStore`

- [ ] **Step 3: Add schema, row types, and `WorkspaceStore` CRUD**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatThreadRecord {
    pub id: i64,
    pub workspace_id: i64,
    pub provider: String,
    pub title: String,
    pub status: String,
    pub native_thread_id: Option<String>,
    pub harness_metadata: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessageRecord {
    pub id: i64,
    pub thread_id: i64,
    pub role: String,
    pub content: String,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
}

pub fn create_chat_thread(
    &self,
    workspace_name: &str,
    provider: &str,
    title: &str,
    harness_metadata: Option<&str>,
) -> Result<ChatThreadRecord> {
    let workspace = self.get_by_name(workspace_name)?;
    let now = timestamp();
    self.conn.execute(
        "INSERT INTO chat_threads (
            workspace_id, provider, title, status, native_thread_id, harness_metadata, created_at, updated_at, archived_at
         ) VALUES (?1, ?2, ?3, 'active', NULL, ?4, ?5, ?5, NULL)",
        params![workspace.id, provider, title, harness_metadata, now],
    )?;
    self.get_chat_thread(self.conn.last_insert_rowid())
}

pub fn append_chat_message(
    &self,
    thread_id: i64,
    role: &str,
    content: &str,
    source: &str,
) -> Result<ChatMessageRecord> {
    let now = timestamp();
    self.conn.execute(
        "INSERT INTO chat_messages (thread_id, role, content, source, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        params![thread_id, role, content, source, now],
    )?;
    self.get_chat_message(self.conn.last_insert_rowid())
}
```

- [ ] **Step 4: Link `processes` rows to threads**

```rust
ensure_column(
    &self.conn,
    "processes",
    "chat_thread_id",
    "ALTER TABLE processes ADD COLUMN chat_thread_id INTEGER REFERENCES chat_threads(id)",
)?;
```

Add `chat_thread_id: Option<i64>` to `ProcessRecord`, row mapping, and `record_process(...)`.

- [ ] **Step 5: Run the targeted tests**

Run: `cargo test -p linux-archductor-core chat_thread_crud_persists_multiple_threads_per_workspace_and_provider chat_messages_persist_user_control_and_agent_rows -- --nocapture`

Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/workspace.rs
git commit -m "feat(core): add chat thread and message persistence"
```

### Task 2: Persist Structured Codex Messages And Exact Resume Identity

**Files:**
- Modify: `crates/core/src/workspace.rs`
- Modify: `crates/core/src/harness.rs`
- Modify: `crates/cli/src/main.rs`
- Test: `crates/core/src/workspace.rs`
- Test: `crates/core/src/harness.rs`

- [ ] **Step 1: Write failing tests for Codex thread linking and native id persistence**

```rust
#[test]
fn codex_thread_messages_merge_screen_updates_into_chat_messages() {
    let store = test_workspace_store();
    let thread = store.create_chat_thread("berlin", "codex", "Bugfix A", None).unwrap();

    store.sync_codex_screen_messages(
        thread.id,
        "╭─ Codex ─╮\n│ Running now.\n╰────",
    ).unwrap();
    store.sync_codex_screen_messages(
        thread.id,
        "╭─ Codex ─╮\n│ Running now.\n│ Tests passed.\n╰────",
    ).unwrap();

    let messages = store.list_chat_messages(thread.id).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, "agent");
    assert_eq!(messages[0].content, "Running now.\nTests passed.");
}

#[test]
fn codex_resume_launch_prefers_stored_native_thread_id() {
    let args = build_session_resume_launch_plan(
        SessionKind::Codex,
        Path::new("/tmp/work"),
        &SessionHarnessOptions::default(),
        Some("native-session-123"),
    ).args;

    assert_eq!(
        args[args.len() - 2..],
        ["resume".to_owned(), "native-session-123".to_owned()]
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p linux-archductor-core codex_thread_messages_merge_screen_updates_into_chat_messages codex_resume_launch_prefers_stored_native_thread_id -- --nocapture`

Expected: FAIL with missing structured sync helper or incorrect resume behavior

- [ ] **Step 3: Add thread-aware Codex screen persistence helpers**

```rust
pub fn sync_codex_screen_messages(&self, thread_id: i64, screen: &str) -> Result<()> {
    let parsed = parse_codex_screen_messages(screen);
    let existing = self.list_chat_messages(thread_id)?
        .into_iter()
        .filter(|message| message.role == "agent")
        .map(|message| ScreenMessage {
            role: ScreenMessageRole::Agent,
            content: message.content,
        })
        .collect::<Vec<_>>();

    let mut merged = existing.clone();
    merge_screen_messages(&mut merged, &parsed.into_iter().filter(|m| m.role == ScreenMessageRole::Agent).collect::<Vec<_>>());

    self.replace_agent_messages(thread_id, &merged)?;
    Ok(())
}
```

- [ ] **Step 4: Capture and store native Codex thread id**

```rust
pub fn set_chat_thread_native_id(&self, thread_id: i64, native_thread_id: &str) -> Result<()> {
    let now = timestamp();
    self.conn.execute(
        "UPDATE chat_threads SET native_thread_id = ?1, updated_at = ?2 WHERE id = ?3",
        params![native_thread_id, now, thread_id],
    )?;
    Ok(())
}
```

Implement deterministic extraction in the Codex PTY owner path. The exact source can be a supported Codex-produced file or screen-confirmed identifier, but the worker must:

- store the real id onto `chat_threads.native_thread_id`
- never synthesize an id
- keep legacy fallback only when the thread has no stored native id

- [ ] **Step 5: Make Codex resume launch consume `native_thread_id`**

```rust
let resume_token = thread.native_thread_id
    .as_deref()
    .or(existing_process.session_resume_id.as_deref());
let launch = store.session_launch_with_options_and_resume(
    workspace_name,
    SessionKind::Codex,
    harness,
    resume_token,
)?;
```

- [ ] **Step 6: Run focused tests**

Run: `cargo test -p linux-archductor-core codex_thread_messages_merge_screen_updates_into_chat_messages codex_resume_launch_prefers_stored_native_thread_id start_session_with_options_uses_real_codex_flags_without_bootstrap_env -- --nocapture`

Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add crates/core/src/workspace.rs crates/core/src/harness.rs crates/cli/src/main.rs
git commit -m "feat(core): persist codex thread ids and structured agent messages"
```

### Task 3: Make GTK Render Real Threads And Real Messages

**Files:**
- Modify: `crates/gtk-app/src/session_surface.rs`
- Modify: `crates/gtk-app/src/state.rs`
- Test: `crates/gtk-app/src/session_surface.rs`

- [ ] **Step 1: Write failing GTK transcript/thread tests**

```rust
#[test]
fn thread_list_keeps_multiple_codex_threads_per_workspace() {
    let threads = vec![
        chat_thread(1, "codex", "Bugfix A"),
        chat_thread(2, "codex", "Bugfix B"),
        chat_thread(3, "claude", "Review"),
    ];
    let labels = thread_list_labels(&threads);
    assert_eq!(labels, vec!["Review", "Bugfix B", "Bugfix A"]);
}

#[test]
fn selected_thread_surface_prefers_structured_messages_over_raw_log_noise() {
    let messages = vec![
        chat_message("user", "run tests"),
        chat_message("agent", "Running now.\nTests passed."),
    ];
    let surface = render_thread_messages_for_display(&messages);
    assert!(surface.contains("You\nrun tests"));
    assert!(surface.contains("Agent\nRunning now.\nTests passed."));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p linux-archductor-gtk --bin linux-archductor-gtk thread_list_keeps_multiple_codex_threads_per_workspace selected_thread_surface_prefers_structured_messages_over_raw_log_noise -- --nocapture`

Expected: FAIL with missing thread/message view helpers

- [ ] **Step 3: Replace selected-session model with selected-thread model**

```rust
let selected_thread: Rc<RefCell<Option<i64>>> = Rc::new(RefCell::new(None));
let thread_state = Rc::new(RefCell::new(Vec::<ChatThreadRecord>::new()));

let threads = WorkspaceStore::open(database_path.clone())
    .and_then(|store| store.list_chat_threads(&workspace))
    .unwrap_or_default();
```

Render the left-side/session chooser as thread rows rather than inferring from `processes`.

- [ ] **Step 4: Render structured messages**

```rust
fn render_thread_messages_for_display(messages: &[ChatMessageRecord]) -> String {
    let mut out = String::new();
    for message in messages {
        let label = match message.role.as_str() {
            "user" => "You",
            "review" => "Review Prompt",
            "system" => "System",
            _ => "Agent",
        };
        out.push_str(label);
        out.push('\n');
        out.push_str(message.content.trim());
        out.push_str("\n\n");
    }
    out
}
```

- [ ] **Step 5: Keep live PTY poll feeding structured persistence**

Use the existing Codex screen helpers to:

- append `chat_messages` for user sends
- sync parsed screen messages into `chat_messages`
- keep `[codex raw]` / `[codex screen]` only for compatibility and debugging

- [ ] **Step 6: Run targeted GTK tests**

Run: `cargo test -p linux-archductor-gtk --bin linux-archductor-gtk thread_list_keeps_multiple_codex_threads_per_workspace selected_thread_surface_prefers_structured_messages_over_raw_log_noise transcript_events_parse_codex_screen_blocks_without_raw_duplication -- --nocapture`

Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add crates/gtk-app/src/session_surface.rs crates/gtk-app/src/state.rs
git commit -m "feat(gtk): render workspace chat threads and structured messages"
```

### Task 4: Replace Mocked Chat Controls With Real Provider Commands

**Files:**
- Modify: `crates/gtk-app/src/session_surface.rs`
- Modify: `crates/core/src/workspace.rs`
- Test: `crates/gtk-app/src/session_surface.rs`
- Test: `crates/core/src/workspace.rs`

- [ ] **Step 1: Write failing tests for pending command behavior**

```rust
#[test]
fn pending_control_commands_flush_before_user_message() {
    let mut pending = vec!["/model gpt-5".to_owned(), "/thinking high".to_owned()];
    let flushed = flush_pending_commands_for_send(&mut pending, "run tests");
    assert_eq!(
        flushed,
        vec!["/model gpt-5".to_owned(), "/thinking high".to_owned(), "run tests".to_owned()]
    );
    assert!(pending.is_empty());
}

#[test]
fn unsupported_live_controls_are_filtered_out_of_toolbar() {
    let controls = visible_live_controls_for_provider("codex");
    assert!(!controls.contains(&"fake-goal-toggle".to_owned()));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p linux-archductor-gtk --bin linux-archductor-gtk pending_control_commands_flush_before_user_message unsupported_live_controls_are_filtered_out_of_toolbar -- --nocapture`

Expected: FAIL with missing pending-command queue and control filtering helpers

- [ ] **Step 3: Add per-thread pending command queue**

```rust
let pending_commands: Rc<RefCell<HashMap<i64, Vec<String>>>> =
    Rc::new(RefCell::new(HashMap::new()));

fn queue_thread_command(
    pending: &RefCell<HashMap<i64, Vec<String>>>,
    thread_id: i64,
    command: String,
) {
    pending.borrow_mut().entry(thread_id).or_default().push(command);
}
```

- [ ] **Step 4: Map visible controls to real Codex commands**

Use explicit helpers instead of ad hoc UI state:

```rust
fn codex_model_command(model: &str) -> Option<String> {
    (!model.trim().is_empty()).then(|| format!("/model {}", model.trim()))
}

fn codex_reasoning_command(level: &str) -> Option<String> {
    match level.trim() {
        "low" | "medium" | "high" | "extra high" => Some(format!("/thinking {}", level.trim())),
        _ => None,
    }
}
```

Any control that cannot map to a live command becomes new-thread-only or is removed.

- [ ] **Step 5: Flush queued commands before the next user send and persist them**

```rust
for command in pending_commands.remove(&thread_id).unwrap_or_default() {
    session.send_line(&command)?;
    store.append_chat_message(thread_id, "system", &command, "control_command")?;
}
store.append_chat_message(thread_id, "user", &user_text, "user_send")?;
session.send_line(&user_text)?;
```

- [ ] **Step 6: Run focused tests**

Run: `cargo test -p linux-archductor-gtk --bin linux-archductor-gtk pending_control_commands_flush_before_user_message unsupported_live_controls_are_filtered_out_of_toolbar -- --nocapture`

Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add crates/gtk-app/src/session_surface.rs crates/core/src/workspace.rs
git commit -m "feat(gtk): make chat controls send real provider commands"
```

### Task 5: Add CLI And History Support For Threads

**Files:**
- Modify: `crates/cli/src/main.rs`
- Modify: `crates/core/src/workspace.rs`
- Test: `crates/cli/tests/cli_sessions.rs`
- Test: `crates/core/src/workspace.rs`

- [ ] **Step 1: Write failing CLI/history tests**

```rust
#[test]
fn local_chat_history_prefers_thread_messages_for_new_chat_threads() {
    let store = test_workspace_store();
    let thread = store.create_chat_thread("berlin", "codex", "Bugfix A", None).unwrap();
    store.append_chat_message(thread.id, "user", "run tests", "user_send").unwrap();
    store.append_chat_message(thread.id, "agent", "Tests passed.", "agent_screen_parse").unwrap();

    let messages = store.chat_thread_history_messages(thread.id).unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[1].content, "Tests passed.");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p linux-archductor-core local_chat_history_prefers_thread_messages_for_new_chat_threads -- --nocapture`

Expected: FAIL with missing thread-aware history methods

- [ ] **Step 3: Add thread-aware CLI/history queries**

```rust
pub fn chat_thread_history_messages(&self, thread_id: i64) -> Result<Vec<LocalChatHistoryMessage>> {
    self.list_chat_messages(thread_id)?
        .into_iter()
        .map(|message| Ok(LocalChatHistoryMessage {
            role: message.role,
            content: message.content,
        }))
        .collect()
}
```

Add CLI commands or adapt existing history/session views so new thread-backed chats are queryable without reconstructing only from raw session logs.

- [ ] **Step 4: Keep transcript compatibility for old consumers**

Do not remove legacy log reads yet. Prefer:

- thread messages for new thread-backed chats
- legacy transcript parsing for old session-only history

- [ ] **Step 5: Run targeted tests**

Run: `cargo test -p linux-archductor-core local_chat_history_prefers_thread_messages_for_new_chat_threads -- --nocapture`

Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/cli/src/main.rs crates/cli/tests/cli_sessions.rs crates/core/src/workspace.rs
git commit -m "feat(cli): add thread-aware chat history support"
```

### Task 6: Full Verification And Cleanup

**Files:**
- Modify: `crates/gtk-app/src/session_surface.rs`
- Modify: `crates/core/src/workspace.rs`
- Modify: `crates/cli/src/main.rs`

- [ ] **Step 1: Remove leftover mocked or dead chat UI paths**

Delete or demote:

- decorative controls with no provider mapping
- stale “selected session” assumptions that should now be “selected thread”
- duplicate transcript-only rendering paths for new thread-backed chats

- [ ] **Step 2: Run the full targeted suites**

Run: `cargo test -p linux-archductor-core -- --nocapture`

Expected: PASS

Run: `cargo test -p linux-archductor -- --nocapture`

Expected: PASS

Run: `cargo test -p linux-archductor-gtk --bin linux-archductor-gtk -- --nocapture`

Expected: PASS

- [ ] **Step 3: Do a final manual regression sweep**

Run:

```bash
cargo run --bin linux-archductor-gtk
```

Manual checks:

- create two Codex threads in one workspace
- switch between them
- send a message in each
- change model and reasoning, then verify the control commands are emitted before the next user message
- stop and resume a Codex thread by exact stored id
- verify old legacy history still renders

- [ ] **Step 4: Commit**

```bash
git add crates/core/src/workspace.rs crates/core/src/harness.rs crates/cli/src/main.rs crates/gtk-app/src/session_surface.rs crates/cli/tests/cli_sessions.rs
git commit -m "feat(chat): add thread-backed sessions with exact codex resume"
```
