//! Local ACP terminal host implemented by the gateway.
//!
//! When the Web client advertises `terminal` capability, Zene issues
//! `terminal/create|wait_for_exit|output|kill|release` requests to the client.
//! The gateway handles those locally and exposes a read-only Web view via events
//! plus HTTP helpers.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, Notify};
use uuid::Uuid;

use crate::event_journal::EventJournal;

const DEFAULT_OUTPUT_LIMIT: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    pub terminal_id: String,
    pub session_id: String,
    pub command: String,
    pub cwd: PathBuf,
    pub status: TerminalStatus,
    pub exit_code: Option<i32>,
    pub output_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TerminalStatus {
    Running,
    Exited,
    Released,
}

struct TerminalSession {
    info: TerminalInfo,
    output: String,
    output_limit: usize,
    child: Option<Child>,
    exit_notify: Arc<Notify>,
}

#[derive(Clone, Default)]
pub struct TerminalHost {
    inner: Arc<Mutex<HashMap<String, TerminalSession>>>,
}

impl TerminalHost {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn list(&self, session_id: Option<&str>) -> Vec<TerminalInfo> {
        let map = self.inner.lock().await;
        map.values()
            .filter(|t| session_id.is_none_or(|sid| t.info.session_id == sid))
            .map(|t| {
                let mut info = t.info.clone();
                info.output_len = t.output.len();
                info
            })
            .collect()
    }

    pub async fn output_since(
        &self,
        terminal_id: &str,
        offset: usize,
    ) -> Result<(String, usize, Option<i32>, TerminalStatus)> {
        let map = self.inner.lock().await;
        let term = map
            .get(terminal_id)
            .ok_or_else(|| anyhow!("unknown terminalId"))?;
        let slice = if offset >= term.output.len() {
            String::new()
        } else {
            term.output[offset..].to_string()
        };
        Ok((
            slice,
            term.output.len(),
            term.info.exit_code,
            term.info.status,
        ))
    }

    pub async fn kill(&self, terminal_id: &str) -> Result<()> {
        let mut map = self.inner.lock().await;
        let term = map
            .get_mut(terminal_id)
            .ok_or_else(|| anyhow!("unknown terminalId"))?;
        if let Some(child) = term.child.as_mut() {
            let _ = child.kill().await;
        }
        Ok(())
    }

    /// Handle an inbound ACP JSON-RPC request aimed at this client.
    pub async fn handle_request(
        &self,
        request: &Value,
        journal: &EventJournal,
        workspace: &std::path::Path,
    ) -> Result<Value> {
        let method = request
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let id = request.get("id").cloned();
        let params = request.get("params").cloned().unwrap_or(json!({}));

        let result = match method {
            "terminal/create" => self.create(&params, journal, workspace).await?,
            "terminal/wait_for_exit" => self.wait_for_exit(&params).await?,
            "terminal/output" => self.read_output(&params).await?,
            "terminal/kill" => {
                let tid = required_str(&params, "terminalId")?;
                self.kill(&tid).await?;
                journal
                    .append(json!({
                        "type": "gateway.terminal",
                        "kind": "killed",
                        "terminalId": tid,
                        "sessionId": params.get("sessionId"),
                    }))
                    .await;
                json!({})
            }
            "terminal/release" => {
                let tid = required_str(&params, "terminalId")?;
                self.release(&tid).await?;
                journal
                    .append(json!({
                        "type": "gateway.terminal",
                        "kind": "released",
                        "terminalId": tid,
                        "sessionId": params.get("sessionId"),
                    }))
                    .await;
                json!({})
            }
            _ => bail!("unsupported terminal method: {method}"),
        };

        Ok(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }))
    }

    async fn create(
        &self,
        params: &Value,
        journal: &EventJournal,
        workspace: &std::path::Path,
    ) -> Result<Value> {
        let session_id = required_str(params, "sessionId")?;
        let command = required_str(params, "command")?;
        let args = params
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let cwd = params
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| workspace.to_path_buf());
        let output_limit = params
            .get("outputByteLimit")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_OUTPUT_LIMIT as u64) as usize;

        let terminal_id = format!("term_{}", Uuid::new_v4().simple());
        let display_cmd = if args.is_empty() {
            command.clone()
        } else {
            format!("{command} {}", args.join(" "))
        };

        let mut child = Command::new(&command)
            .args(&args)
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let exit_notify = Arc::new(Notify::new());

        {
            let mut map = self.inner.lock().await;
            map.insert(
                terminal_id.clone(),
                TerminalSession {
                    info: TerminalInfo {
                        terminal_id: terminal_id.clone(),
                        session_id: session_id.clone(),
                        command: display_cmd.clone(),
                        cwd: cwd.clone(),
                        status: TerminalStatus::Running,
                        exit_code: None,
                        output_len: 0,
                    },
                    output: String::new(),
                    output_limit,
                    child: Some(child),
                    exit_notify: exit_notify.clone(),
                },
            );
        }

        journal
            .append(json!({
                "type": "gateway.terminal",
                "kind": "created",
                "terminalId": terminal_id,
                "sessionId": session_id,
                "command": display_cmd,
                "cwd": cwd,
            }))
            .await;

        let host = self.clone();
        let tid = terminal_id.clone();
        let journal_out = journal.clone();
        tokio::spawn(async move {
            if let Some(out) = stdout {
                pump_output(host.clone(), tid.clone(), journal_out.clone(), out, false).await;
            }
            if let Some(err) = stderr {
                pump_output(host.clone(), tid.clone(), journal_out.clone(), err, true).await;
            }

            let exit_code = {
                let mut map = host.inner.lock().await;
                if let Some(term) = map.get_mut(&tid) {
                    let code = if let Some(child) = term.child.as_mut() {
                        match child.wait().await {
                            Ok(status) => status.code(),
                            Err(_) => None,
                        }
                    } else {
                        None
                    };
                    term.info.exit_code = code;
                    term.info.status = TerminalStatus::Exited;
                    term.child = None;
                    term.exit_notify.notify_waiters();
                    code
                } else {
                    None
                }
            };
            journal_out
                .append(json!({
                    "type": "gateway.terminal",
                    "kind": "exited",
                    "terminalId": tid,
                    "exitCode": exit_code,
                }))
                .await;
        });

        Ok(json!({ "terminalId": terminal_id }))
    }

    async fn wait_for_exit(&self, params: &Value) -> Result<Value> {
        let tid = required_str(params, "terminalId")?;
        let notify = {
            let map = self.inner.lock().await;
            let term = map
                .get(&tid)
                .ok_or_else(|| anyhow!("unknown terminalId"))?;
            if term.info.status != TerminalStatus::Running {
                return Ok(json!({
                    "exitCode": term.info.exit_code.unwrap_or(0),
                }));
            }
            term.exit_notify.clone()
        };
        // Avoid hanging forever in tests / stuck processes.
        let _ = tokio::time::timeout(Duration::from_secs(30 * 60), notify.notified()).await;
        let map = self.inner.lock().await;
        let term = map
            .get(&tid)
            .ok_or_else(|| anyhow!("unknown terminalId"))?;
        Ok(json!({
            "exitCode": term.info.exit_code.unwrap_or(0),
        }))
    }

    async fn read_output(&self, params: &Value) -> Result<Value> {
        let tid = required_str(params, "terminalId")?;
        let map = self.inner.lock().await;
        let term = map
            .get(&tid)
            .ok_or_else(|| anyhow!("unknown terminalId"))?;
        Ok(json!({
            "output": term.output,
            "exitStatus": {
                "exitCode": term.info.exit_code.unwrap_or(0)
            },
            "exitCode": term.info.exit_code.unwrap_or(0),
        }))
    }

    async fn release(&self, terminal_id: &str) -> Result<()> {
        let mut map = self.inner.lock().await;
        if let Some(mut term) = map.remove(terminal_id) {
            if let Some(child) = term.child.as_mut() {
                let _ = child.kill().await;
            }
            // Keep a released tombstone for UI list briefly.
            term.info.status = TerminalStatus::Released;
            map.insert(
                terminal_id.to_string(),
                TerminalSession {
                    info: term.info,
                    output: term.output,
                    output_limit: term.output_limit,
                    child: None,
                    exit_notify: term.exit_notify,
                },
            );
        }
        Ok(())
    }
}

