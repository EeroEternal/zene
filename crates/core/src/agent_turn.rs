//! [`TurnRuntime`] implementation for [`Agent`](crate::Agent).

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use zene_llm::{Message, TokenUsage, ToolCall};
use zene_turn::{max_turns_notice, StepResult, ToolBatchOutcome, TurnRuntime};

use crate::events::{emit_event, AgentEvent};
use crate::Agent;

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
                self.session.record_checkpoint(
                    Some(&turn_id),
                    Some(&step_id),
                    None,
                    "step_started",
                    &format!("{turn_id}/{step_id}/started"),
                );
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
