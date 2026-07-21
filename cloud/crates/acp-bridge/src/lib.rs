use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Thin ACP NDJSON bridge used by the cloud worker.
pub struct AcpBridge {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    pump: tokio::task::JoinHandle<()>,
}

/// Outbound frames from the ACP child (notifications + reverse requests).
#[derive(Debug)]
pub enum BridgeMsg {
    Notification {
        method: String,
        params: Value,
        raw: Value,
    },
    ReverseRequest {
        id: Value,
        method: String,
        params: Value,
    },
}

#[derive(Debug, Clone)]
pub struct AcpEvent {
    pub source_event_id: String,
    pub event_type: String,
    pub payload: Value,
}

impl AcpEvent {
    pub fn from_notification(raw: &Value) -> Self {
        let sid = raw
            .pointer("/params/sessionId")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let update = raw
            .pointer("/params/update/sessionUpdate")
            .and_then(|v| v.as_str())
            .unwrap_or("update");
        Self {
            source_event_id: format!("{sid}-{update}-{}", Uuid::new_v4()),
            event_type: "acp".into(),
            payload: raw.clone(),
        }
    }

    pub fn from_reverse_request(id: &Value, method: &str, params: &Value) -> Self {
        Self {
            source_event_id: format!("rev-{}-{}", method.replace('/', "."), Uuid::new_v4()),
            event_type: "acp".into(),
            payload: json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params,
            }),
        }
    }
}

fn id_key(id: &Value) -> String {
    match id {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

impl AcpBridge {
    pub async fn spawn(
        zene_bin: &Path,
        workdir: &Path,
        yolo: bool,
    ) -> Result<(Self, mpsc::UnboundedReceiver<BridgeMsg>)> {
        let mut cmd = Command::new(zene_bin);
        cmd.current_dir(workdir)
            .arg("acp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if yolo {
            cmd.arg("--yolo");
        }
        let mut child = cmd.spawn().context("spawn zene acp")?;
        let stdin = child.stdin.take().context("missing stdin")?;
        let stdout = child.stdout.take().context("missing stdout")?;
        let stderr = child.stderr.take();

        if let Some(stderr) = stderr {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line).await {
                        Ok(0) => break,
                        Ok(_) => {
                            let trimmed = line.trim();
                            if !trimmed.is_empty() {
                                debug!(target: "zene_acp", "{trimmed}");
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
        }

        let (msg_tx, msg_rx) = mpsc::unbounded_channel();
        let pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let pending_pump = Arc::clone(&pending);
        let pump = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(err) => {
                        warn!(error = %err, "acp stdout read failed");
                        break;
                    }
                }
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let value: Value = match serde_json::from_str(trimmed) {
                    Ok(v) => v,
                    Err(err) => {
                        warn!(error = %err, line = %trimmed, "invalid ACP json");
                        continue;
                    }
                };
                dispatch_frame(&pending_pump, &msg_tx, value).await;
            }
        });

        Ok((
            Self {
                child,
                stdin: Arc::new(Mutex::new(stdin)),
                next_id: AtomicU64::new(1),
                pending,
                pump,
            },
            msg_rx,
        ))
    }

    pub async fn initialize(&self) -> Result<Value> {
        self.request(
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {
                    "fs": { "readTextFile": false, "writeTextFile": false },
                    "terminal": false
                },
                "clientInfo": {
                    "name": "zene-cloud-worker",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        )
        .await
    }

    pub async fn session_new(&self, cwd: &Path) -> Result<String> {
        let session = self
            .request(
                "session/new",
                json!({
                    "cwd": cwd.display().to_string(),
                    "mcpServers": []
                }),
            )
            .await?;
        session
            .get("sessionId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .context("sessionId missing")
    }

    pub async fn initialize_and_new_session(&self, cwd: &Path) -> Result<(String, Vec<AcpEvent>)> {
        let mut events = Vec::new();
        let init = self.initialize().await?;
        events.push(AcpEvent {
            source_event_id: format!("init-{}", Uuid::new_v4()),
            event_type: "acp".into(),
            payload: json!({ "method": "initialize", "result": init }),
        });
        let session_id = self.session_new(cwd).await?;
        events.push(AcpEvent {
            source_event_id: format!("session-new-{session_id}"),
            event_type: "acp".into(),
            payload: json!({
                "method": "session/new",
                "result": { "sessionId": session_id }
            }),
        });
        Ok((session_id, events))
    }

    pub async fn prompt(&self, session_id: &str, text: &str) -> Result<Value> {
        self.request(
            "session/prompt",
            json!({
                "sessionId": session_id,
                "prompt": [{ "type": "text", "text": text }]
            }),
        )
        .await
    }

    pub async fn cancel(&self, session_id: &str) -> Result<()> {
        self.notify("session/cancel", json!({ "sessionId": session_id }))
            .await
    }

    pub async fn respond(&self, id: &Value, result: Value) -> Result<()> {
        self.write(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result
        }))
        .await
    }

    pub async fn respond_error(&self, id: &Value, code: i64, message: &str) -> Result<()> {
        self.write(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message }
        }))
        .await
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let id_value = json!(id);
        let key = id.to_string();
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(key.clone(), tx);
        }
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id_value,
            "method": method,
            "params": params
        });
        if let Err(err) = self.write(&msg).await {
            let mut pending = self.pending.lock().await;
            pending.remove(&key);
            return Err(err);
        }
        match tokio::time::timeout(Duration::from_secs(600), rx).await {
            Ok(Ok(value)) => {
                if let Some(err) = value.get("error") {
                    bail!("{method} failed: {err}");
                }
                Ok(value.get("result").cloned().unwrap_or(Value::Null))
            }
            Ok(Err(_)) => bail!("{method}: response channel closed"),
            Err(_) => {
                let mut pending = self.pending.lock().await;
                pending.remove(&key);
                bail!("{method}: timed out waiting for response")
            }
        }
    }

    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.write(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        }))
        .await
    }

    async fn write(&self, value: &Value) -> Result<()> {
        let mut stdin = self.stdin.lock().await;
        let line = format!("{value}\n");
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    pub async fn kill(mut self) -> Result<()> {
        self.pump.abort();
        let _ = self.child.kill().await;
        Ok(())
    }
}

