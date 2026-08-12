//! Agent-specific actor implementation for the core runtime.
//!
//! The public `RuntimeHandle` facade remains exported by `runtime.rs`; this
//! module owns the Agent-specific actor state and command handling.

use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use anyhow::{anyhow, Context, Result};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::task::{JoinError, JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::{RecoveryDisposition, RecoverySnapshot};
#[cfg(test)]
use zene_permission::PromptChoice;
#[cfg(test)]
use zene_runtime::ApprovalDecision;
use zene_runtime::{ExecutionState, RuntimeCommand, RuntimeResponse};
use zene_session::{AgentRecordWriter, ExecutionCheckpointState, RecoveryPlan};
use zene_turn::{
    EventSequence, RuntimeEvent, RuntimeEventHandler, RuntimeEventKind, SessionId, SteerBuffer,
    TurnId,
};

use crate::{Agent, PromptOptions};

#[cfg(test)]
fn prompt_choice(decision: ApprovalDecision) -> PromptChoice {
    match decision {
        ApprovalDecision::AllowOnce => PromptChoice::AllowOnce,
        ApprovalDecision::AllowSession => PromptChoice::AllowSession,
        ApprovalDecision::Deny => PromptChoice::Deny,
    }
}

struct RuntimeMessage {
    command: RuntimeCommand,
    reply: oneshot::Sender<std::result::Result<RuntimeResponse, String>>,
}

/// Cloneable command/event/state handle for one long-lived runtime actor.
#[derive(Clone)]
pub struct RuntimeHandle {
    commands: mpsc::Sender<RuntimeMessage>,
    events: broadcast::Sender<RuntimeEvent>,
    state: watch::Receiver<ExecutionState>,
    record_writer: AgentRecordWriter,
}

impl RuntimeHandle {
    /// Spawn an actor that exclusively owns `agent`.
    pub fn spawn(agent: Agent) -> (Self, JoinHandle<Result<()>>) {
        Self::spawn_internal(agent, None)
    }

    /// Spawn an actor and automatically resume one safe model-boundary turn.
    ///
    /// Only a single open turn with no active tool is eligible. The durable
    /// resume fence is claimed before the actor starts, so concurrent runtime
    /// instances cannot replay the same model request.
    pub fn spawn_with_automatic_recovery(agent: Agent) -> (Self, JoinHandle<Result<()>>) {
        let record_writer = agent.execution_record_writer();
        let candidate = record_writer
            .recovery_snapshot()
            .ok()
            .filter(|snapshot| {
                let plan = snapshot.plan();
                plan.automatic_resume_implemented
            })
            .and_then(|snapshot| snapshot.resume_candidates.into_iter().next())
            .filter(|candidate| record_writer.claim_safe_resume(candidate).unwrap_or(false));
        Self::spawn_internal(agent, candidate)
    }

    fn spawn_internal(
        mut agent: Agent,
        candidate: Option<zene_session::ResumeCandidate>,
    ) -> (Self, JoinHandle<Result<()>>) {
        if candidate.is_some() {
            agent.resume_existing_turn = true;
        }
        let record_writer = agent.execution_record_writer();
        let (commands, command_rx) = mpsc::channel(32);
        let (events, _) = broadcast::channel(256);
        let (state_tx, state) = watch::channel(ExecutionState::Idle);
        let handle = Self {
            commands,
            events: events.clone(),
            state,
            record_writer: record_writer.clone(),
        };
        let task = tokio::spawn(run_actor(
            agent,
            record_writer,
            command_rx,
            events,
            state_tx,
            candidate,
        ));
        (handle, task)
    }

    /// Send a transport-neutral command and await its acknowledgement/result.
    pub async fn command(&self, command: RuntimeCommand) -> Result<RuntimeResponse> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.commands
            .send(RuntimeMessage {
                command,
                reply: reply_tx,
            })
            .await
            .map_err(|_| anyhow!("runtime actor is unavailable"))?;
        match reply_rx
            .await
            .context("runtime actor dropped command reply")?
        {
            Ok(response) => Ok(response),
            Err(message) => Err(anyhow!(message)),
        }
    }

    pub async fn prompt(&self, text: impl Into<String>) -> Result<String> {
        match self
            .command(RuntimeCommand::Prompt { text: text.into() })
            .await?
        {
            RuntimeResponse::Prompt { text } => Ok(text),
            _ => Err(anyhow!("runtime returned an invalid prompt response")),
        }
    }

    pub async fn steer(&self, text: impl Into<String>) -> Result<()> {
        self.command(RuntimeCommand::Steer { text: text.into() })
            .await
            .map(|_| ())
    }

    pub async fn resume_safe_turn(&self) -> Result<String> {
        match self.command(RuntimeCommand::ResumeSafeTurn).await? {
            RuntimeResponse::Prompt { text } => Ok(text),
            _ => Err(anyhow!("runtime returned an invalid resume response")),
        }
    }

    pub async fn cancel(&self) -> Result<()> {
        self.command(RuntimeCommand::Cancel).await.map(|_| ())
    }

    pub async fn set_mode(&self, mode_id: impl Into<String>) -> Result<String> {
        match self
            .command(RuntimeCommand::SetMode {
                mode_id: mode_id.into(),
            })
            .await?
        {
            RuntimeResponse::Mode { mode_id } => Ok(mode_id),
            _ => Err(anyhow!("runtime returned an invalid mode response")),
        }
    }

    pub async fn current_mode(&self) -> Result<String> {
        match self.command(RuntimeCommand::GetMode).await? {
            RuntimeResponse::Mode { mode_id } => Ok(mode_id),
            _ => Err(anyhow!("runtime returned an invalid mode response")),
        }
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.command(RuntimeCommand::Shutdown).await.map(|_| ())
    }

    /// Subscribe to the ordered runtime event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.events.subscribe()
    }

    /// Subscribe to the latest execution state.
    pub fn state(&self) -> watch::Receiver<ExecutionState> {
        self.state.clone()
    }

    /// Read the durable recovery view without starting or replaying execution.
    pub fn recovery_snapshot(&self) -> Result<RecoverySnapshot> {
        self.record_writer.recovery_snapshot()
    }

    /// Classify durable recovery state without starting or replaying execution.
    pub fn recovery_disposition(&self) -> Result<RecoveryDisposition> {
        Ok(self.recovery_snapshot()?.disposition())
    }

    /// Return the conservative recovery plan without starting execution.
    pub fn recovery_plan(&self) -> Result<RecoveryPlan> {
        Ok(self.recovery_snapshot()?.plan())
    }
}

