//! Main-agent tool-batch orchestration extracted from [`crate::Agent`].
//!
//! Wave 14: execution still goes through [`DefaultToolExecutor`]; this module
//! owns ToolStarted/Completed checkpoints, session event writeback, and
//! mode/permission recording so `Agent::run_tools` stays wiring-only.

use std::sync::Arc;

use anyhow::{bail, Result};
use tokio_util::sync::CancellationToken;
use zene_config::ZeneConfig;
use zene_llm::{Message, ToolCall};
use zene_permission::SharedApprovalBroker;
use zene_sandbox::Sandbox;
use zene_session::{AgentRecordWriter, ExecutionCheckpointState, RecordEntry, SessionRecord};
use zene_tools::{
    RuntimeScope, SharedAskUserPrompter, SharedBackgroundTasks, SharedPlanMode, SharedTodoStore,
    SharedToolPermission, ToolRegistry,
};
use zene_turn::ToolBatchOutcome;
use zene_hooks::HookRunner;

use crate::plan_mode::PlanApprovalPrompter;
use crate::subagent::CoreSubagentRunner;
use crate::tool_dedup::ToolDedup;
use crate::tool_executor::{DefaultToolExecutor, ToolExecutorDeps};
use crate::PromptOptions;

/// Mutable Agent pieces needed to run and persist one tool batch.
pub(crate) struct ToolBatchDeps<'a> {
    pub config: &'a ZeneConfig,
    pub tools: Arc<ToolRegistry>,
    pub sandbox: Arc<dyn Sandbox>,
    pub permission: SharedToolPermission,
    pub approval_broker: Option<SharedApprovalBroker>,
    pub plan_mode: SharedPlanMode,
    pub plan_approval: &'a PlanApprovalPrompter,
    pub todos: SharedTodoStore,
    pub ask_user: SharedAskUserPrompter,
    pub background: SharedBackgroundTasks,
    pub runtime_scope: &'a RuntimeScope,
    pub hooks: &'a HookRunner,
    pub session: &'a mut SessionRecord,
    pub record_writer: &'a AgentRecordWriter,
    pub tool_dedup: &'a mut ToolDedup,
    pub turn_id: Option<String>,
    pub step_id: Option<String>,
}

/// Record ToolStarted, execute via [`DefaultToolExecutor`], then write results back.
pub(crate) async fn run_tool_batch(
    deps: ToolBatchDeps<'_>,
    tool_calls: &[ToolCall],
    options: &PromptOptions,
    cancel: Option<&CancellationToken>,
) -> Result<ToolBatchOutcome> {
    debug_assert_eq!(
        deps.runtime_scope.session_policy.persistence,
        zene_tools::SessionPersistence::Durable,
        "main-agent tool batch expects durable SessionPolicy"
    );
    debug_assert!(
        deps.runtime_scope.tool_policy.plan_mode
            && deps.runtime_scope.tool_policy.ask_user
            && deps.runtime_scope.tool_policy.hooks,
        "main-agent tool batch expects ToolPolicy::agent()"
    );
    let turn_id = deps.turn_id.as_deref();
    let step_id = deps.step_id.as_deref();

    for call in tool_calls {
        let idempotency_key = append_tool_execution_checkpoint(
            deps.record_writer,
            turn_id,
            step_id,
            &call.id,
            ExecutionCheckpointState::ToolStarted,
        )?;
        let tool_event =
            deps.session
                .record_tool_call(turn_id, step_id, &call.id, &call.name, &call.arguments);
        deps.session.record_checkpoint(
            turn_id,
            step_id,
            Some(&call.id),
            "tool_started",
            &idempotency_key,
        );
        deps.record_writer.append_execution_link(
            &idempotency_key,
            &tool_event.id,
            tool_event.sequence,
        )?;
    }

    let subagent_runner = Arc::new(
        CoreSubagentRunner::new(deps.config.clone()).with_broker(deps.approval_broker.clone()),
    );
    let result = {
        let executor = DefaultToolExecutor::new(ToolExecutorDeps {
            tools: Arc::clone(&deps.tools),
            sandbox: Arc::clone(&deps.sandbox),
            permission: Arc::clone(&deps.permission),
            approval_broker: deps.approval_broker.clone(),
            plan_mode: Arc::clone(&deps.plan_mode),
            plan_approval: deps.plan_approval,
            todos: Arc::clone(&deps.todos),
            ask_user: Arc::clone(&deps.ask_user),
            background: Arc::clone(&deps.background),
            subagent: Some(deps.runtime_scope.env(subagent_runner)),
            hooks: deps.hooks,
        });
        executor
            .execute(
                tool_calls,
                options,
                cancel,
                &deps.session.meta.id,
                deps.sandbox.workdir(),
                deps.tool_dedup,
            )
            .await?
    };

    if !result.mode_changes.is_empty() {
        for mode_id in &result.mode_changes {
            deps.session.record_mode_changed(mode_id);
        }
    }
    for decision in &result.permission_decisions {
        deps.session.record_permission_decision(
            turn_id,
            step_id,
            &decision.tool_call_id,
            &decision.tool_name,
            decision.allowed,
        );
    }
    for message in result.messages {
        let content = message.content;
        let idempotency_key = append_tool_execution_checkpoint(
            deps.record_writer,
            turn_id,
            step_id,
            &message.call.id,
            ExecutionCheckpointState::ToolCompleted,
        )?;
        let result_event = deps.session.record_tool_result(
            turn_id,
            step_id,
            &message.call.id,
            &message.call.name,
            &content,
            message.is_error,
            message.duration_ms,
        );
        deps.session.record_checkpoint(
            turn_id,
            step_id,
            Some(&message.call.id),
            "tool_completed",
            &idempotency_key,
        );
        deps.record_writer.append_execution_link(
            &idempotency_key,
            &result_event.id,
            result_event.sequence,
        )?;
        deps.session.push_message(Message::tool_result_with_error(
            &message.call.id,
            &message.call.name,
            content,
            message.is_error,
        ));
    }
    Ok(result.outcome)
}

pub(crate) fn append_tool_execution_checkpoint(
    record_writer: &AgentRecordWriter,
    turn_id: Option<&str>,
    step_id: Option<&str>,
    tool_call_id: &str,
    state: ExecutionCheckpointState,
) -> Result<String> {
    let state_name = match &state {
        ExecutionCheckpointState::ToolStarted => "started",
        ExecutionCheckpointState::ToolCompleted => "completed",
        _ => bail!("invalid tool execution checkpoint state"),
    };
    let idempotency_key = format!(
        "tool/{}/{}/{tool_call_id}/{state_name}",
        turn_id.unwrap_or("unknown"),
        step_id.unwrap_or("unknown"),
    );
    record_writer.append_execution_checkpoint(&RecordEntry::ExecutionCheckpoint {
        turn_id: turn_id.unwrap_or("unknown").to_string(),
        step_id: step_id.map(str::to_string),
        tool_call_id: Some(tool_call_id.to_string()),
        state,
        idempotency_key: idempotency_key.clone(),
        context_epoch: None,
        model_request_hash: None,
        ts: chrono::Utc::now(),
    })?;
    Ok(idempotency_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_checkpoint_key_includes_ids_and_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let writer = AgentRecordWriter::from_path(dir.path().join("record.jsonl")).expect("writer");
        let key = append_tool_execution_checkpoint(
            &writer,
            Some("turn-1"),
            Some("step-2"),
            "call-3",
            ExecutionCheckpointState::ToolStarted,
        )
        .expect("checkpoint");
        assert_eq!(key, "tool/turn-1/step-2/call-3/started");
    }
}
