use std::collections::HashSet;
use std::pin::Pin;

use anyhow::Result;
use async_trait::async_trait;
use futures::Stream;
use zene_llm::{ChatClient, ChatRequest, ChatResponse, Message, StreamEvent, ToolCall};

pub(crate) type ModelStream = Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>;

pub(crate) fn build_request(
    model: &str,
    messages: Vec<Message>,
    tools: Vec<zene_llm::ToolDefinition>,
    stream: bool,
    context: Option<zene_llm::ContextMetadata>,
) -> ChatRequest {
    ChatRequest {
        model: model.to_string(),
        messages,
        tools,
        stream,
        context,
    }
}

#[derive(Debug, Default)]
pub(crate) struct OverflowRetryState {
    truncated: bool,
    summarized: bool,
}

impl OverflowRetryState {
    pub(crate) fn flags(&self) -> (bool, bool) {
        (self.truncated, self.summarized)
    }

    pub(crate) fn set_flags(&mut self, truncated: bool, summarized: bool) {
        self.truncated = truncated;
        self.summarized = summarized;
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UsageSnapshot {
    pub(crate) usage: zene_llm::TokenUsage,
    pub(crate) context_tokens: u32,
    pub(crate) context_window: u32,
    pub(crate) context_percent: u8,
    pub(crate) context_epoch: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct UsageAccumulator {
    total: zene_llm::TokenUsage,
}

impl UsageAccumulator {
    pub(crate) fn reset(&mut self) {
        self.total = zene_llm::TokenUsage::default();
    }

    pub(crate) fn record(&mut self, usage: &zene_llm::TokenUsage) {
        self.total.accumulate(usage);
    }

    pub(crate) fn total(&self) -> &zene_llm::TokenUsage {
        &self.total
    }

    pub(crate) fn snapshot(
        &self,
        context_tokens: u32,
        context_window: u32,
        context_percent: u8,
        context_epoch: u64,
    ) -> UsageSnapshot {
        UsageSnapshot {
            usage: self.total,
            context_tokens,
            context_window,
            context_percent,
            context_epoch,
        }
    }
}

#[derive(Default)]
pub(crate) struct StreamAccumulator {
    text: String,
    tool_calls: Vec<ToolCallBuilder>,
    usage: Option<zene_llm::TokenUsage>,
}

impl StreamAccumulator {
    pub(crate) fn apply(&mut self, event: &StreamEvent) -> bool {
        match event {
            StreamEvent::TextDelta(delta) => self.text.push_str(delta),
            StreamEvent::ToolCallDelta {
                index,
                id,
                name,
                arguments,
            } => {
                while self.tool_calls.len() <= *index {
                    self.tool_calls.push(ToolCallBuilder::default());
                }
                apply_tool_call_delta(
                    &mut self.tool_calls[*index],
                    id.clone(),
                    name.clone(),
                    arguments.clone(),
                );
            }
            StreamEvent::Done { usage } => {
                self.usage = usage.clone();
                return true;
            }
            StreamEvent::ThoughtDelta(_) => {}
        }
        false
    }

    pub(crate) fn has_text(&self) -> bool {
        !self.text.is_empty()
    }

    pub(crate) fn finish(self) -> (Message, Option<zene_llm::TokenUsage>) {
        (assemble_message(self.text, self.tool_calls), self.usage)
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
        build_request("fake", vec![Message::user("hello")], Vec::new(), false, None)
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
    fn usage_accumulator_resets_and_sums_steps() {
        let mut accumulator = UsageAccumulator::default();
        accumulator.record(&zene_llm::TokenUsage {
            prompt_tokens: 10,
            completion_tokens: 2,
            total_tokens: 12,
            cached_tokens: Some(4),
        });
        accumulator.record(&zene_llm::TokenUsage {
            prompt_tokens: 3,
            completion_tokens: 1,
            total_tokens: 4,
            cached_tokens: Some(1),
        });
        assert_eq!(accumulator.total().prompt_tokens, 13);
        assert_eq!(accumulator.total().cached_tokens, Some(5));
        let snapshot = accumulator.snapshot(700, 1000, 70, 3);
        assert_eq!(snapshot.usage.prompt_tokens, 13);
        assert_eq!(snapshot.context_tokens, 700);
        assert_eq!(snapshot.context_window, 1000);
        assert_eq!(snapshot.context_percent, 70);
        assert_eq!(snapshot.context_epoch, 3);
        accumulator.reset();
        assert_eq!(*accumulator.total(), zene_llm::TokenUsage::default());
    }

    #[test]
    fn overflow_retry_state_round_trips_flags() {
        let mut state = OverflowRetryState::default();
        assert_eq!(state.flags(), (false, false));
        state.set_flags(true, false);
        assert_eq!(state.flags(), (true, false));
        state.set_flags(true, true);
        assert_eq!(state.flags(), (true, true));
    }

    #[test]
    fn request_builder_preserves_model_input() {
        let request = build_request(
            "fake",
            vec![Message::user("hello")],
            Vec::new(),
            true,
            None,
        );
        assert_eq!(request.model, "fake");
        assert_eq!(request.messages.len(), 1);
        assert!(request.stream);
    }

    #[test]
    fn accumulator_collects_text_tools_and_usage() {
        let mut accumulator = StreamAccumulator::default();
        assert!(!accumulator.apply(&StreamEvent::TextDelta("hello ".into())));
        assert!(!accumulator.apply(&StreamEvent::ToolCallDelta {
            index: 0,
            id: Some("call-1".into()),
            name: Some("Read".into()),
            arguments: Some("{}".into()),
        }));
        assert!(accumulator.apply(&StreamEvent::Done { usage: None }));
        let (message, usage) = accumulator.finish();
        assert_eq!(message.content.as_deref(), Some("hello "));
        assert_eq!(message.tool_calls.unwrap()[0].name, "Read");
        assert!(usage.is_none());
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
