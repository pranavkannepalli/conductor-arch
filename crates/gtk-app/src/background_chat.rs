use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use archductor_core::archcar::harness::managed_harness_for_kind;
#[cfg(test)]
use archductor_core::archcar::protocol::ArchcarInputKind;
use archductor_core::archcar::protocol::{ArchcarEvent, ArchcarResponse};
use archductor_core::provider_events::ProviderEventStore;
use archductor_core::workspace::{
    ChatMessageRecord, ChatThreadRecord, ProcessRecord, ProcessStatus, SessionKind, WorkspaceStore,
};
use gtk::glib;
use tracing::{debug, warn};

use crate::archcar_async::{
    clear_archcar_ready, note_archcar_ready, spawn_background_job, AsyncArchcarBridge,
    AsyncArchcarMessage, AsyncArchcarResponse,
};
use crate::background_sync::provider_events_have_active_work;
use crate::refresh::RefreshEvent;
use crate::state::{AppState, QueuedChatInputDraft};

const BACKGROUND_CHAT_TICK_SECONDS: u32 = 1;
const BACKGROUND_CHAT_WAKE_DELAY_MS: u64 = 32;

static NEXT_BACKGROUND_CHAT_WAKE_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static BACKGROUND_CHAT_WAKE_REGISTRY: RefCell<HashMap<u64, Rc<dyn Fn()>>> =
        RefCell::new(HashMap::new());
}

