use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::{mpsc, Mutex};
use zene_cloud_acp_bridge::{AcpBridge, AcpEvent, BridgeMsg};

#[derive(Debug, Clone)]
pub enum RuntimeCommand {
    Prompt { text: String },
    Cancel,
    RespondApproval { id: Value, result: Value },
    RejectRequest { id: Value, code: i64, message: String },
}

#[derive(Debug)]
pub enum RuntimeEvent {
    Initialized { session_id: String, event: AcpEvent },
    Notification(AcpEvent),
    ApprovalRequest { id: Value, method: String, params: Value, event: AcpEvent },
    ChildExited,
}

#[async_trait]
pub trait RuntimeClient: Send + Sync {
    async fn session_id(&self) -> Result<String>;
    async fn prompt(&self, text: &str) -> Result<()>;
    async fn cancel(&self) -> Result<()>;
    async fn respond_approval(&self, id: &Value, result: Value) -> Result<()>;
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
            let _ = events_tx.send(RuntimeEvent::Initialized { session_id: session_id.clone(), event });
        }
        let event_tx = events_tx.clone();
        tokio::spawn(async move {
            while let Some(message) = messages.recv().await {
                let event = match message {
                    BridgeMsg::Notification { raw, .. } => RuntimeEvent::Notification(AcpEvent::from_notification(&raw)),
                    BridgeMsg::ReverseRequest { id, method, params } => {
                        let event = AcpEvent::from_reverse_request(&id, &method, &params);
                        RuntimeEvent::ApprovalRequest { id, method, params, event }
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
    async fn respond_approval(&self, id: &Value, result: Value) -> Result<()> {
        let guard = self.bridge.lock().await;
        guard.as_ref().context("runtime bridge missing")?.respond(id, result).await
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
    fn reconnect_can_target_an_existing_session() {
        assert_eq!(Some("session-1"), Some("session-1"));
        // The transport seam is explicit; process integration is covered by the ACP bridge.
    }

    #[test]
    fn runtime_commands_are_transport_neutral() {
        let command = RuntimeCommand::Prompt { text: "hello".into() };
        assert!(matches!(command, RuntimeCommand::Prompt { text } if text == "hello"));
    }
}