struct PendingPrompt {
    text: String,
    reply: oneshot::Sender<std::result::Result<RuntimeResponse, String>>,
}

struct ActivePrompt {
    cancel: CancellationToken,
    reply: oneshot::Sender<std::result::Result<RuntimeResponse, String>>,
    task: JoinHandle<(Agent, Result<String>)>,
}

enum ActivePoll {
    Finished(std::result::Result<(Agent, Result<String>), JoinError>),
    Command(Option<RuntimeMessage>),
}

async fn run_actor(
    agent: Agent,
    record_writer: AgentRecordWriter,
    mut commands: mpsc::Receiver<RuntimeMessage>,
    events: broadcast::Sender<RuntimeEvent>,
    state: watch::Sender<ExecutionState>,
    initial_resume: Option<zene_session::ResumeCandidate>,
) -> Result<()> {
    let steer_buffer = agent.steer_buffer();
    let session_id = SessionId::from_string(agent.session().meta.id.clone());
    let sequence = Arc::new(AtomicU64::new(0));
    let mut agent = Some(agent);
    let mut queued: VecDeque<PendingPrompt> = VecDeque::new();
    let mut active: Option<ActivePrompt> = initial_resume.map(|candidate| {
        let (reply, _response) = oneshot::channel();
        start_prompt(
            agent.take().expect("initial recovery owns agent"),
            PendingPrompt {
                text: candidate.prompt,
                reply,
            },
            &events,
            &state,
            &session_id,
            &sequence,
        )
    });
    let mut shutdown_requested = false;

    loop {
        if active.is_none() {
            if shutdown_requested {
                while let Some(prompt) = queued.pop_front() {
                    let _ = prompt.reply.send(Err("runtime is shutting down".into()));
                }
                let _ = Agent::record_runtime_checkpoint(
                    &record_writer,
                    session_id.as_str(),
                    ExecutionCheckpointState::RuntimeShutdown,
                    Some("shutdown"),
                )
                .await;
                let _ = state.send(ExecutionState::Shutdown);
                if let Some(mut agent) = agent.take() {
                    agent.shutdown().await?;
                }
                return Ok(());
            }

            if let Some(prompt) = queued.pop_front() {
                active = Some(start_prompt(
                    agent.take().expect("idle actor owns agent"),
                    prompt,
                    &events,
                    &state,
                    &session_id,
                    &sequence,
                ));
                continue;
            }

            let Some(message) = commands.recv().await else {
                shutdown_requested = true;
                continue;
            };
            active = handle_idle_command(
                &mut agent,
                message,
                &mut queued,
                &steer_buffer,
                &events,
                &state,
                &mut shutdown_requested,
                &session_id,
                &sequence,
            );
            continue;
        }

        let poll = {
            let current = active.as_mut().expect("active prompt exists");
            tokio::select! {
                result = &mut current.task => ActivePoll::Finished(result),
                message = commands.recv() => ActivePoll::Command(message),
            }
        };

        match poll {
            ActivePoll::Finished(result) => {
                let current = active.take().expect("active prompt exists");
                let cancelled = current.cancel.is_cancelled();
                match result {
                    Ok((finished_agent, prompt_result)) => {
                        agent = Some(finished_agent);
                        let response = match prompt_result {
                            Ok(text) if !cancelled => {
                                let _ = state.send(ExecutionState::Completed);
                                Ok(RuntimeResponse::Prompt { text })
                            }
                            Ok(_text) => {
                                let _ = state.send(ExecutionState::Cancelled);
                                Err("turn cancelled".into())
                            }
                            Err(err) if cancelled || err.to_string().contains("aborted") => {
                                let _ = state.send(ExecutionState::Cancelled);
                                Err("turn cancelled".into())
                            }
                            Err(err) => {
                                let message = err.to_string();
                                let _ = state.send(ExecutionState::Failed {
                                    message: message.clone(),
                                });
                                Err(message)
                            }
                        };
                        let _ = current.reply.send(response);
                    }
                    Err(err) => {
                        let message = format!("runtime turn task failed: {err}");
                        let _ = state.send(ExecutionState::Failed {
                            message: message.clone(),
                        });
                        let _ = current.reply.send(Err(message));
                        // The task owned the Agent; after a panic it cannot be
                        // recovered. Stop cleanly instead of panicking again
                        // in the shutdown path.
                        let _ = Agent::record_runtime_checkpoint(
                            &record_writer,
                            session_id.as_str(),
                            ExecutionCheckpointState::RuntimeFailed,
                            Some("task_failed"),
                        )
                        .await;
                        let _ = state.send(ExecutionState::Shutdown);
                        return Ok(());
                    }
                }
            }
            ActivePoll::Command(Some(message)) => {
                handle_active_command(
                    message,
                    &mut queued,
                    &steer_buffer,
                    active
                        .as_ref()
                        .expect("active prompt exists")
                        .cancel
                        .clone(),
                    &mut shutdown_requested,
                );
            }
            ActivePoll::Command(None) => {
                if let Some(current) = active.as_ref() {
                    current.cancel.cancel();
                }
                shutdown_requested = true;
            }
        }
    }
}

