use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
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
    FollowUp {
        text: String,
    },
    SetSteeringMode {
        mode: zene_turn::QueueMode,
    },
    SetFollowUpMode {
        mode: zene_turn::QueueMode,
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

impl ApprovalDecision {
    pub fn allowed(self) -> bool {
        !matches!(self, Self::Deny)
    }
}

/// Oneshot registry for in-flight tool approvals.
///
/// The turn task waits here; the actor resolves entries when a transport
/// sends [`RuntimeCommand::Approval`].
#[derive(Default)]
pub struct ApprovalWaiters {
    inner: Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>,
}

impl ApprovalWaiters {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, request_id: String) -> oneshot::Receiver<ApprovalDecision> {
        let (tx, rx) = oneshot::channel();
        self.inner
            .lock()
            .expect("approval waiters")
            .insert(request_id, tx);
        rx
    }

    pub fn resolve(&self, request_id: &str, decision: ApprovalDecision) -> Result<()> {
        match self
            .inner
            .lock()
            .expect("approval waiters")
            .remove(request_id)
        {
            Some(tx) => {
                let _ = tx.send(decision);
                Ok(())
            }
            None => Err(anyhow!("no approval request {request_id} is pending")),
        }
    }

    pub fn cancel_all(&self) {
        self.inner.lock().expect("approval waiters").clear();
    }

    pub fn is_empty(&self) -> bool {
        self.inner.lock().expect("approval waiters").is_empty()
    }
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

/// Terminal lifecycle transition emitted by a runtime actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeLifecycle {
    Completed,
    Failed { message: String },
    Cancelled,
    Shutdown,
}

impl RuntimeLifecycle {
    fn event_state(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed { .. } => "failed",
            Self::Cancelled => "cancelled",
            Self::Shutdown => "shutdown",
        }
    }
}

impl From<RuntimeLifecycle> for ExecutionState {
    fn from(lifecycle: RuntimeLifecycle) -> Self {
        match lifecycle {
            RuntimeLifecycle::Completed => Self::Completed,
            RuntimeLifecycle::Failed { message } => Self::Failed { message },
            RuntimeLifecycle::Cancelled => Self::Cancelled,
            RuntimeLifecycle::Shutdown => Self::Shutdown,
        }
    }
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
    async fn follow_up(&self, text: String) -> Result<()>;
    async fn set_steering_mode(&self, mode: zene_turn::QueueMode) -> Result<()>;
    async fn set_follow_up_mode(&self, mode: zene_turn::QueueMode) -> Result<()>;
    async fn resume_safe_turn(&self) -> Result<String>;
    async fn cancel(&self) -> Result<()>;
    async fn set_mode(&self, mode_id: String) -> Result<String>;
    async fn current_mode(&self) -> Result<String>;
    async fn shutdown(&self) -> Result<()>;
    async fn approve(&self, request_id: String, decision: ApprovalDecision) -> Result<()>;
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<RuntimeEvent>;
    fn recovery_info(&self) -> Result<RuntimeRecoveryInfo>;
}

/// A command waiting for a runtime actor to acknowledge it.
pub struct RuntimeCommandMessage {
    pub command: RuntimeCommand,
    pub reply: oneshot::Sender<Result<RuntimeResponse, String>>,
}

/// Receiver owned by the runtime actor implementation.
pub struct RuntimeCommandReceiver {
    receiver: mpsc::Receiver<RuntimeCommandMessage>,
}

impl RuntimeCommandReceiver {
    pub async fn recv(&mut self) -> Option<RuntimeCommandMessage> {
        self.receiver.recv().await
    }
}

/// Cloneable transport-neutral command/event/state endpoint for one runtime.
#[derive(Clone)]
pub struct RuntimeCommandRouter {
    commands: mpsc::Sender<RuntimeCommandMessage>,
    events: broadcast::Sender<RuntimeEvent>,
    state: watch::Receiver<ExecutionState>,
}

impl RuntimeCommandRouter {
    /// Create the actor-facing command receiver and public control endpoint.
    pub fn channel(
        capacity: usize,
    ) -> (
        Self,
        RuntimeCommandReceiver,
        broadcast::Sender<RuntimeEvent>,
        watch::Sender<ExecutionState>,
    ) {
        let (commands, receiver) = mpsc::channel(capacity);
        let (events, _) = broadcast::channel(256);
        let (state_tx, state) = watch::channel(ExecutionState::Idle);
        (
            Self {
                commands,
                events: events.clone(),
                state,
            },
            RuntimeCommandReceiver { receiver },
            events,
            state_tx,
        )
    }

    pub async fn command(&self, command: RuntimeCommand) -> Result<RuntimeResponse> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(RuntimeCommandMessage { command, reply })
            .await
            .map_err(|_| anyhow!("runtime actor is unavailable"))?;
        match response
            .await
            .map_err(|_| anyhow!("runtime actor dropped command reply"))?
        {
            Ok(response) => Ok(response),
            Err(message) => Err(anyhow!(message)),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.events.subscribe()
    }

    pub fn state(&self) -> watch::Receiver<ExecutionState> {
        self.state.clone()
    }
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

