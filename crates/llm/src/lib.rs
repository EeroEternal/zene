mod anthropic;
mod client;
mod context;
mod message;
mod models;
mod openai_compatible;
mod provider;
mod retry;
mod tool;
mod usage;

pub use client::{provider_from_config, selected_provider_kind, ChatClient};
pub use context::{
    ContextDelivery, ContextMetadata, BODY_ZENE_CONTEXT, HEADER_CONTEXT_DELIVERY,
    HEADER_CONTEXT_EPOCH, HEADER_PREFIX_HASH, HEADER_SESSION_ID, HEADER_TAIL_START,
    SESSION_GATEWAY_FIELD,
};
pub use message::{ContentPart, Message, MessageKind, Role, ToolCall};
pub use models::context_window_for_model;
pub use zene_config::default_context_window_for_model;
pub use provider::{ChatRequest, ChatResponse, Provider, StreamEvent};
pub use retry::{
    classify_llm_error, is_context_overflow, is_retryable, with_llm_retry, LlmErrorClass,
    MAX_LLM_ATTEMPTS, RATE_LIMIT_RETRY_THRESHOLD,
};
pub use tool::ToolDefinition;
pub use usage::{parse_usage_from_raw, TokenUsage};
