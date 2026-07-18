use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use zene_llm::ToolDefinition;
use zene_tools::{Tool, ToolContext, ToolResult};

use crate::client::{McpToolInfo, mcp_tool_registry_name};
use crate::transport::McpClientHandle;

pub struct McpTool {
    server_name: String,
    tool_name: String,
    registry_name: String,
    description: String,
    input_schema: Value,
    client: Arc<Mutex<McpClientHandle>>,
}

impl McpTool {
    pub fn from_info(
        server_name: &str,
        info: McpToolInfo,
        client: Arc<Mutex<McpClientHandle>>,
    ) -> Self {
        let registry_name = mcp_tool_registry_name(server_name, &info.name);
        Self {
            server_name: server_name.to_string(),
            tool_name: info.name,
            registry_name,
            description: info.description,
            input_schema: normalize_input_schema(info.input_schema),
            client,
        }
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.registry_name
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.registry_name.clone(),
            description: format!("[MCP:{}] {}", self.server_name, self.description),
            parameters: self.input_schema.clone(),
        }
    }

    async fn execute(&self, arguments: &str, ctx: &ToolContext) -> Result<ToolResult> {
        let args: Value = serde_json::from_str(arguments).unwrap_or(json!({}));
        let mut client = self.client.lock().await;
        let (content, is_error) = client.call_tool(&self.tool_name, args).await?;
        let content =
            crate::truncate::truncate_mcp_output(content, ctx.sandbox.workdir(), &self.registry_name);
        Ok(ToolResult { content, is_error })
    }
}

fn normalize_input_schema(schema: Value) -> Value {
    if schema.is_null() || !schema.is_object() {
        return json!({
            "type": "object",
            "properties": {}
        });
    }
    schema
}
