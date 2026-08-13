//! [`TurnRuntime`] implementation for [`Agent`](crate::Agent).

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use zene_llm::{Message, TokenUsage, ToolCall};
use zene_turn::{
    max_turns_notice, ContextAssemblerPort, EventSinkPort, ModelExecutorPort, PreparedContext,
    StepResult, ToolBatchOutcome, ToolExecutorPort, TurnEnginePorts, TurnRuntime, TurnSessionPort,
};

use crate::events::{emit_event, AgentEvent};
use crate::Agent;

/// Native turn-engine ports for the primary agent runtime.
///
/// This keeps the top-level agent on the explicit port contract while
/// preserving the existing [`TurnRuntime`] implementation as the delegation
/// source. Subagents continue to use the compatibility adapter.
pub(super) struct AgentTurnPorts<'a> {
    agent: &'a mut Agent,
}

impl<'a> AgentTurnPorts<'a> {
    pub(super) fn new(agent: &'a mut Agent) -> Self {
        Self { agent }
    }
}

impl TurnEnginePorts for AgentTurnPorts<'_> {
    type Options = crate::PromptOptions;
}

#[async_trait]
impl TurnSessionPort<crate::PromptOptions> for AgentTurnPorts<'_> {
    fn max_steps(&self) -> u32 {
        <Agent as TurnRuntime>::max_steps(self.agent)
    }

    fn active_turn(&mut self) -> Option<&mut zene_turn::TurnState> {
        <Agent as TurnRuntime>::active_turn(self.agent)
    }

    async fn prepare_turn(&mut self, user_input: &str) -> Result<(), anyhow::Error> {
        <Agent as TurnRuntime>::prepare_turn(self.agent, user_input).await
    }

    fn inject_steer(&mut self, options: &crate::PromptOptions) -> Result<bool, anyhow::Error> {
        <Agent as TurnRuntime>::inject_steer(self.agent, options)
    }

    fn push_assistant(&mut self, message: Message) {
        <Agent as TurnRuntime>::push_assistant(self.agent, message);
    }

    fn on_incomplete_turn(
        &mut self,
        max_steps: u32,
        final_text: &mut String,
        options: &crate::PromptOptions,
    ) -> Result<(), anyhow::Error> {
        <Agent as TurnRuntime>::on_incomplete_turn(self.agent, max_steps, final_text, options)
    }

    async fn finish_turn(&mut self) -> Result<(), anyhow::Error> {
        <Agent as TurnRuntime>::finish_turn(self.agent).await
    }
}

#[async_trait]
impl ContextAssemblerPort<crate::PromptOptions> for AgentTurnPorts<'_> {
    async fn prepare_context(
        &mut self,
        _options: &crate::PromptOptions,
        _cancel: Option<&CancellationToken>,
    ) -> Result<PreparedContext, anyhow::Error> {
        // Agent::run_step assembles context as part of the legacy runtime
        // hook. Keep this deliberate no-op while the native ports migrate.
        Ok(PreparedContext::default())
    }
}

#[async_trait]
impl ModelExecutorPort<crate::PromptOptions> for AgentTurnPorts<'_> {
    async fn run_model(
        &mut self,
        _context: PreparedContext,
        options: &crate::PromptOptions,
        cancel: Option<&CancellationToken>,
    ) -> Result<StepResult, anyhow::Error> {
        <Agent as TurnRuntime>::run_step(self.agent, options, cancel).await
    }

    async fn on_step_usage(
        &mut self,
        usage: &TokenUsage,
        options: &crate::PromptOptions,
    ) -> Result<(), anyhow::Error> {
        <Agent as TurnRuntime>::on_step_usage(self.agent, usage, options).await
    }
}

#[async_trait]
impl ToolExecutorPort<crate::PromptOptions> for AgentTurnPorts<'_> {
    async fn run_tools(
        &mut self,
        tool_calls: &[ToolCall],
        options: &crate::PromptOptions,
        cancel: Option<&CancellationToken>,
    ) -> Result<ToolBatchOutcome, anyhow::Error> {
        <Agent as TurnRuntime>::run_tools(self.agent, tool_calls, options, cancel).await
    }
}

