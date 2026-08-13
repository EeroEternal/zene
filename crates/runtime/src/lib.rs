use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use zene_turn::{RuntimeEvent, RuntimeEventHandler, RuntimeEventKind, SessionId, TurnId};

/// Transport-neutral command accepted by a long-lived runtime.
#[derive(Debug)]
pub enum RuntimeCommand {
    Prompt {
        text: String,
    },
    /// Resume only a safe model-boundary candidate.
    ResumeSafeTurn,
    Steer {
        text: String,
    },
    Cancel,
    Approval {
        request_id: String,
        decision: ApprovalDecision,
    },
    SetMode {
        mode_id: String,
    },
    GetMode,
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    AllowOnce,
    AllowSession,
    Deny,
}

/// Runtime execution state, independent from Cloud Run state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionState {
    Idle,
    Starting,
    Running { turn_id: TurnId, step: u32 },
    AwaitingApproval { request_id: String },
    AwaitingUser,
    Completed,
    Failed { message: String },
    Cancelled,
    Shutdown,
}

/// Acknowledgement returned by a runtime command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeResponse {
    Accepted,
    Prompt { text: String },
    Mode { mode_id: String },
}

/// Transport-neutral recovery state exposed to control-plane adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeRecoveryInfo {
    pub disposition: String,
    pub has_incomplete_execution: bool,
    pub active_turn_count: usize,
    pub active_tool_count: usize,
    pub safe_resume_allowed: bool,
    pub automatic_resume: bool,
    pub reason: String,
}

/// Public control contract for one long-lived runtime actor.
#[async_trait]
pub trait RuntimeControl: Send + Sync {
    async fn prompt(&self, text: String) -> Result<String>;
    async fn steer(&self, text: String) -> Result<()>;
    async fn resume_safe_turn(&self) -> Result<String>;
    async fn cancel(&self) -> Result<()>;
    async fn set_mode(&self, mode_id: String) -> Result<String>;
    async fn current_mode(&self) -> Result<String>;
    async fn shutdown(&self) -> Result<()>;
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<RuntimeEvent>;
    fn recovery_info(&self) -> Result<RuntimeRecoveryInfo>;
}

/// Publishes ordered, session-scoped runtime events and mirrors execution
/// progress into the transport-neutral runtime state channel.
#[derive(Clone)]
pub struct RuntimeEventPublisher {
    events: broadcast::Sender<RuntimeEvent>,
    state: watch::Sender<ExecutionState>,
    session_id: SessionId,
    sequence: Arc<AtomicU64>,
}

