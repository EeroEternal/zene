use std::fmt;

use anyhow::{anyhow, Result};
use uuid::Uuid;

/// Identifier for one user prompt → final assistant response cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TurnId(Uuid);

impl TurnId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl fmt::Display for TurnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Identifier for one LLM invocation within a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StepId(Uuid);

impl StepId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl fmt::Display for StepId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Tracks an in-flight turn (one active turn per agent).
#[derive(Debug)]
pub struct TurnState {
    pub turn_id: TurnId,
    pub step: u32,
}

impl TurnState {
    pub fn begin() -> Self {
        Self {
            turn_id: TurnId::new(),
            step: 0,
        }
    }

    pub fn next_step_id(&mut self) -> StepId {
        self.step += 1;
        StepId::new()
    }
}

pub fn agent_busy_error() -> anyhow::Error {
    anyhow!("agent busy: a turn is already in progress")
}

pub fn max_steps_error(max_steps: u32) -> anyhow::Error {
    anyhow!(
        "Agent reached max_turns ({max_steps}) without producing a final assistant response"
    )
}

pub fn aborted_error() -> anyhow::Error {
    anyhow!("turn aborted")
}

/// Guard: only one active turn at a time.
pub fn begin_turn(active: &mut Option<TurnState>) -> Result<()> {
    if active.is_some() {
        return Err(agent_busy_error());
    }
    *active = Some(TurnState::begin());
    Ok(())
}

pub fn end_turn(active: &mut Option<TurnState>) {
    active.take();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_second_active_turn() {
        let mut active = None;
        begin_turn(&mut active).expect("first turn");
        let err = begin_turn(&mut active).unwrap_err();
        assert!(err.to_string().contains("agent busy"));
        end_turn(&mut active);
        begin_turn(&mut active).expect("turn after end");
    }

    #[test]
    fn max_steps_error_mentions_limit() {
        let err = max_steps_error(3);
        assert!(err.to_string().contains("max_turns (3)"));
    }
}
