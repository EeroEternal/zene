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
}

#[async_trait]
impl Tool for TaskOutputTool {
    fn name(&self) -> &str {
        "TaskOutput"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "TaskOutput".to_string(),
            description: "Inspect or cancel background Bash/Task jobs. Use action=list, get (default), or kill.".to_string(),
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
                        content: format!(
                            "Task `{id}` is already {}.",
                            task.status.as_str()
                        ),
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