async fn dispatch_frame(
    pending: &Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    msg_tx: &mpsc::UnboundedSender<BridgeMsg>,
    value: Value,
) {
    let has_id = value.get("id").is_some();
    let method = value
        .get("method")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    match (has_id, method) {
        (true, None) => {
            // JSON-RPC response to one of our requests.
            let id = value.get("id").cloned().unwrap_or(Value::Null);
            let key = id_key(&id);
            let sender = {
                let mut map = pending.lock().await;
                map.remove(&key)
            };
            if let Some(tx) = sender {
                let _ = tx.send(value);
            } else {
                warn!(%key, "ACP response with unknown id");
            }
        }
        (true, Some(method)) => {
            // Reverse request from agent (e.g. session/request_permission).
            let id = value.get("id").cloned().unwrap_or(Value::Null);
            let params = value.get("params").cloned().unwrap_or(json!({}));
            let _ = msg_tx.send(BridgeMsg::ReverseRequest { id, method, params });
        }
        (false, Some(method)) => {
            let params = value.get("params").cloned().unwrap_or(json!({}));
            let _ = msg_tx.send(BridgeMsg::Notification {
                method,
                params,
                raw: value,
            });
        }
        (false, None) => {
            warn!(frame = %value, "ignoring malformed ACP frame");
        }
    }
}

/// Decision returned to ACP for `session/request_permission`.
#[derive(Debug, Clone)]
pub struct PermissionDecision {
    pub option_id: String,
}

impl PermissionDecision {
    pub fn allow_once() -> Self {
        Self {
            option_id: "allow-once".into(),
        }
    }

    pub fn to_result(&self) -> Value {
        json!({ "outcome": { "optionId": self.option_id } })
    }
}

/// Local mock agent used when `zene` binary is unavailable.
pub struct MockAgent {
    workdir: PathBuf,
    session_id: String,
}

pub enum MockMsg {
    Event(AcpEvent),
    Permission {
        request_key: String,
        params: Value,
        respond: oneshot::Sender<PermissionDecision>,
    },
}

