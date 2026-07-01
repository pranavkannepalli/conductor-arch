use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

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
    SendInput {
        session_id: i64,
        input: String,
        kind: ArchcarInputKind,
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

#[derive(Clone)]
pub struct AsyncArchcarBridge {
    request_tx: Sender<AsyncArchcarRequestEnvelope>,
    message_rx: Arc<Mutex<Receiver<AsyncArchcarMessage>>>,
    next_token: Arc<AtomicU64>,
}

impl AsyncArchcarBridge {
    pub fn new(paths: AppPaths) -> Self {
        let (request_tx, request_rx) = mpsc::channel();
        let (message_tx, message_rx) = mpsc::channel();
        thread::spawn(move || run_archcar_bridge(paths, request_rx, message_tx));
        Self {
            request_tx,
            message_rx: Arc::new(Mutex::new(message_rx)),
            next_token: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn ensure_default_session(&self, workspace: String, kind: SessionKind) -> Option<u64> {
        self.submit(ArchcarRequest::EnsureWorkspaceDefaultSession {
            workspace,
            kind,
            harness: None,
        })
    }

    pub fn get_session_status(&self, session_id: i64) -> Option<u64> {
        self.submit(ArchcarRequest::GetSessionStatus { session_id })
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

fn run_archcar_bridge(
    paths: AppPaths,
    request_rx: Receiver<AsyncArchcarRequestEnvelope>,
    message_tx: Sender<AsyncArchcarMessage>,
) {
    let client = ArchcarClient::from_paths(&paths);
    let mut event_rx = None;
    let mut last_subscribe_attempt = Instant::now()
        .checked_sub(Duration::from_secs(60))
        .unwrap_or_else(Instant::now);

    loop {
        if event_rx.is_none() && last_subscribe_attempt.elapsed() >= Duration::from_secs(1) {
            last_subscribe_attempt = Instant::now();
            match client.subscribe() {
                Ok(rx) => {
                    info!("async archcar bridge subscribed to sidecar events");
                    event_rx = Some(rx);
                }
                Err(err) => {
                    let _ = message_tx.send(AsyncArchcarMessage::BridgeError {
                        message: format!("subscribe archcar events failed: {err:#}"),
                    });
                }
            }
        }

        if let Some(events) = event_rx.as_mut() {
            while let Ok(event) = events.try_recv() {
                if message_tx.send(AsyncArchcarMessage::Event(event)).is_err() {
                    return;
                }
            }
        }

        match request_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(envelope) => {
                let request_kind = request_kind(&envelope.request);
                let result = client.send(envelope.request).map_err(|err| err.to_string());
                if message_tx
                    .send(AsyncArchcarMessage::Response(AsyncArchcarResponse {
                        token: envelope.token,
                        request: request_kind,
                        result,
                    }))
                    .is_err()
                {
                    return;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn request_kind(request: &ArchcarRequest) -> AsyncArchcarRequestKind {
    match request {
        ArchcarRequest::EnsureWorkspaceDefaultSession {
            workspace, kind, ..
        } => AsyncArchcarRequestKind::EnsureWorkspaceDefaultSession {
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
        ArchcarRequest::GetSessionStatus { session_id } => {
            AsyncArchcarRequestKind::GetSessionStatus {
                session_id: *session_id,
            }
        }
        ArchcarRequest::KillSession { session_id } => AsyncArchcarRequestKind::KillSession {
            session_id: *session_id,
        },
        ArchcarRequest::SpawnSession {
            workspace, kind, ..
        } => AsyncArchcarRequestKind::EnsureWorkspaceDefaultSession {
            workspace: workspace.clone(),
            kind: *kind,
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
}
