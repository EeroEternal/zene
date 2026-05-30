use std::io::{self, Write};
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use zene_llm::ToolDefinition;

use crate::registry::{Tool, ToolContext, ToolResult};

#[derive(Debug, Clone, Deserialize)]
pub struct AskUserOption {
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AskUserArgs {
    question: String,
    #[serde(default)]
    options: Option<Vec<AskUserOption>>,
}

pub type AskUserPrompter =
    dyn Fn(&str, Option<&[AskUserOption]>) -> io::Result<String> + Send + Sync;

pub type SharedAskUserPrompter = Arc<AskUserPrompter>;

pub fn default_ask_user_prompter() -> SharedAskUserPrompter {
    Arc::new(stdin_ask_user_prompter)
}

pub fn stdin_ask_user_prompter(
    question: &str,
    options: Option<&[AskUserOption]>,
) -> io::Result<String> {
    eprintln!("\n{question}");
    if let Some(opts) = options.filter(|o| !o.is_empty()) {
        for (idx, opt) in opts.iter().enumerate() {
            if let Some(desc) = opt.description.as_deref().filter(|d| !d.is_empty()) {
                eprintln!("  {}. {} — {}", idx + 1, opt.label, desc);
            } else {
                eprintln!("  {}. {}", idx + 1, opt.label);
            }
        }
        eprint!("Choose 1-{} or type a free-text answer: ", opts.len());
    } else {
        eprint!("Your answer: ");
    }
    let _ = io::stderr().flush();
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let trimmed = line.trim();
    if let Some(opts) = options.filter(|o| !o.is_empty()) {
        if let Ok(num) = trimmed.parse::<usize>() {
            if num >= 1 && num <= opts.len() {
                return Ok(opts[num - 1].label.clone());
            }
        }
    }
    Ok(trimmed.to_string())
}

pub struct AskUserQuestionTool;

#[async_trait]
impl Tool for AskUserQuestionTool {
    fn name(&self) -> &str {
        "AskUserQuestion"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "AskUserQuestion".to_string(),
            description: "Ask the user a structured question when you need a preference, disambiguation, or confirmation. Returns the user's answer.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "question": {
                        "type": "string",
                        "description": "The question to ask the user."
                    },
                    "options": {
                        "type": "array",
                        "description": "Optional numbered choices. User may pick one or type free text.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "label": { "type": "string", "description": "Short option label" },
                                "description": { "type": "string", "description": "Optional explanation" }
                            },
                            "required": ["label"]
                        }
                    }
                },
                "required": ["question"]
            }),
        }
    }

    async fn execute(&self, arguments: &str, ctx: &ToolContext) -> Result<ToolResult> {
        let args: AskUserArgs = serde_json::from_str(arguments).context("parse AskUserQuestion args")?;
        if args.question.trim().is_empty() {
            return Ok(ToolResult {
                content: "AskUserQuestion requires a non-empty `question`.".to_string(),
                is_error: true,
            });
        }

        let prompter = ctx
            .ask_user
            .as_ref()
            .cloned()
            .unwrap_or_else(default_ask_user_prompter);

        match prompter(&args.question, args.options.as_deref()) {
            Ok(answer) => {
                if answer.trim().is_empty() {
                    Ok(ToolResult {
                        content: "User dismissed the question without answering.".to_string(),
                        is_error: false,
                    })
                } else {
                    Ok(ToolResult {
                        content: answer,
                        is_error: false,
                    })
                }
            }
            Err(err) => Ok(ToolResult {
                content: format!("Failed to prompt user: {err}"),
                is_error: true,
            }),
        }
    }
}
