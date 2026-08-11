use std::collections::HashMap;
use std::pin::Pin;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use serde_json::{json, Value};
use unigateway_sdk::core::response::{ChatResponseChunk, ChatResponseFinal};
use unigateway_sdk::core::{
    Endpoint, EndpointCapabilities, ExecutionTarget, LoadBalancingStrategy, ModelPolicy,
    ProviderKind, ProviderPool, ProxyChatRequest, ProxySession, RetryPolicy, SecretString,
    UniGatewayEngine,
};
use zene_config::ZeneConfig;

use crate::context::{
    HEADER_CONTEXT_DELIVERY, HEADER_CONTEXT_EPOCH, HEADER_PREFIX_HASH, HEADER_SESSION_ID,
    HEADER_TAIL_START, SESSION_GATEWAY_FIELD,
};
use crate::message::{Message, Role, ToolCall};
use crate::openai_direct::OpenAiDirectClient;
use crate::provider::{ChatRequest, ChatResponse, Provider, StreamEvent};
use crate::retry::with_llm_retry;
use crate::tool::ToolDefinition;
use crate::usage::{parse_usage_from_raw, TokenUsage};

const DEFAULT_POOL_ID: &str = "zene-default";
const DEFAULT_ENDPOINT_ID: &str = "zene-default";

pub struct OpenAiCompatibleProvider {
    engine: UniGatewayEngine,
    direct: OpenAiDirectClient,
}

impl OpenAiCompatibleProvider {
    pub async fn from_config(config: &ZeneConfig) -> Result<Self> {
        let direct = OpenAiDirectClient::from_config(config)?;
        let engine = UniGatewayEngine::builder()
            .with_builtin_http_drivers()
            .build()
            .map_err(|err| anyhow!("{err}"))?;

        let inference_gateway = std::env::var("ZENE_INFERENCE_GATEWAY_URL")
            .ok()
            .is_some_and(|url| !url.trim().is_empty());
        let forward_metadata_as_headers = inference_gateway.then(|| {
            vec![
                HEADER_SESSION_ID.to_string(),
                HEADER_CONTEXT_EPOCH.to_string(),
                HEADER_CONTEXT_DELIVERY.to_string(),
                HEADER_PREFIX_HASH.to_string(),
                HEADER_TAIL_START.to_string(),
            ]
        });

        let endpoint = Endpoint {
            endpoint_id: DEFAULT_ENDPOINT_ID.to_string(),
            provider_name: Some(DEFAULT_ENDPOINT_ID.to_string()),
            source_endpoint_id: Some(DEFAULT_ENDPOINT_ID.to_string()),
            provider_family: Some(config.provider_family()),
            provider_kind: ProviderKind::OpenAiCompatible,
            driver_id: "openai-compatible".to_string(),
            base_url: config.openai_base_url().trim_end_matches('/').to_string(),
            api_key: SecretString::new(config.openai_api_key()?),
            model_policy: ModelPolicy {
                default_model: Some(config.model.clone()),
                model_mapping: HashMap::new(),
            },
            enabled: true,
            max_concurrency: None,
            capabilities: EndpointCapabilities::default(),
            metadata: HashMap::new(),
            forward_metadata_as_headers: forward_metadata_as_headers.clone(),
        };

        let pool = ProviderPool {
            pool_id: DEFAULT_POOL_ID.to_string(),
            endpoints: vec![endpoint],
            load_balancing: LoadBalancingStrategy::RoundRobin,
            retry_policy: RetryPolicy::default(),
            metadata: HashMap::new(),
            forward_metadata_as_headers,
        };

        engine
            .upsert_pool(pool)
            .await
            .map_err(|err| anyhow!("{err}"))?;

        Ok(Self { engine, direct })
    }
}

#[async_trait]
impl Provider for OpenAiCompatibleProvider {
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

impl OpenAiCompatibleProvider {
    async fn chat_once(&self, request: ChatRequest) -> Result<ChatResponse> {
        if request.gateway.is_some() {
            return self.direct.chat(request).await;
        }
        let proxy_request = to_proxy_request(&request, false)?;
        let target = ExecutionTarget::Pool {
            pool_id: DEFAULT_POOL_ID.to_string(),
        };

        match self
            .engine
            .proxy_chat(proxy_request, target)
            .await
            .map_err(|err| anyhow!("{err}"))?
        {
            ProxySession::Completed(resp) => {
                let usage = parse_usage_from_raw(&resp.response.raw);
                Ok(ChatResponse {
                    message: final_to_message(resp.response),
                    usage,
                })
            }
            ProxySession::Streaming(streaming) => {
                let resp = streaming
                    .into_completion()
                    .await
                    .map_err(|err| anyhow!("{err}"))?;
                let usage = parse_usage_from_raw(&resp.response.raw);
                Ok(ChatResponse {
                    message: final_to_message(resp.response),
                    usage,
                })
            }
        }
    }

