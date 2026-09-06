use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tracing::warn;

use crate::engine::{hook_failure_reason, HookRunRequest};

/// User-visible block reason from a hook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookBlock {
    pub reason: String,
    #[serde(default)]
    pub terminate: bool,
}

impl HookBlock {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            terminate: false,
        }
    }

    pub fn terminate(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            terminate: true,
        }
    }

    pub fn with_terminate(mut self, terminate: bool) -> Self {
        self.terminate = terminate;
        self
    }
}

/// Result of executing one hook invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookOutcome {
    Allow,
    Block(HookBlock),
}

/// Runtime adapter: run planned hooks (subprocess, remote, etc.).
#[async_trait]
pub trait HookExecutor: Send + Sync {
    async fn run(&self, request: &HookRunRequest) -> Result<HookOutcome>;
}

/// Default executor: `bash -c` in a workspace directory.
pub struct BashHookExecutor {
    workdir: std::path::PathBuf,
}

impl BashHookExecutor {
    pub fn new(workdir: impl Into<std::path::PathBuf>) -> Self {
        Self {
            workdir: workdir.into(),
        }
    }

    pub fn workdir(&self) -> &Path {
        &self.workdir
    }
}

#[async_trait]
impl HookExecutor for BashHookExecutor {
    async fn run(&self, request: &HookRunRequest) -> Result<HookOutcome> {
        let mut child = Command::new("bash")
            .arg("-c")
            .arg(&request.command)
            .current_dir(&self.workdir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("spawn hook command: {}", request.command))?;

        if let Some(mut stdin) = child.stdin.take() {
            if let Err(err) = stdin.write_all(request.stdin_json.as_bytes()).await {
                if err.kind() != std::io::ErrorKind::BrokenPipe {
                    return Err(err).context("write hook stdin");
                }
            } else if let Err(err) = stdin.shutdown().await {
                if err.kind() != std::io::ErrorKind::BrokenPipe {
                    return Err(err).context("close hook stdin");
                }
            }
        }

        let output = child
            .wait_with_output()
            .await
            .context("wait for hook command")?;

        // 1. Structured JSON output check (stdout can define block/terminate explicitly)
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
            if let Some(block_val) = value.get("block").and_then(|v| v.as_bool()) {
                if block_val {
                    let reason = value
                        .get("reason")
                        .and_then(|v| v.as_str())
                        .unwrap_or("blocked by hook")
                        .to_string();
                    let terminate = value
                        .get("terminate")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    return Ok(HookOutcome::Block(HookBlock { reason, terminate }));
                }
            } else if let Some(decision) = value.get("decision").and_then(|v| v.as_str()) {
                if decision == "block" || decision == "terminate" {
                    let reason = value
                        .get("reason")
                        .and_then(|v| v.as_str())
                        .unwrap_or("blocked by hook")
                        .to_string();
                    let terminate = decision == "terminate"
                        || value
                            .get("terminate")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                    return Ok(HookOutcome::Block(HookBlock { reason, terminate }));
                }
            }
        }

        // 2. Non-zero exit code check
        if !output.status.success() {
            let reason = hook_failure_reason(&output.stderr, &output.stdout);
            let terminate = output.status.code() == Some(2);
            if request.blocking {
                return Ok(HookOutcome::Block(HookBlock { reason, terminate }));
            }
            warn!(
                command = %request.command,
                reason = %reason,
                "hook exited with non-zero status (non-blocking)"
            );
        }

        Ok(HookOutcome::Allow)
    }
}
