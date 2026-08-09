//! Direct OpenAI-compatible HTTP when gateway session headers are required.

use std::pin::Pin;

use anyhow::{anyhow, Context, Result};
use futures::{Stream, StreamExt};
use reqwest::header::{HeaderMap, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{json, Value};
use zene_config::ZeneConfig;

use crate::gateway::GatewayRequestContext;
use crate::message::{Message, ToolCall};
use crate::openai_compatible::{chunk_to_events_from_raw, message_to_api};
use crate::provider::{ChatRequest, ChatResponse, StreamEvent};
use crate::tool::ToolDefinition;
use crate::usage::parse_usage_from_raw;

pub struct OpenAiDirectClient {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl OpenAiDirectClient {
    pub fn from_config(config: &ZeneConfig) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::new(),
            base_url: config.openai_base_url().trim_end_matches('/').to_string(),
            api_key: config.openai_api_key()?,
        })
    }

    pub async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let gateway = request
            .gateway
            .as_ref()
            .context("direct openai chat requires gateway context")?;
        let body = build_body(&request)?;
        let headers = auth_headers(&self.api_key, gateway)?;
        let url = format!("{}/chat/completions", self.base_url);

        let response = self
            .client
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .context("openai direct chat")?;
        let status = response.status();
        let raw: Value = response.json().await.context("openai direct json")?;
        if !status.is_success() {
            return Err(anyhow!(
                "openai direct chat error ({status}): {raw}"
            ));
        }

        if gateway.mode == crate::gateway::GatewayMode::Publish {
            return Ok(ChatResponse {
                message: Message::assistant(""),
                usage: parse_usage_from_raw(&raw),
            });
        }

        let message = parse_message_from_raw(&raw).unwrap_or_else(|| Message::assistant(""));
        Ok(ChatResponse {
            message,
            usage: parse_usage_from_raw(&raw),
        })
    }

    pub async fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>> {
        let gateway = request
            .gateway
            .clone()
            .context("direct openai stream requires gateway context")?;
        let mut body = build_body(&request)?;
        if let Some(obj) = body.as_object_mut() {
            obj.insert("stream".to_string(), Value::Bool(true));
        }
        let headers = auth_headers(&self.api_key, &gateway)?;
        let url = format!("{}/chat/completions", self.base_url);

        let response = self
            .client
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .context("openai direct stream")?;
        if !response.status().is_success() {
            let status = response.status();
            let raw: Value = response.json().await.unwrap_or(json!({}));
            return Err(anyhow!("openai direct stream error ({status}): {raw}"));
        }

        let byte_stream = response.bytes_stream().map(|chunk| {
            chunk.map_err(|err| anyhow!("openai stream read: {err}"))
        });

        Ok(Box::pin(parse_sse_stream(byte_stream)))
    }
}

fn auth_headers(api_key: &str, gateway: &GatewayRequestContext) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        format!("Bearer {api_key}")
            .parse()
            .context("authorization header")?,
    );
    headers.insert(CONTENT_TYPE, "application/json".parse().unwrap());
    gateway.apply_reqwest_headers(&mut headers);
    Ok(headers)
}

fn build_body(request: &ChatRequest) -> Result<Value> {
    let messages: Vec<Value> = request
        .messages
        .iter()
        .map(message_to_api)
        .collect::<Result<_>>()?;
    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "stream": request.stream,
    });
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(ToolDefinition::to_openai_tool)
            .collect();
        body["tools"] = Value::Array(tools);
    }
    Ok(body)
}

fn parse_message_from_raw(raw: &Value) -> Option<Message> {
    let message = raw.get("choices")?.as_array()?.first()?.get("message")?;
    let role = match message.get("role")?.as_str()? {
        "system" => crate::message::Role::System,
        "assistant" => crate::message::Role::Assistant,
        "tool" => crate::message::Role::Tool,
        _ => crate::message::Role::User,
    };
    let content = message.get("content").and_then(|value| match value {
        Value::String(text) => Some(text.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    });
    let tool_calls = message.get("tool_calls").and_then(|calls| {
        calls.as_array().map(|items| {
            items
                .iter()
                .filter_map(|call| {
                    let id = call.get("id")?.as_str()?.to_string();
                    let function = call.get("function")?;
                    Some(ToolCall {
                        id,
                        name: function.get("name")?.as_str()?.to_string(),
                        arguments: function
                            .get("arguments")
                            .and_then(Value::as_str)
                            .unwrap_or("{}")
                            .to_string(),
                    })
                })
                .collect::<Vec<_>>()
        })
    });
    Some(Message {
        role,
        content,
        tool_calls,
        tool_call_id: None,
        name: None,
        is_error: None,
        kind: None,
    })
}

fn parse_sse_stream(
    byte_stream: impl Stream<Item = Result<bytes::Bytes>> + Send + 'static,
) -> impl Stream<Item = Result<StreamEvent>> + Send {
    async_stream::try_stream! {
        use futures::StreamExt;
        let mut buffer = String::new();
        futures::pin_mut!(byte_stream);
        while let Some(chunk) = byte_stream.next().await {
            let chunk = chunk?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(pos) = buffer.find("\n\n") {
                let frame = buffer[..pos].to_string();
                buffer = buffer[pos + 2..].to_string();
                for line in frame.lines() {
                    let line = line.trim();
                    if let Some(data) = line.strip_prefix("data: ") {
                        if data == "[DONE]" {
                            yield StreamEvent::Done { usage: None };
                            continue;
                        }
                        let raw: Value = serde_json::from_str(data)?;
                        for event in chunk_to_events_from_raw(&raw) {
                            yield event;
                        }
                        if let Some(usage) = parse_usage_from_raw(&raw) {
                            yield StreamEvent::Done { usage: Some(usage) };
                        }
                    }
                }
            }
        }
    }
}
