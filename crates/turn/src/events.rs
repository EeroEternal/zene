//! Shared runtime event envelope for CLI, ACP, Cloud, and recording sinks.

use std::sync::Arc;

use zene_llm::TokenUsage;

use crate::{SessionId, StepId, ToolCallId, TurnId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventSequence(u64);

impl EventSequence {
    pub const fn new(value: u64) -> Self { Self(value) }
    pub const fn value(self) -> u64 { self.0 }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeEvent {
    pub sequence: EventSequence,
    pub session_id: SessionId,
    pub turn_id: Option<TurnId>,
    pub step_id: Option<StepId>,
    pub kind: RuntimeEventKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeEventKind {
    TurnStarted,
    StepStarted { step: u32 },
    TextDelta { delta: String },
    ThoughtDelta { delta: String },
    ToolCall { id: ToolCallId, name: String, arguments: String },
    ToolResult {
        id: ToolCallId,
        name: String,
        content: String,
        is_error: bool,
        duration_ms: Option<u64>,
    },
    UsageUpdate { usage: TokenUsage, context_tokens: u32, context_window: u32, context_percent: u8, context_epoch: u64 },
    TurnEnded { steps: u32 },
    Error { message: String },
    SteerInput { text: String },
    StateChanged { state: String },
}

pub type RuntimeEventHandler = Arc<dyn Fn(RuntimeEvent) + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_sequence_is_ordered() {
        assert!(EventSequence::new(1) < EventSequence::new(2));
        assert_eq!(EventSequence::new(3).value(), 3);
    }

    #[test]
    fn runtime_event_keeps_scope_ids() {
        let event = RuntimeEvent {
            sequence: EventSequence::new(1),
            session_id: SessionId::from_string("session"),
            turn_id: Some(TurnId::new()),
            step_id: Some(StepId::new()),
            kind: RuntimeEventKind::StepStarted { step: 1 },
        };
        assert_eq!(event.session_id.as_str(), "session");
    }
}
