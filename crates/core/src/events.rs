use std::sync::Arc;

use zene_llm::TokenUsage;

use zene_turn::{
    EventSequence, ProjectionInjectedSource, ProjectionPrefixCache, ProjectionReadyEvent,
    ProjectionToolOutput, RuntimeEvent, RuntimeEventHandler, RuntimeEventKind, SessionId, StepId,
    TurnId,
};

pub fn projection_ready_event_from_explain(
    explain: &zene_context::ProjectionExplain,
) -> ProjectionReadyEvent {
    ProjectionReadyEvent {
        source_message_count: explain.source_message_count,
        projected_message_count: explain.projected_message_count,
        source_event_count: explain.source_event_count,
        active_event_count: explain.active_event_count,
        cache_drift_detected: explain.cache_drift_detected,
        used_materialized_fallback: explain.used_materialized_fallback,
        fallback_reason: explain.fallback_reason.clone(),
        active_branch_id: explain.active_branch_id.clone(),
        active_path_start_sequence: explain.active_path_start_sequence,
        injected: explain.injected.clone(),
        retained_message_count: explain.retained_message_count,
        retained_turn_count: explain.retained_turn_count,
        dropped_event_count: explain.dropped_event_count,
        truncated_message_count: explain.truncated_message_count,
        compaction_event_ids: explain.compaction_event_ids.clone(),
        tool_output_provenance: explain
            .tool_output_provenance
            .iter()
            .map(|item| ProjectionToolOutput {
                message_index: item.message_index,
                tool_call_id: item.tool_call_id.clone(),
                tool_name: item.tool_name.clone(),
                kind: item.kind.clone(),
                handle_reference: item.handle_reference.clone(),
            })
            .collect(),
        retained_turn_ids: explain.retained_turn_ids.clone(),
        injected_sources: explain
            .injected_sources
            .iter()
            .map(|item| ProjectionInjectedSource {
                message_index: item.message_index,
                kind: item.kind.clone(),
                source: item.source.clone(),
            })
            .collect(),
        delivery: explain.delivery.as_str().to_string(),
        delivery_tail_start: explain.delivery_tail_start,
        estimate_tokens: explain.estimate_tokens,
        context_epoch: explain.context_epoch,
        prefix_cache: ProjectionPrefixCache {
            prefix_end: explain.prefix_cache.prefix_end,
            body_end: explain.prefix_cache.body_end,
            tail_decoration_count: explain.prefix_cache.tail_decoration_count,
            prefix_fingerprint: explain.prefix_cache.prefix_fingerprint.clone(),
            break_kind: explain.prefix_cache.break_kind.clone(),
            cached_tokens: explain.prefix_cache.cached_tokens,
            gateway_hit_tokens: explain.prefix_cache.gateway_hit_tokens,
            anchor_aligned: explain.prefix_cache.anchor_aligned,
            unchanged_reprocessed_est: explain.prefix_cache.unchanged_reprocessed_est,
        },
    }
}

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
                    id: id.clone(),
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
                    id: id.clone(),
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
            AgentEvent::ProjectionReady(event) => (
                current_turn,
                current_step,
                RuntimeEventKind::ProjectionReady(event.clone()),
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
    ProjectionReady(Box<ProjectionReadyEvent>),
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
        handler(AgentEvent::ProjectionReady(Box::new(
            ProjectionReadyEvent {
                source_message_count: 4,
                projected_message_count: 3,
                source_event_count: 7,
                active_event_count: 5,
                active_branch_id: Some("branch".into()),
                active_path_start_sequence: Some(3),
                injected: vec!["compaction_summary".into()],
                retained_message_count: 3,
                retained_turn_count: 1,
                dropped_event_count: 2,
                truncated_message_count: 1,
                compaction_event_ids: vec!["compact-1".into()],
                tool_output_provenance: vec![ProjectionToolOutput {
                    message_index: 2,
                    tool_call_id: Some("call-1".into()),
                    tool_name: Some("Read".into()),
                    kind: "handle".into(),
                    handle_reference: Some("/tmp/output".into()),
                }],
                retained_turn_ids: vec![turn_id.to_string()],
                injected_sources: vec![ProjectionInjectedSource {
                    message_index: 0,
                    kind: "compaction_summary".into(),
                    source: "compaction_event".into(),
                }],
                delivery: "full".into(),
                estimate_tokens: 128,
                context_epoch: 2,
                prefix_cache: ProjectionPrefixCache {
                    prefix_end: 1,
                    body_end: 3,
                    prefix_fingerprint: Some("abc".into()),
                    break_kind: "none".into(),
                    ..Default::default()
                },
                ..Default::default()
            },
        )));

        let events = collected.lock().unwrap();
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].sequence.value(), 1);
        assert_eq!(events[3].sequence.value(), 4);
        assert_eq!(events[3].turn_id, Some(turn_id));
        assert_eq!(events[3].step_id, Some(step_id));
        match &events[3].kind {
            RuntimeEventKind::ProjectionReady(ev) => {
                assert_eq!(ev.source_message_count, 4);
                assert_eq!(ev.projected_message_count, 3);
                assert_eq!(ev.source_event_count, 7);
                assert_eq!(ev.active_event_count, 5);
                assert!(!ev.cache_drift_detected);
                assert!(!ev.used_materialized_fallback);
                assert_eq!(ev.fallback_reason, None);
                assert_eq!(ev.active_branch_id.as_deref(), Some("branch"));
                assert_eq!(ev.active_path_start_sequence, Some(3));
                assert_eq!(ev.injected, &["compaction_summary"]);
                assert_eq!(ev.tool_output_provenance.len(), 1);
                assert_eq!(ev.tool_output_provenance[0].kind, "handle");
                assert_eq!(ev.retained_turn_ids, &[turn_id.to_string()]);
                assert_eq!(ev.injected_sources[0].source, "compaction_event");
                assert_eq!(ev.retained_message_count, 3);
                assert_eq!(ev.retained_turn_count, 1);
                assert_eq!(ev.dropped_event_count, 2);
                assert_eq!(ev.truncated_message_count, 1);
                assert_eq!(ev.compaction_event_ids, &["compact-1"]);
                assert_eq!(ev.delivery, "full");
                assert_eq!(ev.estimate_tokens, 128);
                assert_eq!(ev.context_epoch, 2);
                assert_eq!(ev.prefix_cache.break_kind, "none");
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