fn start_prompt(
    agent: Agent,
    prompt: PendingPrompt,
    events: &broadcast::Sender<RuntimeEvent>,
    state: &watch::Sender<ExecutionState>,
    session_id: &SessionId,
    sequence: &Arc<AtomicU64>,
) -> ActivePrompt {
    let _ = state.send(ExecutionState::Starting);
    let event_handler = runtime_event_sink(
        events.clone(),
        state.clone(),
        session_id.clone(),
        Arc::clone(sequence),
    );
    let cancel = CancellationToken::new();
    let task_cancel = cancel.clone();
    let task = tokio::spawn(async move {
        let mut agent = agent;
        let result = agent
            .prompt(
                &prompt.text,
                PromptOptions {
                    stream: true,
                    cancel: Some(task_cancel),
                    event_handler: None,
                    runtime_event_handler: Some(event_handler),
                    quiet: true,
                },
            )
            .await;
        (agent, result)
    });
    ActivePrompt {
        cancel,
        reply: prompt.reply,
        task,
    }
}

fn handle_idle_command(
    agent: &mut Option<Agent>,
    message: RuntimeMessage,
    _queued: &mut VecDeque<PendingPrompt>,
    _steer_buffer: &std::sync::Arc<parking_lot::Mutex<SteerBuffer>>,
    events: &broadcast::Sender<RuntimeEvent>,
    state: &watch::Sender<ExecutionState>,
    shutdown_requested: &mut bool,
    session_id: &SessionId,
    sequence: &Arc<AtomicU64>,
) -> Option<ActivePrompt> {
    match message.command {
        RuntimeCommand::ResumeSafeTurn => {
            let snapshot = match agent
                .as_ref()
                .expect("idle actor owns agent")
                .execution_record_writer()
                .recovery_snapshot()
            {
                Ok(snapshot) => snapshot,
                Err(err) => {
                    let _ = message.reply.send(Err(err.to_string()));
                    return None;
                }
            };
            let plan = snapshot.plan();
            let Some(candidate) = snapshot.resume_candidates.into_iter().next()
                .filter(|_| plan.automatic_resume_implemented)
            else {
                let _ = message
                    .reply
                    .send(Err("no unique prompt-backed safe resume candidate is available".into()));
                return None;
            };
            let writer = agent
                .as_ref()
                .expect("idle actor owns agent")
                .execution_record_writer();
            match writer.claim_safe_resume(&candidate) {
                Ok(true) => {}
                Ok(false) => {
                    let _ = message.reply.send(Err(
                        "safe resume rejected: candidate is already claimed or stale".into(),
                    ));
                    return None;
                }
                Err(err) => {
                    let _ = message
                        .reply
                        .send(Err(format!("safe resume fencing failed: {err}")));
                    return None;
                }
            }
            let mut resumed = agent.take().expect("idle actor owns agent");
            resumed.resume_existing_turn = true;
            Some(start_prompt(
                resumed,
                PendingPrompt {
                    text: candidate.prompt,
                    reply: message.reply,
                },
                events,
                state,
                session_id,
                sequence,
            ))
        }
        RuntimeCommand::Prompt { text } => {
            if text.trim().is_empty() {
                let _ = message.reply.send(Err("prompt cannot be empty".into()));
                None
            } else {
                Some(start_prompt(
                    agent.take().expect("idle actor owns agent"),
                    PendingPrompt {
                        text,
                        reply: message.reply,
                    },
                    events,
                    state,
                    session_id,
                    sequence,
                ))
            }
        }
        RuntimeCommand::Steer { .. } => {
            let _ = message.reply.send(Err(
                "no turn in progress; use prompt() to start a new turn".into(),
            ));
            None
        }
        RuntimeCommand::Cancel => {
            let _ = message.reply.send(Ok(RuntimeResponse::Accepted));
            None
        }
        RuntimeCommand::SetMode { mode_id } => match agent
            .as_mut()
            .expect("idle actor owns agent")
            .set_session_mode(&mode_id)
        {
            Ok(active) => {
                let _ = message.reply.send(Ok(RuntimeResponse::Mode {
                    mode_id: active.clone(),
                }));
                publish_state_event(events, session_id, sequence, &active);
                None
            }
            Err(err) => {
                let _ = message.reply.send(Err(err.to_string()));
                None
            }
        },
        RuntimeCommand::Approval { request_id, .. } => {
            let _ = state.send(ExecutionState::AwaitingApproval { request_id });
            let _ = message
                .reply
                .send(Err("no approval request is pending".into()));
            let _ = state.send(ExecutionState::Idle);
            None
        }
        RuntimeCommand::GetMode => {
            let mode_id = agent
                .as_ref()
                .expect("idle actor owns agent")
                .current_session_mode()
                .to_string();
            let _ = message.reply.send(Ok(RuntimeResponse::Mode { mode_id }));
            None
        }
        RuntimeCommand::Shutdown => {
            *shutdown_requested = true;
            let _ = message.reply.send(Ok(RuntimeResponse::Accepted));
            None
        }
    }
}

