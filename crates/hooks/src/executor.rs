use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tracing::warn;

use crate::engine::{hook_failure_reason, HookRunRequest};
use crate::runner::HookBlock;

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

        if !output.status.success() {
            let reason = hook_failure_reason(&output.stderr, &output.stdout);
            if request.blocking {
                return Ok(HookOutcome::Block(HookBlock { reason }));
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
