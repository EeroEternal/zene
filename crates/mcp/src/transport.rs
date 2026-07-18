//! Unified MCP client handle for stdio and HTTP transports.

use anyhow::Result;
use serde_json::Value;

use crate::client::{McpStdioClient, McpToolInfo};
use crate::http::McpHttpClient;

pub enum McpClientHandle {
    Stdio(McpStdioClient),
    Http(McpHttpClient),
}

impl McpClientHandle {
    pub fn server_name(&self) -> &str {
        match self {
            Self::Stdio(c) => c.server_name(),
            Self::Http(c) => c.server_name(),
        }
    }

    pub async fn list_tools(&mut self) -> Result<Vec<McpToolInfo>> {
        match self {
            Self::Stdio(c) => c.list_tools().await,
            Self::Http(c) => c.list_tools().await,
        }
    }

    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<(String, bool)> {
        match self {
            Self::Stdio(c) => c.call_tool(name, arguments).await,
            Self::Http(c) => c.call_tool(name, arguments).await,
        }
    }

    pub async fn disconnect(&mut self) -> Result<()> {
        match self {
            Self::Stdio(c) => c.disconnect().await,
            Self::Http(c) => c.disconnect().await,
        }
    }
}
