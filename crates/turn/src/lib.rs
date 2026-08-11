//! Turn state and multi-step turn loop orchestration.

mod state;
mod turn_loop;

pub use turn_loop::{run_turn_loop, StepResult, TurnRuntime};
pub use state::{
    agent_busy_error, begin_turn, end_turn, is_cancelled, max_turns_notice, aborted_error,
    steer_requires_active_turn, SteerBuffer, StepId, TurnId, TurnState,
};
