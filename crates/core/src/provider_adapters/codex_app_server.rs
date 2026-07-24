use crate::archcar::harness::CodexHarnessController;
use crate::archcar::harness_contract::{
    HarnessAdapterContext, HarnessCapability, HarnessControl, HarnessControlPlan,
    HarnessDescriptor, HarnessEffect, HarnessInput, HarnessPreflightSpec, HarnessRecoveryCause,
    HarnessRecoveryPlan, HarnessSignal, HarnessTurnStatus, ManagedHarness, ManagedHarnessAdapter,
    NativeRecord, NativeWrite, SupportMode, MANAGED_HARNESS_CONTRACT_VERSION,
    REQUIRED_HARNESS_FEATURES,
};
use crate::provider_events::{
    ProviderEventContext, ProviderEventDraft, ProviderEventKind, ProviderEventPhase,
};
use crate::workspace::SessionKind;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

pub const CODEX_APP_SERVER_PROVIDER: &str = "codex";
pub const CODEX_APP_SERVER_DEFAULT_EXECUTABLE: &str = "codex";
pub const CODEX_APP_SERVER_DEFAULT_ARGS: &[&str] = &["app-server"];

const CODEX_OPTIONAL_CAPABILITIES: &[(HarnessCapability, SupportMode)] = &[
    (HarnessCapability::Goals, SupportMode::Native),
    (HarnessCapability::NativeSlashCommands, SupportMode::Native),
];

pub static CODEX_MANAGED_HARNESS_DESCRIPTOR: HarnessDescriptor = HarnessDescriptor {
    contract_version: MANAGED_HARNESS_CONTRACT_VERSION,
    kind: SessionKind::Codex,
    provider_key: CODEX_APP_SERVER_PROVIDER,
    display_name: "Codex",
    default_executable: CODEX_APP_SERVER_DEFAULT_EXECUTABLE,
    preflight: HarnessPreflightSpec {
        command: &["codex", "login", "status"],
        auth_guidance: "Run `codex login`.",
    },
    required_features: REQUIRED_HARNESS_FEATURES,
    optional_capabilities: CODEX_OPTIONAL_CAPABILITIES,
};

impl ManagedHarness for CodexHarnessController {
    fn descriptor(&self) -> &'static HarnessDescriptor {
        &CODEX_MANAGED_HARNESS_DESCRIPTOR
    }

    fn create_adapter(
        &self,
        context: HarnessAdapterContext,
    ) -> Result<Box<dyn ManagedHarnessAdapter>> {
        Ok(Box::new(CodexManagedAdapter::new(context)))
    }
}

pub(crate) struct CodexManagedAdapter {
    context: HarnessAdapterContext,
    next_request_id: u64,
    pending_inputs: HashMap<u64, String>,
    active_input_id: Option<String>,
    active_turn_id: Option<String>,
    completed_turns: HashSet<String>,
}

impl CodexManagedAdapter {
    pub(crate) fn new(context: HarnessAdapterContext) -> Self {
        Self {
            context,
            next_request_id: 1,
            pending_inputs: HashMap::new(),
            active_input_id: None,
            active_turn_id: None,
            completed_turns: HashSet::new(),
        }
    }

    pub(crate) fn set_active_turn_id(&mut self, turn_id: Option<String>) {
        self.active_turn_id = turn_id;
    }

    pub(crate) fn set_native_session_id(&mut self, native_session_id: Option<String>) {
        self.context.native_session_id = native_session_id;
    }

    fn take_request_id(&mut self) -> u64 {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        request_id
    }

    fn provider_context(&self) -> ProviderEventContext {
        ProviderEventContext::runtime(
            None,
            Some(self.context.thread_id),
            Some(self.context.session_id),
            "codex-app-server",
        )
    }
}

impl ManagedHarnessAdapter for CodexManagedAdapter {
    fn encode_input(&mut self, input: HarnessInput) -> Result<NativeWrite> {
        let thread_id = self
            .context
            .native_session_id
            .clone()
            .context("Codex app-server thread is not initialized yet")?;
        let request_id = self.take_request_id();
        let mut payload = Vec::new();
        let native_input = vec![CodexAppServerUserInput::Text {
            text: input.content,
        }];
        let steering_active_turn = self.active_turn_id.is_some();
        if !steering_active_turn && self.active_input_id.is_some() {
            anyhow::bail!("Codex turn start is still pending");
        }

        if let Some(active_turn_id) = self.active_turn_id.clone() {
            write_turn_steer_request_with_id(
                &mut payload,
                request_id,
                &CodexAppServerTurnSteerParams {
                    thread_id,
                    input: native_input,
                    expected_turn_id: Some(active_turn_id),
                },
            )?;
        } else {
            write_turn_start_request_with_id(
                &mut payload,
                request_id,
                &CodexAppServerTurnStartParams {
                    thread_id,
                    input: native_input,
                    cwd: None,
                    approval_policy: self.context.controls.permission_mode.clone(),
                    sandbox_policy: None,
                    model: self.context.controls.model.clone(),
                    effort: self.context.controls.effort.clone(),
                    summary: None,
                    personality: None,
                },
            )?;
        }

        self.pending_inputs
            .insert(request_id, input.local_input_id.clone());
        if !steering_active_turn {
            self.active_input_id = Some(input.local_input_id.clone());
        }
        Ok(NativeWrite {
            provider_key: CODEX_APP_SERVER_PROVIDER,
            local_input_id: Some(input.local_input_id),
            payload,
        })
    }

    fn observe_native(&mut self, record: NativeRecord) -> Result<Vec<HarnessEffect>> {
        anyhow::ensure!(
            record.provider_key == CODEX_APP_SERVER_PROVIDER,
            "Codex adapter received native record for {}",
            record.provider_key,
        );
        let input =
            std::str::from_utf8(&record.payload).context("decode Codex app-server JSONL")?;
        let mut effects = Vec::new();

        for (index, line) in input
            .lines()
            .filter(|line| !line.trim().is_empty())
            .enumerate()
        {
            let message = parse_jsonl_message(line, index + 1)?;
            if let Some(request_id) = message.id.as_ref().and_then(Value::as_u64) {
                if let Some(local_input_id) = self.pending_inputs.remove(&request_id) {
                    if message.value.get("error").is_some() {
                        let failed_pending_start = self.active_turn_id.is_none()
                            && self.active_input_id.as_deref() == Some(local_input_id.as_str());
                        effects.push(HarnessEffect::TurnCompleted {
                            local_input_id: local_input_id.clone(),
                            status: HarnessTurnStatus::Failed,
                        });
                        if failed_pending_start {
                            self.active_input_id = None;
                            effects.push(HarnessEffect::Ready);
                        }
                    } else {
                        effects.push(HarnessEffect::InputAcknowledged { local_input_id });
                    }
                }
            }

            if let Some(native_session_id) = managed_codex_thread_id(&message.value) {
                if self.context.native_session_id.as_deref() != Some(native_session_id.as_str()) {
                    self.context.native_session_id = Some(native_session_id.clone());
                    effects.push(HarnessEffect::Initialized {
                        native_session_id,
                        model: self.context.controls.model.clone(),
                    });
                    effects.push(HarnessEffect::Ready);
                }
            }

            if let Some(turn_id) = managed_codex_turn_id(&message.value) {
                self.active_turn_id = Some(turn_id);
            }
            if message.method.as_deref() == Some("turn/started") {
                if let Some(local_input_id) = self.active_input_id.clone() {
                    effects.push(HarnessEffect::TurnStarted { local_input_id });
                }
            }
            if message.method.as_deref() == Some("turn/completed") {
                let completion_key = managed_codex_turn_id(&message.value)
                    .or_else(|| self.active_turn_id.clone())
                    .unwrap_or_else(|| message.raw_json.clone());
                if self.completed_turns.insert(completion_key) {
                    if let Some(local_input_id) = self.active_input_id.take() {
                        effects.push(HarnessEffect::TurnCompleted {
                            local_input_id,
                            status: managed_codex_turn_status(&message.value),
                        });
                    }
                    effects.push(HarnessEffect::Ready);
                }
                self.active_turn_id = None;
            }

            effects.push(HarnessEffect::ProviderEvent(
                message
                    .to_provider_event_draft()
                    .into_provider_event_draft(self.provider_context()),
            ));
        }

        Ok(effects)
    }

    fn plan_control(&mut self, control: HarnessControl) -> HarnessControlPlan {
        match control {
            HarnessControl::Interrupt => {
                let (Some(thread_id), Some(turn_id)) = (
                    self.context.native_session_id.clone(),
                    self.active_turn_id.clone(),
                ) else {
                    return HarnessControlPlan::Unsupported {
                        reason: "no active Codex turn is available to interrupt".to_owned(),
                    };
                };
                let request_id = self.take_request_id();
                let mut payload = Vec::new();
                if let Err(error) = write_turn_interrupt_request_with_id(
                    &mut payload,
                    request_id,
                    &CodexAppServerTurnInterruptParams { thread_id, turn_id },
                ) {
                    return HarnessControlPlan::Unsupported {
                        reason: format!("failed to encode Codex interrupt: {error:#}"),
                    };
                }
                HarnessControlPlan::NativeWrite(NativeWrite {
                    provider_key: CODEX_APP_SERVER_PROVIDER,
                    local_input_id: None,
                    payload,
                })
            }
            HarnessControl::Kill => {
                HarnessControlPlan::Signal(HarnessSignal::TerminateProcessGroup)
            }
            HarnessControl::SetModel(model) => {
                self.context.controls.model = model;
                HarnessControlPlan::RestartRequired(self.context.controls.clone())
            }
            HarnessControl::SetEffort(effort) => {
                self.context.controls.effort = effort;
                HarnessControlPlan::RestartRequired(self.context.controls.clone())
            }
            HarnessControl::SetPermissionMode(permission_mode) => {
                self.context.controls.permission_mode = permission_mode;
                HarnessControlPlan::RestartRequired(self.context.controls.clone())
            }
            HarnessControl::ResolveInteraction(_) => HarnessControlPlan::Unsupported {
                reason: "Codex interaction resolution is not projected by contract v1 yet"
                    .to_owned(),
            },
        }
    }

