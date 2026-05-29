use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use zene_llm::ToolDefinition;

use crate::registry::{Tool, ToolContext, ToolResult};

pub struct GlobTool;

#[derive(Debug, Deserialize)]
struct GlobArgs {
    pattern: String,
}

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "Glob"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "Glob".to_string(),
            description: "Find files matching a glob pattern (e.g. **/*.rs).".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string" }
                },
                "required": ["pattern"]
            }),
        }
    }

    async fn execute(&self, arguments: &str, ctx: &ToolContext) -> Result<ToolResult> {
        let args: GlobArgs = serde_json::from_str(arguments).context("parse Glob args")?;
        let matches = ctx.sandbox.glob(&args.pattern)?;
        Ok(ToolResult {
            content: if matches.is_empty() {
                "No files matched.".to_string()
            } else {
                matches.join("\n")
            },
            is_error: false,
        })
    }
}
