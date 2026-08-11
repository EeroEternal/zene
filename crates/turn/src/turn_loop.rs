use anyhow::Result;
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use tracing::debug;
use zene_llm::{Message, TokenUsage, ToolCall};

use crate::state::{aborted_error, is_cancelled, max_turns_notice, StepId, TurnId, TurnState};

/// Outcome of one LLM step within a turn.
pub struct StepResult {
    pub message: Message,
    pub usage: Option<TokenUsage>,
    pub had_tool_calls: bool,
}

/// Runtime hooks for the generic turn loop (implemented by `zene-core::Agent`).
#[async_trait]
pub trait TurnRuntime {
    type Options: Send + Sync;

    fn max_steps(&self) -> u32;
    fn active_turn(&mut self) -> Option<&mut TurnState>;

    fn on_step_begin(&self, turn_id: TurnId, step_id: StepId, step: u32, options: &Self::Options);

    async fn prepare_turn(&mut self, user_input: &str) -> Result<()>;
    async fn run_step(
        &mut self,
        options: &Self::Options,
        cancel: Option<&CancellationToken>,
    ) -> Result<StepResult>;
    async fn on_step_usage(&mut self, usage: &TokenUsage, options: &Self::Options) -> Result<()>;
    async fn run_tools(
        &mut self,
        tool_calls: &[ToolCall],
        options: &Self::Options,
        cancel: Option<&CancellationToken>,
    ) -> Result<()>;
    fn inject_steer(&mut self, options: &Self::Options) -> Result<bool>;
    fn push_assistant(&mut self, message: Message);
    fn on_incomplete_turn(
        &mut self,
        max_steps: u32,
        final_text: &mut String,
        options: &Self::Options,
    ) -> Result<()>;
    async fn finish_turn(&mut self) -> Result<()>;
}

/// Multi-step turn: LLM → tools → steer until completion or max_steps.
pub async fn run_turn_loop<R: TurnRuntime>(
    runtime: &mut R,
    user_input: &str,
    options: &R::Options,
    cancel: Option<&CancellationToken>,
) -> Result<String> {
    runtime.prepare_turn(user_input).await?;

    let mut final_text = String::new();
    let max_steps = runtime.max_steps();
    let mut completed = false;
    let mut steps_done = 0u32;

    loop {
        if max_steps > 0 && steps_done >= max_steps {
            break;
        }
        steps_done = steps_done.saturating_add(1);

        if is_cancelled(cancel) {
            return Err(aborted_error());
        }

        let (turn_id, step_id, step_num) = {
            let turn = runtime
                .active_turn()
                .expect("active turn during run_turn_loop");
            let step_id = turn.next_step_id();
            (turn.turn_id, step_id, turn.step)
        };

        debug!(
            turn_id = %turn_id,
            step_id = %step_id,
            step = step_num,
            "step_begin"
        );
        runtime.on_step_begin(turn_id, step_id, step_num, options);

        let step_result = runtime.run_step(options, cancel).await;
        debug!(
            turn_id = %turn_id,
            step_id = %step_id,
            step = step_num,
            ok = step_result.is_ok(),
            "step_end"
        );
        let StepResult {
            message: assistant_message,
            usage,
            had_tool_calls,
        } = step_result?;

        if let Some(usage) = &usage {
            runtime.on_step_usage(usage, options).await?;
        }

        if had_tool_calls {
            if let Some(tool_calls) = assistant_message.tool_calls.clone() {
                runtime.push_assistant(assistant_message);
                runtime.run_tools(&tool_calls, options, cancel).await?;
                if runtime.inject_steer(options)? {
                    continue;
                }
                continue;
            }
        }

        runtime.push_assistant(assistant_message.clone());
        if runtime.inject_steer(options)? {
            continue;
        }

        final_text = assistant_message.content.unwrap_or_default();
        completed = true;
        break;
    }

    if !completed {
        runtime.on_incomplete_turn(max_steps, &mut final_text, options)?;
    }

    runtime.finish_turn().await?;
    Ok(final_text)
}
