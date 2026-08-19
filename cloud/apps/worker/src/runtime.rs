use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tracing::{info, warn};
use uuid::Uuid;
use zene_cloud_domain::{
    ApprovalDecision, ClaimedRun, RunStatus, WorkerCommandKind, WorkerEventRequest, WorkerFence,
};
use zene_cloud_runtime_client::{
    RuntimeClient, RuntimeCommand, RuntimeEvent, RuntimeNotification, RuntimeRequest,
};

use crate::api::{
    ack_command, deliver_event, fetch_commands, persist_runtime_session, resolve_permission,
    set_pending_mode, set_status, set_status_raw, take_pending_mode, ResolvedPermission,
};
use crate::event_outbox::EventOutbox;
use crate::title::{
    format_title_focus, maybe_refresh_run_title, title_is_user_locked, TitleRefresh,
};
use crate::Cli;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunOutcome {
    /// The runtime completed the prompt and the hold ended naturally.
    Completed,
    /// The user cancelled the run; the runner has already recorded Cancelled.
    Cancelled,
    /// The worker is stopping; leave the run for lease recovery/reconciliation.
    Shutdown,
    /// The runtime disappeared before a normal completion.
    RuntimeInterrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HoldExit {
    IdleTimeout,
    Cancelled,
    Shutdown,
    RuntimeInterrupted,
}

fn idle_hold_elapsed(turn_busy: bool, elapsed: Duration, idle: Duration) -> bool {
    !turn_busy && elapsed >= idle
}

fn outcome_for_hold(prompt_completed: bool, exit: HoldExit) -> RunOutcome {
    match exit {
        HoldExit::Cancelled => RunOutcome::Cancelled,
        HoldExit::Shutdown => RunOutcome::Shutdown,
        HoldExit::RuntimeInterrupted => RunOutcome::RuntimeInterrupted,
        HoldExit::IdleTimeout if prompt_completed => RunOutcome::Completed,
        // An idle-looking exit before the prompt completed is not success.
        HoldExit::IdleTimeout => RunOutcome::RuntimeInterrupted,
    }
}

fn event_to_req(event: RuntimeNotification) -> WorkerEventRequest {
    WorkerEventRequest {
        source_event_id: event.source_event_id,
        cursor: event.cursor,
        event_type: event.event_type.into(),
        payload: event.payload.to_value(),
        fence: None,
    }
}

fn spawn_event_pump<R: RuntimeClient + 'static>(
    runtime: Arc<R>,
    client: reqwest::Client,
    api_url: String,
    token: String,
    run_id: Uuid,
    fence: WorkerFence,
    outbox: EventOutbox,
    last_activity: Arc<tokio::sync::Mutex<tokio::time::Instant>>,
) -> (
    tokio::task::JoinHandle<()>,
    Arc<tokio::sync::Mutex<Option<String>>>,
) {
    let event_error = Arc::new(tokio::sync::Mutex::new(None::<String>));
    let event_error_task = event_error.clone();
    let runtime_bg = runtime.clone();
    let pump = tokio::spawn(async move {
        while let Some(event) = runtime_bg.next_event().await {
            {
                let mut ts = last_activity.lock().await;
                *ts = tokio::time::Instant::now();
            }
            match event {
                RuntimeEvent::Initialized { event, .. } | RuntimeEvent::Notification(event) => {
                    if let Err(err) = deliver_event(
                        &outbox,
                        &client,
                        &api_url,
                        &token,
                        run_id,
                        event_to_req(event),
                        &fence,
                    )
                    .await
                    {
                        warn!(run_id = %run_id, error = %err, "event delivery failed");
                        *event_error_task.lock().await = Some(err.to_string());
                        break;
                    }
                }
                RuntimeEvent::Request { request, event } => {
                    if let Err(err) = deliver_event(
                        &outbox,
                        &client,
                        &api_url,
                        &token,
                        run_id,
                        event_to_req(event),
                        &fence,
                    )
                    .await
                    {
                        warn!(run_id = %run_id, error = %err, "event delivery failed");
                        *event_error_task.lock().await = Some(err.to_string());
                        break;
                    }
                    let RuntimeRequest::Approval {
                        request_id,
                        kind,
                        allowed_decisions,
                        context,
                    } = request;
                    let outcome = match resolve_permission(
                        &client,
                        &api_url,
                        &token,
                        run_id,
                        &request_id,
                        kind,
                        allowed_decisions,
                        &context,
                    )
                    .await
                    {
                        Ok(outcome) => outcome,
                        Err(err) => {
                            warn!(error = %err, "permission resolve failed");
                            ResolvedPermission {
                                decision: ApprovalDecision::Deny,
                                option_id: None,
                                answer: None,
                            }
                        }
                    };
                    let _ = runtime_bg
                        .send(RuntimeCommand::Approval {
                            request_id,
                            decision: outcome.decision,
                            option_id: outcome.option_id,
                            answer: outcome.answer,
                        })
                        .await;
                }
                RuntimeEvent::ChildExited => {
                    info!(run_id = %run_id, "runtime child exited");
                    break;
                }
            }
        }
    });
    (pump, event_error)
}

