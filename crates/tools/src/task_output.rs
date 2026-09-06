use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use zene_llm::ToolDefinition;

use crate::background::{BackgroundTaskKind, BackgroundTaskStatus};
use crate::registry::{Tool, ToolContext, ToolResult};

pub struct TaskOutputTool;

#[derive(Debug, Deserialize)]
struct TaskOutputArgs {
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    wait: Option<bool>,
    #[serde(default)]
    timeout_secs: Option<u64>,
}

#[async_trait]
impl Tool for TaskOutputTool {
    fn name(&self) -> &str {
        "TaskOutput"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "TaskOutput".to_string(),
            description: "Inspect or cancel background Bash/Task jobs. Use action=list, get (default), or kill. When wait=true, waits for completion up to 1 hour (3600s).".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "Background task id returned by Bash/Task when run_in_background=true"
                    },
                    "action": {
                        "type": "string",
                        "enum": ["list", "get", "kill"],
                        "description": "list all tasks, get one task's output, or kill a running task"
                    },
                    "wait": {
                        "type": "boolean",
                        "description": "Wait for the task to finish before returning output (capped at 3600s)"
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "Max seconds to wait if wait=true (default 3600, capped at 3600)"
                    }
                },
                "required": []
            }),
        }
    }

    async fn execute(&self, arguments: &str, ctx: &ToolContext) -> Result<ToolResult> {
        let args: TaskOutputArgs =
            serde_json::from_str(arguments).context("parse TaskOutput args")?;
        let Some(store) = ctx.background.as_ref() else {
            return Ok(ToolResult {
                content: "Background tasks are not available in this context.".into(),
                is_error: true,
            });
        };

        let action = args
            .action
            .as_deref()
            .unwrap_or(if args.task_id.is_some() {
                "get"
            } else {
                "list"
            })
            .to_ascii_lowercase();

        match action.as_str() {
            "list" => {
                let tasks = store.lock().list();
                if tasks.is_empty() {
                    return Ok(ToolResult {
                        content: "No background tasks.".into(),
                        is_error: false,
                    });
                }
                let mut lines = Vec::new();
                for task in tasks {
                    let kind = match task.kind {
                        BackgroundTaskKind::Bash => "bash",
                        BackgroundTaskKind::Subagent => "task",
                    };
                    lines.push(format!(
                        "{} [{}] {} — {}",
                        task.id,
                        task.status.as_str(),
                        kind,
                        truncate(&task.label, 80)
                    ));
                }
                Ok(ToolResult {
                    content: lines.join("\n"),
                    is_error: false,
                })
            }
            "get" => {
                let Some(id) = args.task_id.as_deref() else {
                    return Ok(ToolResult {
                        content: "task_id is required for action=get".into(),
                        is_error: true,
                    });
                };
                let should_wait = args.wait.unwrap_or(false);
                let timeout_secs = args.timeout_secs.unwrap_or(3600).min(3600);
                if should_wait {
                    let deadline =
                        tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
                    loop {
                        let is_running = {
                            let guard = store.lock();
                            let Some(task) = guard.get(id) else {
                                return Ok(ToolResult {
                                    content: format!("Unknown task_id `{id}`"),
                                    is_error: true,
                                });
                            };
                            task.status == BackgroundTaskStatus::Running
                        };
                        if !is_running || tokio::time::Instant::now() >= deadline {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    }
                }

                let Some(task) = store.lock().get(id) else {
                    return Ok(ToolResult {
                        content: format!("Unknown task_id `{id}`"),
                        is_error: true,
                    });
                };
                let mut content = format!(
                    "task_id: {}\nstatus: {}\nlabel: {}\n",
                    task.id,
                    task.status.as_str(),
                    task.label
                );
                if let Some(code) = task.exit_code {
                    content.push_str(&format!("exit_code: {code}\n"));
                }
                content.push_str("--- output ---\n");
                content.push_str(if task.output.is_empty() {
                    "(no output yet)"
                } else {
                    &task.output
                });
                Ok(ToolResult {
                    content,
                    is_error: task.status == BackgroundTaskStatus::Failed,
                })
            }
            "kill" => {
                let Some(id) = args.task_id.as_deref() else {
                    return Ok(ToolResult {
                        content: "task_id is required for action=kill".into(),
                        is_error: true,
                    });
                };
                let mut guard = store.lock();
                let Some(task) = guard.get(id) else {
                    return Ok(ToolResult {
                        content: format!("Unknown task_id `{id}`"),
                        is_error: true,
                    });
                };
                if task.status != BackgroundTaskStatus::Running {
                    return Ok(ToolResult {
                        content: format!("Task `{id}` is already {}.", task.status.as_str()),
                        is_error: false,
                    });
                }
                let _ = guard.request_cancel(id);
                Ok(ToolResult {
                    content: format!("Cancelled task `{id}`."),
                    is_error: false,
                })
            }
            other => Ok(ToolResult {
                content: format!("Unknown action `{other}`; use list|get|kill"),
                is_error: true,
            }),
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background::BackgroundTaskStore;
    use parking_lot::Mutex;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;
    use zene_sandbox::LocalSandbox;

    #[tokio::test]
    async fn test_task_output_wait_and_get() {
        let tool = TaskOutputTool;
        let mut store = BackgroundTaskStore::new();
        let cancel = CancellationToken::new();
        store.insert_running(
            "test-1".into(),
            BackgroundTaskKind::Bash,
            "sleep".into(),
            cancel,
        );
        store.finish(
            "test-1",
            BackgroundTaskStatus::Completed,
            "completed output".into(),
            Some(0),
        );

        let shared_store = Arc::new(Mutex::new(store));
        let sandbox = Arc::new(LocalSandbox::new("."));
        let mut ctx = ToolContext::without_subagent(sandbox);
        ctx.background = Some(shared_store);

        let args = json!({
            "task_id": "test-1",
            "action": "get",
            "wait": true,
            "timeout_secs": 10
        })
        .to_string();

        let result = tool.execute(&args, &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("completed output"));
        assert!(result.content.contains("exit_code: 0"));
    }
}