impl MockAgent {
    pub fn new(workdir: PathBuf) -> Self {
        Self {
            workdir,
            session_id: Uuid::new_v4().to_string(),
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Run a prompt, streaming mock ACP events and a permission request.
    pub async fn run_prompt(
        &self,
        prompt: &str,
        msg_tx: mpsc::UnboundedSender<MockMsg>,
    ) -> Result<()> {
        std::fs::create_dir_all(&self.workdir)?;
        std::fs::create_dir_all(self.workdir.join("src"))?;

        let session_id = self.session_id.clone();
        let send_update = |tx: &mpsc::UnboundedSender<MockMsg>, update: Value| {
            let raw = json!({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {
                    "sessionId": session_id,
                    "update": update
                }
            });
            let _ = tx.send(MockMsg::Event(AcpEvent::from_notification(&raw)));
        };

        send_update(
            &msg_tx,
            json!({
                "sessionUpdate": "agent_message_chunk",
                "content": {
                    "type": "text",
                    "text": "收到任务，开始在隔离工作区执行。\n"
                }
            }),
        );
        tokio::time::sleep(Duration::from_millis(80)).await;

        send_update(
            &msg_tx,
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "tool_write_notes",
                "title": "Write AGENT_NOTES.md",
                "kind": "edit",
                "status": "pending",
                "rawInput": { "path": "AGENT_NOTES.md" }
            }),
        );

        // Simulate permission request before writing files.
        let (perm_tx, perm_rx) = oneshot::channel();
        let perm_params = json!({
            "sessionId": session_id,
            "toolCall": {
                "toolCallId": "tool_write_notes",
                "title": "Write AGENT_NOTES.md",
                "kind": "edit",
                "status": "pending",
                "rawInput": { "path": "AGENT_NOTES.md", "prompt": prompt }
            },
            "options": [
                {
                    "optionId": "allow-once",
                    "name": "Allow once",
                    "kind": "allow_once"
                },
                {
                    "optionId": "reject-once",
                    "name": "Reject",
                    "kind": "reject_once"
                }
            ]
        });
        let _ = msg_tx.send(MockMsg::Permission {
            request_key: format!("mock-write-{}", Uuid::new_v4()),
            params: perm_params,
            respond: perm_tx,
        });

        let decision = tokio::time::timeout(Duration::from_secs(300), perm_rx)
            .await
            .map_err(|_| anyhow!("mock permission timed out"))?
            .map_err(|_| anyhow!("mock permission channel closed"))?;

        if decision.option_id.starts_with("reject") {
            send_update(
                &msg_tx,
                json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": {
                        "type": "text",
                        "text": "权限被拒绝，已停止写入。\n"
                    }
                }),
            );
            return Ok(());
        }

        let notes = self.workdir.join("AGENT_NOTES.md");
        let notes_body = format!(
            "# Agent Notes\n\nPrompt:\n\n{}\n\nWorkspace: {}\n",
            prompt,
            self.workdir.display()
        );
        tokio::fs::write(&notes, &notes_body).await?;

        let hello = self.workdir.join("src/hello.rs");
        let hello_body = format!(
            "// generated by mock agent\npub fn greet() -> &'static str {{\n    \"hello from mock agent\"\n}}\n\n// prompt fingerprint: {}\n",
            prompt.chars().take(48).collect::<String>().replace('\"', "'")
        );
        tokio::fs::write(&hello, &hello_body).await?;

        let readme = self.workdir.join("README.md");
        let existing = tokio::fs::read_to_string(&readme)
            .await
            .unwrap_or_else(|_| "# Workspace\n".into());
        let updated = if existing.contains("## Agent Changes") {
            existing
        } else {
            format!(
                "{existing}\n## Agent Changes\n\n- Added `AGENT_NOTES.md`\n- Added `src/hello.rs`\n"
            )
        };
        tokio::fs::write(&readme, updated).await?;

        send_update(
            &msg_tx,
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "tool_write_notes",
                "title": "Write AGENT_NOTES.md",
                "kind": "edit",
                "status": "completed",
                "rawInput": { "path": "AGENT_NOTES.md" },
                "content": [{
                    "type": "diff",
                    "path": "AGENT_NOTES.md",
                    "oldText": "",
                    "newText": notes_body
                }]
            }),
        );
        tokio::time::sleep(Duration::from_millis(60)).await;

        send_update(
            &msg_tx,
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "tool_write_hello",
                "title": "Write src/hello.rs",
                "kind": "edit",
                "status": "completed",
                "rawInput": { "path": "src/hello.rs" }
            }),
        );
        tokio::time::sleep(Duration::from_millis(60)).await;

        send_update(
            &msg_tx,
            json!({
                "sessionUpdate": "agent_message_chunk",
                "content": {
                    "type": "text",
                    "text": format!(
                        "已写入 `AGENT_NOTES.md`、`src/hello.rs`，并更新 `README.md`。\n\n这是 mock agent。若配置或自动发现 `ZENE_BIN`，worker 将改为启动真实 `zene acp`。\n工作区：`{}`\n",
                        self.workdir.display()
                    )
                }
            }),
        );

        Ok(())
    }
}

pub fn resolve_zene_bin(explicit: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        if path.exists() {
            return Some(path);
        }
        warn!(path = %path.display(), "configured zene bin missing");
    }
    if let Ok(path) = std::env::var("ZENE_BIN") {
        let path = PathBuf::from(path);
        if path.exists() {
            info!(path = %path.display(), "using ZENE_BIN");
            return Some(path);
        }
        warn!(path = %path.display(), "ZENE_BIN set but missing");
    }

    let mut candidates = vec![
        PathBuf::from("/workspace/target/debug/zene"),
        PathBuf::from("/workspace/target/release/zene"),
        PathBuf::from("./target/debug/zene"),
        PathBuf::from("./target/release/zene"),
        PathBuf::from("../target/debug/zene"),
        PathBuf::from("../target/release/zene"),
    ];
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("target/debug/zene"));
        candidates.push(cwd.join("target/release/zene"));
        if let Some(parent) = cwd.parent() {
            candidates.push(parent.join("target/debug/zene"));
            candidates.push(parent.join("target/release/zene"));
        }
    }
    for path in candidates {
        if path.exists() {
            info!(path = %path.display(), "auto-discovered zene binary");
            return Some(path);
        }
    }

    if let Ok(output) = std::process::Command::new("which").arg("zene").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                let path = PathBuf::from(path);
                if path.exists() {
                    info!(path = %path.display(), "found zene on PATH");
                    return Some(path);
                }
            }
        }
    }
    None
}
