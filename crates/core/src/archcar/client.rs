use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tracing::{info, warn};
use uuid::Uuid;

use crate::archcar::protocol::{
    archcar_event_summary, archcar_request_summary, archcar_response_summary, ArchcarEvent,
    ArchcarRequest, ArchcarResponse, RpcEnvelope,
};
use crate::archcar::transport::{self, LocalStream};
use crate::paths::AppPaths;

const ARCHCAR_HEALTHCHECK_TIMEOUT: Duration = Duration::from_millis(750);
const ARCHCAR_RPC_TIMEOUT: Duration = Duration::from_secs(30);
const ARCHCAR_STARTUP_ATTEMPTS: usize = 20;
const ARCHCAR_STARTUP_POLL_INTERVAL: Duration = Duration::from_millis(100);
const ARCHCAR_VALIDATION_CACHE_TTL: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub struct ArchcarClient {
    endpoint_path: PathBuf,
    last_validated_at: Arc<Mutex<Option<Instant>>>,
}

impl ArchcarClient {
    pub fn from_paths(paths: &AppPaths) -> Self {
        Self::new(paths.archcar_endpoint_path())
    }

    fn new(endpoint_path: PathBuf) -> Self {
        Self {
            endpoint_path,
            last_validated_at: Arc::new(Mutex::new(None)),
        }
    }

    pub fn send(&self, request: ArchcarRequest) -> Result<ArchcarResponse> {
        let retryable = request_retry_safe_after_response_loss(&request);
        match self.send_once(request.clone()) {
            Ok(response) => Ok(response),
            Err(err) if retryable && response_decode_or_eof_error(&err) => {
                warn!(
                    endpoint = %self.endpoint_path.display(),
                    error = %err,
                    "archcar response decode failed; retrying idempotent request"
                );
                self.send_once(request)
            }
            Err(err) => Err(err),
        }
    }

    pub fn send_without_spawning(&self, request: ArchcarRequest) -> Result<ArchcarResponse> {
        let stream = self.connect_validated()?;
        self.send_on_stream(stream, request)
    }

    fn send_once(&self, request: ArchcarRequest) -> Result<ArchcarResponse> {
        let stream = self.connect_or_spawn()?;
        self.send_on_stream(stream, request)
    }

    fn send_on_stream(
        &self,
        mut stream: LocalStream,
        request: ArchcarRequest,
    ) -> Result<ArchcarResponse> {
        configure_rpc_timeouts(&stream, ARCHCAR_RPC_TIMEOUT)?;
        let request_summary = archcar_request_summary(&request);
        let envelope = RpcEnvelope {
            id: Uuid::new_v4().to_string(),
            payload: request,
        };
        let line = serde_json::to_string(&envelope)?;
        log_archcar_rpc(
            &self.endpoint_path,
            &envelope.id,
            "send",
            "request",
            request_summary,
            &line,
        );
        stream.write_all(line.as_bytes())?;
        stream.write_all(b"\n")?;
        stream.flush()?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .context("read archcar sidecar response")?;
        anyhow::ensure!(
            !line.trim().is_empty(),
            "empty response from archcar sidecar"
        );
        let response: RpcEnvelope<ArchcarResponse> = serde_json::from_str(&line)?;
        log_archcar_rpc(
            &self.endpoint_path,
            &response.id,
            "recv",
            "response",
            archcar_response_summary(&response.payload),
            line.trim_end(),
        );
        Ok(response.payload)
    }

