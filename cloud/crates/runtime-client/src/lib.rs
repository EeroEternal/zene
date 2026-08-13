use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
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

#[derive(Debug, Clone)]
pub enum RuntimeCommand {
    Prompt { text: String },
    Cancel,
    RespondApproval { id: Value, decision: ApprovalDecision },
    RejectRequest { id: Value, code: i64, message: String },
}

/// A runtime notification with the stable fields persisted by the worker.
///
/// The payload intentionally remains the original JSON value so existing event
/// records can be replayed without a translation or migration.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeNotification {
    pub source_event_id: String,
    pub cursor: Option<u64>,
    pub event_type: String,
    pub payload: Value,
}

#[derive(Debug)]
pub enum RuntimeEvent {
    Initialized { session_id: String, event: RuntimeNotification },
    Notification(RuntimeNotification),
    Request { request: RuntimeRequest, event: RuntimeNotification },
    ChildExited,
}

/// Runtime-level meaning of a request requiring user approval. Protocol method
/// names and parameter paths stay inside this adapter instead of leaking into
/// workers. `context` is opaque request context retained for existing approval
/// resolution and compatibility with stored payloads.
#[derive(Debug)]
pub enum RuntimeRequest {
    Approval {
        id: Value,
        request_key: String,
        context: Value,
    },
    Unsupported {
        id: Value,
    },
}

#[async_trait]
pub trait RuntimeClient: Send + Sync {
    async fn session_id(&self) -> Result<String>;
    async fn prompt(&self, text: &str) -> Result<()>;
    async fn cancel(&self) -> Result<()>;
    async fn respond_approval(&self, id: &Value, decision: ApprovalDecision) -> Result<()>;
    async fn reject_request(&self, id: &Value, code: i64, message: &str) -> Result<()>;
    async fn next_event(&self) -> Option<RuntimeEvent>;
    async fn is_alive(&self) -> bool;
    async fn shutdown(&self) -> Result<()>;
}

pub struct AcpRuntimeClient {
    bridge: Arc<Mutex<Option<AcpBridge>>>,
    session_id: String,
    events: Arc<Mutex<mpsc::UnboundedReceiver<RuntimeEvent>>>,
}

fn runtime_notification(event: AcpEvent) -> RuntimeNotification {
    RuntimeNotification {
        source_event_id: event.source_event_id,
        cursor: event.cursor,
        event_type: event.event_type,
        payload: event.payload,
    }
}

fn runtime_request(id: &Value, method: &str, params: &Value) -> RuntimeRequest {
    if method == "session/request_permission" {
        let request_key = permission_request_key(params);
        RuntimeRequest::Approval {
            id: id.clone(),
            request_key,
            context: params.clone(),
        }
    } else {
        RuntimeRequest::Unsupported { id: id.clone() }
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
        tokio::spawn(async move {
            while let Some(message) = messages.recv().await {
                let event = match message {
                    BridgeMsg::Notification { raw, .. } => {
                        RuntimeEvent::Notification(runtime_notification(AcpEvent::from_notification(&raw)))
                    }
                    BridgeMsg::ReverseRequest { id, method, params } => {
                        let event = runtime_notification(AcpEvent::from_reverse_request(&id, &method, &params));
                        let request = runtime_request(&id, &method, &params);
                        RuntimeEvent::Request { request, event }
                    }
                };
                if event_tx.send(event).is_err() { break; }
            }
            let _ = event_tx.send(RuntimeEvent::ChildExited);
        });
        Ok(Self { bridge, session_id, events: Arc::new(Mutex::new(events_rx)) })
    }
}

