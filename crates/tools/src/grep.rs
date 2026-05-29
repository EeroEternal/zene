use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use zene_llm::ToolDefinition;

use crate::registry::{Tool, ToolContext, ToolResult};

pub struct GrepTool;

#[derive(Debug, Deserialize)]
struct GrepArgs {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    case_insensitive: bool,
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "Grep"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "Grep".to_string(),
            description: "Search for a regex pattern in workspace files.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string" },
                    "path": { "type": "string" },
                    "case_insensitive": { "type": "boolean" }
                },
                "required": ["pattern"]
            }),
        }
    }

    async fn execute(&self, arguments: &str, ctx: &ToolContext) -> Result<ToolResult> {
        let args: GrepArgs = serde_json::from_str(arguments).context("parse Grep args")?;
        let matches = ctx
            .sandbox
            .grep(&args.pattern, args.path.as_deref(), args.case_insensitive)
            .await?;
        if matches.is_empty() {
            return Ok(ToolResult {
                content: "No matches found.".to_string(),
                is_error: false,
            });
        }
        let content = matches
            .into_iter()
            .map(|m| format!("{}:{}:{}", m.path, m.line_number, m.line))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(ToolResult {
            content,
            is_error: false,
        })
    }
}