    pub fn subscribe(&self) -> Result<std::sync::mpsc::Receiver<ArchcarEvent>> {
        let mut stream = self.connect_or_spawn()?;
        configure_write_timeout(&stream, ARCHCAR_RPC_TIMEOUT)?;
        let envelope = RpcEnvelope {
            id: Uuid::new_v4().to_string(),
            payload: ArchcarRequest::Subscribe,
        };
        let line = serde_json::to_string(&envelope)?;
        log_archcar_rpc(
            &self.endpoint_path,
            &envelope.id,
            "send",
            "request",
            archcar_request_summary(&ArchcarRequest::Subscribe),
            &line,
        );
        stream.write_all(line.as_bytes())?;
        stream.write_all(b"\n")?;
        stream.flush()?;
        clear_rpc_timeouts(&stream)?;
        let (tx, rx) = std::sync::mpsc::channel();
        let endpoint_path = self.endpoint_path.clone();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stream);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => match serde_json::from_str::<RpcEnvelope<ArchcarEvent>>(&line) {
                        Ok(event) => {
                            log_archcar_rpc(
                                &endpoint_path,
                                &event.id,
                                "recv",
                                "event",
                                archcar_event_summary(&event.payload),
                                line.trim_end(),
                            );
                            let _ = tx.send(event.payload);
                        }
                        Err(err) => {
                            warn!(
                                endpoint = %endpoint_path.display(),
                                error = %err,
                                bytes = line.len(),
                                "archcar local rpc event decode failed"
                            );
                        }
                    },
                    Err(_) => break,
                }
            }
        });
        Ok(rx)
    }

    fn connect_or_spawn(&self) -> Result<LocalStream> {
        match self.connect_validated() {
            Ok(stream) => Ok(stream),
            Err(first_err) => {
                warn!(
                    endpoint = %self.endpoint_path.display(),
                    error = %first_err,
                    "archcar endpoint was not responsive; spawning sidecar"
                );
                remove_stale_endpoint(&self.endpoint_path);
                self.spawn_sidecar()?;
                for _ in 0..ARCHCAR_STARTUP_ATTEMPTS {
                    match self.connect_validated() {
                        Ok(stream) => return Ok(stream),
                        Err(_) => thread::sleep(ARCHCAR_STARTUP_POLL_INTERVAL),
                    }
                }
                Err(first_err)
                    .with_context(|| format!("connect archcar {}", self.endpoint_path.display()))
            }
        }
    }

    fn connect_validated(&self) -> Result<LocalStream> {
        let stream = match transport::connect(&self.endpoint_path) {
            Ok(stream) => stream,
            Err(err) => {
                self.clear_validation_cache();
                return Err(err.into());
            }
        };
        if self.validation_cache_is_fresh() {
            return Ok(stream);
        }
        if let Err(err) = validate_sidecar_responsive(&self.endpoint_path, stream) {
            self.clear_validation_cache();
            return Err(err);
        }
        self.mark_sidecar_validated();
        match transport::connect(&self.endpoint_path) {
            Ok(stream) => Ok(stream),
            Err(err) => {
                self.clear_validation_cache();
                Err(err.into())
            }
        }
    }

    fn spawn_sidecar(&self) -> Result<()> {
        let current_exe = std::env::current_exe().ok();
        let sibling = current_exe
            .as_ref()
            .map(|path| path.with_file_name("archcar"));
        let explicit = std::env::var_os("ARCHDUCTOR_ARCHCAR_BIN").map(PathBuf::from);
        let mut last_err = None;
        for (candidate, args) in explicit
            .into_iter()
            .map(|path| (path, Vec::<&str>::new()))
            .chain(
                current_exe
                    .clone()
                    .into_iter()
                    .map(|path| (path, vec!["--archcar-serve"])),
            )
            .chain(sibling.into_iter().map(|path| (path, Vec::<&str>::new())))
            .chain(std::iter::once((
                PathBuf::from("archcar"),
                Vec::<&str>::new(),
            )))
        {
            match Command::new(&candidate)
                .args(&args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(_) => return Ok(()),
                Err(err) => last_err = Some((candidate, err)),
            }
        }
        let (candidate, err) = last_err.context("no archcar binary candidate available")?;
        Err(err).with_context(|| format!("spawn archcar binary {}", candidate.display()))
    }

    fn validation_cache_is_fresh(&self) -> bool {
        self.last_validated_at
            .lock()
            .ok()
            .and_then(|validated| *validated)
            .is_some_and(|validated| validated.elapsed() <= ARCHCAR_VALIDATION_CACHE_TTL)
    }

    fn mark_sidecar_validated(&self) {
        if let Ok(mut validated) = self.last_validated_at.lock() {
            *validated = Some(Instant::now());
        }
    }

    fn clear_validation_cache(&self) {
        if let Ok(mut validated) = self.last_validated_at.lock() {
            *validated = None;
        }
    }
}