    /// Apply a terminal lifecycle transition and publish its event in order.
    ///
    /// The state channel is updated before the event is sent so an event
    /// consumer that immediately reads the state channel observes the same
    /// terminal transition as the event stream.
    pub fn publish_lifecycle(&self, lifecycle: RuntimeLifecycle) {
        let event_state = lifecycle.event_state();
        self.set_state(lifecycle.into());
        self.publish_state(event_state);
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
                RuntimeEventKind::ApprovalRequested { request_id, .. } => {
                    let _ = publisher.state.send(ExecutionState::AwaitingApproval {
                        request_id: request_id.clone(),
                    });
                }
                _ => {}
            }
            let _ = publisher.events.send(event);
        })
    }

    pub fn publish_kind(&self, kind: RuntimeEventKind) {
        let event = RuntimeEvent {
            sequence: zene_turn::EventSequence::new(
                self.sequence.fetch_add(1, Ordering::Relaxed) + 1,
            ),
            session_id: self.session_id.clone(),
            turn_id: None,
            step_id: None,
            kind,
        };
        if let RuntimeEventKind::ApprovalRequested { request_id, .. } = &event.kind {
            self.set_state(ExecutionState::AwaitingApproval {
                request_id: request_id.clone(),
            });
        }
        let _ = self.events.send(event);
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
    async fn command_router_round_trips_commands_and_exposes_state() {
        let (router, mut receiver, _events, state_tx) = RuntimeCommandRouter::channel(4);
        let mut state = router.state();
        state_tx.send(ExecutionState::Starting).unwrap();
        assert_eq!(*state.borrow_and_update(), ExecutionState::Starting);

        let task = tokio::spawn(async move {
            let message = receiver.recv().await.expect("command");
            assert!(matches!(message.command, RuntimeCommand::GetMode));
            message
                .reply
                .send(Ok(RuntimeResponse::Mode {
                    mode_id: "default".into(),
                }))
                .unwrap();
        });
        let response = router.command(RuntimeCommand::GetMode).await.unwrap();
        assert_eq!(
            response,
            RuntimeResponse::Mode {
                mode_id: "default".into()
            }
        );
        task.await.unwrap();
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

    #[test]
    fn terminal_lifecycle_maps_to_execution_state() {
        let cases = [
            (RuntimeLifecycle::Completed, ExecutionState::Completed),
            (
                RuntimeLifecycle::Failed {
                    message: "boom".into(),
                },
                ExecutionState::Failed {
                    message: "boom".into(),
                },
            ),
            (RuntimeLifecycle::Cancelled, ExecutionState::Cancelled),
            (RuntimeLifecycle::Shutdown, ExecutionState::Shutdown),
        ];

        for (lifecycle, expected) in cases {
            assert_eq!(ExecutionState::from(lifecycle), expected);
        }
    }

    #[tokio::test]
    async fn event_publisher_mirrors_terminal_state_before_event() {
        let (events, mut receiver) = broadcast::channel(4);
        let (state, state_receiver) = watch::channel(ExecutionState::Idle);
        let publisher =
            RuntimeEventPublisher::new(events, state, SessionId::from_string("runtime-session"));

        let handler = publisher.handler();
        handler(RuntimeEvent {
            sequence: zene_turn::EventSequence::new(0),
            session_id: SessionId::from_string("wrong-session"),
            turn_id: None,
            step_id: None,
            kind: RuntimeEventKind::TextDelta { delta: "x".into() },
        });
        publisher.publish_lifecycle(RuntimeLifecycle::Failed {
            message: "boom".into(),
        });

        let first = receiver.try_recv().expect("runtime event");
        let terminal = receiver.try_recv().expect("terminal state event");
        assert_eq!(first.sequence.value(), 1);
        assert_eq!(terminal.sequence.value(), 2);
        assert_eq!(
            terminal.kind,
            RuntimeEventKind::StateChanged {
                state: "failed".into(),
            }
        );
        assert_eq!(
            *state_receiver.borrow(),
            ExecutionState::Failed {
                message: "boom".into(),
            }
        );
    }

    #[tokio::test]
    async fn event_publisher_mirrors_turn_progress_into_runtime_state() {
        let (events, _receiver) = broadcast::channel(4);
        let (state, state_receiver) = watch::channel(ExecutionState::Idle);
        let turn_id = TurnId::new();
        let publisher =
            RuntimeEventPublisher::new(events, state, SessionId::from_string("runtime-session"));
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

    #[tokio::test]
    async fn approval_waiters_resolve_registered_request() {
        let waiters = ApprovalWaiters::new();
        let rx = waiters.register("req-1".into());
        waiters
            .resolve("req-1", ApprovalDecision::AllowOnce)
            .unwrap();
        assert_eq!(rx.await.unwrap(), ApprovalDecision::AllowOnce);
        assert!(waiters.is_empty());
    }

    #[tokio::test]
    async fn approval_waiters_cancel_drops_receivers() {
        let waiters = ApprovalWaiters::new();
        let rx = waiters.register("req-2".into());
        waiters.cancel_all();
        assert!(rx.await.is_err());
        assert!(waiters.resolve("req-2", ApprovalDecision::Deny).is_err());
    }
}
