//! ACP client terminal bridge (`terminal/create|output|wait_for_exit|kill|release`).

use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use zene_sandbox::{ExecResult, RemoteTerminal};

use super::transport::AcpWriter;

pub struct AcpRemoteTerminal {
    writer: AcpWriter,
    session_id: String,
}

impl AcpRemoteTerminal {
    pub fn new(writer: AcpWriter, session_id: impl Into<String>) -> Self {
        Self {
            writer,
            session_id: session_id.into(),
        }
    }
}

#[async_trait]
impl RemoteTerminal for AcpRemoteTerminal {
    async fn exec(
        &self,
        command: &str,
        cwd: &Path,
        timeout: Duration,
        cancel: Option<&CancellationToken>,
        output_byte_limit: usize,
    ) -> Result<ExecResult> {
        let cwd = cwd
            .to_str()
            .ok_or_else(|| anyhow!("cwd is not valid UTF-8"))?;

        #[cfg(unix)]
        let (program, args) = {
            let quoted = shell_quote(command);
            ("bash".to_string(), vec!["-lc".to_string(), quoted])
        };
        #[cfg(not(unix))]
        let (program, args) = (command.to_string(), Vec::<String>::new());

        let created = self
            .writer
            .request(
                "terminal/create",
                json!({
                    "sessionId": self.session_id,
                    "command": program,
                    "args": args,
                    "cwd": cwd,
                    "outputByteLimit": output_byte_limit as u64,
                }),
            )
            .await
            .context("ACP terminal/create")?;
        let terminal_id = created
            .get("terminalId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("terminal/create missing terminalId"))?
            .to_string();

        let wait = self.writer.request(
            "terminal/wait_for_exit",
            json!({
                "sessionId": self.session_id,
                "terminalId": terminal_id,
            }),
        );
        let timed = tokio::time::timeout(timeout, wait);

        let wait_result = if let Some(token) = cancel {
            tokio::select! {
                _ = token.cancelled() => {
                    let _ = self
                        .writer
                        .request(
                            "terminal/kill",
                            json!({
                                "sessionId": self.session_id,
                                "terminalId": terminal_id,
                            }),
                        )
                        .await;
                    Err(anyhow!("aborted"))
                }
                result = timed => match result {
                    Ok(inner) => inner.context("ACP terminal/wait_for_exit"),
                    Err(_) => {
                        let _ = self
                            .writer
                            .request(
                                "terminal/kill",
                                json!({
                                    "sessionId": self.session_id,
                                    "terminalId": terminal_id,
                                }),
                            )
                            .await;
                        Err(anyhow!(
                            "command timed out after {} seconds",
                            timeout.as_secs().max(1)
                        ))
                    }
                },
            }
        } else {
            match timed.await {
                Ok(inner) => inner.context("ACP terminal/wait_for_exit"),
                Err(_) => {
                    let _ = self
                        .writer
                        .request(
                            "terminal/kill",
                            json!({
                                "sessionId": self.session_id,
                                "terminalId": terminal_id,
                            }),
                        )
                        .await;
                    Err(anyhow!(
                        "command timed out after {} seconds",
                        timeout.as_secs().max(1)
                    ))
                }
            }
        };

        let output = self
            .writer
            .request(
                "terminal/output",
                json!({
                    "sessionId": self.session_id,
                    "terminalId": terminal_id,
                }),
            )
            .await
            .context("ACP terminal/output");

        let _ = self
            .writer
            .request(
                "terminal/release",
                json!({
                    "sessionId": self.session_id,
                    "terminalId": terminal_id,
                }),
            )
            .await;

        // Prefer surfacing abort/timeout after cleanup.
        wait_result?;
        let output = output?;

        let exit_code = output
            .pointer("/exitStatus/exitCode")
            .or_else(|| output.get("exitCode"))
            .and_then(|v| v.as_u64())
            .or_else(|| {
                output
                    .pointer("/exitStatus/exit_code")
                    .and_then(|v| v.as_u64())
            })
            .unwrap_or(0) as i32;
        let combined = output
            .get("output")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Ok(ExecResult {
            stdout: combined,
            stderr: String::new(),
            exit_code,
        })
    }
}

#[cfg(unix)]
fn shell_quote(input: &str) -> String {
    // Minimal single-quote escaping for `bash -lc`.
    let mut out = String::from("'");
    for ch in input.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}
