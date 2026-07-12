use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use gtk::glib;
use linux_archductor_core::archcar::client::ArchcarClient;
use linux_archductor_core::archcar::protocol::{
    ArchcarEvent, ArchcarInputKind, ArchcarRequest, ArchcarResponse,
};
use linux_archductor_core::paths::AppPaths;
use linux_archductor_core::workspace::SessionKind;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsyncArchcarRequestKind {
    EnsureWorkspaceDefaultSession {
        workspace: String,
        kind: SessionKind,
    },
    EnsureChatThreadSession {
        workspace: String,
        thread_id: i64,
        kind: SessionKind,
    },
    SpawnSession {
        workspace: String,
        kind: SessionKind,
    },
    SendInput {
        session_id: i64,
        input: String,
        kind: ArchcarInputKind,
    },
    ResizeSession {
        session_id: i64,
        rows: u16,
        cols: u16,
    },
    GetSessionStatus {
        session_id: i64,
    },
    KillSession {
        session_id: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsyncArchcarResponse {
    pub token: u64,
    pub request: AsyncArchcarRequestKind,
    pub result: Result<ArchcarResponse, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsyncArchcarMessage {
    Event(ArchcarEvent),
    Response(AsyncArchcarResponse),
    BridgeError { message: String },
}

#[derive(Debug)]
struct AsyncArchcarRequestEnvelope {
    token: u64,
    request: ArchcarRequest,
}

type BridgeWake = Arc<dyn Fn() + Send + Sync + 'static>;
type BridgeWakeSlot = Arc<Mutex<Option<BridgeWake>>>;
type BridgeConnectLock = Arc<Mutex<()>>;
type BackgroundJobCallback = Box<dyn FnOnce(Box<dyn Any>) + 'static>;
type BackgroundProgressCallback = Box<dyn Fn() + 'static>;

static NEXT_BACKGROUND_JOB_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static BACKGROUND_JOB_CALLBACKS: RefCell<HashMap<u64, BackgroundJobCallback>> =
        RefCell::new(HashMap::new());
    static BACKGROUND_JOB_PROGRESS_CALLBACKS: RefCell<HashMap<u64, BackgroundProgressCallback>> =
        RefCell::new(HashMap::new());
}

#[derive(Clone)]
pub struct AsyncArchcarBridge {
    request_tx: Sender<AsyncArchcarRequestEnvelope>,
    message_rx: Arc<Mutex<Receiver<AsyncArchcarMessage>>>,
    next_token: Arc<AtomicU64>,
    wake: BridgeWakeSlot,
    _shutdown: Arc<BridgeShutdown>,
}

struct BridgeShutdown {
    tx: Sender<()>,
}

impl Drop for BridgeShutdown {
    fn drop(&mut self) {
        let _ = self.tx.send(());
    }
}

impl AsyncArchcarBridge {
    pub fn new(paths: AppPaths) -> Self {
        let (request_tx, request_rx) = mpsc::channel();
        let (message_tx, message_rx) = mpsc::channel();
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let wake = Arc::new(Mutex::new(None));
        let connect_lock = Arc::new(Mutex::new(()));
        thread::spawn({
            let paths = paths.clone();
            let message_tx = message_tx.clone();
            let wake = wake.clone();
            let connect_lock = connect_lock.clone();
            move || run_archcar_request_bridge(paths, request_rx, message_tx, wake, connect_lock)
        });
        thread::spawn({
            let wake = wake.clone();
            move || run_archcar_event_bridge(paths, message_tx, wake, connect_lock, shutdown_rx)
        });
        Self {
            request_tx,
            message_rx: Arc::new(Mutex::new(message_rx)),
            next_token: Arc::new(AtomicU64::new(1)),
            wake,
            _shutdown: Arc::new(BridgeShutdown { tx: shutdown_tx }),
        }
    }

    pub fn set_waker(&self, wake: impl Fn() + Send + Sync + 'static) {
        if let Ok(mut slot) = self.wake.lock() {
            *slot = Some(Arc::new(wake));
        }
    }

    pub fn ensure_default_session(&self, workspace: String, kind: SessionKind) -> Option<u64> {
        self.submit(ArchcarRequest::EnsureWorkspaceDefaultSession {
            workspace,
            kind,
            harness: None,
        })
    }

    pub fn ensure_thread_session(
        &self,
        workspace: String,
        thread_id: i64,
        kind: SessionKind,
    ) -> Option<u64> {
        self.submit(ArchcarRequest::EnsureChatThreadSession {
            workspace,
            thread_id,
            kind,
            harness: None,
        })
    }

    pub fn spawn_session(&self, workspace: String, kind: SessionKind) -> Option<u64> {
        self.submit(ArchcarRequest::SpawnSession {
            workspace,
            kind,
            harness: None,
        })
    }

    pub fn send_input(
        &self,
        session_id: i64,
        input: String,
        kind: ArchcarInputKind,
    ) -> Option<u64> {
        self.submit(ArchcarRequest::SendInput {
            session_id,
            input,
            kind,
        })
    }

    pub fn kill_session(&self, session_id: i64) -> Option<u64> {
        self.submit(ArchcarRequest::KillSession { session_id })
    }

    pub fn resize_session(&self, session_id: i64, rows: u16, cols: u16) -> Option<u64> {
        self.submit(ArchcarRequest::ResizeSession {
            session_id,
            rows,
            cols,
        })
    }

    pub fn try_recv(&self) -> Option<AsyncArchcarMessage> {
        let Ok(rx) = self.message_rx.lock() else {
            return Some(AsyncArchcarMessage::BridgeError {
                message: "async archcar message receiver lock poisoned".to_owned(),
            });
        };
        match rx.try_recv() {
            Ok(message) => Some(message),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => None,
        }
    }

    fn submit(&self, request: ArchcarRequest) -> Option<u64> {
        let token = self.next_token.fetch_add(1, Ordering::Relaxed);
        self.request_tx
            .send(AsyncArchcarRequestEnvelope { token, request })
            .ok()?;
        Some(token)
    }
}

pub fn spawn_archcar_request(paths: AppPaths, request: ArchcarRequest) {
    thread::spawn(move || {
        let client = ArchcarClient::from_paths(&paths);
        let request_kind = request_kind(&request);
        match client.send(request) {
            Ok(response) => {
                debug!(
                    ?request_kind,
                    ?response,
                    "async archcar fire-and-forget request completed"
                );
            }
            Err(err) => {
                warn!(?request_kind, error = %err, "async archcar fire-and-forget request failed");
            }
        }
    });
}

pub(crate) fn spawn_background_job<F, T, C>(job: F, on_complete: C)
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
    C: FnOnce(T) + 'static,
{
    spawn_background_job_with_progress(move |_| job(), || {}, on_complete);
}

pub(crate) fn spawn_background_job_with_progress<F, T, P, C>(job: F, on_progress: P, on_complete: C)
where
    F: FnOnce(Box<dyn Fn() + Send>) -> T + Send + 'static,
    T: Send + 'static,
    P: Fn() + 'static,
    C: FnOnce(T) + 'static,
{
    let id = NEXT_BACKGROUND_JOB_ID.fetch_add(1, Ordering::Relaxed);
    BACKGROUND_JOB_CALLBACKS.with(|callbacks| {
        callbacks.borrow_mut().insert(
            id,
            Box::new(move |payload| {
                if let Ok(result) = payload.downcast::<T>() {
                    on_complete(*result);
                }
            }),
        );
    });
    BACKGROUND_JOB_PROGRESS_CALLBACKS.with(|callbacks| {
        callbacks.borrow_mut().insert(id, Box::new(on_progress));
    });
    let main_context = glib::MainContext::default();
    thread::spawn(move || {
        let progress_context = main_context.clone();
        let progress = Box::new(move || {
            progress_context.invoke(move || {
                BACKGROUND_JOB_PROGRESS_CALLBACKS.with(|callbacks| {
                    if let Some(callback) = callbacks.borrow().get(&id) {
                        callback();
                    }
                });
            });
        });
        let result = job(progress);
        main_context.invoke(move || {
            BACKGROUND_JOB_PROGRESS_CALLBACKS.with(|callbacks| {
                callbacks.borrow_mut().remove(&id);
            });
            BACKGROUND_JOB_CALLBACKS.with(|callbacks| {
                if let Some(callback) = callbacks.borrow_mut().remove(&id) {
                    callback(Box::new(result));
                }
            });
        });
    });
}

fn run_archcar_request_bridge(
    paths: AppPaths,
    request_rx: Receiver<AsyncArchcarRequestEnvelope>,
    message_tx: Sender<AsyncArchcarMessage>,
    wake: BridgeWakeSlot,
    connect_lock: BridgeConnectLock,
) {
    let client = ArchcarClient::from_paths(&paths);
    for envelope in request_rx {
        let request_kind = request_kind(&envelope.request);
        let result = match connect_lock.lock() {
            Ok(_guard) => client.send(envelope.request).map_err(|err| err.to_string()),
            Err(_) => Err("async archcar connect lock poisoned".to_owned()),
        };
        if !send_bridge_message(
            &message_tx,
            &wake,
            AsyncArchcarMessage::Response(AsyncArchcarResponse {
                token: envelope.token,
                request: request_kind,
                result,
            }),
        ) {
            return;
        }
    }
}

fn run_archcar_event_bridge(
    paths: AppPaths,
    message_tx: Sender<AsyncArchcarMessage>,
    wake: BridgeWakeSlot,
    connect_lock: BridgeConnectLock,
    shutdown_rx: Receiver<()>,
) {
    let client = ArchcarClient::from_paths(&paths);
    run_archcar_event_loop(
        message_tx,
        wake,
        connect_lock,
        shutdown_rx,
        || client.subscribe(),
        Duration::from_millis(500),
    );
}

fn run_archcar_event_loop<F, E>(
    message_tx: Sender<AsyncArchcarMessage>,
    wake: BridgeWakeSlot,
    connect_lock: BridgeConnectLock,
    shutdown_rx: Receiver<()>,
    mut subscribe: F,
    reconnect_delay: Duration,
) where
    F: FnMut() -> Result<Receiver<ArchcarEvent>, E>,
    E: std::fmt::Display,
{
    loop {
        if shutdown_rx.try_recv().is_ok() {
            return;
        }
        let subscribe_result = match connect_lock.lock() {
            Ok(_guard) => subscribe(),
            Err(_) => {
                let _ = send_bridge_message(
                    &message_tx,
                    &wake,
                    AsyncArchcarMessage::BridgeError {
                        message: "async archcar connect lock poisoned".to_owned(),
                    },
                );
                return;
            }
        };

        match subscribe_result {
            Ok(rx) => {
                info!("async archcar bridge subscribed to sidecar events");
                loop {
                    if shutdown_rx.try_recv().is_ok() {
                        return;
                    }
                    match rx.recv_timeout(Duration::from_millis(100)) {
                        Ok(event) => {
                            if !send_bridge_message(
                                &message_tx,
                                &wake,
                                AsyncArchcarMessage::Event(event),
                            ) {
                                return;
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                warn!("async archcar event subscription closed; reconnecting");
            }
            Err(err) => {
                if !send_bridge_message(
                    &message_tx,
                    &wake,
                    AsyncArchcarMessage::BridgeError {
                        message: format!("subscribe archcar events failed: {err}"),
                    },
                ) {
                    return;
                }
                warn!(error = %err, "async archcar event subscribe failed; retrying");
            }
        }
        if shutdown_rx.recv_timeout(reconnect_delay).is_ok() {
            return;
        }
    }
}

fn send_bridge_message(
    message_tx: &Sender<AsyncArchcarMessage>,
    wake: &BridgeWakeSlot,
    message: AsyncArchcarMessage,
) -> bool {
    if message_tx.send(message).is_err() {
        return false;
    }
    let wake = wake.lock().ok().and_then(|slot| slot.clone());
    if let Some(wake) = wake {
        wake();
    }
    true
}

fn request_kind(request: &ArchcarRequest) -> AsyncArchcarRequestKind {
    match request {
        ArchcarRequest::EnsureWorkspaceDefaultSession {
            workspace, kind, ..
        } => AsyncArchcarRequestKind::EnsureWorkspaceDefaultSession {
            workspace: workspace.clone(),
            kind: *kind,
        },
        ArchcarRequest::EnsureChatThreadSession {
            workspace,
            thread_id,
            kind,
            ..
        } => AsyncArchcarRequestKind::EnsureChatThreadSession {
            workspace: workspace.clone(),
            thread_id: *thread_id,
            kind: *kind,
        },
        ArchcarRequest::SpawnSession {
            workspace, kind, ..
        } => AsyncArchcarRequestKind::SpawnSession {
            workspace: workspace.clone(),
            kind: *kind,
        },
        ArchcarRequest::SendInput {
            session_id,
            input,
            kind,
        } => AsyncArchcarRequestKind::SendInput {
            session_id: *session_id,
            input: input.clone(),
            kind: kind.clone(),
        },
        ArchcarRequest::ResizeSession {
            session_id,
            rows,
            cols,
        } => AsyncArchcarRequestKind::ResizeSession {
            session_id: *session_id,
            rows: *rows,
            cols: *cols,
        },
        ArchcarRequest::GetSessionStatus { session_id } => {
            AsyncArchcarRequestKind::GetSessionStatus {
                session_id: *session_id,
            }
        }
        ArchcarRequest::KillSession { session_id } => AsyncArchcarRequestKind::KillSession {
            session_id: *session_id,
        },
        ArchcarRequest::GetSessionScreen { session_id } => {
            AsyncArchcarRequestKind::GetSessionStatus {
                session_id: *session_id,
            }
        }
        ArchcarRequest::GetSessionMessages { thread_id } => {
            AsyncArchcarRequestKind::GetSessionStatus {
                session_id: *thread_id,
            }
        }
        ArchcarRequest::Subscribe => AsyncArchcarRequestKind::GetSessionStatus { session_id: -1 },
    }
}

pub fn note_archcar_ready(cache: &mut HashMap<i64, bool>, session_id: i64, ready: bool) {
    cache.insert(session_id, ready);
}

pub fn clear_archcar_ready(cache: &mut HashMap<i64, bool>, session_id: i64) {
    cache.remove(&session_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_cache_updates_and_clears_sessions() {
        let mut cache = HashMap::new();

        note_archcar_ready(&mut cache, 7, false);
        assert_eq!(cache.get(&7), Some(&false));

        note_archcar_ready(&mut cache, 7, true);
        assert_eq!(cache.get(&7), Some(&true));

        clear_archcar_ready(&mut cache, 7);
        assert!(!cache.contains_key(&7));
    }

    #[test]
    fn ready_cache_rebuilds_after_gtk_restart_reconnect() {
        let mut before_restart = HashMap::new();
        note_archcar_ready(&mut before_restart, 7, true);

        let mut after_restart = HashMap::new();
        note_archcar_ready(&mut after_restart, 8, false);
        note_archcar_ready(&mut after_restart, 8, true);

        assert_eq!(before_restart.get(&7), Some(&true));
        assert!(!after_restart.contains_key(&7));
        assert_eq!(after_restart.get(&8), Some(&true));
    }

    #[test]
    fn request_kind_preserves_send_input_metadata() {
        let request = ArchcarRequest::SendInput {
            session_id: 9,
            input: "hello".to_owned(),
            kind: ArchcarInputKind::ReviewPrompt,
        };

        assert_eq!(
            request_kind(&request),
            AsyncArchcarRequestKind::SendInput {
                session_id: 9,
                input: "hello".to_owned(),
                kind: ArchcarInputKind::ReviewPrompt,
            }
        );
    }

    #[test]
    fn async_bridge_request_kinds_do_not_expose_subscribe_as_user_action() {
        assert_eq!(
            request_kind(&ArchcarRequest::Subscribe),
            AsyncArchcarRequestKind::GetSessionStatus { session_id: -1 }
        );
    }

    #[test]
    fn event_bridge_does_not_resubscribe_while_subscription_is_idle() {
        let (event_tx, event_rx) = mpsc::channel();
        let (message_tx, message_rx) = mpsc::channel();
        let wake = Arc::new(Mutex::new(None));
        let connect_lock = Arc::new(Mutex::new(()));
        let (_shutdown_tx, shutdown_rx) = mpsc::channel();
        let subscribe_count = Arc::new(AtomicU64::new(0));
        let count_for_subscribe = Arc::clone(&subscribe_count);
        let mut next_rx = Some(event_rx);

        let handle = thread::spawn(move || {
            run_archcar_event_loop(
                message_tx,
                wake,
                connect_lock,
                shutdown_rx,
                move || {
                    count_for_subscribe.fetch_add(1, Ordering::SeqCst);
                    next_rx
                        .take()
                        .ok_or_else(|| "subscription already consumed".to_owned())
                },
                Duration::from_millis(10),
            );
        });

        for _ in 0..20 {
            if subscribe_count.load(Ordering::SeqCst) == 1 {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(subscribe_count.load(Ordering::SeqCst), 1);

        thread::sleep(Duration::from_millis(50));
        assert_eq!(subscribe_count.load(Ordering::SeqCst), 1);

        drop(event_tx);
        drop(message_rx);
        handle.join().expect("event bridge loop exits");
    }
}
