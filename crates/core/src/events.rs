use std::sync::Arc;

use zene_llm::TokenUsage;

use crate::turn::{StepId, TurnId};

pub type EventHandler = Arc<dyn Fn(AgentEvent) + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentEvent {
    TurnStart {
        turn_id: TurnId,
    },
    StepBegin {
        turn_id: TurnId,
        step_id: StepId,
        step: u32,
    },
    TextDelta {
        delta: String,
    },
    ThoughtDelta {
        delta: String,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    ToolResult {
        id: String,
        name: String,
        content: String,
        is_error: bool,
        duration_ms: Option<u64>,
    },
    /// Session operating mode changed (`default` / `plan`).
    ModeChanged {
        mode_id: String,
    },
    /// Cumulative turn token usage after an LLM step.
    UsageUpdate {
        usage: TokenUsage,
        context_tokens: u32,
        context_window: u32,
        context_percent: u8,
        context_epoch: u64,
    },
    TurnEnd {
        turn_id: TurnId,
        steps: u32,
    },
    Error {
        message: String,
    },
    SteerInput {
        text: String,
    },
}

pub fn emit_event(handler: &Option<EventHandler>, event: AgentEvent) {
    if let Some(handler) = handler {
        handler(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn event_handler_collects_turn_lifecycle() {
        let collected = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&collected);
        let handler: EventHandler = Arc::new(move |event| {
            sink.lock().unwrap().push(event);
        });
        let options = Some(Arc::clone(&handler));

        let turn_id = TurnId::new();
        let step_id = StepId::new();

        emit_event(&options, AgentEvent::TurnStart { turn_id });
        emit_event(
            &options,
            AgentEvent::StepBegin {
                turn_id,
                step_id,
                step: 1,
            },
        );
        emit_event(
            &options,
            AgentEvent::ToolCall {
                id: "call_1".to_string(),
                name: "Read".to_string(),
                arguments: r#"{"path":"foo.rs"}"#.to_string(),
            },
        );
        emit_event(
            &options,
            AgentEvent::ToolResult {
                id: "call_1".to_string(),
                name: "Read".to_string(),
                content: "file contents".to_string(),
                is_error: false,
                duration_ms: Some(12),
            },
        );
        emit_event(&options, AgentEvent::TurnEnd { turn_id, steps: 1 });

        let events = collected.lock().unwrap();
        assert_eq!(events.len(), 5);
        assert!(matches!(events[0], AgentEvent::TurnStart { .. }));
        assert!(matches!(events[1], AgentEvent::StepBegin { .. }));
        assert!(matches!(events[2], AgentEvent::ToolCall { .. }));
        assert!(matches!(events[3], AgentEvent::ToolResult { .. }));
        assert!(matches!(events[4], AgentEvent::TurnEnd { .. }));
    }
}
