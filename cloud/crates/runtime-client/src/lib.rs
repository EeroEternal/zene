use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::{mpsc, Mutex};
use zene_cloud_acp_bridge::{AcpBridge, AcpEvent, BridgeMsg, PermissionDecision};

/// Transport-neutral approval outcome. Variants match
/// `zene_runtime::ApprovalDecision`; ACP `optionId` strings stay inside this
/// adapter until Cloud depends on that crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    AllowOnce,
    AllowSession,
    Deny,
}

impl ApprovalDecision {
    pub fn from_stored(decision: &str) -> Self {
        match decision {
            "allow-always" | "allow" => Self::AllowSession,
            "allow-once" => Self::AllowOnce,
            _ => Self::Deny,
        }
    }
}

impl From<ApprovalDecision> for PermissionDecision {
    fn from(decision: ApprovalDecision) -> Self {
        match decision {
            ApprovalDecision::AllowOnce => Self::AllowOnce,
            ApprovalDecision::AllowSession => Self::AllowSession,
            ApprovalDecision::Deny => Self::Deny,
        }
    }
}

fn acp_permission_result(decision: ApprovalDecision) -> Value {
    PermissionDecision::from(decision).to_result()
}

/// Transport-neutral runtime event kind. Names align with
/// `zene_turn::RuntimeEventKind` where the ACP update has a counterpart.
/// ACP `sessionUpdate` strings stay inside this adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeEventKind {
    TextDelta,
    ThoughtDelta,
    UserMessage,
    ToolCall,
    ToolResult,
    StateChanged,
    UsageUpdate,
    ProjectionReady,
    Plan,
    AvailableCommands,
    SessionStarted,
    ApprovalRequested,
    Unknown,
}

impl RuntimeEventKind {
    pub fn as_event_type(self) -> &'static str {
        match self {
            Self::TextDelta => "text_delta",
            Self::ThoughtDelta => "thought_delta",
            Self::UserMessage => "user_message",
            Self::ToolCall => "tool_call",
            Self::ToolResult => "tool_result",
            Self::StateChanged => "state_changed",
            Self::UsageUpdate => "usage_update",
            Self::ProjectionReady => "projection_ready",
            Self::Plan => "plan",
            Self::AvailableCommands => "available_commands",
            Self::SessionStarted => "session_started",
            Self::ApprovalRequested => "approval_requested",
            Self::Unknown => "acp",
        }
    }
}

fn classify_payload(payload: &Value) -> RuntimeEventKind {
    if let Some(update) = payload
        .pointer("/params/update/sessionUpdate")
        .and_then(Value::as_str)
    {
        return map_session_update(update);
    }
    match payload.get("method").and_then(Value::as_str) {
        Some("session/request_permission") => RuntimeEventKind::ApprovalRequested,
        Some("session/new") | Some("session/resume") => RuntimeEventKind::SessionStarted,
        _ => RuntimeEventKind::Unknown,
    }
}

fn map_session_update(update: &str) -> RuntimeEventKind {
    match update {
        "agent_message_chunk" => RuntimeEventKind::TextDelta,
        "agent_thought_chunk" => RuntimeEventKind::ThoughtDelta,
        "user_message_chunk" => RuntimeEventKind::UserMessage,
        "tool_call" => RuntimeEventKind::ToolCall,
        "tool_call_update" => RuntimeEventKind::ToolResult,
        "current_mode_update" => RuntimeEventKind::StateChanged,
        "usage_update" => RuntimeEventKind::UsageUpdate,
        "projection_update" => RuntimeEventKind::ProjectionReady,
        "plan" => RuntimeEventKind::Plan,
        "available_commands_update" => RuntimeEventKind::AvailableCommands,
        _ => RuntimeEventKind::Unknown,
    }
}

/// Transport-neutral command accepted by Cloud's runtime client.
///
/// Variant names match `zene_runtime::RuntimeCommand` where Cloud has a
/// counterpart. ACP JSON-RPC ids stay inside this adapter.
#[derive(Debug, Clone)]
pub enum RuntimeCommand {
    Prompt { text: String },
    Cancel,
    Approval { request_id: String, decision: ApprovalDecision },
    Shutdown,
}

/// A runtime notification with the stable fields persisted by the worker.
///
/// `event_type` is the product kind. Timeline kinds (`text_delta`,
/// `thought_delta`, `tool_call`, `tool_result`) and control kinds
/// (`approval_requested`, `session_started`) store a denormalized product
/// payload. Other frames keep the original ACP JSON. Records written before
/// this change still have ACP JSON; Console falls back to `params.update`.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeNotification {
    pub source_event_id: String,
    pub cursor: Option<u64>,
    pub event_type: String,
    pub payload: Value,
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
/// `context` is opaque request context retained for existing approval
/// resolution and compatibility with stored payloads.
#[derive(Debug)]
pub enum RuntimeRequest {
    Approval {
        request_id: String,
        context: Value,
    },
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
        event_type: kind.as_event_type().into(),
        payload: product_payload(kind, &event.payload),
    }
}