#[derive(Debug, Clone)]
struct BackgroundQueueCandidate {
    workspace: String,
    branch_prefix: String,
    thread: ChatThreadRecord,
    messages: Vec<ChatMessageRecord>,
    queued_count: usize,
    queued_session_kind: SessionKind,
    running_session_id: Option<i64>,
    active_work: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BackgroundQueuePlan {
    Ensure,
    Send { session_id: i64 },
}

#[derive(Debug, Clone)]
enum BackgroundChatAction {
    Ensure {
        workspace: String,
        thread_id: i64,
        kind: SessionKind,
    },
    UserSend {
        workspace: String,
        thread_id: i64,
        session_id: i64,
        draft: QueuedChatInputDraft,
        checkpoint_id: Option<i64>,
    },
}

#[derive(Default)]
struct BackgroundChatRunnerState {
    ready_sessions: HashMap<i64, bool>,
    session_threads: HashMap<i64, i64>,
    thread_workspaces: HashMap<i64, String>,
    inflight_actions: HashMap<u64, BackgroundChatAction>,
    held_threads: HashSet<i64>,
    working_threads: HashSet<i64>,
}

pub(crate) fn install_background_chat_runner(app_state: &AppState) {
    let bridge = AsyncArchcarBridge::new(app_state.paths.clone());
    let state = Rc::new(RefCell::new(BackgroundChatRunnerState::default()));
    let scan_in_flight = Rc::new(Cell::new(false));
    let db_path = app_state.workspace_database_path();

    let tick: Rc<dyn Fn()> = {
        let app_state = app_state.clone();
        let bridge = bridge.clone();
        let state = Rc::clone(&state);
        let scan_in_flight = Rc::clone(&scan_in_flight);
        Rc::new(move || {
            drain_background_archcar_messages(&bridge, &db_path, &app_state, &state);
            schedule_background_queue_scan(
                bridge.clone(),
                db_path.clone(),
                app_state.clone(),
                Rc::clone(&state),
                Rc::clone(&scan_in_flight),
            );
        })
    };

    install_background_archcar_wake(&bridge, tick.clone());
    glib::timeout_add_seconds_local(BACKGROUND_CHAT_TICK_SECONDS, move || {
        tick();
        glib::ControlFlow::Continue
    });
}

fn install_background_archcar_wake(bridge: &AsyncArchcarBridge, tick: Rc<dyn Fn()>) {
    let wake_id = NEXT_BACKGROUND_CHAT_WAKE_ID.fetch_add(1, Ordering::Relaxed);
    let wake_pending = Arc::new(AtomicBool::new(false));
    BACKGROUND_CHAT_WAKE_REGISTRY.with(|registry| {
        registry.borrow_mut().insert(wake_id, tick);
    });
    let main_context = glib::MainContext::default();
    bridge.set_waker(move || {
        if wake_pending.swap(true, Ordering::AcqRel) {
            return;
        }
        let wake_pending = Arc::clone(&wake_pending);
        main_context.invoke(move || {
            glib::timeout_add_local_once(
                Duration::from_millis(BACKGROUND_CHAT_WAKE_DELAY_MS),
                move || {
                    wake_pending.store(false, Ordering::Release);
                    let tick = BACKGROUND_CHAT_WAKE_REGISTRY
                        .with(|registry| registry.borrow().get(&wake_id).cloned());
                    if let Some(tick) = tick {
                        tick();
                    }
                },
            );
        });
    });
}

fn schedule_background_queue_scan(
    bridge: AsyncArchcarBridge,
    db_path: PathBuf,
    app_state: AppState,
    state: Rc<RefCell<BackgroundChatRunnerState>>,
    scan_in_flight: Rc<Cell<bool>>,
) {
    if scan_in_flight.get() {
        return;
    }
    scan_in_flight.set(true);
    spawn_background_job(
        move || load_background_queue_candidates(&db_path),
        move |result| {
            scan_in_flight.set(false);
            let candidates = match result {
                Ok(candidates) => candidates,
                Err(err) => {
                    warn!(error = %err, "background chat queue scan failed");
                    return;
                }
            };
            drive_background_queue_candidates(&bridge, &app_state, &state, candidates);
        },
    );
}

fn load_background_queue_candidates(
    db_path: &Path,
) -> anyhow::Result<Vec<BackgroundQueueCandidate>> {
    let store = WorkspaceStore::open_app(db_path)?;
    let provider_store = ProviderEventStore::new(db_path);
    let mut candidates = Vec::new();
    for thread_id in store.list_queued_chat_thread_ids()? {
        let queued_inputs = store.list_queued_chat_inputs(thread_id)?;
        let Some(first_queued) = queued_inputs.first() else {
            continue;
        };
        let thread = store.get_chat_thread_record(thread_id)?;
        let workspace = store.get_workspace_record(thread.workspace_id)?;
        let session_kind = first_queued.session_kind;
        let records = store.list_thread_processes(thread_id)?;
        let running_session_id = records
            .iter()
            .filter(|record| {
                record.status == ProcessStatus::Running
                    && session_kind_matches_record(record, session_kind)
            })
            .max_by_key(|record| record.id)
            .map(|record| record.id);
        let events = provider_store.list_for_chat_thread(thread_id)?;
        candidates.push(BackgroundQueueCandidate {
            branch_prefix: store.workspace_branch_prefix(&workspace.name)?,
            messages: store.list_chat_messages(thread_id)?,
            queued_count: queued_inputs.len(),
            queued_session_kind: session_kind,
            workspace: workspace.name,
            thread,
            running_session_id,
            active_work: provider_events_have_active_work(&events),
        });
    }
    Ok(candidates)
}

fn drive_background_queue_candidates(
    bridge: &AsyncArchcarBridge,
    app_state: &AppState,
    state: &Rc<RefCell<BackgroundChatRunnerState>>,
    candidates: Vec<BackgroundQueueCandidate>,
) {
    let selected_visible_thread = app_state.visible_selected_chat_thread();
    for candidate in candidates {
        let thread_id = candidate.thread.id;
        state
            .borrow_mut()
            .thread_workspaces
            .insert(thread_id, candidate.workspace.clone());
        let plan = plan_background_queue_drain(
            &candidate,
            &state.borrow(),
            selected_visible_thread,
            candidate.queued_count,
            candidate.queued_session_kind,
        );
        match plan {
            Some(BackgroundQueuePlan::Ensure) => {
                request_background_ensure(
                    bridge,
                    state,
                    &candidate.workspace,
                    thread_id,
                    candidate.queued_session_kind,
                );
            }
            Some(BackgroundQueuePlan::Send { session_id }) => {
                send_background_queued_input(bridge, app_state, state, candidate, session_id);
            }
            None => {}
        }
    }
}

fn plan_background_queue_drain(
    candidate: &BackgroundQueueCandidate,
    state: &BackgroundChatRunnerState,
    selected_visible_thread: Option<i64>,
    queued_count: usize,
    session_kind: SessionKind,
) -> Option<BackgroundQueuePlan> {
    let thread_id = candidate.thread.id;
    if queued_count == 0 || selected_visible_thread == Some(thread_id) {
        return None;
    }
    if managed_harness_for_kind(session_kind).is_none()
        || candidate.active_work
        || state.held_threads.contains(&thread_id)
        || state.working_threads.contains(&thread_id)
        || has_inflight_user_send_for_thread(state, thread_id)
    {
        return None;
    }
    let Some(session_id) = candidate.running_session_id else {
        if has_inflight_ensure_for_thread(state, thread_id) {
            return None;
        }
        return Some(BackgroundQueuePlan::Ensure);
    };
    if state
        .ready_sessions
        .get(&session_id)
        .copied()
        .unwrap_or(false)
        || claude_first_input_can_send_before_ready(session_kind, &candidate.messages)
    {
        return Some(BackgroundQueuePlan::Send { session_id });
    }
    (!has_inflight_ensure_for_thread(state, thread_id)).then_some(BackgroundQueuePlan::Ensure)
}

fn request_background_ensure(
    bridge: &AsyncArchcarBridge,
    state: &Rc<RefCell<BackgroundChatRunnerState>>,
    workspace: &str,
    thread_id: i64,
    session_kind: SessionKind,
) {
    let Some(token) = bridge.ensure_thread_session(workspace.to_owned(), thread_id, session_kind)
    else {
        return;
    };
    state.borrow_mut().inflight_actions.insert(
        token,
        BackgroundChatAction::Ensure {
            workspace: workspace.to_owned(),
            thread_id,
            kind: session_kind,
        },
    );
}

fn send_background_queued_input(
    bridge: &AsyncArchcarBridge,
    app_state: &AppState,
    state: &Rc<RefCell<BackgroundChatRunnerState>>,
    candidate: BackgroundQueueCandidate,
    _session_id: i64,
) {
    let thread_id = candidate.thread.id;
    if app_state.visible_selected_chat_thread() == Some(thread_id) {
        return;
    }
    let token = bridge.ensure_thread_session(
        candidate.workspace.clone(),
        thread_id,
        candidate.queued_session_kind,
    );
    let Some(token) = token else {
        return;
    };
    state.borrow_mut().inflight_actions.insert(
        token,
        BackgroundChatAction::Ensure {
            workspace: candidate.workspace.clone(),
            thread_id,
            kind: candidate.queued_session_kind,
        },
    );
    app_state.request_refresh(RefreshEvent::WorkspaceChatLifecycleChanged {
        workspace: candidate.workspace,
    });
}

fn drain_background_archcar_messages(
    bridge: &AsyncArchcarBridge,
    db_path: &Path,
    app_state: &AppState,
    state: &Rc<RefCell<BackgroundChatRunnerState>>,
) {
    while let Some(message) = bridge.try_recv() {
        match message {
            AsyncArchcarMessage::Event(event) => {
                request_refresh_for_archcar_event(db_path, app_state, state, &event);
                reduce_background_archcar_event(&event, &mut state.borrow_mut());
            }
            AsyncArchcarMessage::Response(response) => {
                handle_background_archcar_response(app_state, state, response);
            }
            AsyncArchcarMessage::BridgeError { message } => {
                warn!(error = %message, "background archcar bridge error");
            }
        }
    }
}

fn reduce_background_archcar_event(event: &ArchcarEvent, state: &mut BackgroundChatRunnerState) {
    match event {
        ArchcarEvent::SessionStarted {
            session_id,
            thread_id,
            workspace,
            ..
        } => {
            state.session_threads.insert(*session_id, *thread_id);
            state
                .thread_workspaces
                .insert(*thread_id, workspace.clone());
            note_archcar_ready(&mut state.ready_sessions, *session_id, false);
        }
        ArchcarEvent::SessionReady {
            session_id,
            thread_id,
        } => {
            state.session_threads.insert(*session_id, *thread_id);
            note_archcar_ready(&mut state.ready_sessions, *session_id, true);
        }
        ArchcarEvent::TurnCompleted {
            session_id,
            thread_id,
            status,
        } => {
            state.session_threads.insert(*session_id, *thread_id);
            clear_inflight_user_sends_for_thread(state, *thread_id);
            state.working_threads.remove(thread_id);
            if archcar_turn_completion_allows_queue_drain(status.as_deref()) {
                state.held_threads.remove(thread_id);
                note_archcar_ready(&mut state.ready_sessions, *session_id, true);
            } else {
                state.held_threads.insert(*thread_id);
                note_archcar_ready(&mut state.ready_sessions, *session_id, false);
            }
        }
        ArchcarEvent::SessionMessagesUpdated { thread_id } => {
            clear_inflight_user_sends_for_thread(state, *thread_id);
        }
        ArchcarEvent::SessionExited { session_id, .. } => {
            if let Some(thread_id) = state.session_threads.remove(session_id) {
                state.held_threads.insert(thread_id);
                state.working_threads.remove(&thread_id);
                clear_inflight_user_sends_for_thread(state, thread_id);
            }
            clear_archcar_ready(&mut state.ready_sessions, *session_id);
        }
        ArchcarEvent::SessionError {
            session_id,
            thread_id,
            ..
        } => {
            if let Some(session_id) = session_id {
                note_archcar_ready(&mut state.ready_sessions, *session_id, false);
                if let Some(thread_id) = state.session_threads.get(session_id).copied() {
                    state.held_threads.insert(thread_id);
                    state.working_threads.remove(&thread_id);
                    clear_inflight_user_sends_for_thread(state, thread_id);
                }
            }
            if let Some(thread_id) = thread_id {
                state.held_threads.insert(*thread_id);
                state.working_threads.remove(thread_id);
                clear_inflight_user_sends_for_thread(state, *thread_id);
            }
        }
        ArchcarEvent::ProviderInteractionRequested { interaction }
        | ArchcarEvent::ProviderInteractionResolved { interaction } => {
            state
                .thread_workspaces
                .insert(interaction.thread_id, interaction.workspace.clone());
        }
        ArchcarEvent::SessionSpawnQueued { .. }
        | ArchcarEvent::SessionCapabilitiesChanged { .. }
        | ArchcarEvent::ChatQueueUpdated { .. }
        | ArchcarEvent::SessionScreenUpdated { .. } => {}
    }
}

fn handle_background_archcar_response(
    app_state: &AppState,
    state: &Rc<RefCell<BackgroundChatRunnerState>>,
    response: AsyncArchcarResponse,
) {
    let Some(action) = state.borrow_mut().inflight_actions.remove(&response.token) else {
        return;
    };
    match action {
        BackgroundChatAction::Ensure {
            workspace,
            thread_id,
            kind,
        } => match response.result {
            Ok(ArchcarResponse::SessionSpawnQueued { .. })
            | Ok(ArchcarResponse::SessionSpawned { .. })
            | Ok(ArchcarResponse::Ack) => {
                debug!(%workspace, thread_id, ?kind, "background chat ensure accepted");
                app_state
                    .request_refresh(RefreshEvent::WorkspaceChatLifecycleChanged { workspace });
            }
            Ok(other) => {
                warn!(
                    thread_id,
                    ?other,
                    "unexpected background chat ensure response"
                );
                state.borrow_mut().held_threads.insert(thread_id);
                app_state
                    .request_refresh(RefreshEvent::WorkspaceChatLifecycleChanged { workspace });
            }
            Err(err) => {
                warn!(thread_id, error = %err, "background chat ensure failed");
                state.borrow_mut().held_threads.insert(thread_id);
                app_state
                    .request_refresh(RefreshEvent::WorkspaceChatLifecycleChanged { workspace });
            }
        },
        BackgroundChatAction::UserSend {
            workspace,
            thread_id,
            session_id,
            draft,
            checkpoint_id,
        } => match response.result {
            Ok(ArchcarResponse::Ack) => {
                state.borrow_mut().inflight_actions.insert(
                    response.token,
                    BackgroundChatAction::UserSend {
                        workspace,
                        thread_id,
                        session_id,
                        draft,
                        checkpoint_id,
                    },
                );
            }
            Ok(other) => {
                warn!(
                    thread_id,
                    session_id,
                    ?other,
                    "unexpected background chat input response"
                );
                recover_failed_background_send(
                    app_state,
                    state,
                    workspace,
                    thread_id,
                    session_id,
                    draft,
                    checkpoint_id,
                );
            }
            Err(err) => {
                warn!(thread_id, session_id, error = %err, "background chat input send failed");
                recover_failed_background_send(
                    app_state,
                    state,
                    workspace,
                    thread_id,
                    session_id,
                    draft,
                    checkpoint_id,
                );
            }
        },
    }
}

fn recover_failed_background_send(
    app_state: &AppState,
    state: &Rc<RefCell<BackgroundChatRunnerState>>,
    workspace: String,
    thread_id: i64,
    session_id: i64,
    draft: QueuedChatInputDraft,
    checkpoint_id: Option<i64>,
) {
    if let Some(checkpoint_id) = checkpoint_id {
        discard_background_turn_checkpoint(app_state, &workspace, checkpoint_id);
    }
    app_state.requeue_chat_input_front(thread_id, draft);
    let mut state = state.borrow_mut();
    state.held_threads.insert(thread_id);
    state.working_threads.remove(&thread_id);
    note_archcar_ready(&mut state.ready_sessions, session_id, false);
    drop(state);
    app_state.request_refresh(RefreshEvent::WorkspaceChatLifecycleChanged { workspace });
}

fn request_refresh_for_archcar_event(
    db_path: &Path,
    app_state: &AppState,
    state: &Rc<RefCell<BackgroundChatRunnerState>>,
    event: &ArchcarEvent,
) {
    match event {
        ArchcarEvent::SessionStarted {
            workspace,
            thread_id,
            ..
        } => {
            app_state.request_refresh(RefreshEvent::WorkspaceChatLifecycleChanged {
                workspace: workspace.clone(),
            });
            app_state.request_refresh(RefreshEvent::WorkspaceChatMessagesChanged {
                workspace: workspace.clone(),
                thread_id: *thread_id,
            });
        }
        ArchcarEvent::SessionMessagesUpdated { thread_id }
        | ArchcarEvent::SessionReady { thread_id, .. }
        | ArchcarEvent::SessionCapabilitiesChanged { thread_id, .. } => {
            request_thread_chat_refresh(db_path, app_state, state, *thread_id, false);
        }
        ArchcarEvent::ChatQueueUpdated { thread_id } => {
            sync_queued_inputs_cache(db_path.to_path_buf(), app_state.clone(), *thread_id);
            request_thread_chat_refresh(db_path, app_state, state, *thread_id, false);
        }
        ArchcarEvent::TurnCompleted { thread_id, .. } => {
            request_thread_chat_refresh(db_path, app_state, state, *thread_id, true);
            refresh_pull_request_after_background_turn(
                db_path.to_path_buf(),
                app_state.clone(),
                *thread_id,
            );
        }
        ArchcarEvent::SessionExited { session_id, .. } => {
            if let Some(thread_id) = state.borrow().session_threads.get(session_id).copied() {
                request_thread_chat_refresh(db_path, app_state, state, thread_id, true);
            }
        }
        ArchcarEvent::SessionError { thread_id, .. } => {
            if let Some(thread_id) = thread_id {
                request_thread_chat_refresh(db_path, app_state, state, *thread_id, true);
            }
        }
        ArchcarEvent::ProviderInteractionRequested { interaction }
        | ArchcarEvent::ProviderInteractionResolved { interaction } => {
            app_state.request_refresh(RefreshEvent::WorkspaceChatMessagesChanged {
                workspace: interaction.workspace.clone(),
                thread_id: interaction.thread_id,
            });
        }
        ArchcarEvent::SessionSpawnQueued { workspace, .. } => {
            app_state.request_refresh(RefreshEvent::WorkspaceChatLifecycleChanged {
                workspace: workspace.clone(),
            });
        }
        ArchcarEvent::SessionScreenUpdated { .. } => {}
    }
}

fn request_thread_chat_refresh(
    db_path: &Path,
    app_state: &AppState,
    state: &Rc<RefCell<BackgroundChatRunnerState>>,
    thread_id: i64,
    lifecycle: bool,
) {
    if let Some(workspace) = state.borrow().thread_workspaces.get(&thread_id).cloned() {
        request_thread_chat_refresh_for_workspace(app_state, workspace, thread_id, lifecycle);
        return;
    }
    let app_state = app_state.clone();
    let db_path = db_path.to_path_buf();
    spawn_background_job(
        move || workspace_name_for_thread(&db_path, thread_id),
        move |result| match result {
            Ok(workspace) => {
                request_thread_chat_refresh_for_workspace(
                    &app_state, workspace, thread_id, lifecycle,
                );
            }
            Err(err) => {
                warn!(thread_id, error = %err, "failed to resolve background chat workspace")
            }
        },
    );
}

fn request_thread_chat_refresh_for_workspace(
    app_state: &AppState,
    workspace: String,
    thread_id: i64,
    lifecycle: bool,
) {
    if lifecycle {
        app_state.request_refresh(RefreshEvent::WorkspaceChatLifecycleChanged {
            workspace: workspace.clone(),
        });
    }
    app_state.request_refresh(RefreshEvent::WorkspaceChatMessagesChanged {
        workspace,
        thread_id,
    });
}

fn refresh_pull_request_after_background_turn(
    db_path: PathBuf,
    app_state: AppState,
    thread_id: i64,
) {
    spawn_background_job(
        move || {
            let store = WorkspaceStore::open_app(&db_path)?;
            let thread = store.get_chat_thread_record(thread_id)?;
            let workspace = store.get_workspace_record(thread.workspace_id)?.name;
            let _ = store.refresh_pull_request_state(&workspace)?;
            anyhow::Ok(workspace)
        },
        move |result| match result {
            Ok(workspace) => {
                app_state.request_refresh(RefreshEvent::WorkspaceReviewChanged { workspace });
            }
            Err(err) => {
                debug!(thread_id, error = %err, "background PR refresh skipped");
            }
        },
    );
}

fn sync_queued_inputs_cache(db_path: PathBuf, app_state: AppState, thread_id: i64) {
    spawn_background_job(
        move || {
            WorkspaceStore::open_app(&db_path)
                .and_then(|store| store.list_queued_chat_inputs(thread_id))
                .map(|inputs| {
                    inputs
                        .into_iter()
                        .map(|input| QueuedChatInputDraft {
                            id: Some(input.id),
                            input: input.input,
                            visible_input: input.visible_input,
                            kind: input.input_kind,
                            session_kind: input.session_kind,
                        })
                        .collect::<Vec<_>>()
                })
        },
        move |result| match result {
            Ok(inputs) => app_state.replace_queued_chat_inputs(thread_id, inputs),
            Err(err) => warn!(
                thread_id,
                error = %format!("{err:#}"),
                "failed to sync background chat queue cache"
            ),
        },
    );
}

fn workspace_name_for_thread(db_path: &Path, thread_id: i64) -> anyhow::Result<String> {
    let store = WorkspaceStore::open_app(db_path)?;
    let thread = store.get_chat_thread_record(thread_id)?;
    Ok(store.get_workspace_record(thread.workspace_id)?.name)
}

fn create_background_turn_checkpoint(
    app_state: &AppState,
    workspace: &str,
    thread_id: i64,
    session_id: Option<i64>,
    staged_review: bool,
) -> Option<i64> {
    let prompt_kind = if staged_review { "review" } else { "user" };
    WorkspaceStore::open_app(app_state.workspace_database_path())
        .and_then(|store| {
            store.checkpoint_create_turn_start(workspace, thread_id, session_id, prompt_kind)
        })
        .map(|checkpoint| checkpoint.id)
        .map_err(|err| {
            warn!(
                workspace = %workspace,
                thread_id,
                error = %err,
                "background turn checkpoint creation failed"
            );
            err
        })
        .ok()
}

fn discard_background_turn_checkpoint(app_state: &AppState, workspace: &str, checkpoint_id: i64) {
    if let Err(err) = WorkspaceStore::open_app(app_state.workspace_database_path())
        .and_then(|store| store.checkpoint_delete(workspace, checkpoint_id))
    {
        warn!(
            workspace = %workspace,
            checkpoint_id,
            error = %err,
            "background turn checkpoint cleanup failed"
        );
    }
}

fn clear_inflight_user_sends_for_thread(state: &mut BackgroundChatRunnerState, thread_id: i64) {
    state.inflight_actions.retain(|_, action| {
        !matches!(
            action,
            BackgroundChatAction::UserSend {
                thread_id: action_thread_id,
                ..
            } if *action_thread_id == thread_id
        )
    });
}

fn has_inflight_user_send_for_thread(state: &BackgroundChatRunnerState, thread_id: i64) -> bool {
    state.inflight_actions.values().any(|action| {
        matches!(
            action,
            BackgroundChatAction::UserSend {
                thread_id: action_thread_id,
                ..
            } if *action_thread_id == thread_id
        )
    })
}

fn has_inflight_ensure_for_thread(state: &BackgroundChatRunnerState, thread_id: i64) -> bool {
    state.inflight_actions.values().any(|action| {
        matches!(
            action,
            BackgroundChatAction::Ensure {
                thread_id: action_thread_id,
                ..
            } if *action_thread_id == thread_id
        )
    })
}

fn archcar_turn_completion_allows_queue_drain(status: Option<&str>) -> bool {
    !matches!(
        status.map(|status| status.trim().to_ascii_lowercase()),
        Some(status)
            if matches!(
                status.as_str(),
                "failed" | "error" | "interrupted" | "cancelled" | "canceled" | "deferred"
            )
    )
}

fn claude_first_input_can_send_before_ready(
    session_kind: SessionKind,
    messages: &[ChatMessageRecord],
) -> bool {
    session_kind == SessionKind::Claude && messages.is_empty()
}

fn session_kind_from_provider(provider: &str) -> SessionKind {
    match provider.trim().to_ascii_lowercase().as_str() {
        "claude" | "claude-code" | "claude_code" => SessionKind::Claude,
        _ => SessionKind::Codex,
    }
}

fn session_kind_matches_record(record: &ProcessRecord, kind: SessionKind) -> bool {
    let executable = record
        .command
        .split_whitespace()
        .next()
        .and_then(|command| Path::new(command).file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        (kind, executable.as_str()),
        (SessionKind::Codex, "codex") | (SessionKind::Claude, "claude")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archcar_async::AsyncArchcarRequestKind;
    use archductor_core::workspace::ProcessKind;
    use std::path::PathBuf;

    fn thread(provider: &str) -> ChatThreadRecord {
        ChatThreadRecord {
            id: 7,
            workspace_id: 1,
            provider: provider.to_owned(),
            title: "New Chat".to_owned(),
            status: "active".to_owned(),
            native_thread_id: None,
            harness_metadata: None,
            created_at: "1".to_owned(),
            updated_at: "1".to_owned(),
            archived_at: None,
        }
    }

    fn candidate(running_session_id: Option<i64>, active_work: bool) -> BackgroundQueueCandidate {
        BackgroundQueueCandidate {
            workspace: "berlin".to_owned(),
            branch_prefix: "lc".to_owned(),
            thread: thread("codex"),
            messages: Vec::new(),
            queued_count: 1,
            queued_session_kind: SessionKind::Codex,
            running_session_id,
            active_work,
        }
    }

    fn process(command: &str) -> ProcessRecord {
        ProcessRecord {
            id: 11,
            workspace_id: 1,
            chat_thread_id: Some(7),
            kind: ProcessKind::Session,
            command: command.to_owned(),
            pid: 123,
            log_path: PathBuf::from("/tmp/session.log"),
            status: ProcessStatus::Running,
            started_at: "1".to_owned(),
            exit_code: None,
            ended_at: None,
            session_harness_metadata: None,
            session_resume_id: None,
        }
    }

    #[test]
    fn background_queue_skips_visible_selected_chat() {
        let state = BackgroundChatRunnerState::default();
        let plan = plan_background_queue_drain(
            &candidate(Some(11), false),
            &state,
            Some(7),
            1,
            SessionKind::Codex,
        );

        assert_eq!(plan, None);
    }

    #[test]
    fn background_queue_ensures_hidden_chat_without_running_session() {
        let state = BackgroundChatRunnerState::default();
        let plan = plan_background_queue_drain(
            &candidate(None, false),
            &state,
            None,
            1,
            SessionKind::Codex,
        );

        assert_eq!(plan, Some(BackgroundQueuePlan::Ensure));
    }

    #[test]
    fn background_queue_reasks_archcar_to_drain_running_session() {
        let state = BackgroundChatRunnerState::default();
        let plan = plan_background_queue_drain(
            &candidate(Some(11), false),
            &state,
            None,
            1,
            SessionKind::Codex,
        );

        assert_eq!(plan, Some(BackgroundQueuePlan::Ensure));
    }

    #[test]
    fn background_queue_sends_only_ready_idle_hidden_chat() {
        let mut state = BackgroundChatRunnerState::default();
        state.ready_sessions.insert(11, true);

        let plan = plan_background_queue_drain(
            &candidate(Some(11), false),
            &state,
            None,
            1,
            SessionKind::Codex,
        );

        assert_eq!(plan, Some(BackgroundQueuePlan::Send { session_id: 11 }));
    }

    #[test]
    fn background_queue_waits_when_provider_events_show_active_work() {
        let mut state = BackgroundChatRunnerState::default();
        state.ready_sessions.insert(11, true);

        let plan = plan_background_queue_drain(
            &candidate(Some(11), true),
            &state,
            None,
            1,
            SessionKind::Codex,
        );

        assert_eq!(plan, None);
    }

    #[test]
    fn background_turn_failure_holds_queue() {
        let mut state = BackgroundChatRunnerState::default();
        state.ready_sessions.insert(11, true);

        reduce_background_archcar_event(
            &ArchcarEvent::TurnCompleted {
                session_id: 11,
                thread_id: 7,
                status: Some("failed".to_owned()),
            },
            &mut state,
        );

        assert!(state.held_threads.contains(&7));
        assert_eq!(state.ready_sessions.get(&11), Some(&false));
    }

    #[test]
    fn background_turn_success_releases_queue() {
        let mut state = BackgroundChatRunnerState::default();
        state.held_threads.insert(7);

        reduce_background_archcar_event(
            &ArchcarEvent::TurnCompleted {
                session_id: 11,
                thread_id: 7,
                status: Some("completed".to_owned()),
            },
            &mut state,
        );

        assert!(!state.held_threads.contains(&7));
        assert_eq!(state.ready_sessions.get(&11), Some(&true));
    }

    #[test]
    fn session_kind_matching_uses_command_basename() {
        assert!(session_kind_matches_record(
            &process("/usr/bin/codex --json"),
            SessionKind::Codex
        ));
        assert!(session_kind_matches_record(
            &process("claude --resume abc"),
            SessionKind::Claude
        ));
        assert!(!session_kind_matches_record(
            &process("bash"),
            SessionKind::Codex
        ));
    }

    #[test]
    fn user_send_ack_stays_inflight_until_event_boundary() {
        let app_state = AppState::new(
            archductor_core::paths::AppPaths::from_env(),
            Some("berlin".to_owned()),
            crate::state::WorkspaceTab::Chats,
            crate::state::AppPage::Workspace,
        );
        let state = Rc::new(RefCell::new(BackgroundChatRunnerState::default()));
        state.borrow_mut().inflight_actions.insert(
            9,
            BackgroundChatAction::UserSend {
                workspace: "berlin".to_owned(),
                thread_id: 7,
                session_id: 11,
                draft: QueuedChatInputDraft {
                    id: None,
                    input: "run tests".to_owned(),
                    visible_input: None,
                    kind: ArchcarInputKind::User,
                    session_kind: SessionKind::Codex,
                },
                checkpoint_id: None,
            },
        );

        handle_background_archcar_response(
            &app_state,
            &state,
            AsyncArchcarResponse {
                token: 9,
                request: AsyncArchcarRequestKind::SendInput {
                    session_id: 11,
                    input: "run tests".to_owned(),
                    visible_input: None,
                    kind: ArchcarInputKind::User,
                    delivery: archductor_core::archcar::protocol::ArchcarInputDelivery::Auto,
                },
                result: Ok(ArchcarResponse::Ack),
            },
        );

        assert!(has_inflight_user_send_for_thread(&state.borrow(), 7));
    }

    #[test]
    fn user_send_error_requeues_hidden_input_for_later_background_drain() {
        let app_state = AppState::new(
            archductor_core::paths::AppPaths::from_env(),
            Some("berlin".to_owned()),
            crate::state::WorkspaceTab::Changes,
            crate::state::AppPage::Workspace,
        );
        let state = Rc::new(RefCell::new(BackgroundChatRunnerState::default()));
        let draft = QueuedChatInputDraft {
            id: None,
            input: "run tests".to_owned(),
            visible_input: None,
            kind: ArchcarInputKind::User,
            session_kind: SessionKind::Codex,
        };
        state.borrow_mut().inflight_actions.insert(
            9,
            BackgroundChatAction::UserSend {
                workspace: "berlin".to_owned(),
                thread_id: 7,
                session_id: 11,
                draft,
                checkpoint_id: None,
            },
        );

        handle_background_archcar_response(
            &app_state,
            &state,
            AsyncArchcarResponse {
                token: 9,
                request: AsyncArchcarRequestKind::SendInput {
                    session_id: 11,
                    input: "run tests".to_owned(),
                    visible_input: None,
                    kind: ArchcarInputKind::User,
                    delivery: archductor_core::archcar::protocol::ArchcarInputDelivery::Auto,
                },
                result: Err(
                    "codex session 11 is not ready for automatic input; use immediate delivery"
                        .to_owned(),
                ),
            },
        );

        assert_eq!(app_state.queued_chat_inputs_count(7), 1);
        assert_eq!(app_state.queued_chat_inputs(7)[0].input, "run tests");
        assert!(state.borrow().held_threads.contains(&7));
    }

    #[test]
    fn background_chat_wake_debounce_stays_almost_instant() {
        assert_eq!(
            BACKGROUND_CHAT_WAKE_DELAY_MS, 32,
            "background chat Archcar events should update state with the same frame-ish delay"
        );
    }

    #[test]
    fn background_chat_does_not_probe_archcar_readiness_from_gtk() {
        let source = include_str!("background_chat.rs");
        let status_probe = concat!("Status", "Probe");
        let probe_status = concat!("Probe", "Status");
        let get_status = concat!("get", "_session", "_status(");

        assert!(
            !source.contains(status_probe),
            "GTK background chat must not own Archcar readiness probing"
        );
        assert!(
            !source.contains(probe_status),
            "GTK background chat must not plan Archcar readiness probes"
        );
        assert!(
            !source.contains(get_status),
            "GTK background chat must not call Archcar GetSessionStatus"
        );
    }
}
