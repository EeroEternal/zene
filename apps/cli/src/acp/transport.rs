use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use tokio::sync::oneshot;

pub(crate) struct PendingResponse {
    tx: oneshot::Sender<Result<Value, Value>>,
}

pub(crate) struct SharedState {
    pub next_id: u64,
    pub pending: HashMap<String, PendingResponse>,
}

impl SharedState {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            pending: HashMap::new(),
        }
    }

    pub fn insert_pending(&mut self, id: String, tx: oneshot::Sender<Result<Value, Value>>) {
        self.pending.insert(id, PendingResponse { tx });
    }

    pub fn take_pending(&mut self, id: &str) -> Option<oneshot::Sender<Result<Value, Value>>> {
        self.pending.remove(id).map(|p| p.tx)
    }
}

#[derive(Clone)]
pub(crate) struct AcpWriter {
    pub tx: tokio::sync::mpsc::UnboundedSender<String>,
    pub shared: Arc<Mutex<SharedState>>,
}

impl AcpWriter {
    pub fn send_raw(&self, line: String) -> Result<()> {
        self.tx
            .send(line)
            .map_err(|_| anyhow!("ACP stdout writer closed"))
    }

    pub fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.send_raw(
            json!({
                "jsonrpc": "2.0",
                "method": method,
                "params": params,
            })
            .to_string(),
        )
    }

    pub fn session_update(&self, session_id: &str, update: Value) -> Result<()> {
        self.notify(
            "session/update",
            json!({
                "sessionId": session_id,
                "update": update,
            }),
        )
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = {
            let mut g = self.shared.lock().unwrap();
            g.next_id += 1;
            g.next_id
        };
        let id_key = id.to_string();
        let (tx, rx) = oneshot::channel();
        {
            let mut g = self.shared.lock().unwrap();
            g.insert_pending(id_key.clone(), tx);
        }
        self.send_raw(
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params,
            })
            .to_string(),
        )?;
        match rx.await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(anyhow!("ACP client error: {e}")),
            Err(_) => Err(anyhow!("ACP client response channel closed")),
        }
    }
}
