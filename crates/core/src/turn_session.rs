//! Turn-session side effects extracted from [`crate::Agent`].
//!
//! Wave 14 facade: prepare_turn / usage / incomplete-turn / step-started
//! live here so Agent TurnRuntime methods stay wiring-only.

use anyhow::Result;
use tracing::info;
use zene_config::ZeneConfig;
use zene_context::{ContextEngine, EstimateProvider, TokenEstimator};
use zene_llm::{Message, TokenUsage};
use zene_session::{AgentRecordWriter, SessionRecord};
use zene_tools::{ToolCatalog, ToolPolicy, ToolRegistry};
use zene_turn::{max_turns_notice, SteerBuffer};

use crate::events::{emit_event, AgentEvent};
use crate::plan_mode::tool_visible_in_definitions;
use crate::tool_dedup::ToolDedup;
use crate::usage::UsageAccumulator;
use crate::PromptOptions;
use parking_lot::Mutex;
use std::sync::Arc;
use zene_turn::TurnState;

pub(crate) struct PrepareTurnDeps<'a> {
    pub session: &'a mut SessionRecord,
    pub system_prompt: &'a str,
    pub usage_accumulator: &'a mut UsageAccumulator,
    pub tool_dedup: &'a mut ToolDedup,
    pub resume_existing_turn: &'a mut bool,
}

pub(crate) fn prepare_turn(deps: PrepareTurnDeps<'_>, user_input: &str) {
    deps.usage_accumulator.reset();
    deps.tool_dedup.reset();
    deps.session.ensure_system_message(deps.system_prompt);
    if !*deps.resume_existing_turn {
        deps.session.set_title_from_prompt(user_input);
        deps.session.push_message(Message::user(user_input));
    }
    *deps.resume_existing_turn = false;
}

pub(crate) struct StepUsageDeps<'a> {
    pub config: &'a ZeneConfig,
    pub context: &'a mut ContextEngine,
    pub session: &'a mut SessionRecord,
    pub tools: &'a ToolRegistry,
    pub tool_policy: ToolPolicy,
    pub plan_mode_active: bool,
    pub usage_accumulator: &'a mut UsageAccumulator,
}

pub(crate) fn record_step_usage(
    deps: StepUsageDeps<'_>,
    usage: &TokenUsage,
    options: &PromptOptions,
) -> Result<()> {
    deps.usage_accumulator.record(usage);
    let plan_filter = deps.tool_policy.plan_mode && deps.plan_mode_active;
    let tools: Vec<_> = ToolCatalog::definitions(deps.tools)
        .into_iter()
        .filter(|def| tool_visible_in_definitions(&def.name, plan_filter))
        .collect();
    let estimator = TokenEstimator::for_provider(
        EstimateProvider::from_name(&deps.config.provider),
        &deps.config.model,
        deps.config.chars_per_token_for_model(),
    );
    let compaction_config =
        crate::context_config::context_compaction_config(&deps.config.compaction);
    let context_usage = deps.context.record_step_usage(
        usage,
        deps.session,
        &tools,
        &estimator,
        &compaction_config,
    )?;
    let snapshot = deps.usage_accumulator.snapshot(
        context_usage.context_tokens,
        context_usage.context_window,
        context_usage.context_percent,
        deps.context.epoch(),
    );
    emit_event(
        &options.event_handler,
        AgentEvent::UsageUpdate {
            usage: snapshot.usage,
            context_tokens: snapshot.context_tokens,
            context_window: snapshot.context_window,
            context_percent: snapshot.context_percent,
            context_epoch: snapshot.context_epoch,
        },
    );
    Ok(())
}

pub(crate) fn record_step_started(
    session: &mut SessionRecord,
    record_writer: &AgentRecordWriter,
    active_turn: Option<&TurnState>,
) -> Result<()> {
    let Some(turn) = active_turn else {
        return Ok(());
    };
    let Some(step_id) = turn.step_id else {
        return Ok(());
    };
    let turn_id = turn.turn_id.to_string();
    let step_id = step_id.to_string();
    session.record_step_started(&turn_id, &step_id, turn.step);
    let idempotency_key = format!("{turn_id}/{step_id}/started");
    let step_event = session.record_checkpoint(
        Some(&turn_id),
        Some(&step_id),
        None,
        "step_started",
        &idempotency_key,
    );
    record_writer.append_execution_link(&idempotency_key, &step_event.id, step_event.sequence)?;
    Ok(())
}

pub(crate) fn inject_pending_steer(
    steer_enabled: bool,
    steer_buffer: &Arc<Mutex<SteerBuffer>>,
    session: &mut SessionRecord,
    options: &PromptOptions,
) -> bool {
    if !steer_enabled {
        return false;
    }
    let messages = steer_buffer.lock().take_all();
    if messages.is_empty() {
        return false;
    }
    for text in messages {
        info!(steer_chars = text.len(), "steer_injected");
        emit_event(
            &options.event_handler,
            AgentEvent::SteerInput { text: text.clone() },
        );
        session.push_message(Message::user(text));
    }
    true
}

pub(crate) fn on_incomplete_turn(
    session: &mut SessionRecord,
    max_steps: u32,
    final_text: &mut String,
    options: &PromptOptions,
) {
    let notice = max_turns_notice(max_steps);
    let delta = if final_text.trim().is_empty() {
        format!("\n{notice}\n")
    } else {
        format!("\n\n{notice}")
    };
    *final_text = if final_text.trim().is_empty() {
        notice
    } else {
        format!("{final_text}\n\n{notice}")
    };
    session.push_message(Message::assistant(final_text.clone()));
    emit_event(&options.event_handler, AgentEvent::TextDelta { delta });
}
