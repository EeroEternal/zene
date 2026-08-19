use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use zene_index::DEFAULT_TOKEN_BUDGET;
use zene_llm::ToolDefinition;

use crate::registry::{Tool, ToolContext, ToolResult};

pub struct RepoMapTool;

#[derive(Debug, Deserialize)]
struct RepoMapArgs {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    token_budget: Option<u32>,
    #[serde(default)]
    path: Option<String>,
}

#[async_trait]
impl Tool for RepoMapTool {
    fn name(&self) -> &str {
        "RepoMap"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "RepoMap".to_string(),
            description: "Return a compact repository structure map (important signatures and definition names, not implementations). Use this to see what exists and where, then Read/Grep for the actual code. Optional query personalizes ranking to the current task. Output is a tool result in the conversation body, not a prompt prefix.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Optional task or symbol names to rank related files higher"
                    },
                    "path": {
                        "type": "string",
                        "description": "Optional subdirectory to show (index still covers the workspace)"
                    },
                    "token_budget": {
                        "type": "integer",
                        "description": "Approximate token budget for the map (default 2500)"
                    }
                }
            }),
        }
    }

    async fn execute(&self, arguments: &str, ctx: &ToolContext) -> Result<ToolResult> {
        let args: RepoMapArgs = serde_json::from_str(arguments).context("parse RepoMap args")?;
        let workdir = ctx.sandbox.workdir().to_path_buf();
        let query = args.query.filter(|q| !q.trim().is_empty());
        let path = args.path.filter(|p| !p.trim().is_empty());
        let budget = args.token_budget.unwrap_or(DEFAULT_TOKEN_BUDGET);
        let map = tokio::task::spawn_blocking(move || {
            zene_index::build_repo_map(&workdir, query.as_deref(), budget, path.as_deref())
        })
        .await
        .context("RepoMap worker")?
        .context("build repo map")?;
        Ok(ToolResult {
            content: map,
            is_error: false,
        })
    }
}