fn handle_active_command(
    message: RuntimeMessage,
    queued: &mut VecDeque<PendingPrompt>,
    steer_buffer: &std::sync::Arc<parking_lot::Mutex<SteerBuffer>>,
    cancel: CancellationToken,
    shutdown_requested: &mut bool,
) {
    match message.command {
        RuntimeCommand::Prompt { text } => {
            if text.trim().is_empty() {
                let _ = message.reply.send(Err("prompt cannot be empty".into()));
            } else {
                queued.push_back(PendingPrompt {
                    text,
                    reply: message.reply,
                });
            }
        }
        RuntimeCommand::ResumeSafeTurn => {
            let _ = message
                .reply
                .send(Err("cannot resume while a turn is active".into()));
        }
        RuntimeCommand::Steer { text } => {
            let text = text.trim();
            if text.is_empty() {
                let _ = message
                    .reply
                    .send(Err("steer message cannot be empty".into()));
            } else {
                steer_buffer.lock().push(text.to_string());
                let _ = message.reply.send(Ok(RuntimeResponse::Accepted));
            }
        }
        RuntimeCommand::Cancel => {
            cancel.cancel();
            let _ = message.reply.send(Ok(RuntimeResponse::Accepted));
        }
        RuntimeCommand::SetMode { .. } | RuntimeCommand::GetMode => {
            let _ = message.reply.send(Err(
                "cannot change or read mode while a turn is active".into(),
            ));
        }
        RuntimeCommand::Approval { request_id, .. } => {
            let _ = message.reply.send(Err(format!(
                "approval request {request_id} cannot be handled by this runtime"
            )));
        }
        RuntimeCommand::Shutdown => {
            *shutdown_requested = true;
            cancel.cancel();
            let _ = message.reply.send(Ok(RuntimeResponse::Accepted));
        }
    }
}