fn validate_sidecar_responsive(endpoint_path: &Path, mut stream: LocalStream) -> Result<()> {
    configure_rpc_timeouts(&stream, ARCHCAR_HEALTHCHECK_TIMEOUT)?;
    let request = ArchcarRequest::ListProviderInteractions {
        thread_id: None,
        pending_only: true,
    };
    let envelope = RpcEnvelope {
        id: Uuid::new_v4().to_string(),
        payload: request,
    };
    let line = serde_json::to_string(&envelope)?;
    log_archcar_rpc(
        endpoint_path,
        &envelope.id,
        "send",
        "healthcheck",
        archcar_request_summary(&envelope.payload),
        &line,
    );
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .context("read archcar sidecar healthcheck response")?;
    anyhow::ensure!(
        !line.trim().is_empty(),
        "empty response from archcar sidecar healthcheck"
    );
    let response: RpcEnvelope<ArchcarResponse> = serde_json::from_str(&line)?;
    log_archcar_rpc(
        endpoint_path,
        &response.id,
        "recv",
        "healthcheck",
        archcar_response_summary(&response.payload),
        line.trim_end(),
    );
    Ok(())
}

fn configure_rpc_timeouts(stream: &LocalStream, timeout: Duration) -> Result<()> {
    stream
        .set_read_timeout(Some(timeout))
        .context("set archcar read timeout")?;
    configure_write_timeout(stream, timeout)
}

fn configure_write_timeout(stream: &LocalStream, timeout: Duration) -> Result<()> {
    stream
        .set_write_timeout(Some(timeout))
        .context("set archcar write timeout")?;
    Ok(())
}

fn clear_rpc_timeouts(stream: &LocalStream) -> Result<()> {
    stream
        .set_read_timeout(None)
        .context("clear archcar read timeout")?;
    stream
        .set_write_timeout(None)
        .context("clear archcar write timeout")?;
    Ok(())
}

fn remove_stale_endpoint(endpoint_path: &Path) {
    if let Err(err) = std::fs::remove_file(endpoint_path) {
        if err.kind() != std::io::ErrorKind::NotFound {
            warn!(
                endpoint = %endpoint_path.display(),
                error = %err,
                "failed to remove stale archcar endpoint"
            );
        }
    }
}

fn response_decode_or_eof_error(err: &anyhow::Error) -> bool {
    err.to_string()
        .contains("empty response from archcar sidecar")
        || err.to_string().contains("read archcar sidecar response")
            && err
                .downcast_ref::<std::io::Error>()
                .is_some_and(|err| err.kind() == std::io::ErrorKind::TimedOut)
        || err
            .downcast_ref::<serde_json::Error>()
            .is_some_and(serde_json::Error::is_eof)
}

fn request_retry_safe_after_response_loss(request: &ArchcarRequest) -> bool {
    matches!(
        request,
        ArchcarRequest::GetSessionStatus { .. }
            | ArchcarRequest::GetSessionScreen { .. }
            | ArchcarRequest::GetSessionMessages { .. }
            | ArchcarRequest::ResizeSession { .. }
            | ArchcarRequest::SetSessionModel { .. }
            | ArchcarRequest::SetSessionEffort { .. }
            | ArchcarRequest::SetSessionPermissionMode { .. }
    )
}

fn log_archcar_rpc(
    endpoint_path: &Path,
    rpc_id: &str,
    direction: &str,
    message_type: &str,
    summary: String,
    raw_payload: &str,
) {
    if let Some(payload) = archcar_rpc_log_payload(raw_payload) {
        info!(
            endpoint = %endpoint_path.display(),
            %rpc_id,
            direction,
            message_type,
            summary = %summary,
            payload = %payload,
            "archcar local rpc"
        );
    } else {
        info!(
            endpoint = %endpoint_path.display(),
            %rpc_id,
            direction,
            message_type,
            summary = %summary,
            "archcar local rpc"
        );
    }
}