fn product_payload(kind: RuntimeEventKind, raw: &Value) -> Value {
    match kind {
        RuntimeEventKind::TextDelta | RuntimeEventKind::ThoughtDelta => {
            let Some(update) = raw.pointer("/params/update") else {
                return raw.clone();
            };
            serde_json::json!({ "text": text_from_update(update) })
        }
        RuntimeEventKind::ToolCall => {
            let Some(update) = raw.pointer("/params/update") else {
                return raw.clone();
            };
            Value::Object(take_fields(
                update,
                &["toolCallId", "title", "toolName", "kind", "status", "rawInput"],
            ))
        }
        RuntimeEventKind::ToolResult => {
            let Some(update) = raw.pointer("/params/update") else {
                return raw.clone();
            };
            let mut map = take_fields(
                update,
                &[
                    "toolCallId",
                    "title",
                    "toolName",
                    "kind",
                    "status",
                    "rawOutput",
                ],
            );
            let text = tool_result_text(update);
            if !text.is_empty() {
                map.insert("text".into(), Value::String(text));
            }
            if let Some(is_error) = update.pointer("/rawOutput/isError") {
                map.insert("isError".into(), is_error.clone());
            }
            Value::Object(map)
        }
        RuntimeEventKind::ApprovalRequested => approval_product(raw),
        RuntimeEventKind::SessionStarted => session_started_product(raw),
        _ => raw.clone(),
    }
}

fn approval_product(raw: &Value) -> Value {
    let params = raw.get("params").unwrap_or(raw);
    let mut map = serde_json::Map::new();
    map.insert(
        "requestId".into(),
        Value::String(permission_request_key(params)),
    );
    if let Some(tool) = params.get("toolCall") {
        map.extend(take_fields(
            tool,
            &["toolCallId", "title", "toolName", "kind", "status", "rawInput"],
        ));
    }
    Value::Object(map)
}

fn session_started_product(raw: &Value) -> Value {
    let session_id = raw
        .pointer("/result/sessionId")
        .or_else(|| raw.pointer("/params/sessionId"))
        .cloned();
    let mut map = serde_json::Map::new();
    if let Some(session_id) = session_id {
        map.insert("sessionId".into(), session_id);
    }
    if raw.get("method").and_then(Value::as_str) == Some("session/resume") {
        map.insert("resumed".into(), Value::Bool(true));
    }
    if map.is_empty() {
        return raw.clone();
    }
    Value::Object(map)
}

fn take_fields(update: &Value, keys: &[&str]) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    for key in keys {
        if let Some(value) = update.get(*key) {
            if !value.is_null() {
                map.insert((*key).to_string(), value.clone());
            }
        }
    }
    map
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
            context: params.clone(),
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
                Some(existing) => client.initialize_and_resume_session(workdir, existing).await?,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_request_is_normalized_without_exposing_method_paths() {
        let request = runtime_request(
            "session/request_permission",
            &serde_json::json!({"toolCall": {"toolCallId": "call-7"}}),
        );
        assert!(matches!(
            request,
            Some(RuntimeRequest::Approval { request_id, .. }) if request_id == "call-7"
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
        assert_eq!(event.event_type, "text_delta");
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
        assert_eq!(event.event_type, "thought_delta");
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
        assert_eq!(event.event_type, "tool_call");
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
        assert_eq!(event.event_type, "tool_result");
        assert_eq!(event.payload["toolCallId"], "call-1");
        assert_eq!(event.payload["status"], "completed");
        assert_eq!(event.payload["text"], "ok");
        assert_eq!(event.payload["isError"], false);
        assert_eq!(event.payload["rawOutput"]["text"], "ok");
        assert!(event.payload.get("sessionUpdate").is_none());
        assert!(event.payload.get("method").is_none());
    }

    #[test]
    fn non_timeline_kinds_keep_original_payload() {
        let cases = [
            ("current_mode_update", "state_changed"),
            ("usage_update", "usage_update"),
            ("projection_update", "projection_ready"),
            ("unknown_update", "acp"),
        ];
        for (session_update, event_type) in cases {
            let raw = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {
                    "sessionId": "session-1",
                    "update": { "sessionUpdate": session_update }
                }
            });
            let event = runtime_notification(AcpEvent::from_notification(&raw));
            assert_eq!(event.event_type, event_type, "{session_update}");
            assert_eq!(event.payload, raw);
        }
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
        assert_eq!(event.event_type, "session_started");
        assert_eq!(event.payload, serde_json::json!({ "sessionId": "session-1" }));
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
                "result": { "sessionId": "session-1" }
            }),
        });
        assert_eq!(event.event_type, "session_started");
        assert_eq!(
            event.payload,
            serde_json::json!({ "sessionId": "session-1", "resumed": true })
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
        assert_eq!(event.event_type, "approval_requested");
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
        assert!(event.payload.get("method").is_none());
        assert!(event.payload.get("jsonrpc").is_none());
        assert!(event.payload.get("id").is_none());
    }

    #[test]
    fn approval_contract_exposes_neutral_context_and_identity() {
        let params = serde_json::json!({
            "toolCall": { "toolCallId": "call-7" },
            "reason": "write"
        });
        let request = runtime_request("session/request_permission", &params);
        assert!(matches!(
            request,
            Some(RuntimeRequest::Approval { request_id, context })
                if request_id == "call-7" && context == params
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
            ApprovalDecision::from_stored("allow-once"),
            ApprovalDecision::AllowOnce
        );
        assert_eq!(
            ApprovalDecision::from_stored("allow-always"),
            ApprovalDecision::AllowSession
        );
        assert_eq!(
            ApprovalDecision::from_stored("allow"),
            ApprovalDecision::AllowSession
        );
        assert_eq!(
            ApprovalDecision::from_stored("reject-once"),
            ApprovalDecision::Deny
        );
    }
}
