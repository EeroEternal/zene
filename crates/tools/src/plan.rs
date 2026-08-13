use async_trait::async_trait;
use serde_json::json;
use zene_llm::ToolDefinition;

use crate::registry::{Tool, ToolContext, ToolResult};

pub struct EnterPlanModeTool;

pub struct ExitPlanModeTool;

#[async_trait]
impl Tool for EnterPlanModeTool {
    fn name(&self) -> &str {
        "EnterPlanMode"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "EnterPlanMode".to_string(),
            description: "Enter plan mode: only read-only tools (Read, Grep, Glob, RepoMap, Skill) are available until the user approves your plan via ExitPlanMode.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "reason": {
                        "type": "string",
                        "description": "Why you are entering plan mode (optional)"
                    }
                }
            }),
        }
    }

    async fn execute(&self, _arguments: &str, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        Ok(ToolResult {
            content: "EnterPlanMode should be handled by the agent runtime.".to_string(),
            is_error: true,
        })
    }
}

#[async_trait]
impl Tool for ExitPlanModeTool {
    fn name(&self) -> &str {
        "ExitPlanMode"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "ExitPlanMode".to_string(),
            description: "Write the plan to disk and request user approval to exit plan mode. All write/edit/bash tools remain blocked until approved.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "plan": {
                        "type": "string",
                        "description": "The full plan markdown to save and present for approval"
                    }
                },
                "required": ["plan"]
            }),
        }
    }

    async fn execute(&self, _arguments: &str, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        Ok(ToolResult {
            content: "ExitPlanMode should be handled by the agent runtime.".to_string(),
            is_error: true,
        })
    }
}
