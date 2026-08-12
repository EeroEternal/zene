use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use zene_llm::{ChatClient, ChatRequest, ChatResponse};

/// Minimal synchronous-model boundary used by context summarization and memory.
///
/// Streaming inference remains owned by the runtime's `ModelExecutor`; context
/// only needs a complete response for compaction and memory extraction.
#[async_trait]
pub trait ContextModel: Send + Sync {
    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse>;
}

#[async_trait]
impl ContextModel for ChatClient {
    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse> {
        self.chat(request).await
    }
}

#[async_trait]
impl<T> ContextModel for Arc<T>
where
    T: ContextModel + ?Sized,
{
    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse> {
        (**self).complete(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zene_llm::{Message, TokenUsage};

    struct FakeModel;

    #[async_trait]
    impl ContextModel for FakeModel {
        async fn complete(&self, request: ChatRequest) -> Result<ChatResponse> {
            Ok(ChatResponse {
                message: Message::assistant(request.messages.len().to_string()),
                usage: Some(TokenUsage::default()),
            })
        }
    }

    #[tokio::test]
    async fn context_model_is_minimal_and_arc_forwarding_works() {
        let model: Arc<dyn ContextModel> = Arc::new(FakeModel);
        let response = model
            .complete(ChatRequest {
                model: "fake".into(),
                messages: vec![Message::user("hello")],
                tools: Vec::new(),
                stream: false,
                context: None,
            })
            .await
            .unwrap();
        assert_eq!(response.message.content.as_deref(), Some("1"));
    }
}
