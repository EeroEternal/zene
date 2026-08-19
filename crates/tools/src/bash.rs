use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use zene_llm::ToolDefinition;
use zene_sandbox::Sandbox;

use crate::background::{BackgroundTaskKind, BackgroundTaskStatus, BackgroundTaskStore};
use crate::registry::{Tool, ToolContext, ToolResult};

/// Background Bash jobs may run longer than interactive Bash.
const BACKGROUND_BASH_TIMEOUT_SECS: u64 = 30 * 60;

pub struct BashTool;

#[derive(Debug, Deserialize)]
struct BashArgs {
    command: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    run_in_background: bool,
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "Bash"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "Bash".to_string(),
            description: "Run a shell command in the workspace. Set run_in_background=true for long jobs; poll with TaskOutput.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                    "cwd": { "type": "string", "description": "Optional working directory relative to workspace" },
                    "run_in_background": {
                        "type": "boolean",
                        "description": "If true, start the command in the background and return a task_id immediately"
                    }
                },
                "required": ["command"]
            }),
        }
    }

    async fn execute(&self, arguments: &str, ctx: &ToolContext) -> Result<ToolResult> {
        let args: BashArgs = serde_json::from_str(arguments).context("parse Bash args")?;

        if args.run_in_background {
            return spawn_background_bash(ctx, &args.command, args.cwd.as_deref()).await;
        }

        let result = ctx
            .sandbox
            .exec(&args.command, args.cwd.as_deref(), ctx.cancel.as_ref())
            .await?;
        Ok(format_exec_result_for_command(&args.command, result))
    }
}

async fn spawn_background_bash(
    ctx: &ToolContext,
    command: &str,
    cwd: Option<&str>,
) -> Result<ToolResult> {
    let Some(store) = ctx.background.clone() else {
        return Ok(ToolResult {
            content: "Background tasks are not available in this context.".into(),
            is_error: true,
        });
    };

    let id = BackgroundTaskStore::alloc_id("bash");
    let cancel = CancellationToken::new();
    store.lock().insert_running(
        id.clone(),
        BackgroundTaskKind::Bash,
        command.to_string(),
        cancel.clone(),
    );

    let sandbox = Arc::clone(&ctx.sandbox);
    let cwd = cwd.map(str::to_string);
    let store_worker = Arc::clone(&store);
    let id_worker = id.clone();
    let command = command.to_string();

    tokio::spawn(async move {
        let result = exec_with_timeout(
            sandbox.as_ref(),
            &command,
            cwd.as_deref(),
            Some(&cancel),
            BACKGROUND_BASH_TIMEOUT_SECS,
        )
        .await;

        let mut guard = store_worker.lock();
        if cancel.is_cancelled()
            || guard
                .get(&id_worker)
                .is_some_and(|t| t.status == BackgroundTaskStatus::Cancelled)
        {
            return;
        }
        match result {
            Ok(exec) => {
                let content = format_exec_result_for_command(&command, exec.clone()).content;
                let status = if exec.exit_code == 0 {
                    BackgroundTaskStatus::Completed
                } else {
                    BackgroundTaskStatus::Failed
                };
                guard.finish(&id_worker, status, content, Some(exec.exit_code));
            }
            Err(err) => {
                let msg = err.to_string();
                let status = if msg.contains("aborted") {
                    BackgroundTaskStatus::Cancelled
                } else {
                    BackgroundTaskStatus::Failed
                };
                guard.finish(&id_worker, status, msg, None);
            }
        }
    });

    Ok(ToolResult {
        content: format!(
            "Started background bash task `{id}`.\nUse TaskOutput with task_id=\"{id}\" to poll output, or action=\"kill\" to cancel."
        ),
        is_error: false,
    })
}

async fn exec_with_timeout(
    sandbox: &dyn Sandbox,
    command: &str,
    cwd: Option<&str>,
    cancel: Option<&CancellationToken>,
    timeout_secs: u64,
) -> Result<zene_sandbox::ExecResult> {
    if cancel.is_some_and(CancellationToken::is_cancelled) {
        anyhow::bail!("aborted");
    }

    sandbox
        .exec_with_timeout(command, cwd, cancel, Duration::from_secs(timeout_secs))
        .await
}

fn format_exec_result_for_command(command: &str, result: zene_sandbox::ExecResult) -> ToolResult {
    let content = crate::output_sanitizer::OutputSanitizer::sanitize_exec_output(
        command,
        &result.stdout,
        &result.stderr,
        result.exit_code,
    );
    ToolResult {
        content,
        is_error: result.exit_code != 0,
    }
}
