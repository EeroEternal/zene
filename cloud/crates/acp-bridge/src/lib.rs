use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::Mutex;
use tracing::{info, warn};
use uuid::Uuid;

/// Thin ACP NDJSON bridge used by the cloud worker.
pub struct AcpBridge {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    next_id: AtomicU64,
}

#[derive(Debug, Clone)]
pub struct AcpEvent {
    pub source_event_id: String,
    pub event_type: String,
    pub payload: Value,
}

impl AcpBridge {
    pub async fn spawn(zene_bin: &Path, workdir: &Path, yolo: bool) -> Result<Self> {
        let mut cmd = Command::new(zene_bin);
        cmd.current_dir(workdir)
            .arg("acp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if yolo {
            cmd.arg("--yolo");
        }
        let mut child = cmd.spawn().context("spawn zene acp")?;
        let stdin = child.stdin.take().context("missing stdin")?;
        Ok(Self {
            child,
            stdin: Arc::new(Mutex::new(stdin)),
            next_id: AtomicU64::new(1),
        })
    }

    pub async fn initialize_and_new_session(&self, cwd: &Path) -> Result<(String, Vec<AcpEvent>)> {
        let mut events = Vec::new();
        let init = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": 1,
                    "clientCapabilities": {
                        "fs": { "readTextFile": false, "writeTextFile": false },
                        "terminal": false
                    }
                }),
            )
            .await?;
        events.push(AcpEvent {
            source_event_id: format!("init-{}", Uuid::new_v4()),
            event_type: "acp".into(),
            payload: json!({ "method": "initialize", "result": init }),
        });

        let session = self
            .request(
                "session/new",
                json!({
                    "cwd": cwd.display().to_string(),
                    "mcpServers": []
                }),
            )
            .await?;
        let session_id = session
            .get("sessionId")
            .and_then(|v| v.as_str())
            .context("sessionId missing")?
            .to_string();
        events.push(AcpEvent {
            source_event_id: format!("session-new-{}", session_id),
            event_type: "acp".into(),
            payload: json!({ "method": "session/new", "result": session }),
        });
        Ok((session_id, events))
    }

    pub async fn prompt(&self, session_id: &str, text: &str) -> Result<()> {
        let _ = self
            .request(
                "session/prompt",
                json!({
                    "sessionId": session_id,
                    "prompt": [{ "type": "text", "text": text }]
                }),
            )
            .await?;
        Ok(())
    }

    pub async fn cancel(&self, session_id: &str) -> Result<()> {
        self.notify("session/cancel", json!({ "sessionId": session_id }))
            .await
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        self.write(&msg).await?;
        // Phase 0: wait for matching response by reading stdout lines until id matches.
        // A full production bridge should multiplex notifications and reverse requests.
        bail!("request helper requires attached stdout pump; use mock mode or worker pump")
    }

    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        self.write(&msg).await
    }

    async fn write(&self, value: &Value) -> Result<()> {
        let mut stdin = self.stdin.lock().await;
        let line = format!("{value}\n");
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    pub fn take_stdout_reader(&mut self) -> Option<BufReader<tokio::process::ChildStdout>> {
        self.child.stdout.take().map(BufReader::new)
    }

    pub async fn kill(mut self) -> Result<()> {
        let _ = self.child.kill().await;
        Ok(())
    }
}

/// Local mock agent used when `zene` binary is unavailable.
pub struct MockAgent {
    workdir: PathBuf,
}

impl MockAgent {
    pub fn new(workdir: PathBuf) -> Self {
        Self { workdir }
    }

    pub async fn run_prompt(&self, prompt: &str) -> Result<Vec<AcpEvent>> {
        std::fs::create_dir_all(&self.workdir)?;
        let note = self.workdir.join("AGENT_NOTES.md");
        let body = format!(
            "# Agent Notes\n\nPrompt:\n\n{}\n\nWorkspace: {}\n",
            prompt,
            self.workdir.display()
        );
        tokio::fs::write(&note, &body).await?;

        let session_id = Uuid::new_v4().to_string();
        Ok(vec![
            AcpEvent {
                source_event_id: format!("{session_id}-start"),
                event_type: "acp".into(),
                payload: json!({
                    "jsonrpc": "2.0",
                    "method": "session/update",
                    "params": {
                        "sessionId": session_id,
                        "update": {
                            "sessionUpdate": "agent_message_chunk",
                            "content": { "type": "text", "text": "收到任务，开始在隔离工作区执行。\n" }
                        }
                    }
                }),
            },
            AcpEvent {
                source_event_id: format!("{session_id}-tool"),
                event_type: "acp".into(),
                payload: json!({
                    "jsonrpc": "2.0",
                    "method": "session/update",
                    "params": {
                        "sessionId": session_id,
                        "update": {
                            "sessionUpdate": "tool_call",
                            "toolCallId": "tool_write_notes",
                            "title": "Write AGENT_NOTES.md",
                            "status": "completed",
                            "rawInput": { "path": "AGENT_NOTES.md" }
                        }
                    }
                }),
            },
            AcpEvent {
                source_event_id: format!("{session_id}-done"),
                event_type: "acp".into(),
                payload: json!({
                    "jsonrpc": "2.0",
                    "method": "session/update",
                    "params": {
                        "sessionId": session_id,
                        "update": {
                            "sessionUpdate": "agent_message_chunk",
                            "content": {
                                "type": "text",
                                "text": format!(
                                    "已写入 `AGENT_NOTES.md`。\n\n这是 Phase 0 mock agent。若配置 `ZENE_BIN`，worker 将改为启动真实 `zene acp`。\n工作区：`{}`\n",
                                    self.workdir.display()
                                )
                            }
                        }
                    }
                }),
            },
        ])
    }
}

pub async fn read_acp_line<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
) -> Result<Option<Value>> {
    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        return Ok(None);
    }
    let value = serde_json::from_str(line.trim())
        .with_context(|| format!("invalid ACP json: {}", line.trim()))?;
    Ok(Some(value))
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
            return Some(path);
        }
    }
    for candidate in ["zene", "/usr/local/bin/zene"] {
        if let Ok(output) = std::process::Command::new("which").arg(candidate).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    info!(%path, "found zene binary");
                    return Some(PathBuf::from(path));
                }
            }
        }
    }
    None
}