fn publish_state_event(
    events: &broadcast::Sender<RuntimeEvent>,
    session_id: &SessionId,
    sequence: &Arc<AtomicU64>,
    state: &str,
) {
    let _ = events.send(RuntimeEvent {
        sequence: EventSequence::new(sequence.fetch_add(1, Ordering::Relaxed) + 1),
        session_id: session_id.clone(),
        turn_id: None,
        step_id: None,
        kind: RuntimeEventKind::StateChanged {
            state: state.to_string(),
        },
    });
}

fn runtime_event_sink(
    events: broadcast::Sender<RuntimeEvent>,
    state: watch::Sender<ExecutionState>,
    session_id: SessionId,
    sequence: Arc<AtomicU64>,
) -> RuntimeEventHandler {
    std::sync::Arc::new(move |mut event| {
        event.sequence = EventSequence::new(sequence.fetch_add(1, Ordering::Relaxed) + 1);
        event.session_id = session_id.clone();
        match &event.kind {
            RuntimeEventKind::TurnStarted { .. } => {
                let _ = state.send(ExecutionState::Running {
                    turn_id: event.turn_id.unwrap_or_else(TurnId::new),
                    step: 0,
                });
            }
            RuntimeEventKind::StepStarted { step } => {
                if let Some(turn_id) = event.turn_id {
                    let _ = state.send(ExecutionState::Running {
                        turn_id,
                        step: *step,
                    });
                }
            }
            _ => {}
        }
        let _ = events.send(event);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_decisions_map_to_permission_choices() {
        assert_eq!(
            prompt_choice(ApprovalDecision::AllowOnce),
            PromptChoice::AllowOnce
        );
        assert_eq!(
            prompt_choice(ApprovalDecision::AllowSession),
            PromptChoice::AllowSession
        );
        assert_eq!(prompt_choice(ApprovalDecision::Deny), PromptChoice::Deny);
    }

    #[test]
    fn execution_state_separates_runtime_from_cloud_status() {
        let turn_id = TurnId::new();
        let state = ExecutionState::Running { turn_id, step: 2 };
        assert!(matches!(state, ExecutionState::Running { step: 2, .. }));
    }

    #[tokio::test]
    async fn runtime_handle_reads_recovery_without_starting_execution() {
        use chrono::Utc;
        use tempfile::tempdir;
        use zene_config::ZeneConfig;
        use zene_sandbox::LocalSandbox;
        use zene_session::{
            AgentRecordWriter, ExecutionCheckpointState, RecordEntry, SessionRecord,
        };

        let workdir = tempdir().expect("workdir");
        let record_dir = tempdir().expect("record dir");
        let session = SessionRecord::new(workdir.path());
        let writer = AgentRecordWriter::from_path(record_dir.path().join("record.jsonl"))
            .expect("record writer");
        writer
            .append(&RecordEntry::ExecutionCheckpoint {
                turn_id: "turn-incomplete".into(),
                step_id: None,
                tool_call_id: None,
                state: ExecutionCheckpointState::TurnStarted,
                idempotency_key: "turn-incomplete/started".into(),
                context_epoch: None,
                model_request_hash: None,
                ts: Utc::now(),
            })
            .expect("checkpoint");

        let mut config = ZeneConfig::default();
        config.provider = "anthropic".into();
        config.anthropic_api_key = Some("test-key".into());
        let agent = crate::AgentBuilder::new(
            config,
            LocalSandbox::new(workdir.path()),
            session,
            zene_permission::PermissionMode::BypassPermissions,
        )
        .without_mcp()
        .record_writer(writer)
        .build()
        .await
        .expect("build agent without network calls");
        let (runtime, task) = RuntimeHandle::spawn_with_automatic_recovery(agent);

        assert_eq!(
            runtime
                .recovery_disposition()
                .expect("recovery disposition"),
            RecoveryDisposition::SafeToResume
        );
        assert!(runtime
            .recovery_snapshot()
            .expect("recovery snapshot")
            .has_incomplete_execution());
        runtime.shutdown().await.expect("shutdown");
        task.await.expect("actor join").expect("actor result");
    }

    #[tokio::test]
    async fn active_cancel_acknowledges_and_cancels_prompt_token() {
        let (reply, response) = oneshot::channel();
        let cancel = CancellationToken::new();
        let cancel_for_assertion = cancel.clone();
        let mut queued = VecDeque::new();
        let steer = Arc::new(parking_lot::Mutex::new(SteerBuffer::default()));

        handle_active_command(
            RuntimeMessage {
                command: RuntimeCommand::Cancel,
                reply,
            },
            &mut queued,
            &steer,
            cancel,
            &mut false,
        );

        assert!(cancel_for_assertion.is_cancelled());
        assert!(matches!(
            response.await.unwrap(),
            Ok(RuntimeResponse::Accepted)
        ));
        assert!(queued.is_empty());
    }

    #[tokio::test]
    async fn active_mode_commands_are_rejected_without_mutating_queue() {
        for command in [
            RuntimeCommand::SetMode {
                mode_id: "plan".into(),
            },
            RuntimeCommand::GetMode,
        ] {
            let (reply, response) = oneshot::channel();
            let cancel = CancellationToken::new();
            let mut queued = VecDeque::new();
            let steer = Arc::new(parking_lot::Mutex::new(SteerBuffer::default()));

            handle_active_command(
                RuntimeMessage { command, reply },
                &mut queued,
                &steer,
                cancel,
                &mut false,
            );

            let error = response.await.unwrap().unwrap_err();
            assert!(error.contains("active"));
            assert!(queued.is_empty());
        }
    }

    #[tokio::test]
    async fn active_steer_is_buffered_and_empty_steer_is_rejected() {
        let steer = Arc::new(parking_lot::Mutex::new(SteerBuffer::default()));
        let cancel = CancellationToken::new();
        let mut queued = VecDeque::new();
        let (reply, response) = oneshot::channel();
        handle_active_command(
            RuntimeMessage {
                command: RuntimeCommand::Steer {
                    text: "  keep going  ".into(),
                },
                reply,
            },
            &mut queued,
            &steer,
            cancel.clone(),
            &mut false,
        );
        assert!(matches!(
            response.await.unwrap(),
            Ok(RuntimeResponse::Accepted)
        ));
        assert_eq!(steer.lock().take_all(), vec!["keep going"]);

        let (reply, response) = oneshot::channel();
        handle_active_command(
            RuntimeMessage {
                command: RuntimeCommand::Steer { text: "  ".into() },
                reply,
            },
            &mut queued,
            &steer,
            cancel,
            &mut false,
        );
        assert!(response.await.unwrap().unwrap_err().contains("empty"));
    }

    #[tokio::test]
    async fn active_prompt_queue_rejects_empty_and_preserves_valid_prompt() {
        let steer = Arc::new(parking_lot::Mutex::new(SteerBuffer::default()));
        let cancel = CancellationToken::new();
        let mut queued = VecDeque::new();

        let (reply, response) = oneshot::channel();
        handle_active_command(
            RuntimeMessage {
                command: RuntimeCommand::Prompt { text: "  ".into() },
                reply,
            },
            &mut queued,
            &steer,
            cancel.clone(),
            &mut false,
        );
        assert!(response.await.unwrap().unwrap_err().contains("empty"));
        assert!(queued.is_empty());

        let (reply, _response) = oneshot::channel();
        handle_active_command(
            RuntimeMessage {
                command: RuntimeCommand::Prompt {
                    text: "next".into(),
                },
                reply,
            },
            &mut queued,
            &steer,
            cancel,
            &mut false,
        );
        assert_eq!(queued.len(), 1);
        assert_eq!(queued.front().unwrap().text, "next");
    }

    #[test]
    fn runtime_event_sink_assigns_session_scope_and_monotonic_sequence() {
        let (events, mut receiver) = broadcast::channel(4);
        let (state, _state_receiver) = watch::channel(ExecutionState::Idle);
        let session_id = SessionId::from_string("runtime-session");
        let sink = runtime_event_sink(
            events,
            state,
            session_id.clone(),
            Arc::new(AtomicU64::new(0)),
        );
        let event = || RuntimeEvent {
            sequence: EventSequence::new(999),
            session_id: SessionId::from_string("wrong-session"),
            turn_id: None,
            step_id: None,
            kind: RuntimeEventKind::TextDelta { delta: "x".into() },
        };

        sink(event());
        sink(event());
        let first = receiver.try_recv().expect("first event");
        let second = receiver.try_recv().expect("second event");
        assert_eq!(first.sequence.value(), 1);
        assert_eq!(second.sequence.value(), 2);
        assert_eq!(first.session_id, session_id);
        assert_eq!(second.session_id, session_id);
    }
}
