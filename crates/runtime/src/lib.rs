use anyhow::{Result, anyhow};
use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot, watch};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeState {
    Idle,
    Running,
    Cancelling,
    Failed(String),
    Shutdown,
}

#[derive(Debug)]
pub enum RuntimeCommand {
    Run { input: String },
    Cancel,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeResponse {
    Accepted,
    Completed { output: String },
}

#[async_trait]
pub trait RuntimeDriver: Send + 'static {
    async fn run(&mut self, input: String) -> Result<String>;
    async fn cancel(&mut self) -> Result<()>;
}

struct Envelope {
    command: RuntimeCommand,
    reply: oneshot::Sender<Result<RuntimeResponse, String>>,
}

#[derive(Clone)]
pub struct RuntimeHandle {
    commands: mpsc::Sender<Envelope>,
    state: watch::Receiver<RuntimeState>,
}

impl RuntimeHandle {
    pub fn spawn<D: RuntimeDriver>(driver: D) -> (Self, tokio::task::JoinHandle<Result<()>>) {
        let (commands, rx) = mpsc::channel(32);
        let (state_tx, state) = watch::channel(RuntimeState::Idle);
        let handle = Self { commands, state };
        let task = tokio::spawn(run_actor(driver, rx, state_tx));
        (handle, task)
    }

    pub async fn command(&self, command: RuntimeCommand) -> Result<RuntimeResponse> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(Envelope { command, reply })
            .await
            .map_err(|_| anyhow!("runtime actor unavailable"))?;
        match response
            .await
            .map_err(|_| anyhow!("runtime actor dropped response"))?
        {
            Ok(response) => Ok(response),
            Err(error) => Err(anyhow!(error)),
        }
    }

    pub fn state(&self) -> watch::Receiver<RuntimeState> {
        self.state.clone()
    }
}

async fn run_actor<D: RuntimeDriver>(
    mut driver: D,
    mut commands: mpsc::Receiver<Envelope>,
    state: watch::Sender<RuntimeState>,
) -> Result<()> {
    while let Some(envelope) = commands.recv().await {
        match envelope.command {
            RuntimeCommand::Run { input } => {
                let _ = state.send(RuntimeState::Running);
                let result = driver.run(input).await;
                match result {
                    Ok(output) => {
                        let _ = state.send(RuntimeState::Idle);
                        let _ = envelope
                            .reply
                            .send(Ok(RuntimeResponse::Completed { output }));
                    }
                    Err(error) => {
                        let message = error.to_string();
                        let _ = state.send(RuntimeState::Failed(message.clone()));
                        let _ = envelope.reply.send(Err(message));
                    }
                }
            }
            RuntimeCommand::Cancel => {
                let result = driver.cancel().await;
                let _ = state.send(RuntimeState::Cancelling);
                let response = result
                    .map(|_| RuntimeResponse::Accepted)
                    .map_err(|error| error.to_string());
                let _ = envelope.reply.send(response);
            }
            RuntimeCommand::Shutdown => {
                let _ = state.send(RuntimeState::Shutdown);
                let _ = envelope.reply.send(Ok(RuntimeResponse::Accepted));
                return Ok(());
            }
        }
    }
    let _ = state.send(RuntimeState::Shutdown);
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
        let (runtime, task) = RuntimeHandle::spawn(FakeDriver);
        let response = runtime
            .command(RuntimeCommand::Run {
                input: "hello".into(),
            })
            .await
            .unwrap();
        assert_eq!(
            response,
            RuntimeResponse::Completed {
                output: "done:hello".into()
            }
        );
        assert_eq!(*runtime.state().borrow(), RuntimeState::Idle);
        runtime.command(RuntimeCommand::Shutdown).await.unwrap();
        task.await.unwrap().unwrap();
    }
}
