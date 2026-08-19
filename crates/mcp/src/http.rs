//! Minimal Streamable-HTTP MCP client (JSON-RPC over POST).

use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};
use serde_json::{json, Value};
use tracing::debug;

use crate::client::McpToolInfo;

const PROTOCOL_VERSION: &str = "2024-11-05";

pub struct McpHttpClient {
    server_name: String,
    url: String,
    http: reqwest::Client,
    session_id: Option<String>,
    next_id: u64,
}

impl McpHttpClient {
    pub async fn connect(
        server_name: &str,
        url: &str,
        headers: &HashMap<String, String>,
    ) -> Result<Self> {
        let mut default_headers = HeaderMap::new();
        for (k, v) in headers {
            let name = HeaderName::from_bytes(k.as_bytes())
                .with_context(|| format!("invalid header name `{k}`"))?;
            let value = HeaderValue::from_str(v)
                .with_context(|| format!("invalid header value for `{k}`"))?;
            default_headers.insert(name, value);
        }
        default_headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let http = reqwest::Client::builder()
            .default_headers(default_headers)
            .build()
            .context("build MCP HTTP client")?;

        let mut client = Self {
            server_name: server_name.to_string(),
            url: url.trim_end_matches('/').to_string(),
            http,
            session_id: None,
            next_id: 1,
        };
        client.initialize().await?;
        Ok(client)
    }

    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    async fn initialize(&mut self) -> Result<()> {
        let result = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "zene", "version": env!("CARGO_PKG_VERSION") },
                }),
            )
            .await
            .context("mcp http initialize")?;
        debug!(server = %self.server_name, ?result, "mcp http initialized");
        let _ = self.notify("notifications/initialized", json!({})).await;
        Ok(())
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
            let info: McpToolInfo =
                serde_json::from_value(tool).context("parse MCP tool definition")?;
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
        let content = extract_text_content(&response);
        Ok((content, is_error))
    }

    pub async fn disconnect(&mut self) -> Result<()> {
        Ok(())
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let body = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let mut req = self.http.post(&self.url).json(&body);
        if let Some(session) = &self.session_id {
            req = req.header("Mcp-Session-Id", session);
        }
        let _ = req.send().await.context("mcp http notify")?;
        Ok(())
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let mut req = self.http.post(&self.url).json(&body);
        if let Some(session) = &self.session_id {
            req = req.header("Mcp-Session-Id", session);
        }

        let response = req
            .send()
            .await
            .with_context(|| format!("POST MCP `{method}` to {}", self.url))?;

        if let Some(session) = response.headers().get("mcp-session-id") {
            if let Ok(s) = session.to_str() {
                self.session_id = Some(s.to_string());
            }
        }

        let status = response.status();
        let text = response.text().await.context("read MCP HTTP body")?;
        if !status.is_success() {
            return Err(anyhow!(
                "MCP HTTP `{method}` failed with {status}: {}",
                truncate(&text, 400)
            ));
        }

        // Streamable HTTP may return a single JSON object or SSE frames.
        let value = parse_json_or_sse(&text)
            .with_context(|| format!("parse MCP HTTP response for `{method}`"))?;

        if let Some(err) = value.get("error") {
            return Err(anyhow!("MCP error: {err}"));
        }
        Ok(value.get("result").cloned().unwrap_or(Value::Null))
    }
}

fn parse_json_or_sse(text: &str) -> Result<Value> {
    let trimmed = text.trim();
    if trimmed.starts_with('{') {
        return Ok(serde_json::from_str(trimmed)?);
    }
    // Take the last `data: {...}` SSE payload.
    let mut last = None;
    for line in trimmed.lines() {
        let line = line.trim();
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim();
            if data.starts_with('{') {
                last = Some(data.to_string());
            }
        }
    }
    let Some(data) = last else {
        anyhow::bail!("no JSON payload in MCP HTTP response");
    };
    Ok(serde_json::from_str(&data)?)
}

fn extract_text_content(response: &Value) -> String {
    let Some(items) = response.get("content").and_then(Value::as_array) else {
        return response.to_string();
    };
    let mut parts = Vec::new();
    for item in items {
        if item.get("type").and_then(Value::as_str) == Some("text") {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                parts.push(text.to_string());
            }
        }
    }
    if parts.is_empty() {
        response.to_string()
    } else {
        parts.join("\n")
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_json() {
        let v = parse_json_or_sse(r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#).unwrap();
        assert_eq!(v["result"]["ok"], true);
    }

    #[test]
    fn parses_sse_data() {
        let raw =
            "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"tools\":[]}}\n\n";
        let v = parse_json_or_sse(raw).unwrap();
        assert!(v["result"]["tools"].is_array());
    }
}
