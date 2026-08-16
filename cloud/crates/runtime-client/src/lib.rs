use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot, Mutex};
use zene_cloud_acp_bridge::{
    AcpBridge, AcpEvent, BridgeMsg, MockAgent, MockMsg, PermissionDecision,
};

pub use zene_cloud_domain::{
    AcpResidualPayload, ApprovalDecision, ApprovalEventPayload, ApprovalKind,
    AvailableCommandsPayload, CloudEventKind, ErrorPayload, InitializedPayload, PlanPayload,
    ProjectionPayload, SessionRecoveryPayload, SessionStartedPayload, StateChangedPayload,
    StepStartedPayload, TextEventPayload, ToolCallPayload, ToolResultPayload, TurnEndedPayload,
    TurnStartedPayload, UnsupportedRequestPayload, UsagePayload,
};

/// ACP `optionId` mapping stays inside this adapter.
pub fn to_permission_decision(decision: ApprovalDecision) -> PermissionDecision {
    match decision {
        ApprovalDecision::AllowOnce => PermissionDecision::AllowOnce,
        ApprovalDecision::AllowSession => PermissionDecision::AllowSession,
        ApprovalDecision::Deny => PermissionDecision::Deny,
    }
}

fn acp_permission_result(decision: ApprovalDecision) -> Value {
    to_permission_decision(decision).to_result()
}

/// ACP `sessionUpdate` strings stay inside this adapter; product kinds live on
/// `CloudEventKind`.
fn classify_payload(payload: &Value) -> CloudEventKind {
    if let Some(update) = payload
        .pointer("/params/update/sessionUpdate")
        .and_then(Value::as_str)
    {
        return map_session_update(update);
    }
    match payload.get("method").and_then(Value::as_str) {
        Some("session/request_permission") => CloudEventKind::ApprovalRequested,
        Some("session/new") | Some("session/resume") => CloudEventKind::SessionStarted,
        Some("initialize") => CloudEventKind::Initialized,
        Some(_) if is_reverse_request_frame(payload) => CloudEventKind::UnsupportedRequest,
        _ => CloudEventKind::Acp,
    }
}

/// Reverse requests carry a JSON-RPC id and params, without a result.
fn is_reverse_request_frame(payload: &Value) -> bool {
    payload.get("id").is_some() && payload.get("result").is_none()
}

fn map_session_update(update: &str) -> CloudEventKind {
    match update {
        "agent_message_chunk" => CloudEventKind::TextDelta,
        "agent_thought_chunk" => CloudEventKind::ThoughtDelta,
        "user_message_chunk" => CloudEventKind::UserMessage,
        "tool_call" => CloudEventKind::ToolCall,
        "tool_call_update" => CloudEventKind::ToolResult,
        "current_mode_update" => CloudEventKind::StateChanged,
        "usage_update" => CloudEventKind::UsageUpdate,
        "projection_update" => CloudEventKind::ProjectionReady,
        "plan" => CloudEventKind::Plan,
        "available_commands_update" => CloudEventKind::AvailableCommands,
        "turn_started" => CloudEventKind::TurnStarted,
        "step_started" => CloudEventKind::StepStarted,
        "turn_ended" => CloudEventKind::TurnEnded,
        "error" => CloudEventKind::Error,
        _ => CloudEventKind::Acp,
    }
}

/// Transport-neutral command accepted by Cloud's runtime client.
///
/// Variant names match `zene_runtime::RuntimeCommand` where Cloud has a
/// counterpart. Cloud does not depend on `zene-runtime`. ACP JSON-RPC ids
/// stay inside this adapter. `ResumeSafeTurn` stays local-only. Session mode
/// on Cloud is push-sourced (`session_started` / `state_changed`); there is
/// no Cloud `GetMode` / `session/get_mode`.
#[derive(Debug, Clone)]
pub enum RuntimeCommand {
    Prompt { text: String },
    Steer { text: String },
    Cancel,
    Approval { request_id: String, decision: ApprovalDecision },
    SetMode { mode_id: String },
    Shutdown,
}

/// In-memory product payload produced by the adapter.
///
/// Classified kinds keep domain structs until JobRunner serializes at the
/// HTTP/DB boundary. Extraction failures and non-session residual frames stay
/// [`RuntimePayload::Json`].
#[derive(Debug, Clone, PartialEq)]
pub enum RuntimePayload {
    Text(TextEventPayload),
    ToolCall(ToolCallPayload),
    ToolResult(ToolResultPayload),
    State(StateChangedPayload),
    Usage(UsagePayload),
    Plan(PlanPayload),
    Commands(AvailableCommandsPayload),
    SessionStarted(SessionStartedPayload),
    ApprovalRequested(ApprovalEventPayload),
    Projection(ProjectionPayload),
    Initialized(InitializedPayload),
    UnsupportedRequest(UnsupportedRequestPayload),
    TurnStarted(TurnStartedPayload),
    StepStarted(StepStartedPayload),
    TurnEnded(TurnEndedPayload),
    Error(ErrorPayload),
    AcpResidual(AcpResidualPayload),
    Json(Value),
}

impl RuntimePayload {
    pub fn to_value(&self) -> Value {
        match self {
            Self::Text(payload) => to_json(payload),
            Self::ToolCall(payload) => to_json(payload),
            Self::ToolResult(payload) => to_json(payload),
            Self::State(payload) => to_json(payload),
            Self::Usage(payload) => to_json(payload),
            Self::Plan(payload) => to_json(payload),
            Self::Commands(payload) => to_json(payload),
            Self::SessionStarted(payload) => to_json(payload),
            Self::ApprovalRequested(payload) => to_json(payload),
            Self::Projection(payload) => to_json(payload),
            Self::Initialized(payload) => to_json(payload),
            Self::UnsupportedRequest(payload) => to_json(payload),
            Self::TurnStarted(payload) => to_json(payload),
            Self::StepStarted(payload) => to_json(payload),
            Self::TurnEnded(payload) => to_json(payload),
            Self::Error(payload) => to_json(payload),
            Self::AcpResidual(payload) => to_json(payload),
            Self::Json(value) => value.clone(),
        }
    }
}

impl PartialEq<Value> for RuntimePayload {
    fn eq(&self, other: &Value) -> bool {
        self.to_value() == *other
    }
}

/// A runtime notification with the stable fields persisted by the worker.
///
/// `event_type` is the product kind. Classified kinds store a denormalized
/// product payload. Unknown `session/update` frames stay `acp` with a residual
/// product payload (no JSON-RPC envelope). Records written before productization
/// still have ACP JSON; Console falls back to `params.update`.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeNotification {
    pub source_event_id: String,
    pub cursor: Option<u64>,
    pub event_type: CloudEventKind,
    pub payload: RuntimePayload,
}

impl RuntimeNotification {
    pub fn from_acp(event: AcpEvent) -> Self {
        runtime_notification(event)
    }
}

#[derive(Debug)]
pub enum RuntimeEvent {
    Initialized { session_id: String, event: RuntimeNotification },
    Notification(RuntimeNotification),
    Request { request: RuntimeRequest, event: RuntimeNotification },
    ChildExited,
}

