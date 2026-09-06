//! Shared runtime event envelope for CLI, ACP, Cloud, and recording sinks.

use std::sync::Arc;

use zene_llm::TokenUsage;

use crate::{SessionId, StepId, TurnId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventSequence(u64);

impl EventSequence {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProjectionToolOutput {
    pub message_index: usize,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub kind: String,
    pub handle_reference: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProjectionInjectedSource {
    pub message_index: usize,
    pub kind: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProjectionPrefixCache {
    pub prefix_end: usize,
    pub body_end: usize,
    pub tail_decoration_count: usize,
    pub prefix_fingerprint: Option<String>,
    pub break_kind: String,
    pub cached_tokens: Option<u64>,
    pub gateway_hit_tokens: Option<u64>,
    /// Gateway-reported semantic-anchor alignment of the served prefix
    /// (Cortex `usage.gateway_anchor_aligned`).
    pub anchor_aligned: Option<bool>,
    pub unchanged_reprocessed_est: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeEvent {
    pub sequence: EventSequence,
    pub session_id: SessionId,
    pub turn_id: Option<TurnId>,
    pub step_id: Option<StepId>,
    pub kind: RuntimeEventKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProjectionReadyEvent {
    pub source_message_count: usize,
    pub projected_message_count: usize,
    pub source_event_count: usize,
    pub active_event_count: usize,
    pub cache_drift_detected: bool,
    pub used_materialized_fallback: bool,
    pub fallback_reason: Option<String>,
    pub active_branch_id: Option<String>,
    pub active_path_start_sequence: Option<u64>,
    pub injected: Vec<String>,
    pub retained_message_count: usize,
    pub retained_turn_count: usize,
    pub dropped_event_count: usize,
    pub truncated_message_count: usize,
    pub compaction_event_ids: Vec<String>,
    pub tool_output_provenance: Vec<ProjectionToolOutput>,
    pub retained_turn_ids: Vec<String>,
    pub injected_sources: Vec<ProjectionInjectedSource>,
    pub delivery: String,
    pub delivery_tail_start: Option<usize>,
    pub estimate_tokens: u32,
    pub context_epoch: u64,
    pub prefix_cache: ProjectionPrefixCache,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeEventKind {
    TurnStarted,
    StepStarted {
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
    UsageUpdate {
        usage: TokenUsage,
        context_tokens: u32,
        context_window: u32,
        context_percent: u8,
        context_epoch: u64,
    },
    ProjectionReady(Box<ProjectionReadyEvent>),
    TurnEnded {
        steps: u32,
    },
    Error {
        message: String,
    },
    SteerInput {
        text: String,
    },
    StateChanged {
        state: String,
    },
    ApprovalRequested {
        request_id: String,
        tool_name: String,
        arguments: String,
        tool_call_id: Option<String>,
    },
    ApprovalResolved {
        request_id: String,
        allowed: bool,
    },
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