    fn recovery_plan(&self, _cause: HarnessRecoveryCause) -> HarnessRecoveryPlan {
        managed_recovery_plan(&self.context)
    }
}

fn managed_recovery_plan(context: &HarnessAdapterContext) -> HarnessRecoveryPlan {
    match context.native_session_id.clone() {
        Some(native_session_id) => HarnessRecoveryPlan::RestartAndResume {
            native_session_id,
            controls: context.controls.clone(),
        },
        None => HarnessRecoveryPlan::Fail {
            message: "Codex session has no native thread id to resume".to_owned(),
        },
    }
}

fn managed_codex_thread_id(value: &Value) -> Option<String> {
    [
        "/result/thread/id",
        "/result/threadId",
        "/params/thread/id",
        "/params/threadId",
    ]
    .into_iter()
    .find_map(|pointer| value.pointer(pointer).and_then(Value::as_str))
    .map(ToOwned::to_owned)
}

fn managed_codex_turn_id(value: &Value) -> Option<String> {
    [
        "/params/turn/id",
        "/params/turnId",
        "/result/turn/id",
        "/result/turnId",
    ]
    .into_iter()
    .find_map(|pointer| value.pointer(pointer).and_then(Value::as_str))
    .map(ToOwned::to_owned)
}

fn managed_codex_turn_status(value: &Value) -> HarnessTurnStatus {
    let status = [
        "/params/status",
        "/params/turn/status",
        "/result/status",
        "/result/turn/status",
    ]
    .into_iter()
    .find_map(|pointer| value.pointer(pointer).and_then(Value::as_str));
    match status {
        Some("failed" | "error") => HarnessTurnStatus::Failed,
        Some("interrupted" | "cancelled" | "canceled") => HarnessTurnStatus::Interrupted,
        Some("deferred") => HarnessTurnStatus::Deferred,
        _ => HarnessTurnStatus::Success,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexAppServerTransport {
    StdioJsonl,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexAppServerLaunch {
    pub executable: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, OsString)>,
    pub transport: CodexAppServerTransport,
}

impl Default for CodexAppServerLaunch {
    fn default() -> Self {
        Self {
            executable: CODEX_APP_SERVER_DEFAULT_EXECUTABLE.to_owned(),
            args: CODEX_APP_SERVER_DEFAULT_ARGS
                .iter()
                .map(|arg| (*arg).to_owned())
                .collect(),
            cwd: None,
            env: Vec::new(),
            transport: CodexAppServerTransport::StdioJsonl,
        }
    }
}

#[derive(Debug)]
pub struct CodexAppServerClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_request_id: u64,
    next_read_line: usize,
}

impl CodexAppServerClient {
    pub fn spawn(launch: &CodexAppServerLaunch) -> Result<Self> {
        let mut command = Command::new(&launch.executable);
        command
            .args(&launch.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());

        if let Some(cwd) = &launch.cwd {
            command.current_dir(cwd);
        }
        for (key, value) in &launch.env {
            command.env(key, value);
        }

        let mut child = command
            .spawn()
            .with_context(|| format!("failed to spawn {}", launch.executable))?;
        let stdin = child
            .stdin
            .take()
            .context("codex app-server stdin was not piped")?;
        let stdout = child
            .stdout
            .take()
            .context("codex app-server stdout was not piped")?;

        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_request_id: 1,
            next_read_line: 0,
        })
    }

    pub fn initialize(&mut self, params: &CodexAppServerInitializeParams) -> Result<u64> {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        write_initialize_request_with_id(&mut self.stdin, request_id, params)?;
        write_initialized_notification(&mut self.stdin)?;
        Ok(request_id)
    }

    pub fn send_request(&mut self, method: &str, params: Value) -> Result<u64> {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        write_jsonl(
            &mut self.stdin,
            &json!({
                "id": request_id,
                "method": method,
                "params": params,
            }),
        )?;
        Ok(request_id)
    }

    pub fn send_thread_start(&mut self, params: &CodexAppServerThreadStartParams) -> Result<u64> {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        write_thread_start_request_with_id(&mut self.stdin, request_id, params)?;
        Ok(request_id)
    }

    pub fn send_turn_start(&mut self, params: &CodexAppServerTurnStartParams) -> Result<u64> {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        write_turn_start_request_with_id(&mut self.stdin, request_id, params)?;
        Ok(request_id)
    }

    pub fn send_notification(&mut self, method: &str, params: Value) -> Result<()> {
        write_jsonl(
            &mut self.stdin,
            &json!({
                "method": method,
                "params": params,
            }),
        )
    }

    pub fn send_response(&mut self, id: Value, result: Value) -> Result<()> {
        write_jsonl(
            &mut self.stdin,
            &json!({
                "id": id,
                "result": result,
            }),
        )
    }

    pub fn send_error_response(&mut self, id: Value, code: i64, message: &str) -> Result<()> {
        write_jsonl(
            &mut self.stdin,
            &json!({
                "id": id,
                "error": {
                    "code": code,
                    "message": message,
                },
            }),
        )
    }

    pub fn read_message(&mut self) -> Result<Option<CodexAppServerMessage>> {
        read_jsonl_message(&mut self.stdout, &mut self.next_read_line)
    }

    pub fn process_id(&self) -> u32 {
        self.child.id()
    }

    pub fn kill(&mut self) -> Result<()> {
        self.child
            .kill()
            .context("failed to kill codex app-server")?;
        self.child
            .wait()
            .context("failed to reap killed codex app-server")?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexAppServerInitializeParams {
    pub client_name: String,
    pub client_title: Option<String>,
    pub client_version: Option<String>,
    pub workspace_root: Option<PathBuf>,
}

pub fn write_initialize_request<W: Write>(
    writer: &mut W,
    params: &CodexAppServerInitializeParams,
) -> Result<u64> {
    write_initialize_request_with_id(writer, 1, params)?;
    Ok(1)
}

pub fn write_initialize_request_with_id<W: Write>(
    writer: &mut W,
    request_id: u64,
    params: &CodexAppServerInitializeParams,
) -> Result<()> {
    let mut payload = json!({
        "id": request_id,
        "method": "initialize",
        "params": {
            "clientInfo": {
                "name": params.client_name.clone(),
            },
        },
    });

    if let Some(title) = &params.client_title {
        payload["params"]["clientInfo"]["title"] = Value::String(title.clone());
    }
    if let Some(version) = &params.client_version {
        payload["params"]["clientInfo"]["version"] = Value::String(version.clone());
    }

    write_jsonl(writer, &payload)
}

pub fn write_initialized_notification<W: Write>(writer: &mut W) -> Result<()> {
    write_jsonl(
        writer,
        &json!({
            "method": "initialized",
            "params": {},
        }),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexAppServerThreadStartParams {
    pub model: Option<String>,
    pub cwd: Option<PathBuf>,
    pub approval_policy: Option<String>,
    pub sandbox: Option<String>,
    pub service_name: Option<String>,
}

impl CodexAppServerThreadStartParams {
    fn to_value(&self) -> Value {
        let mut params = serde_json::Map::new();
        insert_string(&mut params, "model", self.model.as_deref());
        if let Some(cwd) = &self.cwd {
            params.insert("cwd".to_owned(), Value::String(native_path_string(cwd)));
        }
        insert_string(
            &mut params,
            "approvalPolicy",
            self.approval_policy.as_deref(),
        );
        insert_string(&mut params, "sandbox", self.sandbox.as_deref());
        insert_string(&mut params, "serviceName", self.service_name.as_deref());
        Value::Object(params)
    }
}

pub fn write_thread_start_request<W: Write>(
    writer: &mut W,
    params: &CodexAppServerThreadStartParams,
) -> Result<u64> {
    write_thread_start_request_with_id(writer, 1, params)?;
    Ok(1)
}

pub fn write_thread_start_request_with_id<W: Write>(
    writer: &mut W,
    request_id: u64,
    params: &CodexAppServerThreadStartParams,
) -> Result<()> {
    write_jsonl(
        writer,
        &json!({
            "id": request_id,
            "method": "thread/start",
            "params": params.to_value(),
        }),
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodexAppServerTurnStartParams {
    pub thread_id: String,
    pub input: Vec<CodexAppServerUserInput>,
    pub cwd: Option<PathBuf>,
    pub approval_policy: Option<String>,
    pub sandbox_policy: Option<Value>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub summary: Option<String>,
    pub personality: Option<String>,
}

impl CodexAppServerTurnStartParams {
    fn to_value(&self) -> Value {
        let mut params = serde_json::Map::new();
        params.insert("threadId".to_owned(), Value::String(self.thread_id.clone()));
        params.insert(
            "input".to_owned(),
            Value::Array(
                self.input
                    .iter()
                    .map(CodexAppServerUserInput::to_value)
                    .collect(),
            ),
        );
        if let Some(cwd) = &self.cwd {
            params.insert("cwd".to_owned(), Value::String(native_path_string(cwd)));
        }
        insert_string(
            &mut params,
            "approvalPolicy",
            self.approval_policy.as_deref(),
        );
        if let Some(sandbox_policy) = &self.sandbox_policy {
            params.insert("sandboxPolicy".to_owned(), sandbox_policy.clone());
        }
        insert_string(&mut params, "model", self.model.as_deref());
        insert_string(&mut params, "effort", self.effort.as_deref());
        insert_string(&mut params, "summary", self.summary.as_deref());
        insert_string(&mut params, "personality", self.personality.as_deref());
        Value::Object(params)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexAppServerUserInput {
    Text { text: String },
}

impl CodexAppServerUserInput {
    fn to_value(&self) -> Value {
        match self {
            Self::Text { text } => json!({
                "type": "text",
                "text": text,
            }),
        }
    }
}

pub fn write_turn_start_request<W: Write>(
    writer: &mut W,
    params: &CodexAppServerTurnStartParams,
) -> Result<u64> {
    write_turn_start_request_with_id(writer, 1, params)?;
    Ok(1)
}

pub fn write_turn_start_request_with_id<W: Write>(
    writer: &mut W,
    request_id: u64,
    params: &CodexAppServerTurnStartParams,
) -> Result<()> {
    write_jsonl(
        writer,
        &json!({
            "id": request_id,
            "method": "turn/start",
            "params": params.to_value(),
        }),
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodexAppServerTurnSteerParams {
    pub thread_id: String,
    pub input: Vec<CodexAppServerUserInput>,
    pub expected_turn_id: Option<String>,
}

impl CodexAppServerTurnSteerParams {
    fn to_value(&self) -> Value {
        let mut params = serde_json::Map::new();
        params.insert("threadId".to_owned(), Value::String(self.thread_id.clone()));
        params.insert(
            "input".to_owned(),
            Value::Array(
                self.input
                    .iter()
                    .map(CodexAppServerUserInput::to_value)
                    .collect(),
            ),
        );
        insert_string(
            &mut params,
            "expectedTurnId",
            self.expected_turn_id.as_deref(),
        );
        Value::Object(params)
    }
}

pub fn write_turn_steer_request_with_id<W: Write>(
    writer: &mut W,
    request_id: u64,
    params: &CodexAppServerTurnSteerParams,
) -> Result<()> {
    write_jsonl(
        writer,
        &json!({
            "id": request_id,
            "method": "turn/steer",
            "params": params.to_value(),
        }),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexAppServerTurnInterruptParams {
    pub thread_id: String,
    pub turn_id: String,
}

impl CodexAppServerTurnInterruptParams {
    fn to_value(&self) -> Value {
        json!({
            "threadId": self.thread_id.clone(),
            "turnId": self.turn_id.clone(),
        })
    }
}

pub fn write_turn_interrupt_request_with_id<W: Write>(
    writer: &mut W,
    request_id: u64,
    params: &CodexAppServerTurnInterruptParams,
) -> Result<()> {
    write_jsonl(
        writer,
        &json!({
            "id": request_id,
            "method": "turn/interrupt",
            "params": params.to_value(),
        }),
    )
}

fn insert_string(map: &mut serde_json::Map<String, Value>, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        map.insert(key.to_owned(), Value::String(value.to_owned()));
    }
}

fn write_jsonl<W: Write>(writer: &mut W, value: &Value) -> Result<()> {
    serde_json::to_writer(&mut *writer, value).context("failed to serialize JSON-RPC message")?;
    writer
        .write_all(b"\n")
        .context("failed to write JSON-RPC newline")?;
    writer.flush().context("failed to flush JSON-RPC writer")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexAppServerMessageKind {
    Notification,
    Request,
    Response,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodexAppServerMessage {
    pub raw_json: String,
    pub value: Value,
    pub message_kind: CodexAppServerMessageKind,
    pub method: Option<String>,
    pub id: Option<Value>,
}

impl CodexAppServerMessage {
    pub fn to_provider_event_draft(&self) -> CodexProviderEventDraft {
        let method = self.method.as_deref().unwrap_or("");
        let name = codex_event_name(method, self.message_kind, &self.value);
        CodexProviderEventDraft {
            provider: CODEX_APP_SERVER_PROVIDER.to_owned(),
            message_kind: self.message_kind,
            category: classify_codex_method(&name, self.message_kind),
            name,
            correlation_id: self.id.as_ref().map(json_rpc_id_to_string),
            raw_json: self.raw_json.clone(),
            payload: self.value.clone(),
        }
    }
}

fn codex_event_name(
    method: &str,
    message_kind: CodexAppServerMessageKind,
    payload: &Value,
) -> String {
    if let Some(item_type) = string_at_any(payload, &["/params/item/type"]) {
        if let Some(phase) = method
            .strip_prefix("item/")
            .and_then(|value| value.split('/').next())
        {
            if matches!(phase, "started" | "completed") {
                return format!("item/{item_type}/{phase}");
            }
        }
    }

    if !method.is_empty() {
        method.to_owned()
    } else {
        match message_kind {
            CodexAppServerMessageKind::Response => "response".to_owned(),
            CodexAppServerMessageKind::Unknown => "unknown".to_owned(),
            CodexAppServerMessageKind::Notification | CodexAppServerMessageKind::Request => {
                "unnamed".to_owned()
            }
        }
    }
}

pub fn read_jsonl_messages<R: BufRead>(mut reader: R) -> Result<Vec<CodexAppServerMessage>> {
    let mut messages = Vec::new();
    let mut line = String::new();
    let mut line_number = 0usize;

    loop {
        line.clear();
        let bytes = reader
            .read_line(&mut line)
            .context("failed to read codex app-server JSONL")?;
        if bytes == 0 {
            break;
        }
        line_number += 1;
        let raw = line.trim_end_matches(['\r', '\n']).to_owned();
        if raw.trim().is_empty() {
            continue;
        }
        messages.push(parse_jsonl_message(&raw, line_number)?);
    }

    Ok(messages)
}

fn read_jsonl_message<R: BufRead>(
    reader: &mut R,
    next_line_number: &mut usize,
) -> Result<Option<CodexAppServerMessage>> {
    let mut line = String::new();
    loop {
        line.clear();
        let bytes = reader
            .read_line(&mut line)
            .context("failed to read codex app-server JSONL")?;
        if bytes == 0 {
            return Ok(None);
        }
        *next_line_number += 1;
        let raw = line.trim_end_matches(['\r', '\n']).to_owned();
        if raw.trim().is_empty() {
            continue;
        }
        return parse_jsonl_message(&raw, *next_line_number).map(Some);
    }
}

pub fn parse_jsonl_message(raw: &str, line_number: usize) -> Result<CodexAppServerMessage> {
    let value: Value = serde_json::from_str(raw)
        .with_context(|| format!("invalid codex app-server JSONL at line {line_number}"))?;
    let method = value
        .get("method")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let id = value.get("id").cloned();
    let message_kind = match (method.is_some(), id.is_some()) {
        (true, true) => CodexAppServerMessageKind::Request,
        (true, false) => CodexAppServerMessageKind::Notification,
        (false, true) => CodexAppServerMessageKind::Response,
        (false, false) => CodexAppServerMessageKind::Unknown,
    };

    Ok(CodexAppServerMessage {
        raw_json: raw.to_owned(),
        value,
        message_kind,
        method,
        id,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexProviderEventCategory {
    AccountAuth,
    ThreadSessionLifecycle,
    GoalsTasks,
    Turns,
    UserInput,
    AssistantOutput,
    PlanningReasoning,
    CommandProcessExecution,
    TerminalBackgroundRuntime,
    Filesystem,
    DiffsFileChanges,
    Tools,
    Mcp,
    SkillsPluginsHooks,
    ApprovalsPermissions,
    SubagentsCollaboration,
    WebBrowserMedia,
    EnvironmentConfigModel,
    LimitsFailures,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodexCapabilityCoverage {
    pub category: CodexProviderEventCategory,
    pub label: &'static str,
}

pub const CODEX_APP_SERVER_CAPABILITY_COVERAGE: &[CodexCapabilityCoverage] = &[
    CodexCapabilityCoverage {
        category: CodexProviderEventCategory::AccountAuth,
        label: "account/auth",
    },
    CodexCapabilityCoverage {
        category: CodexProviderEventCategory::ThreadSessionLifecycle,
        label: "thread/session lifecycle",
    },
    CodexCapabilityCoverage {
        category: CodexProviderEventCategory::GoalsTasks,
        label: "goals/tasks",
    },
    CodexCapabilityCoverage {
        category: CodexProviderEventCategory::Turns,
        label: "turns",
    },
    CodexCapabilityCoverage {
        category: CodexProviderEventCategory::UserInput,
        label: "user input",
    },
    CodexCapabilityCoverage {
        category: CodexProviderEventCategory::AssistantOutput,
        label: "assistant output",
    },
    CodexCapabilityCoverage {
        category: CodexProviderEventCategory::PlanningReasoning,
        label: "planning/reasoning",
    },
    CodexCapabilityCoverage {
        category: CodexProviderEventCategory::CommandProcessExecution,
        label: "command/process execution",
    },
    CodexCapabilityCoverage {
        category: CodexProviderEventCategory::TerminalBackgroundRuntime,
        label: "terminal/background runtime",
    },
    CodexCapabilityCoverage {
        category: CodexProviderEventCategory::Filesystem,
        label: "filesystem",
    },
    CodexCapabilityCoverage {
        category: CodexProviderEventCategory::DiffsFileChanges,
        label: "diffs/file changes",
    },
    CodexCapabilityCoverage {
        category: CodexProviderEventCategory::Tools,
        label: "tools",
    },
    CodexCapabilityCoverage {
        category: CodexProviderEventCategory::Mcp,
        label: "MCP",
    },
    CodexCapabilityCoverage {
        category: CodexProviderEventCategory::SkillsPluginsHooks,
        label: "skills/plugins/hooks",
    },
    CodexCapabilityCoverage {
        category: CodexProviderEventCategory::ApprovalsPermissions,
        label: "approvals/permissions",
    },
    CodexCapabilityCoverage {
        category: CodexProviderEventCategory::SubagentsCollaboration,
        label: "subagents/collaboration",
    },
    CodexCapabilityCoverage {
        category: CodexProviderEventCategory::WebBrowserMedia,
        label: "web/browser/media",
    },
    CodexCapabilityCoverage {
        category: CodexProviderEventCategory::EnvironmentConfigModel,
        label: "environment/config/model",
    },
    CodexCapabilityCoverage {
        category: CodexProviderEventCategory::LimitsFailures,
        label: "limits/failures",
    },
    CodexCapabilityCoverage {
        category: CodexProviderEventCategory::Unknown,
        label: "unknown events",
    },
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodexProviderEventDraft {
    pub provider: String,
    pub message_kind: CodexAppServerMessageKind,
    pub category: CodexProviderEventCategory,
    pub name: String,
    pub correlation_id: Option<String>,
    pub raw_json: String,
    pub payload: Value,
}

impl CodexProviderEventDraft {
    pub fn into_provider_event_draft(self, context: ProviderEventContext) -> ProviderEventDraft {
        let raw_json = serde_json::from_str(&self.raw_json)
            .unwrap_or_else(|_| Value::String(self.raw_json.clone()));
        let (kind, provider_subtype) =
            codex_canonical_kind_and_subtype(self.category, &self.name, &self.payload);
        let phase = if is_mcp_startup_status_event(&self.name) {
            codex_mcp_startup_status_phase(&self.payload)
        } else {
            codex_phase_for(&self.name, self.message_kind)
        };
        let provider_event_id = self.correlation_id.clone().or_else(|| {
            string_at_any(
                &self.payload,
                &[
                    "/params/event/id",
                    "/params/eventId",
                    "/params/event_id",
                    "/params/id",
                    "/result/id",
                ],
            )
        });
        let provider_item_id = if is_mcp_startup_status_event(&self.name) {
            let server = string_at_any(&self.payload, &["/params/name", "/params/serverName"])
                .unwrap_or_else(|| "unknown".to_owned());
            Some(format!("mcp-startup-status:{server}"))
        } else {
            string_at_any(
                &self.payload,
                &[
                    "/params/item/id",
                    "/params/itemId",
                    "/params/item_id",
                    "/params/message/id",
                    "/params/toolCall/id",
                    "/params/tool_call/id",
                    "/params/callId",
                    "/result/item/id",
                    "/result/message/id",
                ],
            )
        };
        let provider_thread_id = string_at_any(
            &self.payload,
            &[
                "/params/thread/id",
                "/params/threadId",
                "/params/thread_id",
                "/params/session/id",
                "/params/sessionId",
                "/params/session_id",
                "/result/threadId",
                "/result/thread_id",
            ],
        );
        let provider_turn_id = string_at_any(
            &self.payload,
            &[
                "/params/turn/id",
                "/params/turnId",
                "/params/turn_id",
                "/result/turnId",
                "/result/turn_id",
            ],
        );
        let parent_provider_item_id = string_at_any(
            &self.payload,
            &[
                "/params/parent/id",
                "/params/parentItemId",
                "/params/parent_item_id",
                "/params/parent/item_id",
            ],
        );
        let parent_provider_thread_id = string_at_any(
            &self.payload,
            &[
                "/params/parentThreadId",
                "/params/parent_thread_id",
                "/params/parent/thread_id",
            ],
        );
        let body = string_at_any(
            &self.payload,
            &[
                "/params/text",
                "/params/delta",
                "/params/message/text",
                "/params/message/content",
                "/result/text",
                "/result/message",
            ],
        )
        .unwrap_or_default();
        let title = codex_event_title(&self.name, kind, &self.payload);
        let body = if is_mcp_startup_status_event(&self.name) {
            codex_mcp_startup_status_body(&self.payload, phase)
        } else if body.is_empty() {
            codex_payload_body(&self.payload)
        } else {
            body
        };
        let stream_delta = (phase == ProviderEventPhase::Delta).then(|| body.clone());

        ProviderEventDraft {
            provider: self.provider,
            provider_event_id,
            provider_item_id,
            provider_thread_id,
            provider_turn_id,
            parent_provider_item_id,
            parent_provider_thread_id,
            workspace_id: context.workspace_id,
            chat_thread_id: context.chat_thread_id,
            process_id: context.process_id,
            phase,
            kind,
            provider_subtype,
            provider_sequence: number_at_any(
                &self.payload,
                &[
                    "/params/sequence",
                    "/params/seq",
                    "/result/sequence",
                    "/result/seq",
                ],
            )
            .and_then(|value| i64::try_from(value).ok()),
            occurred_at_ms: context.occurred_at_ms,
            normalized_payload: json!({
                "title": title,
                "body": body,
                "stream_delta": stream_delta,
                "message_kind": self.message_kind,
            }),
            raw_json,
            schema_version: context.schema_version,
            adapter_version: context.adapter_version,
        }
    }
}

fn codex_event_title(name: &str, kind: ProviderEventKind, payload: &Value) -> String {
    if is_mcp_startup_status_event(name) {
        return codex_mcp_startup_status_title(payload);
    }

    match string_at_any(payload, &["/params/item/type"]).as_deref() {
        Some("agentMessage") => "Assistant".to_owned(),
        Some("userMessage") => "User input".to_owned(),
        Some("reasoning") => "Reasoning".to_owned(),
        Some("plan") => "Plan".to_owned(),
        Some("commandExecution") => {
            command_from_payload(payload).unwrap_or_else(|| "Command".to_owned())
        }
        Some("fileChange") => "File changes".to_owned(),
        Some("mcpToolCall") => {
            tool_title_from_payload(payload).unwrap_or_else(|| "MCP tool".to_owned())
        }
        Some("dynamicToolCall") => {
            tool_title_from_payload(payload).unwrap_or_else(|| "Dynamic tool".to_owned())
        }
        Some("webSearch") => "Web search".to_owned(),
        Some("imageView") => "Image".to_owned(),
        Some("enteredReviewMode") | Some("exitedReviewMode") => "Review".to_owned(),
        Some("contextCompaction") => "Context compaction".to_owned(),
        _ => match kind {
            ProviderEventKind::AssistantOutput => "Assistant".to_owned(),
            ProviderEventKind::PlanningReasoning => "Reasoning".to_owned(),
            ProviderEventKind::CommandProcess => "Command".to_owned(),
            ProviderEventKind::DiffFileChange => "File changes".to_owned(),
            ProviderEventKind::Tool => "Tool".to_owned(),
            ProviderEventKind::Mcp => "MCP tool".to_owned(),
            ProviderEventKind::ApprovalPermission => "Approval".to_owned(),
            ProviderEventKind::LimitFailure => "Provider event".to_owned(),
            _ => name.to_owned(),
        },
    }
}

fn codex_payload_body(payload: &Value) -> String {
    match string_at_any(payload, &["/params/item/type"]).as_deref() {
        Some("reasoning") => return reasoning_body_from_payload(payload).unwrap_or_default(),
        Some("commandExecution") => return command_body_from_payload(payload).unwrap_or_default(),
        Some("fileChange") => return file_change_body_from_payload(payload).unwrap_or_default(),
        Some("mcpToolCall") | Some("dynamicToolCall") | Some("webSearch") | Some("imageView") => {
            return tool_body_from_payload(payload).unwrap_or_default();
        }
        _ => {}
    }

    string_at_any(
        payload,
        &[
            "/params/item/text",
            "/params/item/review",
            "/params/item/summary",
            "/params/item/content",
            "/params/item/aggregatedOutput",
            "/params/item/result",
            "/params/item/error/message",
            "/params/item/status",
            "/params/error/message",
            "/error/message",
        ],
    )
    .or_else(|| command_body_from_payload(payload))
    .or_else(|| file_change_body_from_payload(payload))
    .or_else(|| tool_body_from_payload(payload))
    .unwrap_or_default()
}

fn reasoning_body_from_payload(payload: &Value) -> Option<String> {
    let parts = ["/params/item/summary", "/params/item/content"]
        .iter()
        .filter_map(|pointer| payload.pointer(pointer))
        .filter_map(display_value)
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

fn command_body_from_payload(payload: &Value) -> Option<String> {
    let command = command_from_payload(payload)?;
    let output = string_at_any(payload, &["/params/item/aggregatedOutput"]);
    Some(match output {
        Some(output) if !output.trim().is_empty() => format!("{command}\n{output}"),
        _ => command,
    })
}

fn command_from_payload(payload: &Value) -> Option<String> {
    let value = payload.pointer("/params/item/command")?;
    let command = match value {
        Value::String(command) => command.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        other => other.to_string(),
    };
    (!command.trim().is_empty()).then_some(command)
}

fn file_change_body_from_payload(payload: &Value) -> Option<String> {
    let changes = payload.pointer("/params/item/changes")?.as_array()?;
    let lines = changes
        .iter()
        .filter_map(|change| {
            let path = change.get("path").and_then(Value::as_str)?;
            let kind = change
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("changed");
            Some(format!("{kind} {path}"))
        })
        .collect::<Vec<_>>();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn tool_body_from_payload(payload: &Value) -> Option<String> {
    value_at_any(
        payload,
        &[
            "/params/item/error",
            "/params/item/result",
            "/params/item/arguments",
            "/params/item/query",
            "/params/item/path",
        ],
    )
    .and_then(display_value)
}

fn tool_title_from_payload(payload: &Value) -> Option<String> {
    string_at_any(
        payload,
        &[
            "/params/item/tool",
            "/params/item/server",
            "/params/item/action",
            "/params/item/path",
        ],
    )
}

fn is_mcp_startup_status_event(name: &str) -> bool {
    name == "mcpServer/startupStatus/updated"
}

fn codex_mcp_startup_status_title(payload: &Value) -> String {
    match codex_mcp_startup_status_phase(payload) {
        ProviderEventPhase::Failed => "MCP failed".to_owned(),
        ProviderEventPhase::Completed => "MCP loaded".to_owned(),
        _ => "MCP loading".to_owned(),
    }
}

fn codex_mcp_startup_status_phase(payload: &Value) -> ProviderEventPhase {
    let status = string_at_any(payload, &["/params/status"])
        .unwrap_or_default()
        .to_ascii_lowercase();
    let error = string_at_any(
        payload,
        &[
            "/params/error/message",
            "/params/error",
            "/params/failureReason",
            "/params/failure_reason",
        ],
    );

    if error
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || status_contains_any(&status, &["fail", "error", "unavailable"])
    {
        ProviderEventPhase::Failed
    } else if status_contains_any(&status, &["cancel", "cancelled", "canceled"]) {
        ProviderEventPhase::Interrupted
    } else if status_contains_any(
        &status,
        &[
            "ready",
            "loaded",
            "complete",
            "completed",
            "success",
            "connected",
            "running",
            "disabled",
            "skipped",
            "stopped",
        ],
    ) {
        ProviderEventPhase::Completed
    } else {
        ProviderEventPhase::Progress
    }
}

fn codex_mcp_startup_status_body(payload: &Value, phase: ProviderEventPhase) -> String {
    if phase == ProviderEventPhase::Progress {
        return String::new();
    }

    let server = string_at_any(payload, &["/params/name", "/params/serverName"])
        .unwrap_or_default()
        .trim()
        .to_owned();
    let status = string_at_any(payload, &["/params/status"])
        .unwrap_or_default()
        .trim()
        .to_owned();
    let error = string_at_any(
        payload,
        &[
            "/params/error/message",
            "/params/error",
            "/params/failureReason",
            "/params/failure_reason",
        ],
    )
    .unwrap_or_default()
    .trim()
    .to_owned();

    let mut parts = Vec::new();
    if !server.is_empty() && !status.is_empty() {
        parts.push(format!("{server}: {status}"));
    } else if !server.is_empty() {
        parts.push(server);
    } else if !status.is_empty() {
        parts.push(status);
    }
    if !error.is_empty() {
        parts.push(error);
    }

    parts.join("\n")
}

fn status_contains_any(status: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| status.contains(needle))
}

pub fn classify_codex_method(
    method: &str,
    message_kind: CodexAppServerMessageKind,
) -> CodexProviderEventCategory {
    if message_kind == CodexAppServerMessageKind::Response && method == "response" {
        return CodexProviderEventCategory::Unknown;
    }

    if method.is_empty() {
        return CodexProviderEventCategory::Unknown;
    }

    let normalized = split_camel_segments(method).to_ascii_lowercase();
    let tokens: Vec<_> = normalized
        .split(|ch: char| !(ch.is_ascii_alphanumeric()))
        .filter(|token| !token.is_empty())
        .collect();

    if has_any(
        &tokens,
        &["auth", "account", "login", "logout", "signin", "userinfo"],
    ) {
        CodexProviderEventCategory::AccountAuth
    } else if has_any(
        &tokens,
        &[
            "thread",
            "session",
            "conversation",
            "resume",
            "initialize",
            "initialized",
        ],
    ) {
        CodexProviderEventCategory::ThreadSessionLifecycle
    } else if has_any(
        &tokens,
        &["goal", "goals", "task", "tasks", "todo", "todos"],
    ) {
        CodexProviderEventCategory::GoalsTasks
    } else if has_any(&tokens, &["turn", "turns"]) {
        CodexProviderEventCategory::Turns
    } else if has_any(&tokens, &["user", "input", "prompt", "composer"]) {
        CodexProviderEventCategory::UserInput
    } else if has_any(
        &tokens,
        &["assistant", "answer", "message", "output", "response"],
    ) {
        CodexProviderEventCategory::AssistantOutput
    } else if has_any(
        &tokens,
        &["plan", "planning", "reasoning", "thought", "analysis"],
    ) {
        CodexProviderEventCategory::PlanningReasoning
    } else if has_any(
        &tokens,
        &[
            "approval",
            "approvals",
            "permission",
            "permissions",
            "sandbox",
            "policy",
        ],
    ) {
        CodexProviderEventCategory::ApprovalsPermissions
    } else if has_any(
        &tokens,
        &["command", "exec", "execution", "process", "subprocess"],
    ) {
        CodexProviderEventCategory::CommandProcessExecution
    } else if has_any(
        &tokens,
        &["terminal", "background", "runtime", "shell", "pty"],
    ) {
        CodexProviderEventCategory::TerminalBackgroundRuntime
    } else if has_any(
        &tokens,
        &["diff", "patch", "change", "changes", "edit", "edited"],
    ) {
        CodexProviderEventCategory::DiffsFileChanges
    } else if has_any(
        &tokens,
        &["file", "files", "filesystem", "fs", "directory", "path"],
    ) {
        CodexProviderEventCategory::Filesystem
    } else if has_any(&tokens, &["tool", "tools", "toolcall"]) {
        CodexProviderEventCategory::Tools
    } else if has_any(&tokens, &["mcp"]) {
        CodexProviderEventCategory::Mcp
    } else if has_any(
        &tokens,
        &["skill", "skills", "plugin", "plugins", "hook", "hooks"],
    ) {
        CodexProviderEventCategory::SkillsPluginsHooks
    } else if has_any(
        &tokens,
        &[
            "subagent",
            "subagents",
            "collaboration",
            "delegate",
            "worker",
        ],
    ) {
        CodexProviderEventCategory::SubagentsCollaboration
    } else if has_any(&tokens, &["web", "browser", "media", "image", "screenshot"]) {
        CodexProviderEventCategory::WebBrowserMedia
    } else if has_any(
        &tokens,
        &["environment", "env", "config", "model", "settings"],
    ) {
        CodexProviderEventCategory::EnvironmentConfigModel
    } else if has_any(
        &tokens,
        &[
            "limit",
            "limits",
            "failure",
            "failures",
            "error",
            "cancel",
            "cancelled",
        ],
    ) {
        CodexProviderEventCategory::LimitsFailures
    } else {
        CodexProviderEventCategory::Unknown
    }
}

fn has_any(tokens: &[&str], needles: &[&str]) -> bool {
    needles.iter().any(|needle| tokens.contains(needle))
}

fn codex_canonical_kind_and_subtype(
    category: CodexProviderEventCategory,
    name: &str,
    payload: &Value,
) -> (ProviderEventKind, Option<String>) {
    if let Some(tool_name) = codex_dynamic_tool_name(name, payload) {
        if let Some((kind, subtype)) = codex_local_tool_action_kind_and_subtype(&tool_name) {
            return (kind, Some(subtype.to_owned()));
        }
    }

    (
        codex_category_to_provider_kind(category),
        Some(name.to_owned()),
    )
}

fn codex_dynamic_tool_name(name: &str, payload: &Value) -> Option<String> {
    let is_dynamic_thread_item =
        string_at_any(payload, &["/params/item/type"]).as_deref() == Some("dynamicToolCall");
    let is_dynamic_tool_request = name == "item/tool/call";
    if !(is_dynamic_thread_item || is_dynamic_tool_request) {
        return None;
    }

    string_at_any(payload, &["/params/item/tool", "/params/tool"])
}

fn codex_local_tool_action_kind_and_subtype(
    tool_name: &str,
) -> Option<(ProviderEventKind, &'static str)> {
    let normalized = tool_name.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "bash" | "shell" | "exec" | "exec_command" => {
            Some((ProviderEventKind::CommandProcess, "command"))
        }
        "read" | "ls" | "glob" | "grep" => Some((ProviderEventKind::FileSystem, "read")),
        "edit" | "multiedit" | "notebookedit" | "apply_patch" | "patch" => {
            Some((ProviderEventKind::FileSystem, "edit"))
        }
        "write" | "create" => Some((ProviderEventKind::FileSystem, "write")),
        _ => None,
    }
}

fn codex_category_to_provider_kind(category: CodexProviderEventCategory) -> ProviderEventKind {
    match category {
        CodexProviderEventCategory::AccountAuth => ProviderEventKind::AccountAuth,
        CodexProviderEventCategory::ThreadSessionLifecycle => ProviderEventKind::ThreadSession,
        CodexProviderEventCategory::GoalsTasks => ProviderEventKind::GoalTask,
        CodexProviderEventCategory::Turns => ProviderEventKind::Turn,
        CodexProviderEventCategory::UserInput => ProviderEventKind::UserInput,
        CodexProviderEventCategory::AssistantOutput => ProviderEventKind::AssistantOutput,
        CodexProviderEventCategory::PlanningReasoning => ProviderEventKind::PlanningReasoning,
        CodexProviderEventCategory::CommandProcessExecution => ProviderEventKind::CommandProcess,
        CodexProviderEventCategory::TerminalBackgroundRuntime => ProviderEventKind::TerminalRuntime,
        CodexProviderEventCategory::Filesystem => ProviderEventKind::FileSystem,
        CodexProviderEventCategory::DiffsFileChanges => ProviderEventKind::DiffFileChange,
        CodexProviderEventCategory::Tools => ProviderEventKind::Tool,
        CodexProviderEventCategory::Mcp => ProviderEventKind::Mcp,
        CodexProviderEventCategory::SkillsPluginsHooks => ProviderEventKind::SkillPluginHook,
        CodexProviderEventCategory::ApprovalsPermissions => ProviderEventKind::ApprovalPermission,
        CodexProviderEventCategory::SubagentsCollaboration => {
            ProviderEventKind::SubagentCollaboration
        }
        CodexProviderEventCategory::WebBrowserMedia => ProviderEventKind::WebBrowserMedia,
        CodexProviderEventCategory::EnvironmentConfigModel => {
            ProviderEventKind::EnvironmentConfigModel
        }
        CodexProviderEventCategory::LimitsFailures => ProviderEventKind::LimitFailure,
        CodexProviderEventCategory::Unknown => ProviderEventKind::Unknown,
    }
}

fn codex_phase_for(name: &str, message_kind: CodexAppServerMessageKind) -> ProviderEventPhase {
    let normalized = name.to_ascii_lowercase();
    let tokens: Vec<_> = normalized
        .split(|ch: char| !(ch.is_ascii_alphanumeric()))
        .filter(|token| !token.is_empty())
        .collect();

    if has_any(&tokens, &["error", "failed", "failure"]) {
        ProviderEventPhase::Failed
    } else if has_any(&tokens, &["declined", "denied", "rejected"]) {
        ProviderEventPhase::Declined
    } else if has_any(
        &tokens,
        &["interrupt", "interrupted", "cancel", "cancelled"],
    ) {
        ProviderEventPhase::Interrupted
    } else if has_any(&tokens, &["delta", "partial", "chunk"]) {
        ProviderEventPhase::Delta
    } else if has_any(&tokens, &["progress", "running", "stdout", "stderr"]) {
        ProviderEventPhase::Progress
    } else if has_any(
        &tokens,
        &[
            "completed",
            "complete",
            "done",
            "finished",
            "finish",
            "stop",
            "stopped",
            "closed",
            "result",
        ],
    ) || message_kind == CodexAppServerMessageKind::Response
    {
        ProviderEventPhase::Completed
    } else if has_any(
        &tokens,
        &["started", "start", "begin", "opened", "created", "request"],
    ) || message_kind == CodexAppServerMessageKind::Request
    {
        ProviderEventPhase::Started
    } else {
        ProviderEventPhase::Unknown
    }
}

fn string_at_any(value: &Value, pointers: &[&str]) -> Option<String> {
    pointers.iter().find_map(|pointer| {
        value.pointer(pointer).and_then(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .or_else(|| {
                    value.as_array().and_then(|values| {
                        let parts = values.iter().filter_map(Value::as_str).collect::<Vec<_>>();
                        (!parts.is_empty()).then(|| parts.join("\n"))
                    })
                })
                .or_else(|| value.as_i64().map(|number| number.to_string()))
                .or_else(|| value.as_u64().map(|number| number.to_string()))
        })
    })
}

fn value_at_any<'a>(value: &'a Value, pointers: &[&str]) -> Option<&'a Value> {
    pointers.iter().find_map(|pointer| value.pointer(pointer))
}

fn display_value(value: &Value) -> Option<String> {
    let display = match value {
        Value::Null => return None,
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Array(values) => {
            let strings = values.iter().filter_map(Value::as_str).collect::<Vec<_>>();
            if strings.len() == values.len() {
                strings.join("\n")
            } else {
                serde_json::to_string_pretty(value).ok()?
            }
        }
        Value::Object(_) => serde_json::to_string_pretty(value).ok()?,
    };
    (!display.trim().is_empty()).then_some(display)
}

fn number_at_any(value: &Value, pointers: &[&str]) -> Option<u64> {
    pointers
        .iter()
        .find_map(|pointer| value.pointer(pointer).and_then(Value::as_u64))
}

fn json_rpc_id_to_string(id: &Value) -> String {
    match id {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        other => other.to_string(),
    }
}

fn split_camel_segments(value: &str) -> String {
    let mut output = String::new();
    let mut previous_lower_or_digit = false;
    for ch in value.chars() {
        if ch.is_ascii_uppercase() && previous_lower_or_digit {
            output.push('_');
        }
        previous_lower_or_digit = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        output.push(ch);
    }
    output
}

fn native_path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
fn file_uri(path: &Path) -> String {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    format!("file://{}", percent_encode_path(&path.to_string_lossy()))
}

#[cfg(test)]
fn percent_encode_path(path: &str) -> String {
    let mut encoded = String::new();
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(char::from(byte));
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;
    use std::path::Path;

    #[test]
    fn codex_error_response_fails_and_settles_steer_input() {
        let mut adapter = CodexManagedAdapter::new(HarnessAdapterContext {
            session_id: 7,
            thread_id: 11,
            workspace: "berlin".to_owned(),
            native_session_id: Some("codex-thread-1".to_owned()),
            controls: Default::default(),
        });
        adapter
            .encode_input(managed_input("turn-input", "run tests", false))
            .unwrap();
        adapter
            .observe_native(managed_record(
                r#"{"method":"turn/started","params":{"threadId":"codex-thread-1","turn":{"id":"turn-1"}}}"#,
            ))
            .unwrap();
        adapter
            .encode_input(managed_input("steer-input", "also run clippy", true))
            .unwrap();
        assert_eq!(adapter.active_input_id.as_deref(), Some("turn-input"));
        assert_eq!(
            adapter.pending_inputs.get(&2).map(String::as_str),
            Some("steer-input")
        );

        let effects = adapter
            .observe_native(managed_record(
                r#"{"id":2,"error":{"code":-32000,"message":"turn already completed"}}"#,
            ))
            .unwrap();

        assert!(!effects.iter().any(|effect| matches!(
            effect,
            HarnessEffect::InputAcknowledged { local_input_id } if local_input_id == "steer-input"
        )));
        assert!(effects.iter().any(|effect| matches!(
            effect,
            HarnessEffect::TurnCompleted {
                local_input_id,
                status: HarnessTurnStatus::Failed,
            } if local_input_id == "steer-input"
        )));
        assert!(!adapter.pending_inputs.contains_key(&2));
        assert_eq!(adapter.active_input_id.as_deref(), Some("turn-input"));
    }

    #[test]
    fn codex_rejects_new_start_while_start_request_is_pending() {
        let mut adapter = CodexManagedAdapter::new(HarnessAdapterContext {
            session_id: 7,
            thread_id: 11,
            workspace: "berlin".to_owned(),
            native_session_id: Some("codex-thread-1".to_owned()),
            controls: Default::default(),
        });

        adapter
            .encode_input(managed_input("first-input", "run tests", false))
            .unwrap();
        let err = adapter
            .encode_input(managed_input("second-input", "run clippy", false))
            .unwrap_err();

        assert!(err.to_string().contains("pending"));
        assert_eq!(adapter.active_input_id.as_deref(), Some("first-input"));
        assert_eq!(
            adapter.pending_inputs.get(&1).map(String::as_str),
            Some("first-input")
        );
        assert!(!adapter
            .pending_inputs
            .values()
            .any(|id| id == "second-input"));
    }

    #[test]
    fn codex_start_failure_clears_pending_start_and_reports_ready() {
        let mut adapter = CodexManagedAdapter::new(HarnessAdapterContext {
            session_id: 7,
            thread_id: 11,
            workspace: "berlin".to_owned(),
            native_session_id: Some("codex-thread-1".to_owned()),
            controls: Default::default(),
        });
        adapter
            .encode_input(managed_input("first-input", "run tests", false))
            .unwrap();

        let effects = adapter
            .observe_native(managed_record(
                r#"{"id":1,"error":{"code":-32000,"message":"failed to start turn"}}"#,
            ))
            .unwrap();

        assert!(effects.iter().any(|effect| matches!(
            effect,
            HarnessEffect::TurnCompleted {
                local_input_id,
                status: HarnessTurnStatus::Failed,
            } if local_input_id == "first-input"
        )));
        assert!(effects
            .iter()
            .any(|effect| matches!(effect, HarnessEffect::Ready)));
        assert!(adapter.active_input_id.is_none());
        assert!(adapter.pending_inputs.is_empty());
    }

    fn managed_input(local_input_id: &str, content: &str, immediate: bool) -> HarnessInput {
        HarnessInput {
            local_input_id: local_input_id.to_owned(),
            content: content.to_owned(),
            visible_content: None,
            kind: crate::archcar::protocol::ArchcarInputKind::User,
            delivery: if immediate {
                crate::archcar::protocol::ArchcarInputDelivery::Immediate
            } else {
                crate::archcar::protocol::ArchcarInputDelivery::Auto
            },
        }
    }

    fn managed_record(payload: &str) -> NativeRecord {
        NativeRecord {
            provider_key: CODEX_APP_SERVER_PROVIDER,
            payload: format!("{payload}\n").into_bytes(),
        }
    }

    #[test]
    fn capability_coverage_names_all_required_codex_categories() {
        let categories: Vec<_> = CODEX_APP_SERVER_CAPABILITY_COVERAGE
            .iter()
            .map(|coverage| coverage.category)
            .collect();

        assert_eq!(
            categories,
            vec![
                CodexProviderEventCategory::AccountAuth,
                CodexProviderEventCategory::ThreadSessionLifecycle,
                CodexProviderEventCategory::GoalsTasks,
                CodexProviderEventCategory::Turns,
                CodexProviderEventCategory::UserInput,
                CodexProviderEventCategory::AssistantOutput,
                CodexProviderEventCategory::PlanningReasoning,
                CodexProviderEventCategory::CommandProcessExecution,
                CodexProviderEventCategory::TerminalBackgroundRuntime,
                CodexProviderEventCategory::Filesystem,
                CodexProviderEventCategory::DiffsFileChanges,
                CodexProviderEventCategory::Tools,
                CodexProviderEventCategory::Mcp,
                CodexProviderEventCategory::SkillsPluginsHooks,
                CodexProviderEventCategory::ApprovalsPermissions,
                CodexProviderEventCategory::SubagentsCollaboration,
                CodexProviderEventCategory::WebBrowserMedia,
                CodexProviderEventCategory::EnvironmentConfigModel,
                CodexProviderEventCategory::LimitsFailures,
                CodexProviderEventCategory::Unknown,
            ]
        );
    }

    #[test]
    fn initialize_and_initialized_match_codex_app_server_wire_shape() {
        let mut output = Vec::new();
        let request_id = write_initialize_request(
            &mut output,
            &CodexAppServerInitializeParams {
                client_name: "archductor-test".to_owned(),
                client_title: Some("Archductor Test".to_owned()),
                client_version: Some("0.1.0".to_owned()),
                workspace_root: Some(Path::new("/tmp/workspace").to_path_buf()),
            },
        )
        .unwrap();
        write_initialized_notification(&mut output).unwrap();

        assert_eq!(request_id, 1);
        let lines: Vec<_> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect();

        assert_eq!(lines.len(), 2);
        assert!(lines[0].get("jsonrpc").is_none());
        assert_eq!(lines[0]["id"], 1);
        assert_eq!(lines[0]["method"], "initialize");
        assert_eq!(lines[0]["params"]["clientInfo"]["name"], "archductor-test");
        assert_eq!(lines[0]["params"]["clientInfo"]["title"], "Archductor Test");
        assert_eq!(lines[0]["params"]["clientInfo"]["version"], "0.1.0");
        assert!(lines[0]["params"].get("capabilities").is_none());
        assert!(lines[0]["params"].get("workspaceFolders").is_none());
        assert!(lines[0]["params"].get("rootUri").is_none());
        assert_eq!(lines[1], json!({"method": "initialized", "params": {}}));
    }

    #[test]
    fn current_app_server_messages_preserve_raw_json_and_classify_methods() {
        let input = Cursor::new(
            r#"{"method":"turn/started","params":{"turn":{"id":"t1"},"threadId":"thr_1"}}"#
                .to_owned()
                + "\n"
                + r#"{"method":"item/agentMessage/delta","params":{"threadId":"thr_1","turnId":"t1","itemId":"msg_1","delta":"hi"}}"#
                + "\n"
                + r#"{"id":7,"method":"item/commandExecution/requestApproval","params":{"threadId":"thr_1","turnId":"t1","itemId":"cmd_1"}}"#
                + "\n",
        );

        let messages = read_jsonl_messages(input).unwrap();

        assert_eq!(messages.len(), 3);
        assert_eq!(
            messages[0].raw_json,
            r#"{"method":"turn/started","params":{"turn":{"id":"t1"},"threadId":"thr_1"}}"#
        );
        assert_eq!(
            messages[0].message_kind,
            CodexAppServerMessageKind::Notification
        );
        assert_eq!(messages[0].method.as_deref(), Some("turn/started"));

        let drafts: Vec<_> = messages
            .iter()
            .map(CodexAppServerMessage::to_provider_event_draft)
            .collect();

        assert_eq!(drafts[0].category, CodexProviderEventCategory::Turns);
        assert_eq!(
            drafts[1].category,
            CodexProviderEventCategory::AssistantOutput
        );
        assert_eq!(
            drafts[2].category,
            CodexProviderEventCategory::ApprovalsPermissions
        );
        assert_eq!(drafts[2].message_kind, CodexAppServerMessageKind::Request);
        assert_eq!(drafts[2].correlation_id.as_deref(), Some("7"));
        assert_eq!(drafts[2].raw_json, messages[2].raw_json);
    }

    #[test]
    fn response_messages_remain_unknown_without_request_method_context() {
        let message = parse_jsonl_message(r#"{"id":7,"result":{"ok":true}}"#, 1).unwrap();

        let draft = message.to_provider_event_draft();

        assert_eq!(draft.message_kind, CodexAppServerMessageKind::Response);
        assert_eq!(draft.category, CodexProviderEventCategory::Unknown);
    }

    #[test]
    fn camel_case_item_methods_classify_to_core_categories() {
        assert_eq!(
            classify_codex_method(
                "item/agentMessage/delta",
                CodexAppServerMessageKind::Notification
            ),
            CodexProviderEventCategory::AssistantOutput
        );
        assert_eq!(
            classify_codex_method(
                "approval/commandApproval/request",
                CodexAppServerMessageKind::Request
            ),
            CodexProviderEventCategory::ApprovalsPermissions
        );
    }

    #[test]
    fn item_lifecycle_messages_classify_from_documented_item_type() {
        let cases = [
            (
                r#"{"method":"item/completed","params":{"threadId":"thr_1","turnId":"turn_1","item":{"type":"agentMessage","id":"msg_1","text":"Done."}}}"#,
                CodexProviderEventCategory::AssistantOutput,
                "item/agentMessage/completed",
                "Done.",
            ),
            (
                r#"{"method":"item/started","params":{"threadId":"thr_1","turnId":"turn_1","item":{"type":"reasoning","id":"reason_1","summary":"Checking the mapper."}}}"#,
                CodexProviderEventCategory::PlanningReasoning,
                "item/reasoning/started",
                "Checking the mapper.",
            ),
            (
                r#"{"method":"item/completed","params":{"threadId":"thr_1","turnId":"turn_1","item":{"type":"commandExecution","id":"cmd_1","command":["cargo","test"],"aggregatedOutput":"ok"}}}"#,
                CodexProviderEventCategory::CommandProcessExecution,
                "item/commandExecution/completed",
                "cargo test\nok",
            ),
            (
                r#"{"method":"item/completed","params":{"threadId":"thr_1","turnId":"turn_1","item":{"type":"fileChange","id":"file_1","changes":[{"path":"src/main.rs","kind":"modified","diff":"@@"}],"status":"completed"}}}"#,
                CodexProviderEventCategory::DiffsFileChanges,
                "item/fileChange/completed",
                "modified src/main.rs",
            ),
        ];

        for (raw, category, name, body) in cases {
            let draft = parse_jsonl_message(raw, 1)
                .unwrap()
                .to_provider_event_draft();

            assert_eq!(draft.category, category);
            assert_eq!(draft.name, name);
            let event =
                draft.into_provider_event_draft(crate::provider_events::ProviderEventContext {
                    workspace_id: Some(1),
                    chat_thread_id: Some(7),
                    process_id: Some(9),
                    occurred_at_ms: 42,
                    schema_version: 1,
                    adapter_version: "codex-app-server-test".to_owned(),
                });
            assert_eq!(event.provider_thread_id.as_deref(), Some("thr_1"));
            assert_eq!(event.provider_turn_id.as_deref(), Some("turn_1"));
            assert_eq!(event.normalized_payload["body"], body);
        }
    }

    #[test]
    fn dynamic_tool_calls_map_local_tool_names_to_action_kinds() {
        let cases = [
            (
                "Bash",
                r#"{"command":"cargo test"}"#,
                crate::provider_events::ProviderEventKind::CommandProcess,
                "command",
            ),
            (
                "Read",
                r#"{"path":"src/main.rs"}"#,
                crate::provider_events::ProviderEventKind::FileSystem,
                "read",
            ),
            (
                "Edit",
                r#"{"path":"src/main.rs","old_string":"a","new_string":"b"}"#,
                crate::provider_events::ProviderEventKind::FileSystem,
                "edit",
            ),
            (
                "MultiEdit",
                r#"{"path":"src/main.rs","edits":[]}"#,
                crate::provider_events::ProviderEventKind::FileSystem,
                "edit",
            ),
            (
                "Write",
                r#"{"path":"src/main.rs","content":"x"}"#,
                crate::provider_events::ProviderEventKind::FileSystem,
                "write",
            ),
        ];

        for (tool_name, arguments, expected_kind, expected_subtype) in cases {
            let arguments_value: serde_json::Value = serde_json::from_str(arguments).unwrap();
            let raw = serde_json::json!({
                "method": "item/completed",
                "params": {
                    "threadId": "thr_1",
                    "turnId": "turn_1",
                    "item": {
                        "type": "dynamicToolCall",
                        "id": "tool_1",
                        "tool": tool_name,
                        "arguments": arguments_value,
                        "status": "completed",
                    },
                },
            })
            .to_string();
            let event = parse_jsonl_message(&raw, 1)
                .unwrap()
                .to_provider_event_draft()
                .into_provider_event_draft(crate::provider_events::ProviderEventContext {
                    workspace_id: Some(1),
                    chat_thread_id: Some(7),
                    process_id: Some(9),
                    occurred_at_ms: 42,
                    schema_version: 1,
                    adapter_version: "codex-app-server-test".to_owned(),
                });

            assert_eq!(event.kind, expected_kind, "{tool_name}");
            assert_eq!(
                event.provider_subtype.as_deref(),
                Some(expected_subtype),
                "{tool_name}"
            );
        }
    }

    #[test]
    fn dynamic_tool_call_requests_map_local_tool_names_to_action_kinds() {
        let cases = [
            (
                "Bash",
                serde_json::json!({"command": "cargo test"}),
                crate::provider_events::ProviderEventKind::CommandProcess,
                "command",
            ),
            (
                "Read",
                serde_json::json!({"path": "src/main.rs"}),
                crate::provider_events::ProviderEventKind::FileSystem,
                "read",
            ),
            (
                "Edit",
                serde_json::json!({"path": "src/main.rs", "old_string": "a", "new_string": "b"}),
                crate::provider_events::ProviderEventKind::FileSystem,
                "edit",
            ),
        ];

        for (tool_name, arguments, expected_kind, expected_subtype) in cases {
            let raw = serde_json::json!({
                "id": 9,
                "method": "item/tool/call",
                "params": {
                    "callId": "call_1",
                    "threadId": "thr_1",
                    "turnId": "turn_1",
                    "tool": tool_name,
                    "arguments": arguments,
                },
            })
            .to_string();
            let event = parse_jsonl_message(&raw, 1)
                .unwrap()
                .to_provider_event_draft()
                .into_provider_event_draft(crate::provider_events::ProviderEventContext {
                    workspace_id: Some(1),
                    chat_thread_id: Some(7),
                    process_id: Some(9),
                    occurred_at_ms: 42,
                    schema_version: 1,
                    adapter_version: "codex-app-server-test".to_owned(),
                });

            assert_eq!(event.kind, expected_kind, "{tool_name}");
            assert_eq!(
                event.provider_subtype.as_deref(),
                Some(expected_subtype),
                "{tool_name}"
            );
        }
    }

    #[test]
    fn mcp_startup_status_normalizes_to_one_readable_status_item() {
        let loading = parse_jsonl_message(
            r#"{"method":"mcpServer/startupStatus/updated","params":{"threadId":"thr_1","name":"github","status":"starting","error":null,"failureReason":null}}"#,
            1,
        )
        .unwrap()
        .to_provider_event_draft()
        .into_provider_event_draft(crate::provider_events::ProviderEventContext {
            workspace_id: Some(1),
            chat_thread_id: Some(7),
            process_id: Some(9),
            occurred_at_ms: 42,
            schema_version: 1,
            adapter_version: "codex-app-server-test".to_owned(),
        });

        assert_eq!(loading.kind, crate::provider_events::ProviderEventKind::Mcp);
        assert_eq!(
            loading.phase,
            crate::provider_events::ProviderEventPhase::Progress
        );
        assert_eq!(loading.provider_thread_id.as_deref(), Some("thr_1"));
        assert_eq!(
            loading.provider_item_id.as_deref(),
            Some("mcp-startup-status:github")
        );
        assert_eq!(
            loading.provider_subtype.as_deref(),
            Some("mcpServer/startupStatus/updated")
        );
        assert_eq!(loading.normalized_payload["title"], "MCP loading");
        assert_eq!(loading.normalized_payload["body"], "");

        let loaded = parse_jsonl_message(
            r#"{"method":"mcpServer/startupStatus/updated","params":{"threadId":"thr_1","name":"github","status":"ready","error":null,"failureReason":null}}"#,
            1,
        )
        .unwrap()
        .to_provider_event_draft()
        .into_provider_event_draft(crate::provider_events::ProviderEventContext {
            workspace_id: Some(1),
            chat_thread_id: Some(7),
            process_id: Some(9),
            occurred_at_ms: 43,
            schema_version: 1,
            adapter_version: "codex-app-server-test".to_owned(),
        });

        assert_eq!(
            loaded.phase,
            crate::provider_events::ProviderEventPhase::Completed
        );
        assert_eq!(
            loaded.provider_item_id.as_deref(),
            Some("mcp-startup-status:github")
        );
        assert_eq!(loaded.normalized_payload["title"], "MCP loaded");
        assert_eq!(loaded.normalized_payload["body"], "github: ready");
        assert_ne!(
            loaded.normalized_payload["title"],
            "mcpServer/startupStatus/updated"
        );
    }

    #[test]
    fn mcp_startup_status_keeps_each_server_identity_and_canceled_is_terminal() {
        let github = parse_jsonl_message(
            r#"{"method":"mcpServer/startupStatus/updated","params":{"threadId":"thr_1","name":"github","status":"failed","error":"bad token"}}"#,
            1,
        )
        .unwrap()
        .to_provider_event_draft()
        .into_provider_event_draft(crate::provider_events::ProviderEventContext {
            workspace_id: Some(1),
            chat_thread_id: Some(7),
            process_id: Some(9),
            occurred_at_ms: 42,
            schema_version: 1,
            adapter_version: "codex-app-server-test".to_owned(),
        });
        let linear = parse_jsonl_message(
            r#"{"method":"mcpServer/startupStatus/updated","params":{"threadId":"thr_1","name":"linear","status":"cancelled","error":null}}"#,
            1,
        )
        .unwrap()
        .to_provider_event_draft()
        .into_provider_event_draft(crate::provider_events::ProviderEventContext {
            workspace_id: Some(1),
            chat_thread_id: Some(7),
            process_id: Some(9),
            occurred_at_ms: 43,
            schema_version: 1,
            adapter_version: "codex-app-server-test".to_owned(),
        });

        assert_eq!(
            github.provider_item_id.as_deref(),
            Some("mcp-startup-status:github")
        );
        assert_eq!(
            linear.provider_item_id.as_deref(),
            Some("mcp-startup-status:linear")
        );
        assert_eq!(
            linear.phase,
            crate::provider_events::ProviderEventPhase::Interrupted
        );
        assert_eq!(linear.normalized_payload["body"], "linear: cancelled");
    }

    #[test]
    fn codex_completed_items_accept_documented_wire_shapes() {
        let cases = [
            (
                r#"{"method":"item/completed","params":{"threadId":"thr_1","turnId":"turn_1","item":{"type":"reasoning","id":"reason_1","summary":["Checked","constraints"],"content":["line 1","line 2"]}}}"#,
                "Checked\nconstraints\nline 1\nline 2",
            ),
            (
                r#"{"method":"item/completed","params":{"threadId":"thr_1","turnId":"turn_1","item":{"type":"commandExecution","id":"cmd_1","command":"cargo test","aggregatedOutput":"ok"}}}"#,
                "cargo test\nok",
            ),
            (
                r#"{"method":"item/completed","params":{"threadId":"thr_1","turnId":"turn_1","item":{"type":"mcpToolCall","id":"tool_1","result":{"ok":true},"arguments":{"secret":"ignored"}}}}"#,
                "{\n  \"ok\": true\n}",
            ),
        ];

        for (raw, body) in cases {
            let event = parse_jsonl_message(raw, 1)
                .unwrap()
                .to_provider_event_draft()
                .into_provider_event_draft(crate::provider_events::ProviderEventContext {
                    workspace_id: Some(1),
                    chat_thread_id: Some(7),
                    process_id: Some(9),
                    occurred_at_ms: 42,
                    schema_version: 1,
                    adapter_version: "codex-app-server-test".to_owned(),
                });

            assert_eq!(event.normalized_payload["body"], body);
        }
    }

    #[test]
    fn json_rpc_responses_are_written_with_original_ids() {
        let mut output = Vec::new();
        write_jsonl(
            &mut output,
            &json!({"id": "req-1", "result": {"approved": true}}),
        )
        .unwrap();

        let line: Value = serde_json::from_slice(&output).unwrap();
        assert!(line.get("jsonrpc").is_none());
        assert_eq!(line["id"], "req-1");
        assert_eq!(line["result"]["approved"], true);
        assert!(line.get("method").is_none());
    }

    #[test]
    fn provider_event_conversion_maps_tool_events_to_canonical_tool_kind() {
        let message = parse_jsonl_message(
            r#"{"method":"item/mcpToolCall/progress","params":{"threadId":"thread-1","itemId":"tool-1","delta":"running"}}"#,
            1,
        )
        .unwrap();

        let event = message.to_provider_event_draft().into_provider_event_draft(
            crate::provider_events::ProviderEventContext {
                workspace_id: Some(1),
                chat_thread_id: Some(7),
                process_id: Some(9),
                occurred_at_ms: 42,
                schema_version: 1,
                adapter_version: "codex-app-server-test".to_owned(),
            },
        );

        assert_eq!(event.kind, crate::provider_events::ProviderEventKind::Tool);
        assert_eq!(
            event.phase,
            crate::provider_events::ProviderEventPhase::Progress
        );
        assert_eq!(event.provider_thread_id.as_deref(), Some("thread-1"));
        assert_eq!(event.provider_item_id.as_deref(), Some("tool-1"));
        assert_eq!(event.normalized_payload["body"], "running");
        assert_eq!(event.raw_json["method"], "item/mcpToolCall/progress");
    }

    #[test]
    fn provider_event_conversion_preserves_unknown_future_raw_json() {
        let message = parse_jsonl_message(
            r#"{"method":"future/provider/thing","params":{"new":true}}"#,
            1,
        )
        .unwrap();

        let event = message.to_provider_event_draft().into_provider_event_draft(
            crate::provider_events::ProviderEventContext {
                workspace_id: None,
                chat_thread_id: Some(7),
                process_id: None,
                occurred_at_ms: 42,
                schema_version: 1,
                adapter_version: "codex-app-server-test".to_owned(),
            },
        );

        assert_eq!(
            event.kind,
            crate::provider_events::ProviderEventKind::Unknown
        );
        assert_eq!(event.raw_json["params"]["new"], true);
    }

    #[test]
    fn invalid_jsonl_reports_line_number_and_keeps_blank_lines_ignored() {
        let input = Cursor::new("\n{\"method\":\"thread/started\"}\nnot-json\n".as_bytes());

        let error = read_jsonl_messages(input).unwrap_err().to_string();

        assert!(error.contains("line 3"), "{error}");
    }

    #[test]
    fn file_uri_percent_encodes_reserved_path_characters() {
        assert_eq!(
            file_uri(Path::new("/tmp/work space/branch#1")),
            "file:///tmp/work%20space/branch%231"
        );
    }

    #[test]
    fn launch_command_uses_codex_app_server_stdio_jsonl() {
        let command = CodexAppServerLaunch::default();

        assert_eq!(command.executable, "codex");
        assert_eq!(command.args, vec!["app-server"]);
        assert_eq!(command.transport, CodexAppServerTransport::StdioJsonl);
    }

    #[test]
    fn thread_start_request_uses_documented_params() {
        let mut output = Vec::new();
        write_thread_start_request_with_id(
            &mut output,
            10,
            &CodexAppServerThreadStartParams {
                model: Some("gpt-5.4".to_owned()),
                cwd: Some(Path::new("/tmp/workspace").to_path_buf()),
                approval_policy: Some("never".to_owned()),
                sandbox: Some("workspaceWrite".to_owned()),
                service_name: Some("archductor".to_owned()),
            },
        )
        .unwrap();

        let line: Value = serde_json::from_slice(&output).unwrap();
        assert!(line.get("jsonrpc").is_none());
        assert_eq!(line["id"], 10);
        assert_eq!(line["method"], "thread/start");
        assert_eq!(line["params"]["model"], "gpt-5.4");
        assert_eq!(line["params"]["cwd"], "/tmp/workspace");
        assert_eq!(line["params"]["approvalPolicy"], "never");
        assert_eq!(line["params"]["sandbox"], "workspaceWrite");
        assert_eq!(line["params"]["serviceName"], "archductor");
    }

    #[test]
    fn turn_start_request_uses_text_input_items_and_sandbox_policy() {
        let mut output = Vec::new();
        write_turn_start_request_with_id(
            &mut output,
            30,
            &CodexAppServerTurnStartParams {
                thread_id: "thr_123".to_owned(),
                input: vec![CodexAppServerUserInput::Text {
                    text: "Run tests".to_owned(),
                }],
                cwd: Some(Path::new("/tmp/workspace").to_path_buf()),
                approval_policy: Some("unlessTrusted".to_owned()),
                sandbox_policy: Some(json!({
                    "type": "workspaceWrite",
                    "writableRoots": ["/tmp/workspace"],
                    "networkAccess": true,
                })),
                model: Some("gpt-5.4".to_owned()),
                effort: Some("medium".to_owned()),
                summary: Some("concise".to_owned()),
                personality: Some("friendly".to_owned()),
            },
        )
        .unwrap();

        let line: Value = serde_json::from_slice(&output).unwrap();
        assert!(line.get("jsonrpc").is_none());
        assert_eq!(line["id"], 30);
        assert_eq!(line["method"], "turn/start");
        assert_eq!(line["params"]["threadId"], "thr_123");
        assert_eq!(
            line["params"]["input"][0],
            json!({"type": "text", "text": "Run tests"})
        );
        assert_eq!(line["params"]["cwd"], "/tmp/workspace");
        assert_eq!(line["params"]["approvalPolicy"], "unlessTrusted");
        assert_eq!(line["params"]["sandboxPolicy"]["type"], "workspaceWrite");
        assert_eq!(line["params"]["model"], "gpt-5.4");
        assert_eq!(line["params"]["effort"], "medium");
        assert_eq!(line["params"]["summary"], "concise");
        assert_eq!(line["params"]["personality"], "friendly");
    }

    #[test]
    fn turn_steer_request_uses_documented_active_turn_method() {
        let mut output = Vec::new();
        write_turn_steer_request_with_id(
            &mut output,
            31,
            &CodexAppServerTurnSteerParams {
                thread_id: "thr_123".to_owned(),
                input: vec![CodexAppServerUserInput::Text {
                    text: "Adjust course".to_owned(),
                }],
                expected_turn_id: Some("turn_456".to_owned()),
            },
        )
        .unwrap();

        let line: Value = serde_json::from_slice(&output).unwrap();
        assert!(line.get("jsonrpc").is_none());
        assert_eq!(line["id"], 31);
        assert_eq!(line["method"], "turn/steer");
        assert_eq!(line["params"]["threadId"], "thr_123");
        assert_eq!(
            line["params"]["input"][0],
            json!({"type": "text", "text": "Adjust course"})
        );
        assert_eq!(line["params"]["expectedTurnId"], "turn_456");
    }

    #[test]
    fn turn_interrupt_request_uses_documented_active_turn_method() {
        let mut output = Vec::new();
        write_turn_interrupt_request_with_id(
            &mut output,
            32,
            &CodexAppServerTurnInterruptParams {
                thread_id: "thr_123".to_owned(),
                turn_id: "turn_456".to_owned(),
            },
        )
        .unwrap();

        let line: Value = serde_json::from_slice(&output).unwrap();
        assert!(line.get("jsonrpc").is_none());
        assert_eq!(line["id"], 32);
        assert_eq!(line["method"], "turn/interrupt");
        assert_eq!(line["params"]["threadId"], "thr_123");
        assert_eq!(line["params"]["turnId"], "turn_456");
    }
}