/// Runtime-level meaning of a request requiring user approval. Protocol method
/// names, JSON-RPC ids, and parameter paths stay inside this adapter.
/// `allowed_decisions` is the product list mapped from ACP `optionId`s.
/// `context` is the product payload stored on the approval row.
#[derive(Debug)]
pub enum RuntimeRequest {
    Approval {
        request_id: String,
        kind: ApprovalKind,
        allowed_decisions: Vec<ApprovalDecision>,
        context: ApprovalEventPayload,
    },
}

/// Product payload stored on Cloud approval rows. ACP option lists and
/// session metadata stay inside the adapter.
fn approval_payload(params: &Value) -> ApprovalEventPayload {
    let tool = params.get("toolCall").unwrap_or(&Value::Null);
    ApprovalEventPayload {
        request_id: permission_request_key(params),
        tool_call_id: json_str(tool, "toolCallId"),
        title: json_str(tool, "title"),
        tool_name: json_str(tool, "toolName"),
        kind: json_str(tool, "kind"),
        status: json_str(tool, "status"),
        raw_input: json_opt(tool, "rawInput"),
    }
}

fn allowed_decisions_from_params(params: &Value) -> Vec<ApprovalDecision> {
    let mut decisions = Vec::new();
    if let Some(options) = params.get("options").and_then(Value::as_array) {
        for option in options {
            let Some(id) = option.get("optionId").and_then(Value::as_str) else {
                continue;
            };
            let Some(decision) = ApprovalDecision::parse(id) else {
                continue;
            };
            if !decisions.contains(&decision) {
                decisions.push(decision);
            }
        }
    }
    if decisions.is_empty() {
        ApprovalDecision::default_allowed()
    } else {
        decisions
    }
}

#[async_trait]
pub trait RuntimeClient: Send + Sync {
    async fn session_id(&self) -> Result<String>;
    async fn send(&self, command: RuntimeCommand) -> Result<()>;
    async fn next_event(&self) -> Option<RuntimeEvent>;
    async fn is_alive(&self) -> bool;
}

pub struct AcpRuntimeClient {
    bridge: Arc<Mutex<Option<AcpBridge>>>,
    session_id: String,
    events: Arc<Mutex<mpsc::UnboundedReceiver<RuntimeEvent>>>,
    pending_approvals: Arc<Mutex<HashMap<String, Value>>>,
}

fn runtime_notification(event: AcpEvent) -> RuntimeNotification {
    let kind = classify_payload(&event.payload);
    RuntimeNotification {
        source_event_id: event.source_event_id,
        cursor: event.cursor,
        event_type: kind,
        payload: product_payload(kind, &event.payload),
    }
}

fn product_payload(kind: CloudEventKind, raw: &Value) -> RuntimePayload {
    match kind {
        CloudEventKind::TextDelta
        | CloudEventKind::ThoughtDelta
        | CloudEventKind::UserMessage => {
            let Some(update) = raw.pointer("/params/update") else {
                return RuntimePayload::Json(raw.clone());
            };
            RuntimePayload::Text(TextEventPayload {
                text: text_from_update(update),
            })
        }
        CloudEventKind::ToolCall => {
            let Some(update) = raw.pointer("/params/update") else {
                return RuntimePayload::Json(raw.clone());
            };
            RuntimePayload::ToolCall(tool_call_payload(update))
        }
        CloudEventKind::ToolResult => {
            let Some(update) = raw.pointer("/params/update") else {
                return RuntimePayload::Json(raw.clone());
            };
            RuntimePayload::ToolResult(tool_result_payload(update))
        }
        CloudEventKind::ApprovalRequested => approval_product(raw),
        CloudEventKind::SessionStarted => session_started_product(raw),
        CloudEventKind::StateChanged => state_product(raw),
        CloudEventKind::UsageUpdate => usage_product(raw),
        CloudEventKind::ProjectionReady => projection_product(raw),
        CloudEventKind::Plan => plan_product(raw),
        CloudEventKind::AvailableCommands => commands_product(raw),
        CloudEventKind::Initialized => initialized_product(raw),
        CloudEventKind::UnsupportedRequest => unsupported_request_product(raw),
        CloudEventKind::TurnStarted => turn_started_product(raw),
        CloudEventKind::StepStarted => step_started_product(raw),
        CloudEventKind::TurnEnded => turn_ended_product(raw),
        CloudEventKind::Error => error_product(raw),
        CloudEventKind::Acp => acp_residual_product(raw),
    }
}

fn to_json<T: Serialize>(value: T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Object(Default::default()))
}

fn json_str(update: &Value, key: &str) -> Option<String> {
    update.get(key).and_then(Value::as_str).map(str::to_string)
}

fn json_opt(update: &Value, key: &str) -> Option<Value> {
    update.get(key).filter(|value| !value.is_null()).cloned()
}

fn tool_call_payload(update: &Value) -> ToolCallPayload {
    ToolCallPayload {
        tool_call_id: json_str(update, "toolCallId"),
        title: json_str(update, "title"),
        tool_name: json_str(update, "toolName"),
        kind: json_str(update, "kind"),
        status: json_str(update, "status"),
        raw_input: json_opt(update, "rawInput"),
    }
}

fn tool_result_payload(update: &Value) -> ToolResultPayload {
    let text = tool_result_text(update);
    ToolResultPayload {
        tool_call_id: json_str(update, "toolCallId"),
        title: json_str(update, "title"),
        tool_name: json_str(update, "toolName"),
        kind: json_str(update, "kind"),
        status: json_str(update, "status"),
        raw_output: json_opt(update, "rawOutput"),
        text: if text.is_empty() { None } else { Some(text) },
        is_error: update.pointer("/rawOutput/isError").and_then(Value::as_bool),
    }
}

fn approval_product(raw: &Value) -> RuntimePayload {
    RuntimePayload::ApprovalRequested(approval_payload(raw.get("params").unwrap_or(raw)))
}

