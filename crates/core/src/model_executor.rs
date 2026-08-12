use std::collections::HashSet;
use std::pin::Pin;

use anyhow::Result;
use async_trait::async_trait;
use futures::Stream;
use zene_llm::{ChatClient, ChatRequest, ChatResponse, Message, StreamEvent, ToolCall};

pub(crate) type ModelStream = Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>;

/// Runtime-facing model boundary. Provider-specific details stay behind this seam.
#[async_trait]
pub(crate) trait ModelExecutor: Send + Sync {
    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse>;

    async fn stream(&self, request: ChatRequest) -> Result<ModelStream>;
}

/// Default executor backed by the existing unified ChatClient.
pub(crate) struct ChatClientExecutor<'a> {
    client: &'a ChatClient,
}

impl<'a> ChatClientExecutor<'a> {
    pub(crate) fn new(client: &'a ChatClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl ModelExecutor for ChatClientExecutor<'_> {
    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse> {
        self.client.chat(request).await
    }

    async fn stream(&self, request: ChatRequest) -> Result<ModelStream> {
        self.client.chat_stream(request).await
    }
}

#[derive(Default)]
pub(crate) struct ToolCallBuilder {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) arguments: String,
}

pub(crate) fn apply_tool_call_delta(
    call: &mut ToolCallBuilder,
    id: Option<String>,
    name: Option<String>,
    arguments: Option<String>,
) {
    if let Some(id) = id {
        call.id = id;
    }
    if let Some(name) = name {
        call.name = name;
    }
    if let Some(arguments) = arguments {
        call.arguments.push_str(&arguments);
    }
}

pub(crate) fn assemble_message(text: String, builders: Vec<ToolCallBuilder>) -> Message {
    let calls = normalize_tool_calls(
        builders
            .into_iter()
            .filter(|call| !call.name.is_empty())
            .map(|call| ToolCall {
                id: call.id,
                name: call.name,
                arguments: call.arguments,
            })
            .collect(),
    );
    if calls.is_empty() {
        Message::assistant(text)
    } else {
        Message::assistant_with_tools((!text.is_empty()).then_some(text), calls)
    }
}

fn normalize_tool_calls(mut calls: Vec<ToolCall>) -> Vec<ToolCall> {
    let mut used_ids = HashSet::new();
    for (index, call) in calls.iter_mut().enumerate() {
        if call.id.trim().is_empty() {
            call.id = format!("call_{index}");
        }
        let base = call.id.clone();
        let mut unique = base.clone();
        let mut suffix = 0u32;
        while !used_ids.insert(unique.clone()) {
            suffix += 1;
            unique = format!("{base}_{suffix}");
        }
        call.id = unique;
    }
    calls
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{stream, StreamExt};

    struct FakeExecutor;

    #[async_trait]
    impl ModelExecutor for FakeExecutor {
        async fn complete(&self, request: ChatRequest) -> Result<ChatResponse> {
            Ok(ChatResponse {
                message: Message::assistant(request.messages.len().to_string()),
                usage: None,
            })
        }

        async fn stream(&self, _request: ChatRequest) -> Result<ModelStream> {
            Ok(Box::pin(stream::iter([Ok(StreamEvent::Done { usage: None })])))
        }
    }

    fn request() -> ChatRequest {
        ChatRequest {
            model: "fake".into(),
            messages: vec![Message::user("hello")],
            tools: Vec::new(),
            stream: false,
            context: None,
        }
    }

    #[tokio::test]
    async fn fake_executor_covers_complete_and_stream_boundaries() {
        let executor = FakeExecutor;
        let response = executor.complete(request()).await.unwrap();
        assert_eq!(response.message.content.as_deref(), Some("1"));

        let mut stream = executor.stream(request()).await.unwrap();
        let event = stream.next().await.unwrap().unwrap();
        assert!(matches!(event, StreamEvent::Done { usage: None }));
    }

    #[test]
    fn assembles_streamed_text() {
        let message = assemble_message("hello world".into(), Vec::new());
        assert_eq!(message.content.as_deref(), Some("hello world"));
        assert!(message.tool_calls.is_none());
    }

    #[test]
    fn assembles_multiple_tool_deltas() {
        let mut first = ToolCallBuilder::default();
        apply_tool_call_delta(
            &mut first,
            Some("call-a".into()),
            Some("Read".into()),
            Some("{\"path\":".into()),
        );
        apply_tool_call_delta(&mut first, None, None, Some("\"a.rs\"}".into()));
        let mut second = ToolCallBuilder::default();
        apply_tool_call_delta(&mut second, None, Some("Write".into()), Some("{}".into()));
        let message = assemble_message("checking".into(), vec![first, second]);
        let calls = message.tool_calls.expect("tool calls");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].arguments, "{\"path\":\"a.rs\"}");
        assert_eq!(calls[1].id, "call_1");
    }

    #[test]
    fn normalizes_duplicate_and_missing_ids() {
        let calls = normalize_tool_calls(vec![
            ToolCall { id: "".into(), name: "A".into(), arguments: "{}".into() },
            ToolCall { id: "".into(), name: "B".into(), arguments: "{}".into() },
            ToolCall { id: "call_0".into(), name: "C".into(), arguments: "{}".into() },
        ]);
        assert_eq!(
            calls.iter().map(|call| call.id.as_str()).collect::<Vec<_>>(),
            ["call_0", "call_1", "call_0_1"]
        );
    }
}
