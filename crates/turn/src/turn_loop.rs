use anyhow::Result;
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use tracing::debug;
use zene_llm::{Message, TokenUsage, ToolCall};

use crate::state::{aborted_error, is_cancelled, StepId, TurnId, TurnState};

/// Outcome of one LLM step within a turn.
pub struct StepResult {
    pub message: Message,
    pub usage: Option<TokenUsage>,
    pub had_tool_calls: bool,
}

/// Controls whether a completed tool batch should cause another model call.
///
/// `Continue` preserves the normal LLM → tools → LLM loop. `Terminate` is the
/// escape hatch for executors and extensions that have produced the terminal
/// result for a turn and must not trigger an automatic follow-up model call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolBatchOutcome {
    Continue,
    Terminate,
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
    ) -> Result<ToolBatchOutcome>;
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
                let tool_outcome = runtime.run_tools(&tool_calls, options, cancel).await?;
                if tool_outcome == ToolBatchOutcome::Terminate {
                    completed = true;
                    break;
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

    fn block_on<F: Future>(mut future: F) -> F::Output {
        fn no_op(_: *const ()) {}
        fn clone(_: *const ()) -> RawWaker {
            RawWaker::new(std::ptr::null(), &VTABLE)
        }
        static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, no_op, no_op, no_op);
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
        let mut context = Context::from_waker(&waker);
        let mut future = unsafe { Pin::new_unchecked(&mut future) };
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(value) => return value,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    struct FakeRuntime {
        active: Option<TurnState>,
        model_calls: Arc<AtomicUsize>,
        tool_outcome: ToolBatchOutcome,
    }

    #[async_trait]
    impl TurnRuntime for FakeRuntime {
        type Options = ();

        fn max_steps(&self) -> u32 {
            4
        }

        fn active_turn(&mut self) -> Option<&mut TurnState> {
            self.active.as_mut()
        }

        fn on_step_begin(
            &self,
            _turn_id: TurnId,
            _step_id: StepId,
            _step: u32,
            _options: &Self::Options,
        ) {
        }

        async fn prepare_turn(&mut self, _user_input: &str) -> Result<()> {
            self.active = Some(TurnState::begin());
            Ok(())
        }

        async fn run_step(
            &mut self,
            _options: &Self::Options,
            _cancel: Option<&CancellationToken>,
        ) -> Result<StepResult> {
            let call = self.model_calls.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                Ok(StepResult {
                    message: Message::assistant_with_tools(
                        None,
                        vec![ToolCall {
                            id: "call_1".into(),
                            name: "Terminal".into(),
                            arguments: "{}".into(),
                        }],
                    ),
                    usage: None,
                    had_tool_calls: true,
                })
            } else {
                Ok(StepResult {
                    message: Message::assistant("final"),
                    usage: None,
                    had_tool_calls: false,
                })
            }
        }

        async fn on_step_usage(
            &mut self,
            _usage: &TokenUsage,
            _options: &Self::Options,
        ) -> Result<()> {
            Ok(())
        }

        async fn run_tools(
            &mut self,
            _tool_calls: &[ToolCall],
            _options: &Self::Options,
            _cancel: Option<&CancellationToken>,
        ) -> Result<ToolBatchOutcome> {
            Ok(self.tool_outcome)
        }

        fn inject_steer(&mut self, _options: &Self::Options) -> Result<bool> {
            Ok(false)
        }

        fn push_assistant(&mut self, _message: Message) {}

        fn on_incomplete_turn(
            &mut self,
            _max_steps: u32,
            _final_text: &mut String,
            _options: &Self::Options,
        ) -> Result<()> {
            Ok(())
        }

        async fn finish_turn(&mut self) -> Result<()> {
            self.active = None;
            Ok(())
        }
    }

    #[test]
    fn terminate_tool_batch_skips_follow_up_model_call() {
        let model_calls = Arc::new(AtomicUsize::new(0));
        let mut runtime = FakeRuntime {
            active: None,
            model_calls: Arc::clone(&model_calls),
            tool_outcome: ToolBatchOutcome::Terminate,
        };

        let result = block_on(run_turn_loop(&mut runtime, "prompt", &(), None))
            .expect("turn completes");
        assert_eq!(result, "");
        assert_eq!(model_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn continue_tool_batch_preserves_follow_up_model_call() {
        let model_calls = Arc::new(AtomicUsize::new(0));
        let mut runtime = FakeRuntime {
            active: None,
            model_calls: Arc::clone(&model_calls),
            tool_outcome: ToolBatchOutcome::Continue,
        };

        let result = block_on(run_turn_loop(&mut runtime, "prompt", &(), None))
            .expect("turn completes");
        assert_eq!(result, "final");
        assert_eq!(model_calls.load(Ordering::SeqCst), 2);
    }
}