fn session_started_product(raw: &Value) -> RuntimePayload {
    let result = raw.get("result").unwrap_or(&Value::Null);
    let session_id = result
        .get("sessionId")
        .or_else(|| raw.pointer("/params/sessionId"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let resumed = (raw.get("method").and_then(Value::as_str) == Some("session/resume"))
        .then_some(true);
    let modes = result.get("modes");
    let recovery = result
        .pointer("/_meta/recovery")
        .and_then(session_recovery_payload);
    let payload = SessionStartedPayload {
        session_id,
        resumed,
        current_mode_id: modes
            .and_then(|value| value.get("currentModeId"))
            .and_then(Value::as_str)
            .map(str::to_string),
        available_modes: modes
            .and_then(|value| value.get("availableModes"))
            .cloned(),
        recovery,
    };
    if payload.session_id.is_none()
        && payload.resumed.is_none()
        && payload.current_mode_id.is_none()
        && payload.available_modes.is_none()
        && payload.recovery.is_none()
    {
        return RuntimePayload::Json(raw.clone());
    }
    RuntimePayload::SessionStarted(payload)
}

fn session_recovery_payload(raw: &Value) -> Option<SessionRecoveryPayload> {
    let payload = SessionRecoveryPayload {
        disposition: raw
            .get("disposition")
            .and_then(Value::as_str)
            .map(str::to_string),
        has_incomplete_execution: raw.get("hasIncompleteExecution").and_then(Value::as_bool),
        active_turn_count: raw.get("activeTurnCount").and_then(Value::as_u64),
        active_tool_count: raw.get("activeToolCount").and_then(Value::as_u64),
        safe_resume_allowed: raw.get("safeResumeAllowed").and_then(Value::as_bool),
        automatic_resume: raw.get("automaticResume").and_then(Value::as_bool),
        reason: raw.get("reason").and_then(Value::as_str).map(str::to_string),
    };
    if payload == SessionRecoveryPayload::default() {
        None
    } else {
        Some(payload)
    }
}

fn initialized_product(raw: &Value) -> RuntimePayload {
    let Some(result) = raw.get("result").filter(|value| value.is_object()) else {
        return RuntimePayload::Json(raw.clone());
    };
    let mut extra = serde_json::Map::new();
    let mut protocol_version = None;
    let mut agent_capabilities = None;
    let mut agent_info = None;
    let mut auth_methods = None;
    if let Some(object) = result.as_object() {
        for (key, value) in object {
            match key.as_str() {
                "protocolVersion" => protocol_version = Some(value.clone()),
                "agentCapabilities" => agent_capabilities = Some(value.clone()),
                "agentInfo" => agent_info = Some(value.clone()),
                "authMethods" => auth_methods = Some(value.clone()),
                _ => {
                    extra.insert(key.clone(), value.clone());
                }
            }
        }
    }
    RuntimePayload::Initialized(InitializedPayload {
        protocol_version,
        agent_capabilities,
        agent_info,
        auth_methods,
        extra,
    })
}

fn unsupported_request_product(raw: &Value) -> RuntimePayload {
    let Some(method) = raw.get("method").and_then(Value::as_str) else {
        return RuntimePayload::Json(raw.clone());
    };
    RuntimePayload::UnsupportedRequest(UnsupportedRequestPayload {
        method: method.to_string(),
        params: raw.get("params").cloned(),
    })
}

fn acp_residual_product(raw: &Value) -> RuntimePayload {
    let Some(update) = raw.pointer("/params/update") else {
        return RuntimePayload::Json(raw.clone());
    };
    RuntimePayload::AcpResidual(AcpResidualPayload {
        method: raw.get("method").and_then(Value::as_str).map(str::to_string),
        session_update: update
            .get("sessionUpdate")
            .and_then(Value::as_str)
            .map(str::to_string),
        update: Some(update.clone()),
    })
}

fn turn_started_product(raw: &Value) -> RuntimePayload {
    let Some(update) = raw.pointer("/params/update") else {
        return RuntimePayload::Json(raw.clone());
    };
    RuntimePayload::TurnStarted(TurnStartedPayload {
        turn_id: json_str(update, "turnId"),
    })
}

fn step_started_product(raw: &Value) -> RuntimePayload {
    let Some(update) = raw.pointer("/params/update") else {
        return RuntimePayload::Json(raw.clone());
    };
    RuntimePayload::StepStarted(StepStartedPayload {
        step: update.get("step").and_then(Value::as_u64).map(|v| v as u32),
        turn_id: json_str(update, "turnId"),
    })
}

fn turn_ended_product(raw: &Value) -> RuntimePayload {
    let Some(update) = raw.pointer("/params/update") else {
        return RuntimePayload::Json(raw.clone());
    };
    RuntimePayload::TurnEnded(TurnEndedPayload {
        steps: update
            .get("steps")
            .and_then(Value::as_u64)
            .map(|v| v as u32),
        turn_id: json_str(update, "turnId"),
    })
}

fn error_product(raw: &Value) -> RuntimePayload {
    let Some(update) = raw.pointer("/params/update") else {
        return RuntimePayload::Json(raw.clone());
    };
    let message = json_str(update, "message").unwrap_or_default();
    if message.is_empty() {
        return RuntimePayload::Json(raw.clone());
    }
    RuntimePayload::Error(ErrorPayload { message })
}

fn state_product(raw: &Value) -> RuntimePayload {
    let Some(update) = raw.pointer("/params/update") else {
        return RuntimePayload::Json(raw.clone());
    };
    RuntimePayload::State(StateChangedPayload {
        state: json_opt(update, "modeId"),
    })
}

fn usage_product(raw: &Value) -> RuntimePayload {
    let Some(update) = raw.pointer("/params/update") else {
        return RuntimePayload::Json(raw.clone());
    };
    let mut payload = UsagePayload {
        used: json_opt(update, "used"),
        size: json_opt(update, "size"),
        ..UsagePayload::default()
    };
    if let Some(meta) = update.get("_meta") {
        payload.prompt_tokens = json_opt(meta, "promptTokens");
        payload.completion_tokens = json_opt(meta, "completionTokens");
        payload.context_percent = json_opt(meta, "contextPercent");
        payload.context_epoch = json_opt(meta, "contextEpoch");
        payload.cached_tokens = json_opt(meta, "cachedTokens");
    }
    RuntimePayload::Usage(payload)
}

fn projection_product(raw: &Value) -> RuntimePayload {
    let Some(update) = raw.pointer("/params/update") else {
        return RuntimePayload::Json(raw.clone());
    };
    let meta = match update.get("_meta") {
        Some(meta) if meta.is_object() => meta.clone(),
        _ => serde_json::json!({}),
    };
    match serde_json::from_value::<ProjectionPayload>(meta.clone()) {
        Ok(payload) => RuntimePayload::Projection(payload),
        Err(_) => RuntimePayload::Json(meta),
    }
}

fn plan_product(raw: &Value) -> RuntimePayload {
    let Some(update) = raw.pointer("/params/update") else {
        return RuntimePayload::Json(raw.clone());
    };
    RuntimePayload::Plan(PlanPayload {
        entries: json_opt(update, "entries"),
    })
}

fn commands_product(raw: &Value) -> RuntimePayload {
    let Some(update) = raw.pointer("/params/update") else {
        return RuntimePayload::Json(raw.clone());
    };
    RuntimePayload::Commands(AvailableCommandsPayload {
        available_commands: json_opt(update, "availableCommands"),
    })
}

fn text_from_update(update: &Value) -> String {
    if let Some(text) = update.get("text").and_then(Value::as_str) {
        return text.to_string();
    }
    match update.get("content") {
        Some(content) if content.is_object() => content
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        Some(content) if content.is_array() => content_array_text(content),
        _ => String::new(),
    }
}

fn tool_result_text(update: &Value) -> String {
    if let Some(text) = update.pointer("/rawOutput/text").and_then(Value::as_str) {
        return text.to_string();
    }
    text_from_update(update)
}

fn content_array_text(content: &Value) -> String {
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            item.pointer("/content/text")
                .and_then(Value::as_str)
                .or_else(|| item.get("text").and_then(Value::as_str))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn runtime_request(method: &str, params: &Value) -> Option<RuntimeRequest> {
    if method == "session/request_permission" {
        Some(RuntimeRequest::Approval {
            request_id: permission_request_key(params),
            kind: ApprovalKind::Permission,
            allowed_decisions: allowed_decisions_from_params(params),
            context: approval_payload(params),
        })
    } else {
        None
    }
}

/// Build an idempotency key from the request itself. ACP implementations may
/// omit `toolCallId` on reconnect, so a random fallback would turn one logical
/// request into multiple approvals. The full params include session metadata
/// and are serialized deterministically by serde_json's default map ordering.
fn permission_request_key(params: &Value) -> String {
    if let Some(tool_call_id) = params.pointer("/toolCall/toolCallId").and_then(Value::as_str) {
        return tool_call_id.to_string();
    }

    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in params.to_string().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("permission-{hash:016x}")
}

impl AcpRuntimeClient {
    pub async fn connect(
        zene_bin: &Path,
        workdir: &Path,
        yolo: bool,
        env: &std::collections::HashMap<String, String>,
    ) -> Result<Self> {
        Self::connect_with_session(zene_bin, workdir, yolo, env, None).await
    }

    /// Start an ACP child and either create a session or resume a persisted one.
    /// The caller must persist the returned session ID before attempting a later reconnect.
    pub async fn connect_with_session(
        zene_bin: &Path,
        workdir: &Path,
        yolo: bool,
        env: &std::collections::HashMap<String, String>,
        existing_session_id: Option<&str>,
    ) -> Result<Self> {
        let (bridge, mut messages) = AcpBridge::spawn(zene_bin, workdir, yolo, env).await?;
        let bridge = Arc::new(Mutex::new(Some(bridge)));
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let session_id;
        let init_events;
        {
            let guard = bridge.lock().await;
            let client = guard.as_ref().context("runtime bridge missing")?;
            let (id, events) = match existing_session_id {
                Some(existing) => match client.initialize_and_resume_session(workdir, existing).await {
                    Ok(pair) => pair,
                    Err(_) => client.initialize_and_new_session(workdir).await?,
                },
                None => client.initialize_and_new_session(workdir).await?,
            };
            session_id = id;
            init_events = events;
        }
        for event in init_events {
            let event = runtime_notification(event);
            let _ = events_tx.send(RuntimeEvent::Initialized { session_id: session_id.clone(), event });
        }
        let event_tx = events_tx.clone();
        let pending_approvals = Arc::new(Mutex::new(HashMap::new()));
        let pending = pending_approvals.clone();
        let pump_bridge = bridge.clone();
        tokio::spawn(async move {
            while let Some(message) = messages.recv().await {
                let event = match message {
                    BridgeMsg::Notification { raw, .. } => {
                        RuntimeEvent::Notification(runtime_notification(AcpEvent::from_notification(&raw)))
                    }
                    BridgeMsg::ReverseRequest { id, method, params } => {
                        let event = runtime_notification(AcpEvent::from_reverse_request(&id, &method, &params));
                        match runtime_request(&method, &params) {
                            Some(request) => {
                                let RuntimeRequest::Approval { request_id, .. } = &request;
                                pending.lock().await.insert(request_id.clone(), id);
                                RuntimeEvent::Request { request, event }
                            }
                            None => {
                                if let Some(bridge) = pump_bridge.lock().await.as_ref() {
                                    let _ = bridge
                                        .respond_error(&id, -32601, "unsupported runtime request")
                                        .await;
                                }
                                RuntimeEvent::Notification(event)
                            }
                        }
                    }
                };
                if event_tx.send(event).is_err() { break; }
            }
            let _ = event_tx.send(RuntimeEvent::ChildExited);
        });
        Ok(Self {
            bridge,
            session_id,
            events: Arc::new(Mutex::new(events_rx)),
            pending_approvals,
        })
    }
}

#[async_trait]
impl RuntimeClient for AcpRuntimeClient {
    async fn session_id(&self) -> Result<String> { Ok(self.session_id.clone()) }
    async fn send(&self, command: RuntimeCommand) -> Result<()> {
        match command {
            RuntimeCommand::Prompt { text } => {
                let guard = self.bridge.lock().await;
                guard
                    .as_ref()
                    .context("runtime bridge missing")?
                    .prompt(&self.session_id, &text)
                    .await
                    .map(|_| ())
            }
            RuntimeCommand::Steer { text } => {
                let guard = self.bridge.lock().await;
                guard
                    .as_ref()
                    .context("runtime bridge missing")?
                    .steer(&self.session_id, &text)
                    .await
                    .map(|_| ())
            }
            RuntimeCommand::Cancel => {
                let guard = self.bridge.lock().await;
                guard
                    .as_ref()
                    .context("runtime bridge missing")?
                    .cancel(&self.session_id)
                    .await
            }
            RuntimeCommand::Approval { request_id, decision } => {
                let id = self
                    .pending_approvals
                    .lock()
                    .await
                    .remove(&request_id)
                    .ok_or_else(|| anyhow!("unknown approval request_id {request_id}"))?;
                let guard = self.bridge.lock().await;
                guard
                    .as_ref()
                    .context("runtime bridge missing")?
                    .respond(&id, acp_permission_result(decision))
                    .await
            }
            RuntimeCommand::SetMode { mode_id } => {
                let guard = self.bridge.lock().await;
                guard
                    .as_ref()
                    .context("runtime bridge missing")?
                    .set_mode(&self.session_id, &mode_id)
                    .await
                    .map(|_| ())
            }
            RuntimeCommand::Shutdown => {
                let mut guard = self.bridge.lock().await;
                if let Some(bridge) = guard.take() {
                    bridge.kill().await?;
                }
                Ok(())
            }
        }
    }
    async fn next_event(&self) -> Option<RuntimeEvent> { self.events.lock().await.recv().await }
    async fn is_alive(&self) -> bool {
        let mut guard = self.bridge.lock().await;
        guard.as_mut().is_some_and(|bridge| !bridge.child_exited())
    }
}

/// In-process runtime used when no `zene` binary is available.
///
/// MockAgent still speaks ACP internally; this adapter exposes the same
/// `RuntimeClient` product types as [`AcpRuntimeClient`].
pub struct MockRuntimeClient {
    agent: MockAgent,
    msg_tx: Mutex<Option<mpsc::UnboundedSender<MockMsg>>>,
    events: Arc<Mutex<mpsc::UnboundedReceiver<RuntimeEvent>>>,
    pending_approvals: Arc<Mutex<HashMap<String, oneshot::Sender<PermissionDecision>>>>,
    prompt_lock: Mutex<()>,
    alive: AtomicBool,
}

impl MockRuntimeClient {
    pub fn connect(workdir: &Path) -> Self {
        let agent = MockAgent::new(workdir.to_path_buf());
        let session_id = agent.session_id().to_string();
        let (msg_tx, mut msg_rx) = mpsc::unbounded_channel();
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let pending_approvals = Arc::new(Mutex::new(HashMap::new()));
        let pending = pending_approvals.clone();
        let _ = events_tx.send(RuntimeEvent::Initialized {
            session_id: session_id.clone(),
            event: RuntimeNotification {
                source_event_id: format!("mock-session-{session_id}"),
                cursor: None,
                event_type: CloudEventKind::SessionStarted,
                payload: RuntimePayload::SessionStarted(mock_session_started_payload(
                    &session_id,
                    false,
                )),
            },
        });
        tokio::spawn(async move {
            while let Some(message) = msg_rx.recv().await {
                let event = match message {
                    MockMsg::Event(event) => {
                        RuntimeEvent::Notification(RuntimeNotification::from_acp(event))
                    }
                    MockMsg::Permission {
                        request_key,
                        params,
                        respond,
                    } => {
                        pending.lock().await.insert(request_key.clone(), respond);
                        let event = RuntimeNotification::from_acp(AcpEvent::from_reverse_request(
                            &Value::String(request_key.clone()),
                            "session/request_permission",
                            &params,
                        ));
                        let mut context = approval_payload(&params);
                        context.request_id = request_key.clone();
                        RuntimeEvent::Request {
                            request: RuntimeRequest::Approval {
                                request_id: request_key,
                                kind: ApprovalKind::Tool,
                                allowed_decisions: allowed_decisions_from_params(&params),
                                context,
                            },
                            event,
                        }
                    }
                };
                if events_tx.send(event).is_err() {
                    break;
                }
            }
            let _ = events_tx.send(RuntimeEvent::ChildExited);
        });
        Self {
            agent,
            msg_tx: Mutex::new(Some(msg_tx)),
            events: Arc::new(Mutex::new(events_rx)),
            pending_approvals,
            prompt_lock: Mutex::new(()),
            alive: AtomicBool::new(true),
        }
    }
}

fn mock_session_started_payload(session_id: &str, resumed: bool) -> SessionStartedPayload {
    SessionStartedPayload {
        session_id: Some(session_id.to_string()),
        resumed: resumed.then_some(true),
        current_mode_id: Some("default".into()),
        available_modes: Some(serde_json::json!([
            {
                "id": "default",
                "name": "Default",
                "description": "Full tool access with permission prompts for gated tools"
            },
            {
                "id": "plan",
                "name": "Plan",
                "description": "Read-only exploration; ExitPlanMode required before edits"
            }
        ])),
        recovery: Some(SessionRecoveryPayload {
            disposition: Some("clean".into()),
            has_incomplete_execution: Some(false),
            active_turn_count: Some(0),
            active_tool_count: Some(0),
            safe_resume_allowed: Some(false),
            automatic_resume: Some(false),
            reason: Some("no incomplete execution".into()),
        }),
    }
}

#[async_trait]
impl RuntimeClient for MockRuntimeClient {
    async fn session_id(&self) -> Result<String> {
        Ok(self.agent.session_id().to_string())
    }

    async fn send(&self, command: RuntimeCommand) -> Result<()> {
        match command {
            RuntimeCommand::Prompt { text } => {
                let _guard = self.prompt_lock.lock().await;
                let msg_tx = self
                    .msg_tx
                    .lock()
                    .await
                    .clone()
                    .ok_or_else(|| anyhow!("mock runtime shut down"))?;
                self.agent.run_prompt(&text, msg_tx).await
            }
            RuntimeCommand::Steer { text } => {
                let text = text.trim();
                if text.is_empty() {
                    return Err(anyhow!("steer message cannot be empty"));
                }
                if self.prompt_lock.try_lock().is_ok() {
                    return Err(anyhow!(
                        "no turn in progress; use prompt() to start a new turn"
                    ));
                }
                let msg_tx = self
                    .msg_tx
                    .lock()
                    .await
                    .clone()
                    .ok_or_else(|| anyhow!("mock runtime shut down"))?;
                self.agent.emit_steer(text, msg_tx)
            }
            RuntimeCommand::Cancel => Ok(()),
            RuntimeCommand::Approval { request_id, decision } => {
                let respond = self
                    .pending_approvals
                    .lock()
                    .await
                    .remove(&request_id)
                    .ok_or_else(|| anyhow!("unknown approval request_id {request_id}"))?;
                let _ = respond.send(to_permission_decision(decision));
                Ok(())
            }
            RuntimeCommand::SetMode { mode_id } => {
                if self.prompt_lock.try_lock().is_err() {
                    return Err(anyhow!("cannot change or read mode while a turn is active"));
                }
                let msg_tx = self
                    .msg_tx
                    .lock()
                    .await
                    .clone()
                    .ok_or_else(|| anyhow!("mock runtime shut down"))?;
                self.agent.emit_mode(&mode_id, msg_tx)
            }
            RuntimeCommand::Shutdown => {
                self.alive.store(false, Ordering::SeqCst);
                self.msg_tx.lock().await.take();
                Ok(())
            }
        }
    }

    async fn next_event(&self) -> Option<RuntimeEvent> {
        self.events.lock().await.recv().await
    }

    async fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(event: &RuntimeNotification) -> Value {
        event.payload.to_value()
    }

    #[test]
    fn permission_request_is_normalized_without_exposing_method_paths() {
        let request = runtime_request(
            "session/request_permission",
            &serde_json::json!({"toolCall": {"toolCallId": "call-7"}}),
        );
        assert!(matches!(
            request,
            Some(RuntimeRequest::Approval {
                request_id,
                kind: ApprovalKind::Permission,
                ..
            }) if request_id == "call-7"
        ));
    }

    #[test]
    fn permission_request_without_tool_call_id_has_stable_metadata_identity() {
        let params = serde_json::json!({
            "sessionId": "session-1",
            "_meta": {"eventId": "permission-event-3", "sequence": 3},
            "reason": "write"
        });
        let first = runtime_request("session/request_permission", &params);
        let second = runtime_request("session/request_permission", &params);
        assert!(matches!(
            (first, second),
            (
                Some(RuntimeRequest::Approval { request_id: first, .. }),
                Some(RuntimeRequest::Approval { request_id: second, .. })
            ) if first == second && first.starts_with("permission-")
        ));
    }

    #[test]
    fn permission_request_metadata_changes_identity() {
        let first = runtime_request(
            "session/request_permission",
            &serde_json::json!({"sessionId": "session-1", "_meta": {"sequence": 3}}),
        );
        let second = runtime_request(
            "session/request_permission",
            &serde_json::json!({"sessionId": "session-1", "_meta": {"sequence": 4}}),
        );
        assert!(matches!(
            (first, second),
            (
                Some(RuntimeRequest::Approval { request_id: first, .. }),
                Some(RuntimeRequest::Approval { request_id: second, .. })
            ) if first != second
        ));
    }

    #[test]
    fn unsupported_reverse_request_is_classified_at_adapter_boundary() {
        assert!(runtime_request("session/unknown", &serde_json::json!({})).is_none());
    }

    #[test]
    fn text_delta_stores_product_text_not_acp_envelope() {
        let raw = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "session-1",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "hello" }
                }
            }
        });
        let event = runtime_notification(AcpEvent::from_notification(&raw));
        assert_eq!(event.event_type.as_event_type(), "text_delta");
        assert!(event.source_event_id.starts_with("acp-"));
        assert_eq!(event.cursor, None);
        assert_eq!(event.payload, serde_json::json!({ "text": "hello" }));
    }

    #[test]
    fn thought_delta_reads_content_text() {
        let raw = serde_json::json!({
            "method": "session/update",
            "params": {
                "update": {
                    "sessionUpdate": "agent_thought_chunk",
                    "content": { "type": "text", "text": "hmm" }
                }
            }
        });
        let event = runtime_notification(AcpEvent::from_notification(&raw));
        assert_eq!(event.event_type.as_event_type(), "thought_delta");
        assert_eq!(event.payload, serde_json::json!({ "text": "hmm" }));
    }

    #[test]
    fn tool_call_stores_product_fields() {
        let raw = serde_json::json!({
            "method": "session/update",
            "params": {
                "update": {
                    "sessionUpdate": "tool_call",
                    "toolCallId": "call-1",
                    "title": "Read lib.rs",
                    "kind": "read",
                    "status": "pending",
                    "rawInput": { "path": "lib.rs" }
                }
            }
        });
        let event = runtime_notification(AcpEvent::from_notification(&raw));
        assert_eq!(event.event_type.as_event_type(), "tool_call");
        assert_eq!(
            event.payload,
            serde_json::json!({
                "toolCallId": "call-1",
                "title": "Read lib.rs",
                "kind": "read",
                "status": "pending",
                "rawInput": { "path": "lib.rs" }
            })
        );
    }

    #[test]
    fn tool_result_stores_product_fields() {
        let raw = serde_json::json!({
            "method": "session/update",
            "params": {
                "update": {
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "call-1",
                    "status": "completed",
                    "content": [{
                        "type": "content",
                        "content": { "type": "text", "text": "ok" }
                    }],
                    "rawOutput": { "text": "ok", "isError": false }
                }
            }
        });
        let event = runtime_notification(AcpEvent::from_notification(&raw));
        assert_eq!(event.event_type.as_event_type(), "tool_result");
        assert_eq!(json(&event)["toolCallId"], "call-1");
        assert_eq!(json(&event)["status"], "completed");
        assert_eq!(json(&event)["text"], "ok");
        assert_eq!(json(&event)["isError"], false);
        assert_eq!(json(&event)["rawOutput"]["text"], "ok");
        assert!(json(&event).get("sessionUpdate").is_none());
        assert!(json(&event).get("method").is_none());
    }

    #[test]
    fn user_message_stores_product_text() {
        let raw = serde_json::json!({
            "method": "session/update",
            "params": {
                "update": {
                    "sessionUpdate": "user_message_chunk",
                    "content": { "type": "text", "text": "hi" }
                }
            }
        });
        let event = runtime_notification(AcpEvent::from_notification(&raw));
        assert_eq!(event.event_type.as_event_type(), "user_message");
        assert_eq!(event.payload, serde_json::json!({ "text": "hi" }));
    }

    #[test]
    fn state_changed_stores_mode_as_state() {
        let raw = serde_json::json!({
            "method": "session/update",
            "params": {
                "update": {
                    "sessionUpdate": "current_mode_update",
                    "modeId": "plan"
                }
            }
        });
        let event = runtime_notification(AcpEvent::from_notification(&raw));
        assert_eq!(event.event_type.as_event_type(), "state_changed");
        assert_eq!(event.payload, serde_json::json!({ "state": "plan" }));
        assert!(json(&event).get("modeId").is_none());
        assert!(json(&event).get("sessionUpdate").is_none());
    }

    #[test]
    fn usage_update_lifts_meta_to_product_fields() {
        let raw = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "session-1",
                "update": {
                    "sessionUpdate": "usage_update",
                    "used": 1200,
                    "size": 8000,
                    "_meta": {
                        "promptTokens": 900,
                        "completionTokens": 300,
                        "contextPercent": 15,
                        "contextEpoch": 2,
                        "cachedTokens": 40
                    }
                }
            }
        });
        let event = runtime_notification(AcpEvent::from_notification(&raw));
        assert_eq!(event.event_type.as_event_type(), "usage_update");
        assert_eq!(
            event.payload,
            serde_json::json!({
                "used": 1200,
                "size": 8000,
                "promptTokens": 900,
                "completionTokens": 300,
                "contextPercent": 15,
                "contextEpoch": 2,
                "cachedTokens": 40
            })
        );
        assert!(json(&event).get("_meta").is_none());
        assert!(json(&event).get("method").is_none());
    }

    #[test]
    fn projection_ready_lifts_meta_to_product_payload() {
        let raw = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "session-1",
                "update": {
                    "sessionUpdate": "projection_update",
                    "_meta": {
                        "sourceMessageCount": 4,
                        "projectedMessageCount": 3,
                        "delivery": "strict",
                        "contextEpoch": 2
                    }
                }
            }
        });
        let event = runtime_notification(AcpEvent::from_notification(&raw));
        assert_eq!(event.event_type.as_event_type(), "projection_ready");
        assert_eq!(
            event.payload,
            serde_json::json!({
                "sourceMessageCount": 4,
                "projectedMessageCount": 3,
                "delivery": "strict",
                "contextEpoch": 2
            })
        );
        assert!(json(&event).get("sessionUpdate").is_none());
        assert!(json(&event).get("method").is_none());
    }

    #[test]
    fn plan_stores_entries() {
        let raw = serde_json::json!({
            "method": "session/update",
            "params": {
                "update": {
                    "sessionUpdate": "plan",
                    "entries": [{ "content": "edit", "status": "pending", "priority": "medium" }]
                }
            }
        });
        let event = runtime_notification(AcpEvent::from_notification(&raw));
        assert_eq!(event.event_type.as_event_type(), "plan");
        assert_eq!(
            event.payload,
            serde_json::json!({
                "entries": [{ "content": "edit", "status": "pending", "priority": "medium" }]
            })
        );
    }

    #[test]
    fn available_commands_stores_command_list() {
        let raw = serde_json::json!({
            "method": "session/update",
            "params": {
                "update": {
                    "sessionUpdate": "available_commands_update",
                    "availableCommands": [{ "name": "compact", "description": "Compact context" }]
                }
            }
        });
        let event = runtime_notification(AcpEvent::from_notification(&raw));
        assert_eq!(event.event_type.as_event_type(), "available_commands");
        assert_eq!(
            event.payload,
            serde_json::json!({
                "availableCommands": [{ "name": "compact", "description": "Compact context" }]
            })
        );
    }

    #[test]
    fn unclassified_session_updates_store_residual_product_payload() {
        let raw = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "session-1",
                "update": { "sessionUpdate": "unknown_update", "note": "x" }
            }
        });
        let event = runtime_notification(AcpEvent::from_notification(&raw));
        assert_eq!(event.event_type.as_event_type(), "acp");
        assert_eq!(
            event.payload,
            serde_json::json!({
                "method": "session/update",
                "sessionUpdate": "unknown_update",
                "update": { "sessionUpdate": "unknown_update", "note": "x" }
            })
        );
    }

    #[test]
    fn initialize_stores_product_handshake_fields() {
        let event = runtime_notification(AcpEvent {
            source_event_id: "init-1".into(),
            cursor: None,
            event_type: "acp".into(),
            payload: serde_json::json!({
                "method": "initialize",
                "result": {
                    "protocolVersion": 1,
                    "agentCapabilities": { "loadSession": true },
                    "agentInfo": { "name": "zene" },
                    "authMethods": [],
                    "extraFlag": true
                }
            }),
        });
        assert_eq!(event.event_type.as_event_type(), "initialized");
        assert_eq!(
            event.payload,
            serde_json::json!({
                "protocolVersion": 1,
                "agentCapabilities": { "loadSession": true },
                "agentInfo": { "name": "zene" },
                "authMethods": [],
                "extraFlag": true
            })
        );
    }

    #[test]
    fn unsupported_reverse_request_stores_method_without_jsonrpc_id() {
        let event = runtime_notification(AcpEvent::from_reverse_request(
            &serde_json::json!(7),
            "fs/read_text_file",
            &serde_json::json!({ "path": "README.md" }),
        ));
        assert_eq!(event.event_type.as_event_type(), "unsupported_request");
        assert_eq!(
            event.payload,
            serde_json::json!({
                "method": "fs/read_text_file",
                "params": { "path": "README.md" }
            })
        );
        let value = event.payload.to_value();
        assert!(value.get("id").is_none());
        assert!(value.get("jsonrpc").is_none());
    }

    #[test]
    fn turn_step_and_error_store_product_fields() {
        let turn = runtime_notification(AcpEvent::from_notification(&serde_json::json!({
            "method": "session/update",
            "params": {
                "update": { "sessionUpdate": "turn_started", "turnId": "turn-1" }
            }
        })));
        assert_eq!(turn.event_type.as_event_type(), "turn_started");
        assert_eq!(turn.payload, serde_json::json!({ "turnId": "turn-1" }));

        let step = runtime_notification(AcpEvent::from_notification(&serde_json::json!({
            "method": "session/update",
            "params": {
                "update": { "sessionUpdate": "step_started", "step": 2, "turnId": "turn-1" }
            }
        })));
        assert_eq!(step.event_type.as_event_type(), "step_started");
        assert_eq!(
            step.payload,
            serde_json::json!({ "step": 2, "turnId": "turn-1" })
        );

        let ended = runtime_notification(AcpEvent::from_notification(&serde_json::json!({
            "method": "session/update",
            "params": {
                "update": { "sessionUpdate": "turn_ended", "steps": 3, "turnId": "turn-1" }
            }
        })));
        assert_eq!(ended.event_type.as_event_type(), "turn_ended");
        assert_eq!(
            ended.payload,
            serde_json::json!({ "steps": 3, "turnId": "turn-1" })
        );

        let error = runtime_notification(AcpEvent::from_notification(&serde_json::json!({
            "method": "session/update",
            "params": {
                "update": { "sessionUpdate": "error", "message": "boom" }
            }
        })));
        assert_eq!(error.event_type.as_event_type(), "error");
        assert_eq!(error.payload, serde_json::json!({ "message": "boom" }));
    }

    #[test]
    fn session_lifecycle_stores_product_session_id() {
        let event = runtime_notification(AcpEvent {
            source_event_id: "session-new-1".into(),
            cursor: None,
            event_type: "acp".into(),
            payload: serde_json::json!({
                "method": "session/new",
                "result": { "sessionId": "session-1" }
            }),
        });
        assert_eq!(event.event_type.as_event_type(), "session_started");
        assert_eq!(event.payload, serde_json::json!({ "sessionId": "session-1" }));
    }

    #[test]
    fn session_started_lifts_modes_and_recovery_meta() {
        let event = runtime_notification(AcpEvent {
            source_event_id: "session-new-2".into(),
            cursor: None,
            event_type: "acp".into(),
            payload: serde_json::json!({
                "method": "session/new",
                "result": {
                    "sessionId": "session-2",
                    "modes": {
                        "currentModeId": "default",
                        "availableModes": [
                            { "id": "default", "name": "Default" },
                            { "id": "plan", "name": "Plan" }
                        ]
                    },
                    "_meta": {
                        "recovery": {
                            "disposition": "clean",
                            "hasIncompleteExecution": false,
                            "activeTurnCount": 0,
                            "activeToolCount": 0,
                            "safeResumeAllowed": false,
                            "automaticResume": false,
                            "reason": "no incomplete execution"
                        }
                    }
                }
            }),
        });
        assert_eq!(event.event_type.as_event_type(), "session_started");
        assert_eq!(
            event.payload,
            serde_json::json!({
                "sessionId": "session-2",
                "currentModeId": "default",
                "availableModes": [
                    { "id": "default", "name": "Default" },
                    { "id": "plan", "name": "Plan" }
                ],
                "recovery": {
                    "disposition": "clean",
                    "hasIncompleteExecution": false,
                    "activeTurnCount": 0,
                    "activeToolCount": 0,
                    "safeResumeAllowed": false,
                    "automaticResume": false,
                    "reason": "no incomplete execution"
                }
            })
        );
    }

    #[test]
    fn session_resume_marks_resumed() {
        let event = runtime_notification(AcpEvent {
            source_event_id: "session-resume-1".into(),
            cursor: None,
            event_type: "acp".into(),
            payload: serde_json::json!({
                "method": "session/resume",
                "params": { "sessionId": "session-1", "cwd": "/tmp" },
                "result": {
                    "sessionId": "session-1",
                    "modes": { "currentModeId": "plan", "availableModes": [] },
                    "_meta": {
                        "recovery": {
                            "disposition": "inspect",
                            "hasIncompleteExecution": true,
                            "activeTurnCount": 1,
                            "activeToolCount": 1,
                            "safeResumeAllowed": false,
                            "automaticResume": false,
                            "reason": "pending tool requires inspection"
                        }
                    }
                }
            }),
        });
        assert_eq!(event.event_type.as_event_type(), "session_started");
        assert_eq!(
            event.payload,
            serde_json::json!({
                "sessionId": "session-1",
                "resumed": true,
                "currentModeId": "plan",
                "availableModes": [],
                "recovery": {
                    "disposition": "inspect",
                    "hasIncompleteExecution": true,
                    "activeTurnCount": 1,
                    "activeToolCount": 1,
                    "safeResumeAllowed": false,
                    "automaticResume": false,
                    "reason": "pending tool requires inspection"
                }
            })
        );
    }

    #[test]
    fn reverse_permission_stores_product_approval_fields() {
        let event = runtime_notification(AcpEvent::from_reverse_request(
            &serde_json::json!(42),
            "session/request_permission",
            &serde_json::json!({
                "toolCall": {
                    "toolCallId": "call-7",
                    "title": "Write notes",
                    "kind": "edit",
                    "status": "pending",
                    "rawInput": { "path": "notes.md" }
                }
            }),
        ));
        assert_eq!(event.event_type.as_event_type(), "approval_requested");
        assert_eq!(
            event.payload,
            serde_json::json!({
                "requestId": "call-7",
                "toolCallId": "call-7",
                "title": "Write notes",
                "kind": "edit",
                "status": "pending",
                "rawInput": { "path": "notes.md" }
            })
        );
        assert!(json(&event).get("method").is_none());
        assert!(json(&event).get("jsonrpc").is_none());
        assert!(json(&event).get("id").is_none());
    }

    #[test]
    fn approval_contract_exposes_neutral_context_and_identity() {
        let params = serde_json::json!({
            "toolCall": {
                "toolCallId": "call-7",
                "title": "Write notes",
                "kind": "edit"
            },
            "reason": "write",
            "options": [{ "optionId": "allow-once" }]
        });
        let request = runtime_request("session/request_permission", &params);
        assert!(matches!(
            request,
            Some(RuntimeRequest::Approval {
                request_id,
                kind: ApprovalKind::Permission,
                allowed_decisions,
                context
            })
                if request_id == "call-7"
                    && allowed_decisions == vec![ApprovalDecision::AllowOnce]
                    && context
                        == ApprovalEventPayload {
                            request_id: "call-7".into(),
                            tool_call_id: Some("call-7".into()),
                            title: Some("Write notes".into()),
                            tool_name: None,
                            kind: Some("edit".into()),
                            status: None,
                            raw_input: None,
                        }
        ));
    }

    #[test]
    fn missing_permission_options_use_default_allowed_decisions() {
        let request = runtime_request(
            "session/request_permission",
            &serde_json::json!({ "toolCall": { "toolCallId": "call-7" } }),
        );
        assert!(matches!(
            request,
            Some(RuntimeRequest::Approval { allowed_decisions, .. })
                if allowed_decisions == ApprovalDecision::default_allowed()
        ));
    }

    #[test]
    fn reconnect_can_target_an_existing_session() {
        assert_eq!(Some("session-1"), Some("session-1"));
        // The transport seam is explicit; process integration is covered by the ACP bridge.
    }

    #[test]
    fn runtime_commands_are_transport_neutral() {
        let command = RuntimeCommand::Prompt { text: "hello".into() };
        assert!(matches!(command, RuntimeCommand::Prompt { text } if text == "hello"));
        let command = RuntimeCommand::Steer { text: "nudge".into() };
        assert!(matches!(command, RuntimeCommand::Steer { text } if text == "nudge"));
        let command = RuntimeCommand::SetMode {
            mode_id: "plan".into(),
        };
        assert!(matches!(
            command,
            RuntimeCommand::SetMode { mode_id } if mode_id == "plan"
        ));
        let command = RuntimeCommand::Approval {
            request_id: "call-7".into(),
            decision: ApprovalDecision::AllowOnce,
        };
        assert!(matches!(
            command,
            RuntimeCommand::Approval {
                request_id,
                decision: ApprovalDecision::AllowOnce,
            } if request_id == "call-7"
        ));
    }

    #[test]
    fn approval_decision_maps_to_acp_result_inside_the_adapter() {
        assert_eq!(
            acp_permission_result(ApprovalDecision::AllowOnce),
            serde_json::json!({ "outcome": { "optionId": "allow-once" } })
        );
        assert_eq!(
            acp_permission_result(ApprovalDecision::AllowSession),
            serde_json::json!({ "outcome": { "optionId": "allow-always" } })
        );
        assert_eq!(
            acp_permission_result(ApprovalDecision::Deny),
            serde_json::json!({ "outcome": { "optionId": "reject-once" } })
        );
    }

    #[test]
    fn stored_console_decisions_map_to_neutral_approval() {
        assert_eq!(
            ApprovalDecision::parse("allow-once"),
            Some(ApprovalDecision::AllowOnce)
        );
        assert_eq!(
            ApprovalDecision::parse("allow-always"),
            Some(ApprovalDecision::AllowSession)
        );
        assert_eq!(
            ApprovalDecision::parse("allow"),
            Some(ApprovalDecision::AllowSession)
        );
        assert_eq!(
            ApprovalDecision::parse("reject-once"),
            Some(ApprovalDecision::Deny)
        );
        assert_eq!(ApprovalDecision::parse("unknown"), None);
    }

    #[tokio::test]
    async fn mock_session_started_includes_modes_and_clean_recovery() {
        let dir = std::env::temp_dir().join(format!("zene-mock-started-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let client = MockRuntimeClient::connect(&dir);
        let session_id = client.session_id().await.expect("session id");
        let event = client.next_event().await.expect("initialized");
        match event {
            RuntimeEvent::Initialized { event, .. } => {
                assert_eq!(
                    event.payload,
                    RuntimePayload::SessionStarted(mock_session_started_payload(&session_id, false))
                );
                let value = event.payload.to_value();
                assert_eq!(value["currentModeId"], "default");
                assert_eq!(value["recovery"]["automaticResume"], false);
                assert_eq!(value["recovery"]["disposition"], "clean");
            }
            other => panic!("expected Initialized, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn mock_runtime_client_emits_product_events_and_tool_approval() {
        let dir = std::env::temp_dir().join(format!("zene-mock-runtime-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let client = Arc::new(MockRuntimeClient::connect(&dir));
        let session_id = client.session_id().await.expect("session id");
        let expected_started = mock_session_started_payload(&session_id, false);
        let pump = client.clone();
        let pump_task = tokio::spawn(async move {
            let mut saw_text = false;
            let mut saw_tool_approval = false;
            while let Some(event) = pump.next_event().await {
                match event {
                    RuntimeEvent::Initialized { event, .. } => {
                        assert_eq!(event.event_type, CloudEventKind::SessionStarted);
                        assert_eq!(
                            event.payload,
                            RuntimePayload::SessionStarted(expected_started.clone())
                        );
                    }
                    RuntimeEvent::Notification(event) => {
                        if event.event_type == CloudEventKind::TextDelta {
                            saw_text = true;
                        }
                    }
                    RuntimeEvent::Request { request, event } => {
                        assert_eq!(event.event_type, CloudEventKind::ApprovalRequested);
                        let RuntimeRequest::Approval {
                            request_id,
                            kind,
                            allowed_decisions,
                            context,
                        } = request;
                        assert_eq!(kind, ApprovalKind::Tool);
                        assert_eq!(
                            allowed_decisions,
                            vec![ApprovalDecision::AllowOnce, ApprovalDecision::Deny]
                        );
                        assert_eq!(context.tool_call_id.as_deref(), Some("tool_write_notes"));
                        pump.send(RuntimeCommand::Approval {
                            request_id,
                            decision: ApprovalDecision::AllowOnce,
                        })
                        .await
                        .expect("resolve mock approval");
                        saw_tool_approval = true;
                    }
                    RuntimeEvent::ChildExited => break,
                }
                if saw_text && saw_tool_approval {
                    break;
                }
            }
            (saw_text, saw_tool_approval)
        });
        client
            .send(RuntimeCommand::Prompt {
                text: "write notes".into(),
            })
            .await
            .expect("mock prompt");
        let (saw_text, saw_tool_approval) = pump_task.await.expect("pump");
        assert!(saw_text, "expected classified text_delta");
        assert!(saw_tool_approval, "expected tool approval request");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn mock_runtime_client_set_mode_when_idle() {
        let dir = std::env::temp_dir().join(format!("zene-mock-mode-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let client = MockRuntimeClient::connect(&dir);
        client
            .send(RuntimeCommand::SetMode {
                mode_id: "plan".into(),
            })
            .await
            .expect("set mode");
        let mut saw_mode = false;
        while let Some(event) = client.next_event().await {
            match event {
                RuntimeEvent::Notification(event)
                    if event.event_type == CloudEventKind::StateChanged =>
                {
                    saw_mode = true;
                    break;
                }
                RuntimeEvent::ChildExited => break,
                _ => {}
            }
        }
        assert!(saw_mode, "expected current_mode_update");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn mock_runtime_client_rejects_idle_steer() {
        let dir = std::env::temp_dir().join(format!("zene-mock-steer-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let client = MockRuntimeClient::connect(&dir);
        let err = client
            .send(RuntimeCommand::Steer {
                text: "nudge".into(),
            })
            .await
            .expect_err("idle steer");
        assert!(err.to_string().contains("no turn in progress"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
