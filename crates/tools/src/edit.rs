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

#[derive(Debug, Clone, Copy)]
struct LineSpan {
    start: usize,
    end: usize,
}

fn find_whitespace_tolerant_match(
    content: &str,
    needle: &str,
) -> Result<Option<(usize, usize)>, usize> {
    let needle_lines: Vec<&str> = needle
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l).trim())
        .collect();

    if needle_lines.is_empty() || (needle_lines.len() == 1 && needle_lines[0].is_empty()) {
        return Ok(None);
    }

    let mut file_lines: Vec<(&str, LineSpan)> = Vec::new();
    let mut offset = 0;
    for raw_line in content.split_inclusive('\n') {
        let stripped = raw_line
            .strip_suffix("\r\n")
            .or_else(|| raw_line.strip_suffix('\n'))
            .unwrap_or(raw_line);
        file_lines.push((
            stripped.trim(),
            LineSpan {
                start: offset,
                end: offset + stripped.len(),
            },
        ));
        offset += raw_line.len();
    }

    if file_lines.len() < needle_lines.len() {
        return Ok(None);
    }

    let mut matched_spans = Vec::new();
    let window_size = needle_lines.len();

    for i in 0..=file_lines.len() - window_size {
        let mut all_match = true;
        for j in 0..window_size {
            if file_lines[i + j].0 != needle_lines[j] {
                all_match = false;
                break;
            }
        }
        if all_match {
            let start = file_lines[i].1.start;
            let end = file_lines[i + window_size - 1].1.end;
            matched_spans.push((start, end));
        }
    }

    match matched_spans.len() {
        0 => Ok(None),
        1 => Ok(Some(matched_spans[0])),
        n => Err(n),
    }
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
                // Fallback to whitespace-tolerant matching to avoid roundtrip failures
                // caused by minor indentation or trailing space discrepancies.
                match find_whitespace_tolerant_match(content, &args.old_string) {
                    Ok(Some((start, end))) => {
                        let mut new_content = String::with_capacity(
                            content.len().saturating_sub(end - start) + args.new_string.len(),
                        );
                        new_content.push_str(&content[..start]);
                        new_content.push_str(&args.new_string);
                        new_content.push_str(&content[end..]);

                        let materialized =
                            materialize_model_text(&new_content, model_view.line_ending_style);
                        ctx.sandbox.write_text(&args.path, &materialized).await?;
                        return Ok(ToolResult {
                            content: format!(
                                "Replaced 1 occurrence (matched via whitespace-tolerant alignment) in {}",
                                args.path
                            ),
                            is_error: false,
                        });
                    }
                    Err(fuzzy_count) => {
                        return Ok(ToolResult {
                            content: format!(
                                "old_string not found exactly, and matched {} locations with normalized whitespace in {}. Please provide more surrounding lines in old_string to disambiguate.\n",
                                fuzzy_count, args.path
                            ),
                            is_error: true,
                        });
                    }
                    Ok(None) => {
                        return Ok(ToolResult {
                            content: format!(
                                "old_string not found in {}, The file contents may be out of date. Please use the Read Tool to reload the content.\n",
                                args.path
                            ),
                            is_error: true,
                        });
                    }
                }
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