    async fn chat_stream_once(
        &self,
        request: ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>> {
        if request.gateway.is_some() {
            return self.direct.chat_stream(request).await;
        }
        let proxy_request = to_proxy_request(&request, true)?;
        let target = ExecutionTarget::Pool {
            pool_id: DEFAULT_POOL_ID.to_string(),
        };

        let session = self
            .engine
            .proxy_chat(proxy_request, target)
            .await
            .map_err(|err| anyhow!("{err}"))?;

        match session {
            ProxySession::Completed(resp) => {
                let message = final_to_message(resp.response.clone());
                let usage = parse_usage_from_raw(&resp.response.raw);
                let events = completed_message_to_events(message, usage);
                Ok(Box::pin(futures::stream::iter(events.into_iter().map(Ok))))
            }
            ProxySession::Streaming(streaming) => {
                let completion = streaming.completion;
                let stream = streaming
                    .stream
                    .map(|chunk| {
                        chunk
                            .map_err(|err| anyhow!("{err}"))
                            .map(|chunk| chunk_to_events(&chunk))
                    })
                    .flat_map(|events| {
                        let items: Vec<Result<StreamEvent>> = match events {
                            Ok(events) => events.into_iter().map(Ok).collect(),
                            Err(err) => vec![Err(err)],
                        };
                        futures::stream::iter(items)
                    });

                let done_stream = futures::stream::once(async move {
                    let usage = match completion.await {
                        Ok(Ok(resp)) => parse_usage_from_raw(&resp.response.raw),
                        _ => None,
                    };
                    Ok(StreamEvent::Done { usage })
                });

                Ok(Box::pin(stream.chain(done_stream)))
            }
        }
    }
}

pub(crate) fn to_proxy_request(request: &ChatRequest, stream: bool) -> Result<ProxyChatRequest> {
    let tools = if request.tools.is_empty() {
        None
    } else {
        Some(Value::Array(
            request
                .tools
                .iter()
                .map(ToolDefinition::to_openai_tool)
                .collect(),
        ))
    };

    let raw_messages = Value::Array(
        request
            .messages
            .iter()
            .map(message_to_api)
            .collect::<Result<Vec<_>>>()?,
    );

    let mut proxy = ProxyChatRequest {
        model: request.model.clone(),
        messages: Vec::new(),
        temperature: None,
        top_p: None,
        top_k: None,
        max_tokens: None,
        stop_sequences: None,
        stream,
        system: None,
        tools,
        tool_choice: None,
        raw_messages: Some(raw_messages),
        gateway_fields: HashMap::new(),
        extra: HashMap::new(),
        metadata: HashMap::new(),
    };
    proxy.mark_openai_raw_messages();
    if let Some(ctx) = &request.context {
        proxy
            .metadata
            .insert(HEADER_SESSION_ID.to_string(), ctx.session_id.clone());
        proxy.metadata.insert(
            HEADER_CONTEXT_EPOCH.to_string(),
            ctx.context_epoch.to_string(),
        );
        proxy.metadata.insert(
            HEADER_CONTEXT_DELIVERY.to_string(),
            ctx.delivery.as_str().to_string(),
        );
        if let Some(hash) = &ctx.prefix_hash {
            proxy
                .metadata
                .insert(HEADER_PREFIX_HASH.to_string(), hash.clone());
        }
        if let Some(start) = ctx.tail_start {
            proxy
                .metadata
                .insert(HEADER_TAIL_START.to_string(), start.to_string());
        }
        let mut session_context = serde_json::json!({
            "session_id": ctx.session_id,
            "epoch": ctx.context_epoch,
            "delivery": ctx.delivery.as_str(),
            "tail_start": ctx.tail_start,
        });
        if let Some(hash) = &ctx.prefix_hash {
            session_context["prefix_hash"] = serde_json::json!(hash);
            session_context["fingerprint"] = serde_json::json!({
                "algorithm": "zene-v1",
                "value": hash,
            });
        }
        proxy.gateway_fields.insert(
            SESSION_GATEWAY_FIELD.to_string(),
            session_context,
        );
    }
    Ok(proxy)
}

pub(crate) fn message_to_api(message: &Message) -> Result<Value> {
    let role = match message.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };

    if message.role == Role::Tool {
        let tool_call_id = message
            .tool_call_id
            .clone()
            .ok_or_else(|| anyhow!("tool message missing tool_call_id"))?;
        let mut object = serde_json::Map::from_iter([
            ("role".to_string(), Value::String(role.to_string())),
            (
                "tool_call_id".to_string(),
                Value::String(tool_call_id),
            ),
            (
                "content".to_string(),
                Value::String(message.content.clone().unwrap_or_default()),
            ),
        ]);
        if let Some(name) = &message.name {
            object.insert("name".to_string(), Value::String(name.clone()));
        }
        if message.is_error == Some(true) {
            object.insert("is_error".to_string(), Value::Bool(true));
        }
        return Ok(Value::Object(object));
    }

    let mut object = serde_json::Map::from_iter([(
        "role".to_string(),
        Value::String(role.to_string()),
    )]);

    if let Some(content) = &message.content {
        object.insert("content".to_string(), Value::String(content.clone()));
    }