impl EventSinkPort<crate::PromptOptions> for AgentTurnPorts<'_> {
    fn on_step_begin(
        &self,
        turn_id: zene_turn::TurnId,
        step_id: zene_turn::StepId,
        step: u32,
        options: &crate::PromptOptions,
    ) {
        <Agent as TurnRuntime>::on_step_begin(self.agent, turn_id, step_id, step, options);
    }
}

#[async_trait]
impl TurnRuntime for Agent {
    type Options = crate::PromptOptions;

    fn max_steps(&self) -> u32 {
        self.config.max_turns
    }

    fn active_turn(&mut self) -> Option<&mut zene_turn::TurnState> {
        self.active_turn.as_mut()
    }

    fn on_step_begin(
        &self,
        turn_id: zene_turn::TurnId,
        step_id: zene_turn::StepId,
        step: u32,
        options: &Self::Options,
    ) {
        emit_event(
            &options.event_handler,
            AgentEvent::StepBegin {
                turn_id,
                step_id,
                step,
            },
        );
    }

    async fn prepare_turn(&mut self, user_input: &str) -> Result<(), anyhow::Error> {
        self.usage_accumulator.reset();
        self.tool_dedup.reset();
        self.session.ensure_system_message(&self.system_prompt);
        if !self.resume_existing_turn {
            self.session.set_title_from_prompt(user_input);
            self.session.push_message(Message::user(user_input));
        }
        self.resume_existing_turn = false;
        Ok(())
    }

    async fn run_step(
        &mut self,
        options: &Self::Options,
        cancel: Option<&CancellationToken>,
    ) -> Result<StepResult, anyhow::Error> {
        if let Some(turn) = self.active_turn.as_ref() {
            if let Some(step_id) = turn.step_id {
                let turn_id = turn.turn_id.to_string();
                let step_id = step_id.to_string();
                self.session
                    .record_step_started(&turn_id, &step_id, turn.step);
                let idempotency_key = format!("{turn_id}/{step_id}/started");
                let step_event = self.session.record_checkpoint(
                    Some(&turn_id),
                    Some(&step_id),
                    None,
                    "step_started",
                    &idempotency_key,
                );
                self.record_writer.append_execution_link(
                    &idempotency_key,
                    &step_event.id,
                    step_event.sequence,
                )?;
            }
        }
        let (message, usage, had_tool_calls) = Agent::run_step(self, options, cancel).await?;
        Ok(StepResult {
            message,
            usage,
            had_tool_calls,
        })
    }

    async fn on_step_usage(
        &mut self,
        usage: &TokenUsage,
        options: &Self::Options,
    ) -> Result<(), anyhow::Error> {
        self.usage_accumulator.record(usage);
        let tools = self.tool_definitions_for_llm();
        let estimator = self.token_estimator();
        let compaction_config =
            crate::context_config::context_compaction_config(&self.config.compaction);
        let context_usage = self.context.record_step_usage(
            usage,
            &mut self.session,
            &tools,
            &estimator,
            &compaction_config,
        )?;
        let snapshot = self.usage_accumulator.snapshot(
            context_usage.context_tokens,
            context_usage.context_window,
            context_usage.context_percent,
            self.context.epoch(),
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

    async fn run_tools(
        &mut self,
        tool_calls: &[ToolCall],
        options: &Self::Options,
        cancel: Option<&CancellationToken>,
    ) -> Result<ToolBatchOutcome, anyhow::Error> {
        Agent::run_tools(self, tool_calls, options, cancel).await
    }

    fn inject_steer(&mut self, options: &Self::Options) -> Result<bool, anyhow::Error> {
        self.inject_pending_steer(options)
    }

    fn push_assistant(&mut self, message: Message) {
        self.session.push_message(message);
    }

    fn on_incomplete_turn(
        &mut self,
        max_steps: u32,
        final_text: &mut String,
        options: &Self::Options,
    ) -> Result<(), anyhow::Error> {
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
        self.session
            .push_message(Message::assistant(final_text.clone()));
        emit_event(&options.event_handler, AgentEvent::TextDelta { delta });
        Ok(())
    }

    async fn finish_turn(&mut self) -> Result<(), anyhow::Error> {
        self.sync_todos_to_session();
        self.save_session()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_turn_engine_ports<P: TurnEnginePorts>() {}

    #[test]
    fn agent_turn_ports_implements_turn_engine_ports() {
        assert_turn_engine_ports::<AgentTurnPorts<'static>>();
    }
}
