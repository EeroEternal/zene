use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use zene_llm::ToolDefinition;

use crate::line_endings::{materialize_model_text, to_model_text_view};
use crate::registry::{Tool, ToolContext, ToolResult};

pub struct EditTool;

#[derive(Debug, Deserialize)]
struct EditArgs {
    path: String,
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
}

fn count_occurrences(content: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    let mut count = 0;
    let mut pos = 0;
    while pos < content.len() {
        if let Some(idx) = content[pos..].find(needle) {
            count += 1;
            pos += idx + needle.len();
        } else {
            break;
        }
    }
    count
}

fn replace_once_literal(content: &str, old_string: &str, new_string: &str) -> String {
    let Some(index) = content.find(old_string) else {
        return content.to_string();
    };
    let mut result =
        String::with_capacity(content.len().saturating_sub(old_string.len()) + new_string.len());
    result.push_str(&content[..index]);
    result.push_str(new_string);
    result.push_str(&content[index + old_string.len()..]);
    result
}

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "Edit"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "Edit".to_string(),
            description: "Replace text in a file.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "old_string": { "type": "string" },
                    "new_string": { "type": "string" },
                    "replace_all": { "type": "boolean" }
                },
                "required": ["path", "old_string", "new_string"]
            }),
        }
    }

    async fn execute(&self, arguments: &str, ctx: &ToolContext) -> Result<ToolResult> {
        let args: EditArgs = serde_json::from_str(arguments).context("parse Edit args")?;

        if args.old_string.is_empty() {
            return Ok(ToolResult {
                content: "old_string must not be empty.".to_string(),
                is_error: true,
            });
        }

        if args.old_string == args.new_string {
            return Ok(ToolResult {
                content: "No changes to make: old_string and new_string are exactly the same."
                    .to_string(),
                is_error: true,
            });
        }

        let raw = ctx.sandbox.read_text(&args.path).await?;
        let model_view = to_model_text_view(&raw);
        let content = &model_view.text;

        if !args.replace_all {
            let count = count_occurrences(content, &args.old_string);
            if count == 0 {
                return Ok(ToolResult {
                    content: format!(
                        "old_string not found in {}, The file contents may be out of date. Please use the Read Tool to reload the content.\n",
                        args.path
                    ),
                    is_error: true,
                });
            }
            if count > 1 {
                return Ok(ToolResult {
                    content: format!(
                        "old_string is not unique in {} (found {} occurrences). To replace every occurrence, set replace_all=true. To replace only one occurrence, include more surrounding context in old_string.",
                        args.path, count
                    ),
                    is_error: true,
                });
            }

            let new_content = replace_once_literal(content, &args.old_string, &args.new_string);
            let materialized = materialize_model_text(&new_content, model_view.line_ending_style);
            ctx.sandbox.write_text(&args.path, &materialized).await?;
            return Ok(ToolResult {
                content: format!("Replaced 1 occurrence in {}", args.path),
                is_error: false,
            });
        }

        let parts: Vec<&str> = content.split(&args.old_string).collect();
        let replacement_count = parts.len().saturating_sub(1);
        if replacement_count == 0 {
            return Ok(ToolResult {
                content: format!(
                    "old_string not found in {}, The file contents may be out of date. Please use the Read Tool to reload the content.\n",
                    args.path
                ),
                is_error: true,
            });
        }

        let new_content = parts.join(&args.new_string);
        let materialized = materialize_model_text(&new_content, model_view.line_ending_style);
        ctx.sandbox.write_text(&args.path, &materialized).await?;
        Ok(ToolResult {
            content: format!(
                "Replaced {} occurrences in {}",
                replacement_count, args.path
            ),
            is_error: false,
        })
    }
}
