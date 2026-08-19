use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use zene_llm::ToolDefinition;

use crate::background::{BackgroundTaskKind, BackgroundTaskStatus, BackgroundTaskStore};
use crate::registry::{Tool, ToolContext, ToolResult};
use crate::subagent::SubagentProfile;

pub struct TaskTool;

#[derive(Debug, Deserialize)]
struct TaskArgs {
    prompt: String,
    #[serde(default)]
    agent: Option<SubagentProfile>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    run_in_background: bool,
}

#[async_trait]
impl Tool for TaskTool {
    fn name(&self) -> &str {
        "Task"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "Task".to_string(),
            description: "Spawn a subagent with its own context to handle a focused subtask. Returns the subagent's final report. Set run_in_background=true and poll with TaskOutput.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "Task description for the subagent"
                    },
                    "agent": {
                        "type": "string",
                        "enum": ["explore", "coder"],
                        "description": "Subagent profile: explore (read-only) or coder (read/write). Defaults to explore."
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Optional working directory relative to the workspace"
                    },
                    "run_in_background": {
                        "type": "boolean",
                        "description": "If true, start the subagent in the background and return a task_id immediately"
                    }
                },
                "required": ["prompt"]
            }),
        }
    }

    async fn execute(&self, arguments: &str, ctx: &ToolContext) -> Result<ToolResult> {
        let args: TaskArgs = serde_json::from_str(arguments).context("parse Task args")?;
        let profile = args.agent.unwrap_or_default();

        let Some(env) = ctx.subagent.as_ref() else {
            return Ok(ToolResult {
                content: "Task tool is not available in this context.".to_string(),
                is_error: true,
            });
        };

        if env.depth >= env.max_depth {
            return Ok(ToolResult {
                content: format!(
                    "Subagent nesting limit reached (max depth {}). Task cannot spawn another subagent.",
                    env.max_depth
                ),
                is_error: true,
            });
        }

        if args.run_in_background {
            return spawn_background_task(ctx, &args.prompt, profile, args.cwd.as_deref()).await;
        }

        let cwd = args.cwd.as_deref().map(Path::new);
        match env
            .runner
            .run_subagent(&args.prompt, profile, cwd, ctx)
            .await
        {
            Ok(text) => Ok(ToolResult {
                content: format_subagent_report(profile, &text),
                is_error: false,
            }),
            Err(err) => Ok(ToolResult {
                content: err.to_string(),
                is_error: true,
            }),
        }
    }
}

async fn spawn_background_task(
    ctx: &ToolContext,
    prompt: &str,
    profile: SubagentProfile,
    cwd: Option<&str>,
) -> Result<ToolResult> {
    let Some(store) = ctx.background.clone() else {
        return Ok(ToolResult {
            content: "Background tasks are not available in this context.".into(),
            is_error: true,
        });
    };
    let Some(env) = ctx.subagent.clone() else {
        return Ok(ToolResult {
            content: "Task tool is not available in this context.".into(),
            is_error: true,
        });
    };

    let id = BackgroundTaskStore::alloc_id("task");
    let cancel = CancellationToken::new();
    let label = format!("[{profile:?}] {}", truncate(prompt, 120));
    store.lock().insert_running(
        id.clone(),
        BackgroundTaskKind::Subagent,
        label,
        cancel.clone(),
    );

    let prompt = prompt.to_string();
    let cwd_owned = cwd.map(str::to_string);
    let parent_ctx = ToolContext {
        sandbox: Arc::clone(&ctx.sandbox),
        cancel: Some(cancel.clone()),
        subagent: Some(env.clone()),
        permission: ctx.permission.clone(),
        plan_mode: None,
        todos: None,
        ask_user: None,
        background: None,
    };
    let store_worker = Arc::clone(&store);
    let id_worker = id.clone();

    tokio::spawn(async move {
        let cwd = cwd_owned.as_deref().map(Path::new);
        let result = env
            .runner
            .run_subagent(&prompt, profile, cwd, &parent_ctx)
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
            Ok(text) => {
                let report = format_subagent_report(profile, &text);
                guard.finish(&id_worker, BackgroundTaskStatus::Completed, report, Some(0));
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
            "Started background subagent task `{id}` (profile={profile:?}).\nUse TaskOutput with task_id=\"{id}\" to poll the report, or action=\"kill\" to cancel."
        ),
        is_error: false,
    })
}

fn format_subagent_report(profile: SubagentProfile, text: &str) -> String {
    format!("<subagent-report profile=\"{profile:?}\">\n{text}\n</subagent-report>")
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}
