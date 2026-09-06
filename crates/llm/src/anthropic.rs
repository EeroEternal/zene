use std::pin::Pin;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde_json::{json, Value};
use zene_config::ZeneConfig;

use crate::message::{Message, Role, ToolCall};
use crate::provider::{ChatRequest, ChatResponse, Provider, StreamEvent};
use crate::retry::with_llm_retry;
use crate::tool::ToolDefinition;
use crate::usage::TokenUsage;

pub struct AnthropicProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl AnthropicProvider {
    pub fn from_config(config: &ZeneConfig) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::new(),
            base_url: config
                .anthropic_base_url()
                .trim_end_matches('/')
                .to_string(),
            api_key: config.anthropic_api_key()?,
        })
    }

    async fn chat_once(&self, request: ChatRequest) -> Result<ChatResponse> {
        let (system, messages) = split_system_and_messages(&request.messages)?;
        let body = build_request_body(
            &request.model,
            system,
            &messages,
            &request.tools,
            false,
            request.reasoning_effort.as_deref(),
        );

        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .headers(anthropic_headers(&self.api_key)?)
            .json(&body)
            .send()
            .await
            .context("anthropic request failed")?;

        let status = response.status();
        let raw: Value = response
            .json()
            .await
            .context("anthropic response parse failed")?;

        if !status.is_success() {
            let message = raw
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("unknown anthropic error");
            anyhow::bail!("Anthropic API error ({status}): {message}");
        }

        parse_anthropic_response(&raw)
    }

    async fn chat_stream_once(
        &self,
        request: ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>> {
        let (system, messages) = split_system_and_messages(&request.messages)?;
        let body = build_request_body(
            &request.model,
            system,
            &messages,
            &request.tools,
            true,
            request.reasoning_effort.as_deref(),
        );

        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .headers(anthropic_headers(&self.api_key)?)
            .json(&body)
            .send()
            .await
            .context("anthropic stream request failed")?;

        let status = response.status();
        if !status.is_success() {
            let raw: Value = response.json().await.unwrap_or(json!({}));
            let message = raw
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("unknown anthropic error");
            anyhow::bail!("Anthropic API error ({status}): {message}");
        }

        let byte_stream = response.bytes_stream();
        let stream = byte_stream
            .map(|chunk| chunk.map_err(|err| anyhow!("{err}")))
            .scan(AnthropicStreamState::default(), |state, chunk| {
                let result = match chunk {
                    Ok(bytes) => state.push_chunk(bytes),
                    Err(_) => None,
                };
                std::future::ready(result)
            })
            .flat_map(|events| futures::stream::iter(events.into_iter().map(Ok)));

        Ok(Box::pin(stream))
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        with_llm_retry(|| self.chat_once(request.clone())).await
    }

    async fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>> {
        with_llm_retry(|| self.chat_stream_once(request.clone())).await
    }
}

fn anthropic_headers(api_key: &str) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        "x-api-key",
        HeaderValue::from_str(api_key).context("invalid anthropic api key header")?,
    );
    headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
    Ok(headers)
}

fn split_system_and_messages(messages: &[Message]) -> Result<(Option<String>, Vec<Message>)> {
    let mut system_parts = Vec::new();
    let mut rest = Vec::new();
    for message in messages {
        if message.role == Role::System {
            if let Some(content) = &message.content {
                system_parts.push(content.clone());
            }
        } else {
            rest.push(message.clone());
        }
    }

    let system = if system_parts.is_empty() {
        None
    } else {
        Some(system_parts.join("\n\n"))
    };

    Ok((system, rest))
}

