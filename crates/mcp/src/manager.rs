use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;
use tracing::{info, warn};
use zene_sandbox::LocalSandbox;
use zene_tools::ToolRegistry;

use crate::client::McpStdioClient;
use crate::config::{load_mcp_config, McpConfig};
use crate::http::McpHttpClient;
use crate::tool::McpTool;
use crate::transport::McpClientHandle;

pub struct McpManager {
    clients: Vec<Arc<Mutex<McpClientHandle>>>,
}

impl McpManager {
    pub async fn connect(workdir: &Path) -> Result<(Self, ToolRegistry)> {
        let config = load_mcp_config(workdir)?;
        Self::from_config(config).await
    }

    pub async fn connect_with_sandbox(
        workdir: &Path,
        sandbox: &LocalSandbox,
    ) -> Result<(Self, ToolRegistry)> {
        let config = load_mcp_config(workdir)?;
        Self::from_config_inner(config, Some(sandbox)).await
    }

    pub async fn from_config(config: McpConfig) -> Result<(Self, ToolRegistry)> {
        Self::from_config_inner(config, None).await
    }

    async fn from_config_inner(
        config: McpConfig,
        sandbox: Option<&LocalSandbox>,
    ) -> Result<(Self, ToolRegistry)> {
        let mut clients = Vec::new();
        let mut tool_boxes: Vec<Box<dyn zene_tools::Tool>> = Vec::new();

        for (server_name, server_config) in config.servers {
            if let Err(err) = server_config.validate(&server_name) {
                warn!(server = %server_name, error = %err, "mcp config invalid");
                continue;
            }

            let connected = if server_config.is_http() {
                let url = server_config.url.as_deref().unwrap();
                if let Some(sandbox) = sandbox {
                    if let Err(err) = sandbox.authorize_egress(url).await {
                        warn!(
                            server = %server_name,
                            error = %err,
                            "mcp http egress denied by sandbox"
                        );
                        continue;
                    }
                }
                McpHttpClient::connect(&server_name, url, &server_config.headers)
                    .await
                    .map(McpClientHandle::Http)
            } else {
                McpStdioClient::connect(&server_name, &server_config, sandbox)
                    .await
                    .map(Box::new)
                    .map(McpClientHandle::Stdio)
            };

            match connected {
                Ok(mut client) => {
                    let tools = match client.list_tools().await {
                        Ok(tools) => tools,
                        Err(err) => {
                            warn!(server = %server_name, error = %err, "mcp list_tools failed");
                            let _ = client.disconnect().await;
                            continue;
                        }
                    };

                    let client = Arc::new(Mutex::new(client));
                    clients.push(Arc::clone(&client));
                    let registered = tools.len();
                    for tool in tools {
                        tool_boxes.push(Box::new(McpTool::from_info(
                            &server_name,
                            tool,
                            Arc::clone(&client),
                        )));
                    }
                    info!(
                        server = %server_name,
                        tool_count = registered,
                        transport = if server_config.is_http() { "http" } else { "stdio" },
                        "mcp connected"
                    );
                }
                Err(err) => {
                    warn!(server = %server_name, error = %err, "mcp connect failed");
                }
            }
        }

        Ok((Self { clients }, ToolRegistry::new(tool_boxes)))
    }

    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    pub async fn disconnect(&mut self) -> Result<()> {
        for client in self.clients.drain(..) {
            let mut guard = client.lock().await;
            if let Err(err) = guard.disconnect().await {
                warn!(server = %guard.server_name(), error = %err, "mcp disconnect failed");
            }
        }
        Ok(())
    }
}

impl Drop for McpManager {
    fn drop(&mut self) {
        self.clients.clear();
    }
}