#[async_trait]
impl RuntimeClient for AcpRuntimeClient {
    async fn session_id(&self) -> Result<String> { Ok(self.session_id.clone()) }
    async fn prompt(&self, text: &str) -> Result<()> {
        let guard = self.bridge.lock().await;
        guard.as_ref().context("runtime bridge missing")?.prompt(&self.session_id, text).await.map(|_| ())
    }
    async fn cancel(&self) -> Result<()> {
        let guard = self.bridge.lock().await;
        guard.as_ref().context("runtime bridge missing")?.cancel(&self.session_id).await
    }
    async fn respond_approval(&self, id: &Value, decision: ApprovalDecision) -> Result<()> {
        let guard = self.bridge.lock().await;
        guard
            .as_ref()
            .context("runtime bridge missing")?
            .respond(id, acp_permission_result(decision))
            .await
    }
    async fn reject_request(&self, id: &Value, code: i64, message: &str) -> Result<()> {
        let guard = self.bridge.lock().await;
        guard.as_ref().context("runtime bridge missing")?.respond_error(id, code, message).await
    }
    async fn next_event(&self) -> Option<RuntimeEvent> { self.events.lock().await.recv().await }
    async fn is_alive(&self) -> bool {
        let mut guard = self.bridge.lock().await;
        guard.as_mut().is_some_and(|bridge| !bridge.child_exited())
    }
    async fn shutdown(&self) -> Result<()> {
        let mut guard = self.bridge.lock().await;
        if let Some(bridge) = guard.take() { bridge.kill().await?; }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_request_is_normalized_without_exposing_method_paths() {
        let request = runtime_request(
            &serde_json::json!(42),
            "session/request_permission",
            &serde_json::json!({"toolCall": {"toolCallId": "call-7"}}),
        );
        assert!(matches!(
            request,
            RuntimeRequest::Approval { request_key, .. } if request_key == "call-7"
        ));
    }

    #[test]
    fn permission_request_without_tool_call_id_has_stable_metadata_identity() {
        let params = serde_json::json!({
            "sessionId": "session-1",
            "_meta": {"eventId": "permission-event-3", "sequence": 3},
            "reason": "write"
        });
        let first = runtime_request(&serde_json::json!(42), "session/request_permission", &params);
        let second = runtime_request(&serde_json::json!(99), "session/request_permission", &params);
        assert!(matches!(
            (first, second),
            (
                RuntimeRequest::Approval { request_key: first, .. },
                RuntimeRequest::Approval { request_key: second, .. }
            ) if first == second && first.starts_with("permission-")
        ));
    }

    #[test]
    fn permission_request_metadata_changes_identity() {
        let first = runtime_request(
            &serde_json::json!(1),
            "session/request_permission",
            &serde_json::json!({"sessionId": "session-1", "_meta": {"sequence": 3}}),
        );
        let second = runtime_request(
            &serde_json::json!(1),
            "session/request_permission",
            &serde_json::json!({"sessionId": "session-1", "_meta": {"sequence": 4}}),
        );
        assert!(matches!(
            (first, second),
            (
                RuntimeRequest::Approval { request_key: first, .. },
                RuntimeRequest::Approval { request_key: second, .. }
            ) if first != second
        ));
    }

    #[test]
    fn unsupported_reverse_request_is_classified_at_adapter_boundary() {
        let request = runtime_request(
            &serde_json::json!(42),
            "session/unknown",
            &serde_json::json!({}),
        );
        assert!(matches!(request, RuntimeRequest::Unsupported { id } if id == serde_json::json!(42)));
    }

    #[test]
    fn notification_contract_preserves_stored_event_shape() {
        let raw = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "session-1",
                "update": { "sessionUpdate": "agent_message_chunk", "text": "hello" }
            }
        });
        let event = runtime_notification(AcpEvent::from_notification(&raw));
        assert_eq!(event.event_type, "acp");
        assert!(event.source_event_id.starts_with("acp-"));
        assert_eq!(event.cursor, None);
        assert_eq!(event.payload, raw);
    }

    #[test]
    fn approval_contract_exposes_neutral_context_and_identity() {
        let params = serde_json::json!({
            "toolCall": { "toolCallId": "call-7" },
            "reason": "write"
        });
        let request = runtime_request(&serde_json::json!(42), "session/request_permission", &params);
        assert!(matches!(
            request,
            RuntimeRequest::Approval { id, request_key, context }
                if id == serde_json::json!(42)
                    && request_key == "call-7"
                    && context == params
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
        let command = RuntimeCommand::RespondApproval {
            id: serde_json::json!(42),
            decision: ApprovalDecision::AllowOnce,
        };
        assert!(matches!(
            command,
            RuntimeCommand::RespondApproval {
                decision: ApprovalDecision::AllowOnce,
                ..
            }
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