fn spawn_command_poller<R: RuntimeClient + 'static>(
    runtime: Arc<R>,
    client: reqwest::Client,
    api_url: String,
    token: String,
    run_id: Uuid,
    fence: WorkerFence,
    cancelled: Arc<AtomicBool>,
    last_activity: Arc<tokio::sync::Mutex<tokio::time::Instant>>,
    turn_busy: Arc<AtomicBool>,
    title_state: Arc<tokio::sync::Mutex<TitleRefresh>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match fetch_commands(&client, &api_url, &token, run_id, &fence).await {
                Ok(response) => {
                    let current_title = response.title.clone();
                    if let Some(mode_id) = response.mode_id {
                        if turn_busy.load(Ordering::SeqCst) {
                            // SetMode is idle-only; put the mode back until the turn ends.
                            let _ =
                                set_pending_mode(&client, &api_url, &token, run_id, &mode_id).await;
                        } else {
                            match runtime
                                .send(RuntimeCommand::SetMode {
                                    mode_id: mode_id.clone(),
                                })
                                .await
                            {
                                Ok(()) => {
                                    let mut ts = last_activity.lock().await;
                                    *ts = tokio::time::Instant::now();
                                }
                                Err(err) => {
                                    warn!(error = %err, "set_mode failed; re-queue pending mode");
                                    let _ = set_pending_mode(
                                        &client, &api_url, &token, run_id, &mode_id,
                                    )
                                    .await;
                                }
                            }
                        }
                    }
                    for cmd in response.commands {
                        if cmd.kind == WorkerCommandKind::Cancel {
                            cancelled.store(true, Ordering::SeqCst);
                            let _ = runtime.send(RuntimeCommand::Cancel).await;
                            return;
                        }
                        if cmd.kind == WorkerCommandKind::Prompt {
                            if let Some(text) = cmd.text {
                                {
                                    let mut ts = last_activity.lock().await;
                                    *ts = tokio::time::Instant::now();
                                }
                                let busy = turn_busy.load(Ordering::SeqCst);
                                if busy {
                                    match runtime.send(RuntimeCommand::Steer { text }).await {
                                        Ok(()) => {
                                            if let Some(message_id) = cmd.message_id {
                                                if let Err(err) = ack_command(
                                                    &client, &api_url, &token, run_id, &fence,
                                                    message_id,
                                                )
                                                .await
                                                {
                                                    warn!(error = %err, "steer command acknowledgement failed");
                                                }
                                            }
                                        }
                                        Err(err) => {
                                            warn!(error = %err, "steer failed; command will be retried");
                                        }
                                    }
                                } else {
                                    let _ = set_status_raw(
                                        &client,
                                        &api_url,
                                        &token,
                                        run_id,
                                        &fence,
                                        RunStatus::Running,
                                    )
                                    .await;
                                    turn_busy.store(true, Ordering::SeqCst);
                                    match runtime
                                        .send(RuntimeCommand::Prompt { text: text.clone() })
                                        .await
                                    {
                                        Ok(()) => {
                                            if let Some(message_id) = cmd.message_id {
                                                if let Err(err) = ack_command(
                                                    &client, &api_url, &token, run_id, &fence,
                                                    message_id,
                                                )
                                                .await
                                                {
                                                    warn!(error = %err, "follow-up command acknowledgement failed");
                                                }
                                            }
                                        }
                                        Err(err) => {
                                            warn!(error = %err, "follow-up prompt failed; command will be retried");
                                        }
                                    }
                                    {
                                        let mut ts = last_activity.lock().await;
                                        *ts = tokio::time::Instant::now();
                                    }
                                    turn_busy.store(false, Ordering::SeqCst);
                                    if !cancelled.load(Ordering::SeqCst) {
                                        let _ = set_status_raw(
                                            &client,
                                            &api_url,
                                            &token,
                                            run_id,
                                            &fence,
                                            RunStatus::WaitingForUser,
                                        )
                                        .await;
                                        let (focus, source_prompt, llm_env, skip) = {
                                            let mut st = title_state.lock().await;
                                            st.recent.push(text.clone());
                                            if st.recent.len() > 5 {
                                                st.recent.remove(0);
                                            }
                                            let skip = title_is_user_locked(
                                                current_title.as_deref().unwrap_or(""),
                                                st.last_auto.as_deref(),
                                                &st.seed,
                                            );
                                            (
                                                format_title_focus(&st.original_prompt, &st.recent),
                                                st.original_prompt.clone(),
                                                st.llm_env.clone(),
                                                skip,
                                            )
                                        };
                                        if !skip {
                                            if let Some(llm_env) = llm_env.as_ref() {
                                                match maybe_refresh_run_title(
                                                    &client,
                                                    &api_url,
                                                    &token,
                                                    run_id,
                                                    &fence,
                                                    &focus,
                                                    current_title.as_deref(),
                                                    &source_prompt,
                                                    llm_env,
                                                )
                                                .await
                                                {
                                                    Ok(Some(title)) => {
                                                        title_state.lock().await.last_auto =
                                                            Some(title);
                                                    }
                                                    Ok(None) => {}
                                                    Err(err) => {
                                                        warn!(run_id = %run_id, error = %err, "run title refresh failed");
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                {
                                    let mut ts = last_activity.lock().await;
                                    *ts = tokio::time::Instant::now();
                                }
                            }
                        }
                    }
                }
                Err(err) => warn!(error = %err, "commands poll failed"),
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    })
}

pub(crate) async fn run_runtime_session<R: RuntimeClient + 'static>(
    runtime: Arc<R>,
    client: &reqwest::Client,
    cli: &Cli,
    claimed: &ClaimedRun,
    fence: &WorkerFence,
    outbox: EventOutbox,
    shutdown: Arc<AtomicBool>,
    idle: Duration,
    llm_env: Option<&HashMap<String, String>>,
    persist_session: bool,
    resume_without_prompt: bool,
) -> Result<RunOutcome> {
    let run_id = claimed.run.id;
    if persist_session {
        let session_id = runtime.session_id().await?;
        persist_runtime_session(client, cli, run_id, fence, &session_id).await?;
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let last_activity = Arc::new(tokio::sync::Mutex::new(tokio::time::Instant::now()));
    let turn_busy = Arc::new(AtomicBool::new(false));
    let (pump, event_error) = spawn_event_pump(
        runtime.clone(),
        client.clone(),
        cli.api_url.clone(),
        cli.worker_token.clone(),
        run_id,
        (*fence).clone(),
        outbox,
        last_activity.clone(),
    );
    let title_state = Arc::new(tokio::sync::Mutex::new(TitleRefresh {
        seed: claimed.run.title.clone(),
        original_prompt: claimed.run.prompt.clone(),
        llm_env: llm_env.cloned(),
        recent: Vec::new(),
        last_auto: None,
    }));
    let cmd_task = spawn_command_poller(
        runtime.clone(),
        client.clone(),
        cli.api_url.clone(),
        cli.worker_token.clone(),
        run_id,
        (*fence).clone(),
        cancelled.clone(),
        last_activity.clone(),
        turn_busy.clone(),
        title_state.clone(),
    );

    if let Some(mode_id) = take_pending_mode(client, cli, run_id).await? {
        runtime
            .send(RuntimeCommand::SetMode { mode_id })
            .await
            .context("runtime set_mode")?;
    }

    if !resume_without_prompt {
        if let Some(llm_env) = llm_env {
            let title_state = title_state.clone();
            let client = client.clone();
            let api_url = cli.api_url.clone();
            let token = cli.worker_token.clone();
            let fence = fence.clone();
            let focus = format_title_focus(&claimed.run.prompt, &[]);
            let current_title = claimed.run.title.clone();
            let source_prompt = claimed.run.prompt.clone();
            let llm_env = llm_env.clone();
            tokio::spawn(async move {
                match maybe_refresh_run_title(
                    &client,
                    &api_url,
                    &token,
                    run_id,
                    &fence,
                    &focus,
                    Some(current_title.as_str()),
                    &source_prompt,
                    &llm_env,
                )
                .await
                {
                    Ok(Some(title)) => {
                        title_state.lock().await.last_auto = Some(title);
                    }
                    Ok(None) => {}
                    Err(err) => {
                        warn!(run_id = %run_id, error = %err, "run title refresh failed");
                    }
                }
            });
        }
        turn_busy.store(true, Ordering::SeqCst);
        let prompt_result = runtime
            .send(RuntimeCommand::Prompt {
                text: claimed.run.prompt.clone(),
            })
            .await
            .context("runtime prompt");
        turn_busy.store(false, Ordering::SeqCst);
        prompt_result?;
        {
            let mut ts = last_activity.lock().await;
            *ts = tokio::time::Instant::now();
        }
    } else {
        info!(run_id = %run_id, "resuming session without initial prompt");
    }
    if !cancelled.load(Ordering::SeqCst) {
        set_status(
            client,
            cli,
            run_id,
            fence,
            RunStatus::WaitingForUser,
            None,
            None,
        )
        .await?;
    }

    let hold_exit = loop {
        if cancelled.load(Ordering::SeqCst) {
            break HoldExit::Cancelled;
        }
        if shutdown.load(Ordering::SeqCst) {
            info!(run_id = %run_id, "ending hold due to shutdown signal");
            break HoldExit::Shutdown;
        }
        if event_error.lock().await.is_some() {
            warn!(run_id = %run_id, "ending hold after event delivery failure");
            break HoldExit::RuntimeInterrupted;
        }
        if !runtime.is_alive().await {
            info!(run_id = %run_id, "runtime child exited");
            break HoldExit::RuntimeInterrupted;
        }
        let elapsed = last_activity.lock().await.elapsed();
        if idle_hold_elapsed(turn_busy.load(Ordering::SeqCst), elapsed, idle) {
            info!(run_id = %run_id, idle_secs = idle.as_secs(), "runtime idle timeout");
            break HoldExit::IdleTimeout;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    };

    cmd_task.abort();
    let _ = runtime.send(RuntimeCommand::Shutdown).await;
    let _ = pump.await;

    if let Some(err) = event_error.lock().await.clone() {
        bail!("event delivery failed: {err}");
    }
    if cancelled.load(Ordering::SeqCst) {
        set_status(client, cli, run_id, fence, RunStatus::Cancelled, None, None).await?;
        return Ok(RunOutcome::Cancelled);
    }
    if shutdown.load(Ordering::SeqCst) {
        return Ok(RunOutcome::Shutdown);
    }
    Ok(outcome_for_hold(true, hold_exit))
}

#[cfg(test)]
mod tests {
    use super::{idle_hold_elapsed, outcome_for_hold, HoldExit, RunOutcome};
    use std::time::Duration;

    #[test]
    fn completed_prompt_may_finish_after_idle_hold() {
        assert_eq!(
            outcome_for_hold(true, HoldExit::IdleTimeout),
            RunOutcome::Completed
        );
    }

    #[test]
    fn idle_hold_does_not_fire_while_a_turn_is_busy() {
        let idle = Duration::from_secs(600);
        assert!(!idle_hold_elapsed(true, Duration::from_secs(601), idle));
        assert!(idle_hold_elapsed(false, Duration::from_secs(601), idle));
        assert!(!idle_hold_elapsed(false, Duration::from_secs(10), idle));
    }

    #[test]
    fn shutdown_and_interruption_never_map_to_completed() {
        assert_eq!(
            outcome_for_hold(true, HoldExit::Shutdown),
            RunOutcome::Shutdown
        );
        assert_eq!(
            outcome_for_hold(true, HoldExit::RuntimeInterrupted),
            RunOutcome::RuntimeInterrupted
        );
        assert_eq!(
            outcome_for_hold(false, HoldExit::IdleTimeout),
            RunOutcome::RuntimeInterrupted
        );
    }

    #[test]
    fn cancellation_is_distinct_from_idle_completion() {
        assert_eq!(
            outcome_for_hold(true, HoldExit::Cancelled),
            RunOutcome::Cancelled
        );
        assert_ne!(
            outcome_for_hold(true, HoldExit::Cancelled),
            outcome_for_hold(true, HoldExit::IdleTimeout)
        );
    }
}
