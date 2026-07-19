use std::process::Stdio;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tracing::debug;
use zene_sandbox::{InteractiveProcess, LocalSandbox};

use crate::config::McpServerConfig;

const PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolInfo {
    pub name: String,
    pub description: String,
    #[serde(default, rename = "inputSchema")]
    pub input_schema: Value,
}

pub struct McpStdioClient {
    server_name: String,
    process: Option<McpProcess>,
    stdin: ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    next_id: u64,
}

enum McpProcess {
    Direct(Child),
    Sandboxed(InteractiveProcess),
}

impl McpStdioClient {
    pub async fn connect(
        server_name: &str,
        config: &McpServerConfig,
        sandbox: Option<&LocalSandbox>,
    ) -> Result<Self> {
        let cmd = config
            .command
            .as_deref()
            .ok_or_else(|| anyhow!("MCP server `{server_name}` missing stdio command"))?;
        let (process, stdin, stdout) = if let Some(sandbox) = sandbox {
            let env: Vec<(String, String)> = config
                .env
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            let mut process = sandbox
                .spawn_stdio(cmd, &config.args, &env)
                .await
                .with_context(|| format!("spawn MCP server `{server_name}` ({cmd}) with Keel"))?;
            let stdin = process
                .take_stdin()
                .ok_or_else(|| anyhow!("MCP server `{server_name}` stdin unavailable"))?;
            let stdout = process
                .take_stdout()
                .ok_or_else(|| anyhow!("MCP server `{server_name}` stdout unavailable"))?;
            (McpProcess::Sandboxed(process), stdin, stdout)
        } else {
            let mut command = Command::new(cmd);
            command
                .args(&config.args)
                .envs(&config.env)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .kill_on_drop(true);
            let mut child = command
                .spawn()
                .with_context(|| format!("spawn MCP server `{server_name}` ({cmd})"))?;
            let stdin = child
                .stdin
                .take()
                .ok_or_else(|| anyhow!("MCP server `{server_name}` stdin unavailable"))?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| anyhow!("MCP server `{server_name}` stdout unavailable"))?;
            (McpProcess::Direct(child), stdin, stdout)
        };

        let mut client = Self {
            server_name: server_name.to_string(),
            process: Some(process),
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        };

        client.initialize().await?;
        Ok(client)
    }

    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    pub async fn list_tools(&mut self) -> Result<Vec<McpToolInfo>> {
        let response = self
            .request("tools/list", json!({}))
            .await
            .context("tools/list")?;
        let tools = response
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut parsed = Vec::new();
        for tool in tools {
            let info: McpToolInfo = serde_json::from_value(tool)
                .context("parse MCP tool definition")?;
            parsed.push(info);
        }
        Ok(parsed)
    }

    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<(String, bool)> {
        let response = self
            .request(
                "tools/call",
                json!({
                    "name": name,
                    "arguments": arguments,
                }),
            )
            .await
            .context("tools/call")?;

        let is_error = response
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let content = format_tool_content(response.get("content"));
        Ok((content, is_error))
    }

    pub async fn disconnect(&mut self) -> Result<()> {
        let Some(process) = self.process.take() else {
            return Ok(());
        };
        match process {
            McpProcess::Direct(mut child) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
            McpProcess::Sandboxed(process) => {
                process.cancel().await?;
            }
        }
        Ok(())
    }

    async fn initialize(&mut self) -> Result<()> {
        let result = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {
                        "name": "zene",
                        "version": env!("CARGO_PKG_VERSION"),
                    }
                }),
            )
            .await
            .context("initialize")?;

        debug!(
            server = %self.server_name,
            protocol = result
                .get("protocolVersion")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown"),
            "mcp initialized"
        );

        self.notify("notifications/initialized", json!({})).await?;
        Ok(())
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.write_message(&message).await?;
        self.read_response(id).await
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let message = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.write_message(&message).await
    }

    async fn write_message(&mut self, message: &Value) -> Result<()> {
        let line = serde_json::to_string(message).context("serialize MCP message")?;
        self.stdin
            .write_all(line.as_bytes())
            .await
            .context("write MCP message")?;
        self.stdin.write_all(b"\n").await.context("write MCP newline")?;
        self.stdin.flush().await.context("flush MCP stdin")?;
        Ok(())
    }

    async fn read_response(&mut self, expected_id: u64) -> Result<Value> {
        loop {
            let mut line = String::new();
            let bytes = self
                .stdout
                .read_line(&mut line)
                .await
                .context("read MCP response")?;
            if bytes == 0 {
                return Err(anyhow!(
                    "MCP server `{}` closed stdout before response",
                    self.server_name
                ));
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let message: Value =
                serde_json::from_str(trimmed).context("parse MCP response JSON")?;

            if message.get("method").is_some() && message.get("id").is_none() {
                debug!(
                    server = %self.server_name,
                    method = message
                        .get("method")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown"),
                    "ignored MCP notification"
                );
                continue;
            }

            if let Some(error) = message.get("error") {
                return Err(anyhow!(
                    "MCP server `{}` error: {}",
                    self.server_name,
                    error
                ));
            }

            if message.get("id").and_then(Value::as_u64) == Some(expected_id) {
                return message
                    .get("result")
                    .cloned()
                    .ok_or_else(|| anyhow!("MCP response missing result"));
            }
        }
    }
}

fn format_tool_content(content: Option<&Value>) -> String {
    let Some(content) = content.and_then(Value::as_array) else {
        return String::new();
    };

    let mut parts = Vec::new();
    for item in content {
        match item.get("type").and_then(serde_json::Value::as_str) {
            Some("text") => {
                if let Some(text) = item.get("text").and_then(serde_json::Value::as_str) {
                    parts.push(text.to_string());
                }
            }
            Some(other) => {
                parts.push(format!("[{other} content omitted]"));
            }
            None => {
                parts.push(item.to_string());
            }
        }
    }
    parts.join("\n")
}

pub fn mcp_tool_registry_name(server: &str, tool: &str) -> String {
    format!("mcp__{server}__{tool}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_text_tool_content() {
        let content = json!([
            { "type": "text", "text": "hello" },
            { "type": "text", "text": "world" }
        ]);
        assert_eq!(format_tool_content(Some(&content)), "hello\nworld");
    }

    #[test]
    fn registry_name_is_prefixed() {
        assert_eq!(
            mcp_tool_registry_name("git", "status"),
            "mcp__git__status"
        );
    }
}
