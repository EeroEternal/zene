use std::sync::Arc;

use zene_llm::TokenUsage;

use zene_turn::{
    EventSequence, RuntimeEvent, RuntimeEventHandler, RuntimeEventKind, SessionId, StepId,
    ToolCallId, TurnId,
};

pub type EventHandler = Arc<dyn Fn(AgentEvent) + Send + Sync>;

/// Adapt legacy AgentEvent consumers to the shared runtime event envelope.
pub fn runtime_event_handler(
    session_id: SessionId,
    legacy: Option<EventHandler>,
    runtime: Option<RuntimeEventHandler>,
) -> EventHandler {
    let sequence = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let scope = Arc::new(std::sync::Mutex::new((None, None)));
    Arc::new(move |event| {
        if let Ok(mut current) = scope.lock() {
            match &event {
                AgentEvent::TurnStart { turn_id } => {
                    current.0 = Some(*turn_id);
                    current.1 = None;
                }
                AgentEvent::StepBegin {
                    turn_id, step_id, ..
                } => {
                    current.0 = Some(*turn_id);
                    current.1 = Some(*step_id);
                }
                AgentEvent::TurnEnd { .. } => current.1 = None,
                _ => {}
            }
        }
        if let Some(handler) = &legacy {
            handler(event.clone());
        }
        let Some(handler) = &runtime else { return };
        let sequence =
            EventSequence::new(sequence.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1);
        let (current_turn, current_step) = scope
            .lock()
            .map(|current| (current.0, current.1))
            .unwrap_or((None, None));
        let (turn_id, step_id, kind) = match &event {
            AgentEvent::TurnStart { turn_id } => {
                (Some(*turn_id), None, RuntimeEventKind::TurnStarted)
            }
            AgentEvent::StepBegin {
                turn_id,
                step_id,
                step,
            } => (
                Some(*turn_id),
                Some(*step_id),
                RuntimeEventKind::StepStarted { step: *step },
            ),
            AgentEvent::TextDelta { delta } => (
                current_turn,
                current_step,
                RuntimeEventKind::TextDelta {
                    delta: delta.clone(),
                },
            ),
            AgentEvent::ThoughtDelta { delta } => (
                current_turn,
                current_step,
                RuntimeEventKind::ThoughtDelta {
                    delta: delta.clone(),
                },
            ),
            AgentEvent::ToolCall {
                id,
                name,
                arguments,
            } => (
                current_turn,
                current_step,
                RuntimeEventKind::ToolCall {
                    id: ToolCallId::from_string(id.clone()),
                    name: name.clone(),
                    arguments: arguments.clone(),
                },
            ),
            AgentEvent::ToolResult {
                id,
                name,
                content,
                is_error,
                duration_ms,
            } => (
                current_turn,
                current_step,
                RuntimeEventKind::ToolResult {
                    id: ToolCallId::from_string(id.clone()),
                    name: name.clone(),
                    content: content.clone(),
                    is_error: *is_error,
                    duration_ms: *duration_ms,
                },
            ),
            AgentEvent::UsageUpdate {
                usage,
                context_tokens,
                context_window,
                context_percent,
                context_epoch,
            } => (
                current_turn,
                current_step,
                RuntimeEventKind::UsageUpdate {
                    usage: *usage,
                    context_tokens: *context_tokens,
                    context_window: *context_window,
                    context_percent: *context_percent,
                    context_epoch: *context_epoch,
                },
            ),
            AgentEvent::ProjectionReady {
                source_message_count,
                projected_message_count,
                source_event_count,
                active_event_count,
                cache_drift_detected,
                used_materialized_fallback,
                fallback_reason,
                active_branch_id,
                active_path_start_sequence,
                injected,
                retained_message_count,
                retained_turn_count,
                dropped_event_count,
                truncated_message_count,
                compaction_event_ids,
                delivery,
                delivery_tail_start,
                estimate_tokens,
                context_epoch,
            } => (
                current_turn,
                current_step,
                RuntimeEventKind::ProjectionReady {
                    source_message_count: *source_message_count,
                    projected_message_count: *projected_message_count,
                    source_event_count: *source_event_count,
                    active_event_count: *active_event_count,
                    cache_drift_detected: *cache_drift_detected,
                    used_materialized_fallback: *used_materialized_fallback,
                    fallback_reason: fallback_reason.clone(),
                    active_branch_id: active_branch_id.clone(),
                    active_path_start_sequence: *active_path_start_sequence,
                    injected: injected.clone(),
                    retained_message_count: *retained_message_count,
                    retained_turn_count: *retained_turn_count,
                    dropped_event_count: *dropped_event_count,
                    truncated_message_count: *truncated_message_count,
                    compaction_event_ids: compaction_event_ids.clone(),
                    delivery: delivery.clone(),
                    delivery_tail_start: *delivery_tail_start,
                    estimate_tokens: *estimate_tokens,
                    context_epoch: *context_epoch,
                },
            ),
            AgentEvent::TurnEnd { turn_id, steps } => (
                Some(*turn_id),
                None,
                RuntimeEventKind::TurnEnded { steps: *steps },
            ),
            AgentEvent::Error { message } => (
                current_turn,
                current_step,
                RuntimeEventKind::Error {
                    message: message.clone(),
                },
            ),
            AgentEvent::SteerInput { text } => (
                current_turn,
                current_step,
                RuntimeEventKind::SteerInput { text: text.clone() },
            ),
            AgentEvent::ModeChanged { mode_id } => (
                current_turn,
                current_step,
                RuntimeEventKind::StateChanged {
                    state: mode_id.clone(),
                },
            ),
        };
        handler(RuntimeEvent {
            sequence,
            session_id: session_id.clone(),
            turn_id,
            step_id,
            kind,
        });
    })
}

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
    ProjectionReady {
        source_message_count: usize,
        projected_message_count: usize,
        source_event_count: usize,
        active_event_count: usize,
        cache_drift_detected: bool,
        used_materialized_fallback: bool,
        fallback_reason: Option<String>,
        active_branch_id: Option<String>,
        active_path_start_sequence: Option<u64>,
        injected: Vec<String>,
        retained_message_count: usize,
        retained_turn_count: usize,
        dropped_event_count: usize,
        truncated_message_count: usize,
        compaction_event_ids: Vec<String>,
        delivery: String,
        delivery_tail_start: Option<usize>,
        estimate_tokens: u32,
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
    fn runtime_adapter_assigns_sequence_and_scope() {
        let collected = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&collected);
        let runtime: RuntimeEventHandler = Arc::new(move |event| {
            sink.lock().unwrap().push(event);
        });
        let handler = runtime_event_handler(SessionId::from_string("session"), None, Some(runtime));
        let turn_id = TurnId::new();
        let step_id = StepId::new();
        handler(AgentEvent::TurnStart { turn_id });
        handler(AgentEvent::StepBegin {
            turn_id,
            step_id,
            step: 1,
        });
        handler(AgentEvent::TextDelta {
            delta: "hello".into(),
        });
        handler(AgentEvent::ProjectionReady {
            source_message_count: 4,
            projected_message_count: 3,
            source_event_count: 7,
            active_event_count: 5,
            cache_drift_detected: false,
            used_materialized_fallback: false,
            fallback_reason: None,
            active_branch_id: Some("branch".into()),
            active_path_start_sequence: Some(3),
            injected: vec!["compaction_summary".into()],
            retained_message_count: 3,
            retained_turn_count: 1,
            dropped_event_count: 2,
            truncated_message_count: 1,
            compaction_event_ids: vec!["compact-1".into()],
            delivery: "full".into(),
            delivery_tail_start: None,
            estimate_tokens: 128,
            context_epoch: 2,
        });

        let events = collected.lock().unwrap();
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].sequence.value(), 1);
        assert_eq!(events[3].sequence.value(), 4);
        assert_eq!(events[3].turn_id, Some(turn_id));
        assert_eq!(events[3].step_id, Some(step_id));
        match &events[3].kind {
            RuntimeEventKind::ProjectionReady {
                source_message_count,
                projected_message_count,
                source_event_count,
                active_event_count,
                cache_drift_detected,
                used_materialized_fallback,
                fallback_reason,
                active_branch_id,
                active_path_start_sequence,
                injected,
                retained_message_count,
                retained_turn_count,
                dropped_event_count,
                truncated_message_count,
                compaction_event_ids,
                delivery,
                delivery_tail_start,
                estimate_tokens,
                context_epoch,
            } => {
                assert_eq!(*source_message_count, 4);
                assert_eq!(*projected_message_count, 3);
                assert_eq!(*source_event_count, 7);
                assert_eq!(*active_event_count, 5);
                assert!(!cache_drift_detected);
                assert!(!used_materialized_fallback);
                assert_eq!(fallback_reason, &None);
                assert_eq!(active_branch_id.as_deref(), Some("branch"));
                assert_eq!(*active_path_start_sequence, Some(3));
                assert_eq!(injected, &["compaction_summary"]);
                assert_eq!(*retained_message_count, 3);
                assert_eq!(*retained_turn_count, 1);
                assert_eq!(*dropped_event_count, 2);
                assert_eq!(*truncated_message_count, 1);
                assert_eq!(compaction_event_ids, &["compact-1"]);
                assert_eq!(delivery, "full");
                assert_eq!(*delivery_tail_start, None);
                assert_eq!(*estimate_tokens, 128);
                assert_eq!(*context_epoch, 2);
            }
            other => panic!("unexpected runtime event: {other:?}"),
        }
    }

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
