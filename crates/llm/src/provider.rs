use std::pin::Pin;

use anyhow::Result;
use async_trait::async_trait;
use futures::Stream;

use crate::context::ContextMetadata;
use crate::message::Message;
use crate::tool::ToolDefinition;
use crate::usage::TokenUsage;

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub stream: bool,
    /// Optional session linkage for inference gateways (see docs/context-engine.md).
    pub context: Option<ContextMetadata>,
    /// Optional reasoning effort configuration (e.g. "low", "medium", "high", or token budget)
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    TextDelta(String),
    /// Model reasoning / thinking tokens (when the provider streams them).
    ThoughtDelta(String),
    ToolCallDelta {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments: Option<String>,
    },
    Done {
        usage: Option<TokenUsage>,
    },
}

#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub message: Message,
    pub usage: Option<TokenUsage>,
}

#[async_trait]
pub trait Provider: Send + Sync {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse>;
    async fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>>;
}
