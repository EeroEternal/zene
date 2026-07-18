use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use zene_llm::ToolDefinition;

use crate::registry::{Tool, ToolContext, ToolResult};
use crate::todo_store::{TodoItem, TodoStatus};

#[derive(Debug, Deserialize)]
struct TodoWriteArgs {
    todos: Vec<TodoUpdate>,
    #[serde(default)]
    merge: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct TodoUpdate {
    id: String,
    content: String,
    status: TodoStatus,
}

pub struct TodoWriteTool;

#[async_trait]
impl Tool for TodoWriteTool {
    fn name(&self) -> &str {
        "TodoWrite"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "TodoWrite".to_string(),
            description: "Create or update todo items by id. Merges into the session todo list and returns the current summary.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "todos": {
                        "type": "array",
                        "description": "Todo items to create or update.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string", "description": "Stable todo id" },
                                "content": { "type": "string", "description": "Todo description" },
                                "status": {
                                    "type": "string",
                                    "enum": ["pending", "in_progress", "completed"],
                                    "description": "Current status"
                                }
                            },
                            "required": ["id", "content", "status"]
                        }
                    },
                    "merge": {
                        "type": "boolean",
                        "description": "When true (default), merge by id. When false, replace the entire list."
                    }
                },
                "required": ["todos"]
            }),
        }
    }

    async fn execute(&self, arguments: &str, ctx: &ToolContext) -> Result<ToolResult> {
        let args: TodoWriteArgs = serde_json::from_str(arguments).context("parse TodoWrite args")?;
        let Some(store) = ctx.todos.as_ref() else {
            return Ok(ToolResult {
                content: "Todo store unavailable.".to_string(),
                is_error: true,
            });
        };

        let updates: Vec<TodoItem> = args
            .todos
            .into_iter()
            .map(|t| TodoItem {
                id: t.id,
                content: t.content,
                status: t.status,
            })
            .collect();

        let summary = {
            let mut store = store.lock();
            if args.merge == Some(false) {
                *store = crate::todo_store::TodoStore::default();
            }
            store.merge(&updates);
            store.render_summary()
        };

        Ok(ToolResult {
            content: summary,
            is_error: false,
        })
    }
}

pub struct TodoListTool;

#[async_trait]
impl Tool for TodoListTool {
    fn name(&self) -> &str {
        "TodoList"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "TodoList".to_string(),
            description: "Read the current session todo list without making changes.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {}
            }),
        }
    }

    async fn execute(&self, _arguments: &str, ctx: &ToolContext) -> Result<ToolResult> {
        let Some(store) = ctx.todos.as_ref() else {
            return Ok(ToolResult {
                content: "Todo store unavailable.".to_string(),
                is_error: true,
            });
        };

        let summary = {
            let store = store.lock();
            store.render_summary()
        };

        Ok(ToolResult {
            content: summary,
            is_error: false,
        })
    }
}
