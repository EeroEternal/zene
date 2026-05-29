use std::path::Path;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use zene_llm::ToolDefinition;

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
}

#[async_trait]
impl Tool for TaskTool {
    fn name(&self) -> &str {
        "Task"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "Task".to_string(),
            description: "Spawn a subagent with its own context to handle a focused subtask. Returns the subagent's final report.".to_string(),
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

        let cwd = args.cwd.as_deref().map(Path::new);
        match env
            .runner
            .run_subagent(&args.prompt, profile, cwd, ctx)
            .await
        {
            Ok(text) => Ok(ToolResult {
                content: text,
                is_error: false,
            }),
            Err(err) => Ok(ToolResult {
                content: err.to_string(),
                is_error: true,
            }),
        }
    }
}
