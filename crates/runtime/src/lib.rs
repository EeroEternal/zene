use anyhow::{anyhow, Result};
use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot, watch};
use zene_turn::TurnId;

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
