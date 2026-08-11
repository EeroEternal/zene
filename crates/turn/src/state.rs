use std::fmt;

use anyhow::{anyhow, Result};
use uuid::Uuid;

/// Stable identity for a persisted or running agent session.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(String);

impl SessionId {
    pub fn new() -> Self { Self(Uuid::new_v4().to_string()) }
    pub fn from_string(value: impl Into<String>) -> Self { Self(value.into()) }
    pub fn as_str(&self) -> &str { &self.0 }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) }
}

/// Stable identity for a model tool call.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolCallId(String);

impl ToolCallId {
    pub fn new() -> Self { Self(Uuid::new_v4().to_string()) }
    pub fn from_string(value: impl Into<String>) -> Self { Self(value.into()) }
    pub fn as_str(&self) -> &str { &self.0 }
}

impl fmt::Display for ToolCallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) }
}

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

/// Buffered follow-up user messages injected between steps (kimi `steerBuffer`).
#[derive(Debug, Default)]
pub struct SteerBuffer {
    pending: Vec<String>,
}

impl SteerBuffer {
    pub fn push(&mut self, text: String) {
        self.pending.push(text);
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn take_all(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending)
    }
}

pub fn agent_busy_error() -> anyhow::Error {
    anyhow!(
        "agent busy: a turn is already in progress; use steer() for follow-up guidance or wait for the turn to finish"
    )
}

pub fn steer_requires_active_turn() -> anyhow::Error {
    anyhow!("no turn in progress; use prompt() to start a new turn")
}

pub fn max_turns_notice(max_steps: u32) -> String {
    format!(
        "[notice] Reached max_turns ({max_steps}) without a final answer. Send a follow-up to continue, or raise max turns / use Unlimited."
    )
}

pub fn aborted_error() -> anyhow::Error {
    anyhow!("turn aborted")
}

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

pub fn is_cancelled(cancel: Option<&tokio_util::sync::CancellationToken>) -> bool {
    cancel.is_some_and(tokio_util::sync::CancellationToken::is_cancelled)
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
    fn steer_buffer_fifo() {
        let mut buf = SteerBuffer::default();
        buf.push("first".into());
        buf.push("second".into());
        assert_eq!(buf.len(), 2);
        let drained = buf.take_all();
        assert_eq!(drained, vec!["first", "second"]);
        assert!(buf.is_empty());
    }
}