impl RuntimeEventPublisher {
    pub fn new(
        events: broadcast::Sender<RuntimeEvent>,
        state: watch::Sender<ExecutionState>,
        session_id: SessionId,
    ) -> Self {
        Self {
            events,
            state,
            session_id,
            sequence: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn set_state(&self, state: ExecutionState) {
        let _ = self.state.send(state);
    }

    pub fn publish_state(&self, state: impl Into<String>) {
        let _ = self.events.send(RuntimeEvent {
            sequence: zene_turn::EventSequence::new(
                self.sequence.fetch_add(1, Ordering::Relaxed) + 1,
            ),
            session_id: self.session_id.clone(),
            turn_id: None,
            step_id: None,
            kind: RuntimeEventKind::StateChanged {
                state: state.into(),
            },
        });
    }

    pub fn handler(&self) -> RuntimeEventHandler {
        let publisher = self.clone();
        Arc::new(move |mut event| {
            event.sequence = zene_turn::EventSequence::new(
                publisher.sequence.fetch_add(1, Ordering::Relaxed) + 1,
            );
            event.session_id = publisher.session_id.clone();
            match &event.kind {
                RuntimeEventKind::TurnStarted => {
                    let _ = publisher.state.send(ExecutionState::Running {
                        turn_id: event.turn_id.unwrap_or_else(TurnId::new),
                        step: 0,
                    });
                }
                RuntimeEventKind::StepStarted { step } => {
                    if let Some(turn_id) = event.turn_id {
                        let _ = publisher.state.send(ExecutionState::Running {
                            turn_id,
                            step: *step,
                        });
                    }
                }
                _ => {}
            }
            let _ = publisher.events.send(event);
        })
    }
}

#[async_trait]
pub trait RuntimeDriver: Send + 'static {
    async fn run(&mut self, input: String) -> Result<String>;
    async fn cancel(&mut self) -> Result<()>;
}

struct Envelope {
    command: DriverCommand,
    reply: oneshot::Sender<Result<DriverResponse, String>>,
}

/// Minimal driver-only actor kept for adapters and tests.
#[derive(Debug)]
pub enum DriverCommand {
    Run { input: String },
    Cancel,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverResponse {
    Accepted,
    Completed { output: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverState {
    Idle,
    Running,
    Cancelling,
    Failed(String),
    Shutdown,
}

#[derive(Clone)]
pub struct DriverHandle {
    commands: mpsc::Sender<Envelope>,
    state: watch::Receiver<DriverState>,
}

impl DriverHandle {
    pub fn spawn<D: RuntimeDriver>(driver: D) -> (Self, tokio::task::JoinHandle<Result<()>>) {
        let (commands, rx) = mpsc::channel(32);
        let (state_tx, state) = watch::channel(DriverState::Idle);
        let handle = Self { commands, state };
        let task = tokio::spawn(run_driver_actor(driver, rx, state_tx));
        (handle, task)
    }

    pub async fn command(&self, command: DriverCommand) -> Result<DriverResponse> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(Envelope { command, reply })
            .await
            .map_err(|_| anyhow!("runtime driver unavailable"))?;
        match response
            .await
            .map_err(|_| anyhow!("runtime driver dropped response"))?
        {
            Ok(response) => Ok(response),
            Err(error) => Err(anyhow!(error)),
        }
    }

    pub fn state(&self) -> watch::Receiver<DriverState> {
        self.state.clone()
    }
}

async fn run_driver_actor<D: RuntimeDriver>(
    mut driver: D,
    mut commands: mpsc::Receiver<Envelope>,
    state: watch::Sender<DriverState>,
) -> Result<()> {
    while let Some(envelope) = commands.recv().await {
        match envelope.command {
            DriverCommand::Run { input } => {
                let _ = state.send(DriverState::Running);
                let result = driver.run(input).await;
                match result {
                    Ok(output) => {
                        let _ = state.send(DriverState::Idle);
                        let _ = envelope
                            .reply
                            .send(Ok(DriverResponse::Completed { output }));
                    }
                    Err(error) => {
                        let message = error.to_string();
                        let _ = state.send(DriverState::Failed(message.clone()));
                        let _ = envelope.reply.send(Err(message));
                    }
                }
            }
            DriverCommand::Cancel => {
                let result = driver.cancel().await;
                let _ = state.send(DriverState::Cancelling);
                let response = result
                    .map(|_| DriverResponse::Accepted)
                    .map_err(|error| error.to_string());
                let _ = envelope.reply.send(response);
            }
            DriverCommand::Shutdown => {
                let _ = state.send(DriverState::Shutdown);
                let _ = envelope.reply.send(Ok(DriverResponse::Accepted));
                return Ok(());
            }
        }
    }
    let _ = state.send(DriverState::Shutdown);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeDriver;
    #[async_trait]
    impl RuntimeDriver for FakeDriver {
        async fn run(&mut self, input: String) -> Result<String> {
            Ok(format!("done:{input}"))
        }
        async fn cancel(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn recovery_info_is_transport_neutral() {
        let info = RuntimeRecoveryInfo {
            disposition: "safe_to_resume".into(),
            has_incomplete_execution: true,
            active_turn_count: 1,
            active_tool_count: 0,
            safe_resume_allowed: true,
            automatic_resume: true,
            reason: "only a model-boundary turn is open".into(),
        };
        assert!(info.automatic_resume);
        assert_eq!(info.active_turn_count, 1);
    }

    #[tokio::test]
    async fn event_publisher_assigns_scope_and_monotonic_sequence() {
        let (events, mut receiver) = broadcast::channel(4);
        let (state, state_receiver) = watch::channel(ExecutionState::Idle);
        let session_id = SessionId::from_string("runtime-session");
        let publisher = RuntimeEventPublisher::new(events, state, session_id.clone());
        let handler = publisher.handler();
        let event = || RuntimeEvent {
            sequence: zene_turn::EventSequence::new(999),
            session_id: SessionId::from_string("wrong-session"),
            turn_id: None,
            step_id: None,
            kind: RuntimeEventKind::TextDelta { delta: "x".into() },
        };

        handler(event());
        handler(event());
        let first = receiver.try_recv().unwrap();
        let second = receiver.try_recv().unwrap();
        assert_eq!(first.sequence.value(), 1);
        assert_eq!(second.sequence.value(), 2);
        assert_eq!(first.session_id, session_id);
        assert_eq!(second.session_id, session_id);
        assert_eq!(*state_receiver.borrow(), ExecutionState::Idle);
    }

    #[tokio::test]
    async fn event_publisher_mirrors_turn_progress_into_runtime_state() {
        let (events, _receiver) = broadcast::channel(4);
        let (state, state_receiver) = watch::channel(ExecutionState::Idle);
        let turn_id = TurnId::new();
        let publisher = RuntimeEventPublisher::new(
            events,
            state,
            SessionId::from_string("runtime-session"),
        );
        let handler = publisher.handler();
        handler(RuntimeEvent {
            sequence: zene_turn::EventSequence::new(0),
            session_id: SessionId::from_string("wrong-session"),
            turn_id: Some(turn_id),
            step_id: None,
            kind: RuntimeEventKind::TurnStarted,
        });
        assert_eq!(
            *state_receiver.borrow(),
            ExecutionState::Running { turn_id, step: 0 }
        );
        handler(RuntimeEvent {
            sequence: zene_turn::EventSequence::new(0),
            session_id: SessionId::from_string("wrong-session"),
            turn_id: Some(turn_id),
            step_id: None,
            kind: RuntimeEventKind::StepStarted { step: 3 },
        });
        assert_eq!(
            *state_receiver.borrow(),
            ExecutionState::Running { turn_id, step: 3 }
        );
    }

    #[tokio::test]
    async fn actor_runs_driver_without_core_dependency() {
        let (runtime, task) = DriverHandle::spawn(FakeDriver);
        let response = runtime
            .command(DriverCommand::Run {
                input: "hello".into(),
            })
            .await
            .unwrap();
        assert_eq!(
            response,
            DriverResponse::Completed {
                output: "done:hello".into()
            }
        );
        assert_eq!(*runtime.state().borrow(), DriverState::Idle);
        runtime.command(DriverCommand::Shutdown).await.unwrap();
        task.await.unwrap().unwrap();
    }
}
