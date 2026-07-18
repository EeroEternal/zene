use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use jsonschema::Validator;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use zene_llm::ToolDefinition;
use zene_sandbox::LocalSandbox;

use crate::ask_user::SharedAskUserPrompter;
use crate::background::SharedBackgroundTasks;
use crate::permission::SharedToolPermission;
use crate::plan_mode::SharedPlanMode;
use crate::subagent::SubagentEnv;
use crate::todo_store::SharedTodoStore;

pub struct ToolContext {
    pub sandbox: Arc<LocalSandbox>,
    pub cancel: Option<CancellationToken>,
    pub subagent: Option<SubagentEnv>,
    pub permission: Option<SharedToolPermission>,
    pub plan_mode: Option<SharedPlanMode>,
    pub todos: Option<SharedTodoStore>,
    pub ask_user: Option<SharedAskUserPrompter>,
    pub background: Option<SharedBackgroundTasks>,
}

impl ToolContext {
    pub fn without_subagent(sandbox: Arc<LocalSandbox>) -> Self {
        Self {
            sandbox,
            cancel: None,
            subagent: None,
            permission: None,
            plan_mode: None,
            todos: None,
            ask_user: None,
            background: None,
        }
    }
}

impl ToolRegistry {
    pub fn filter_definitions<F>(&self, mut keep: F) -> Vec<ToolDefinition>
    where
        F: FnMut(&str) -> bool,
    {
        self.tools
            .iter()
            .filter(|tool| keep(tool.name()))
            .map(|tool| tool.definition())
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn definition(&self) -> ToolDefinition;
    async fn execute(&self, arguments: &str, ctx: &ToolContext) -> Result<ToolResult>;
}

pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new(tools: Vec<Box<dyn Tool>>) -> Self {
        Self { tools }
    }

    pub fn extend(&mut self, other: Self) {
        self.tools.extend(other.tools);
    }

    pub fn merge(mut base: Self, other: Self) -> Self {
        base.extend(other);
        base
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.iter().map(|tool| tool.definition()).collect()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.tools.iter().any(|tool| tool.name() == name)
    }

    pub async fn execute(&self, name: &str, arguments: &str, ctx: &ToolContext) -> Result<ToolResult> {
        let tool = self
            .tools
            .iter()
            .find(|tool| tool.name() == name)
            .with_context(|| format!("unknown tool: {name}"))?;

        let definition = tool.definition();
        if let Some(result) = validate_tool_arguments(&definition, arguments) {
            return Ok(result);
        }

        tool.execute(arguments, ctx).await
    }
}

fn validate_tool_arguments(definition: &ToolDefinition, arguments: &str) -> Option<ToolResult> {
    let parsed: Value = match serde_json::from_str(arguments) {
        Ok(value) => value,
        Err(err) => {
            return Some(ToolResult {
                content: format!(
                    "Invalid JSON arguments for tool `{}`: {err}",
                    definition.name
                ),
                is_error: true,
            });
        }
    };

    let validator = match Validator::new(&definition.parameters) {
        Ok(validator) => validator,
        Err(err) => {
            return Some(ToolResult {
                content: format!(
                    "Internal schema error for tool `{}`: {err}",
                    definition.name
                ),
                is_error: true,
            });
        }
    };

    let messages: Vec<String> = validator
        .iter_errors(&parsed)
        .map(|err| err.to_string())
        .collect();
    if !messages.is_empty() {
        return Some(ToolResult {
            content: format!(
                "Tool `{}` argument validation failed: {}",
                definition.name,
                messages.join("; ")
            ),
            is_error: true,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejects_invalid_json_arguments() {
        let definition = ToolDefinition {
            name: "Test".to_string(),
            description: "test".to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
        };
        let result = validate_tool_arguments(&definition, "not json").unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid JSON"));
    }

    #[test]
    fn rejects_schema_violation() {
        let definition = ToolDefinition {
            name: "Test".to_string(),
            description: "test".to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
        };
        let result = validate_tool_arguments(&definition, "{}").unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("validation failed"));
    }

    #[test]
    fn accepts_valid_arguments() {
        let definition = ToolDefinition {
            name: "Test".to_string(),
            description: "test".to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
        };
        assert!(validate_tool_arguments(&definition, r#"{"path":"foo.rs"}"#).is_none());
    }
}
