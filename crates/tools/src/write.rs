use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use zene_llm::ToolDefinition;

use crate::line_endings::{detect_line_ending_style, materialize_model_text};
use crate::registry::{Tool, ToolContext, ToolResult};

pub struct WriteTool;

#[derive(Debug, Deserialize)]
struct WriteArgs {
    path: String,
    content: String,
}

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "Write"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "Write".to_string(),
            description: "Write content to a file, creating parent directories if needed."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn execute(&self, arguments: &str, ctx: &ToolContext) -> Result<ToolResult> {
        let args: WriteArgs = serde_json::from_str(arguments).context("parse Write args")?;

        let content = if let Ok(existing) = ctx.sandbox.read_text(&args.path).await {
            let style = detect_line_ending_style(&existing);
            materialize_model_text(&args.content, style)
        } else {
            args.content.clone()
        };

        ctx.sandbox.write_text(&args.path, &content).await?;
        Ok(ToolResult {
            content: format!("Wrote {} bytes to {}", content.len(), args.path),
            is_error: false,
        })
    }
}