fn build_request_body(
    model: &str,
    system: Option<String>,
    messages: &[Message],
    tools: &[ToolDefinition],
    stream: bool,
    reasoning_effort: Option<&str>,
) -> Value {
    let mut body = json!({
        "model": model,
        "max_tokens": 8192,
        "messages": messages_to_anthropic(messages),
    });

    if let Some(effort) = reasoning_effort {
        let budget = match effort.to_ascii_lowercase().as_str() {
            "low" => 2048,
            "medium" => 4096,
            "high" => 8192,
            s => s.parse::<u32>().unwrap_or(4096),
        };
        body["thinking"] = json!({
            "type": "enabled",
            "budget_tokens": budget,
        });
        // max_tokens must be larger than budget_tokens
        body["max_tokens"] = json!(budget + 4096);
    }

    if let Some(system) = system {
        body["system"] = Value::String(system);
    }

    if !tools.is_empty() {
        body["tools"] = Value::Array(
            tools
                .iter()
                .map(|tool| {
                    json!({
                        "name": tool.name,
                        "description": tool.description,
                        "input_schema": tool.parameters,
                    })
                })
                .collect(),
        );
    }

    if stream {
        body["stream"] = Value::Bool(true);
    }

    body
}

fn messages_to_anthropic(messages: &[Message]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut i = 0;

    while i < messages.len() {
        match messages[i].role {
            Role::User => {
                out.push(json!({
                    "role": "user",
                    "content": messages[i].content.clone().unwrap_or_default(),
                }));
                i += 1;
            }
            Role::Assistant => {
                let mut content = Vec::new();
                if let Some(text) = &messages[i].content {
                    if !text.is_empty() {
                        content.push(json!({"type": "text", "text": text}));
                    }
                }
                if let Some(tool_calls) = &messages[i].tool_calls {
                    for call in tool_calls {
                        let input = serde_json::from_str(&call.arguments).unwrap_or(json!({}));
                        content.push(json!({
                            "type": "tool_use",
                            "id": call.id,
                            "name": call.name,
                            "input": input,
                        }));
                    }
                }
                out.push(json!({"role": "assistant", "content": content}));
                i += 1;
            }
            Role::Tool => {
                let mut tool_results = Vec::new();
                while i < messages.len() && messages[i].role == Role::Tool {
                    let msg = &messages[i];
                    let is_error = msg.is_error == Some(true);
                    let mut block = json!({
                        "type": "tool_result",
                        "tool_use_id": msg.tool_call_id.clone().unwrap_or_default(),
                        "content": msg.content.clone().unwrap_or_default(),
                    });
                    if is_error {
                        block["is_error"] = Value::Bool(true);
                    }
                    tool_results.push(block);
                    i += 1;
                }
                out.push(json!({"role": "user", "content": tool_results}));
            }
            Role::System => {
                i += 1;
            }
        }
    }

    out
}

fn parse_anthropic_response(raw: &Value) -> Result<ChatResponse> {
    let content_blocks = raw
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("anthropic response missing content"))?;

    let mut text = String::new();
    let mut tool_calls = Vec::new();

    for block in content_blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(part) = block.get("text").and_then(Value::as_str) {
                    text.push_str(part);
                }
            }
            Some("tool_use") => {
                let id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let arguments = block
                    .get("input")
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "{}".to_string());
                tool_calls.push(ToolCall {
                    id,
                    name,
                    arguments,
                });
            }
            _ => {}
        }
    }

    let usage = raw.get("usage").map(|usage| TokenUsage {
        prompt_tokens: usage
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        completion_tokens: usage
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        total_tokens: usage
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            + usage
                .get("output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        cached_tokens: None,
        gateway_hit_tokens: usage
            .get("gateway_cache_hit_tokens")
            .and_then(Value::as_u64)
            .or_else(|| usage.get("cache_hit_tokens").and_then(Value::as_u64)),
        gateway_anchor_aligned: usage
            .get("gateway_anchor_aligned")
            .and_then(Value::as_bool)
            .or_else(|| usage.get("anchor_aligned").and_then(Value::as_bool)),
    });

    let message = if tool_calls.is_empty() {
        Message::assistant(text)
    } else {
        Message::assistant_with_tools(if text.is_empty() { None } else { Some(text) }, tool_calls)
    };

    Ok(ChatResponse { message, usage })
}

#[derive(Default)]
struct AnthropicStreamState {
    buffer: String,
    text: String,
    tool_calls: Vec<ToolCallBuilder>,
    usage: Option<TokenUsage>,
    done: bool,
}