fn archcar_rpc_log_payload(raw_payload: &str) -> Option<String> {
    archcar_rpc_log_payload_for_flag(
        raw_payload,
        crate::env_flags::enabled("ARCHDUCTOR_LOG_ARCHCAR_PAYLOADS"),
    )
}

fn archcar_rpc_log_payload_for_flag(raw_payload: &str, enabled: bool) -> Option<String> {
    enabled.then(|| crate::redaction::redact_sensitive_text(raw_payload))
}

#[cfg(test)]
mod tests {
    use super::{
        archcar_rpc_log_payload_for_flag, request_retry_safe_after_response_loss,
        response_decode_or_eof_error,
    };
    use crate::archcar::protocol::{
        ArchcarInputDelivery, ArchcarInputKind, ArchcarRequest, RpcEnvelope,
    };
    use std::io::{BufRead, BufReader, Write};
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    };
    use std::time::Duration;

    #[test]
    fn client_rpc_log_payload_redacts_sensitive_values_when_payload_logging_is_enabled() {
        let envelope = RpcEnvelope {
            id: "abc".to_owned(),
            payload: ArchcarRequest::SendInput {
                session_id: 42,
                input: "OPENAI_API_KEY=sk-secret bearer ghp_secret --password swordfish".to_owned(),
                visible_input: None,
                kind: ArchcarInputKind::User,
                delivery: ArchcarInputDelivery::Auto,
            },
        };
        let line = serde_json::to_string(&envelope).unwrap();

        let payload = archcar_rpc_log_payload_for_flag(&line, true).unwrap();

        assert!(payload.contains("[redacted]"));
        assert!(!payload.contains("sk-secret"));
        assert!(!payload.contains("ghp_secret"));
        assert!(!payload.contains("swordfish"));
    }

    #[cfg(unix)]
    #[test]
    fn client_returns_when_endpoint_accepts_but_never_answers() {
        let temp = tempfile::tempdir().unwrap();
        let endpoint = temp.path().join("archcar-stopped.sock");
        let listener = crate::archcar::transport::bind(&endpoint).unwrap();
        let (accepted_tx, accepted_rx) = mpsc::channel();
        let server_endpoint = endpoint.clone();
        let _server = std::thread::spawn(move || {
            let (stream, _) =
                crate::archcar::transport::accept(&listener, &server_endpoint).unwrap();
            accepted_tx.send(()).unwrap();
            let _stream = stream;
            std::thread::sleep(Duration::from_secs(10));
        });

        let _env_guard = archcar_bin_env_lock().lock().unwrap();
        let previous_archcar_bin = std::env::var_os("ARCHDUCTOR_ARCHCAR_BIN");
        std::env::set_var("ARCHDUCTOR_ARCHCAR_BIN", "/bin/false");
        let client = super::ArchcarClient::new(endpoint);
        let (tx, rx) = mpsc::channel();
        let _client_thread = std::thread::spawn(move || {
            let result = client.send(ArchcarRequest::GetSessionMessages { thread_id: 1 });
            let _ = tx.send(result);
        });

        accepted_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("client did not connect to test endpoint");
        let result = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("client hung on an unresponsive archcar endpoint");
        assert!(result.is_err());

        if let Some(value) = previous_archcar_bin {
            std::env::set_var("ARCHDUCTOR_ARCHCAR_BIN", value);
        } else {
            std::env::remove_var("ARCHDUCTOR_ARCHCAR_BIN");
        }
    }

    fn archcar_bin_env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[cfg(unix)]
    #[test]
    fn client_reuses_recent_sidecar_validation() {
        let temp = tempfile::tempdir().unwrap();
        let endpoint = temp.path().join("archcar-responsive.sock");
        let listener = crate::archcar::transport::bind(&endpoint).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let healthchecks = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(AtomicUsize::new(0));
        let server_endpoint = endpoint.clone();
        let server_stop = stop.clone();
        let server_healthchecks = healthchecks.clone();
        let server_requests = requests.clone();
        let server = std::thread::spawn(move || {
            while !server_stop.load(Ordering::SeqCst) {
                let (mut stream, _) =
                    match crate::archcar::transport::accept(&listener, &server_endpoint) {
                        Ok(stream) => stream,
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(10));
                            continue;
                        }
                        Err(err) => panic!("accept archcar test connection: {err}"),
                    };
                let mut line = String::new();
                {
                    let mut reader = BufReader::new(&mut stream);
                    reader.read_line(&mut line).unwrap();
                }
                let request: RpcEnvelope<ArchcarRequest> = serde_json::from_str(&line).unwrap();
                let response = match request.payload {
                    ArchcarRequest::ListProviderInteractions { .. } => {
                        server_healthchecks.fetch_add(1, Ordering::SeqCst);
                        super::ArchcarResponse::ProviderInteractions {
                            interactions: Vec::new(),
                        }
                    }
                    ArchcarRequest::GetSessionMessages { thread_id } => {
                        server_requests.fetch_add(1, Ordering::SeqCst);
                        super::ArchcarResponse::SessionMessages {
                            thread_id,
                            messages: Vec::new(),
                        }
                    }
                    other => panic!("unexpected archcar test request: {other:?}"),
                };
                let response = RpcEnvelope {
                    id: request.id,
                    payload: response,
                };
                let line = serde_json::to_string(&response).unwrap();
                stream.write_all(line.as_bytes()).unwrap();
                stream.write_all(b"\n").unwrap();
                stream.flush().unwrap();
            }
        });

        let client = super::ArchcarClient::new(endpoint);

        client
            .send(ArchcarRequest::GetSessionMessages { thread_id: 1 })
            .unwrap();
        client
            .send(ArchcarRequest::GetSessionMessages { thread_id: 1 })
            .unwrap();
        stop.store(true, Ordering::SeqCst);
        server.join().unwrap();

        assert_eq!(requests.load(Ordering::SeqCst), 2);
        assert_eq!(healthchecks.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn response_loss_retry_is_limited_to_idempotent_requests() {
        assert!(request_retry_safe_after_response_loss(
            &ArchcarRequest::GetSessionStatus { session_id: 42 }
        ));
        assert!(request_retry_safe_after_response_loss(
            &ArchcarRequest::ResizeSession {
                session_id: 42,
                rows: 24,
                cols: 80,
            }
        ));
        assert!(request_retry_safe_after_response_loss(
            &ArchcarRequest::SetSessionModel {
                session_id: 42,
                model: Some("gpt-5".to_owned()),
            }
        ));
        assert!(request_retry_safe_after_response_loss(
            &ArchcarRequest::SetSessionEffort {
                session_id: 42,
                effort: Some("high".to_owned()),
            }
        ));
        assert!(request_retry_safe_after_response_loss(
            &ArchcarRequest::SetSessionPermissionMode {
                session_id: 42,
                mode: "default".to_owned(),
            }
        ));
        assert!(!request_retry_safe_after_response_loss(
            &ArchcarRequest::SendInput {
                session_id: 42,
                input: "hello".to_owned(),
                visible_input: None,
                kind: ArchcarInputKind::User,
                delivery: ArchcarInputDelivery::Auto,
            }
        ));
        assert!(!request_retry_safe_after_response_loss(
            &ArchcarRequest::SpawnSession {
                workspace: "berlin".to_owned(),
                kind: crate::workspace::SessionKind::Codex,
                harness: None,
            }
        ));
    }

    #[test]
    fn response_loss_detects_any_json_eof_decode_error() {
        let err: anyhow::Error = serde_json::from_str::<serde_json::Value>(r#"{"payload":["#)
            .unwrap_err()
            .into();

        assert!(response_decode_or_eof_error(&err));
    }
}
