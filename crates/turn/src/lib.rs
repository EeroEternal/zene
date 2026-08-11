//! Turn state and multi-step turn loop orchestration.

mod events;
mod state;
mod turn_loop;

pub use events::{EventSequence, RuntimeEvent, RuntimeEventHandler, RuntimeEventKind};
pub use turn_loop::{
    run_turn_loop, ContextAssemblerPort, EventSinkPort, LegacyTurnPorts, ModelExecutorPort,
    PreparedContext, StepResult, ToolBatchOutcome, ToolExecutorPort, TurnEngine, TurnEnginePorts,
    TurnOutcome, TurnRequest, TurnRuntime, TurnSessionPort, TurnStatus,
};
pub use state::{
    aborted_error, agent_busy_error, begin_turn, end_turn, is_cancelled, max_turns_notice,
    steer_requires_active_turn, SessionId, SteerBuffer, StepId, ToolCallId, TurnId, TurnState,
};