async fn pump_output<R: tokio::io::AsyncRead + Unpin>(
    host: TerminalHost,
    terminal_id: String,
    journal: EventJournal,
    reader: R,
    is_stderr: bool,
) {
    let mut reader = BufReader::new(reader);
    let mut buf = [0u8; 2048];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                {
                    let mut map = host.inner.lock().await;
                    if let Some(term) = map.get_mut(&terminal_id) {
                        term.output.push_str(&chunk);
                        if term.output.len() > term.output_limit {
                            let keep = term.output_limit / 2;
                            term.output = term.output[term.output.len() - keep..].to_string();
                        }
                        term.info.output_len = term.output.len();
                    }
                }
                journal
                    .append(json!({
                        "type": "gateway.terminal",
                        "kind": "output",
                        "terminalId": terminal_id,
                        "stream": if is_stderr { "stderr" } else { "stdout" },
                        "chunk": chunk,
                    }))
                    .await;
            }
            Err(_) => break,
        }
    }
}

fn required_str(params: &Value, key: &str) -> Result<String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("{key} required"))
}

pub fn is_terminal_request(payload: &Value) -> bool {
    payload.get("id").is_some()
        && payload
            .get("method")
            .and_then(|m| m.as_str())
            .is_some_and(|m| m.starts_with("terminal/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn create_wait_output_release() {
        let host = TerminalHost::new();
        let journal = EventJournal::new();
        let dir = tempdir().unwrap();
        let create = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "terminal/create",
            "params": {
                "sessionId": "s1",
                "command": "printf",
                "args": ["hello-term"],
                "cwd": dir.path(),
                "outputByteLimit": 4096
            }
        });
        let created = host
            .handle_request(&create, &journal, dir.path())
            .await
            .unwrap();
        let tid = created["result"]["terminalId"].as_str().unwrap().to_string();

        let wait = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "terminal/wait_for_exit",
            "params": { "sessionId": "s1", "terminalId": tid }
        });
        let waited = host
            .handle_request(&wait, &journal, dir.path())
            .await
            .unwrap();
        assert_eq!(waited["result"]["exitCode"], 0);

        let output = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "terminal/output",
            "params": { "sessionId": "s1", "terminalId": tid }
        });
        let out = host
            .handle_request(&output, &journal, dir.path())
            .await
            .unwrap();
        assert!(out["result"]["output"]
            .as_str()
            .unwrap()
            .contains("hello-term"));
    }
}