struct ToolCallBuilder {
    id: String,
    name: String,
    arguments: String,
}

impl AnthropicStreamState {
    fn push_chunk(&mut self, chunk: bytes::Bytes) -> Option<Vec<StreamEvent>> {
        self.buffer.push_str(&String::from_utf8_lossy(&chunk));
        let mut events = Vec::new();

        while let Some(line_end) = self.buffer.find('\n') {
            let line = self.buffer[..line_end].trim().to_string();
            self.buffer.drain(..=line_end);

            if !line.starts_with("data: ") {
                continue;
            }
            let data = line.trim_start_matches("data: ").trim();
            if data.is_empty() {
                continue;
            }

            let parsed: Value = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            match parsed.get("type").and_then(Value::as_str) {
                Some("content_block_delta") => {
                    let index = parsed.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                    if let Some(delta) = parsed.get("delta") {
                        match delta.get("type").and_then(Value::as_str) {
                            Some("text_delta") => {
                                if let Some(text) = delta.get("text").and_then(Value::as_str) {
                                    self.text.push_str(text);
                                    events.push(StreamEvent::TextDelta(text.to_string()));
                                }
                            }
                            Some("thinking_delta") => {
                                if let Some(text) = delta
                                    .get("thinking")
                                    .or_else(|| delta.get("text"))
                                    .and_then(Value::as_str)
                                {
                                    if !text.is_empty() {
                                        events.push(StreamEvent::ThoughtDelta(text.to_string()));
                                    }
                                }
                            }
                            Some("input_json_delta") => {
                                while self.tool_calls.len() <= index {
                                    self.tool_calls.push(ToolCallBuilder {
                                        id: String::new(),
                                        name: String::new(),
                                        arguments: String::new(),
                                    });
                                }
                                if let Some(partial) =
                                    delta.get("partial_json").and_then(Value::as_str)
                                {
                                    self.tool_calls[index].arguments.push_str(partial);
                                    events.push(StreamEvent::ToolCallDelta {
                                        index,
                                        id: if self.tool_calls[index].id.is_empty() {
                                            None
                                        } else {
                                            Some(self.tool_calls[index].id.clone())
                                        },
                                        name: if self.tool_calls[index].name.is_empty() {
                                            None
                                        } else {
                                            Some(self.tool_calls[index].name.clone())
                                        },
                                        arguments: Some(partial.to_string()),
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Some("content_block_start") => {
                    let index = parsed.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                    if let Some(block) = parsed.get("content_block") {
                        if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                            while self.tool_calls.len() <= index {
                                self.tool_calls.push(ToolCallBuilder {
                                    id: String::new(),
                                    name: String::new(),
                                    arguments: String::new(),
                                });
                            }
                            self.tool_calls[index].id = block
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string();
                            self.tool_calls[index].name = block
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string();
                            events.push(StreamEvent::ToolCallDelta {
                                index,
                                id: Some(self.tool_calls[index].id.clone()),
                                name: Some(self.tool_calls[index].name.clone()),
                                arguments: None,
                            });
                        }
                    }
                }
                Some("message_delta") => {
                    if let Some(usage) = parsed.get("usage") {
                        self.usage = Some(TokenUsage {
                            prompt_tokens: 0,
                            completion_tokens: usage
                                .get("output_tokens")
                                .and_then(Value::as_u64)
                                .unwrap_or(0),
                            total_tokens: usage
                                .get("output_tokens")
                                .and_then(Value::as_u64)
                                .unwrap_or(0),
                            cached_tokens: None,
                            gateway_anchor_aligned: None,
                            gateway_hit_tokens: usage
                                .get("cache_hit_tokens")
                                .and_then(Value::as_u64),
                        });
                    }
                }
                Some("message_stop") => {
                    self.done = true;
                    events.push(StreamEvent::Done { usage: self.usage });
                }
                _ => {}
            }
        }

        if events.is_empty() {
            None
        } else {
            Some(events)
        }
    }
}