    if let Some(tool_calls) = &message.tool_calls {
        let calls = tool_calls
            .iter()
            .map(|call| {
                Ok(json!({
                    "id": call.id,
                    "type": "function",
                    "function": {
                        "name": call.name,
                        "arguments": call.arguments,
                    }
                }))
            })
            .collect::<Result<Vec<_>>>()?;
        object.insert("tool_calls".to_string(), Value::Array(calls));
    }

    Ok(Value::Object(object))
}

fn final_to_message(response: ChatResponseFinal) -> Message {
    parse_message_from_raw(&response.raw).unwrap_or_else(|| {
        Message::assistant(response.output_text.unwrap_or_default())
    })
}

fn parse_message_from_raw(raw: &Value) -> Option<Message> {
    let message = raw
        .get("choices")?
        .as_array()?
        .first()?
        .get("message")?;

    let role = match message.get("role")?.as_str()? {
        "system" => Role::System,
        "assistant" => Role::Assistant,
        "tool" => Role::Tool,
        _ => Role::User,
    };

    let content = message
        .get("content")
        .and_then(|value| match value {
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

pub fn chunk_to_events_from_raw(raw: &Value) -> Vec<StreamEvent> {
    let mut events = Vec::new();

    let delta_obj = raw
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("delta"));

    if let Some(delta) = delta_obj {
        if let Some(reasoning) = delta
            .get("reasoning_content")
            .or_else(|| delta.get("reasoning"))
            .and_then(Value::as_str)
        {
            if !reasoning.is_empty() {
                events.push(StreamEvent::ThoughtDelta(reasoning.to_string()));
            }
        }
        if let Some(content) = delta.get("content").and_then(Value::as_str) {
            if !content.is_empty() {
                events.push(StreamEvent::TextDelta(content.to_string()));
            }
        }
    }

    if let Some(tool_calls) = delta_obj
        .and_then(|delta| delta.get("tool_calls"))
        .and_then(Value::as_array)
    {
        for call in tool_calls {
            events.push(StreamEvent::ToolCallDelta {
                index: call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize,
                id: call.get("id").and_then(Value::as_str).map(str::to_string),
                name: call
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                arguments: call
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
            });
        }
    }

    events
}

fn chunk_to_events(chunk: &ChatResponseChunk) -> Vec<StreamEvent> {
    let mut events = Vec::new();

    let delta_obj = chunk
        .raw
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("delta"));

    if let Some(delta) = delta_obj {
        if let Some(reasoning) = delta
            .get("reasoning_content")
            .or_else(|| delta.get("reasoning"))
            .and_then(Value::as_str)
        {
            if !reasoning.is_empty() {
                events.push(StreamEvent::ThoughtDelta(reasoning.to_string()));
            }
        }
    }

    if let Some(delta) = &chunk.delta {
        if !delta.is_empty() {
            events.push(StreamEvent::TextDelta(delta.clone()));
        }
    }

    if let Some(tool_calls) = delta_obj.and_then(|delta| delta.get("tool_calls")).and_then(Value::as_array)
    {
        for call in tool_calls {
            events.push(StreamEvent::ToolCallDelta {
                index: call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize,
                id: call.get("id").and_then(Value::as_str).map(str::to_string),
                name: call
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                arguments: call
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
            });
        }
    }

    events
}

fn completed_message_to_events(message: Message, usage: Option<TokenUsage>) -> Vec<StreamEvent> {
    let mut events = Vec::new();

    if let Some(content) = message.content {
        if !content.is_empty() {
            events.push(StreamEvent::TextDelta(content));
        }
    }

    if let Some(tool_calls) = message.tool_calls {
        for (index, call) in tool_calls.into_iter().enumerate() {
            events.push(StreamEvent::ToolCallDelta {
                index,
                id: Some(call.id),
                name: Some(call.name),
                arguments: Some(call.arguments),
            });
        }
    }

    events.push(StreamEvent::Done { usage });
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn tool_error_message_includes_is_error_field() {
        let message = Message::tool_result_with_error("call_1", "Read", "failed", true);
        let api = message_to_api(&message).expect("api message");
        assert_eq!(api.get("is_error").and_then(Value::as_bool), Some(true));
        assert_eq!(
            api.get("tool_call_id").and_then(Value::as_str),
            Some("call_1")
        );
    }

    #[test]
    fn successful_tool_message_omits_is_error_field() {
        let message = Message::tool_result("call_1", "Read", "ok");
        let api = message_to_api(&message).expect("api message");
        assert!(api.get("is_error").is_none());
    }

    #[test]
    fn chunk_emits_thought_delta_from_reasoning_content() {
        let chunk = ChatResponseChunk {
            delta: Some("answer".into()),
            raw: json!({
                "choices": [{
                    "delta": {
                        "reasoning_content": "thinking...",
                        "content": "answer"
                    }
                }]
            }),
        };
        let events = chunk_to_events(&chunk);
        assert!(events.iter().any(|e| matches!(
            e,
            StreamEvent::ThoughtDelta(t) if t == "thinking..."
        )));
        assert!(events.iter().any(|e| matches!(
            e,
            StreamEvent::TextDelta(t) if t == "answer"
        )));
    }
}
