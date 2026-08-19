mod event_outbox;
pub(crate) mod git;
mod github_auth;
mod supervisor;
pub(crate) mod title;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::Parser;
use git::{bare_cache_ready, prepare_workspace, workspace_ready};
use title::{format_title_focus, maybe_refresh_run_title, title_is_user_locked, TitleRefresh};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;
use zene_cloud_acp_bridge::resolve_zene_bin;
use zene_cloud_domain::{
    ApprovalDecision, ApprovalEventPayload, ApprovalKind, ApprovalRequest, ApprovalRisk,
    ApprovalStatus, ClaimedRun, CloneAuthResponse, CreateApprovalRequest, LlmAuthResponse,
    PermissionMode, RunStatus, WorkerClaimRequest, WorkerCommandAckRequest, WorkerCommandKind,
    WorkerCommandsResponse, WorkerEventRequest, WorkerFence, WorkerSessionRequest,
    WorkerStatusRequest,
};
use zene_cloud_runtime_client::{
    AcpRuntimeClient, MockRuntimeClient, RuntimeClient, RuntimeCommand, RuntimeEvent,
    RuntimeNotification, RuntimeRequest,
};

#[derive(Debug, Clone, Parser)]
#[command(name = "zene-cloud-worker")]
pub(crate) struct Cli {
    #[arg(
        long,
        env = "ZENE_CLOUD_API_URL",
        default_value = "http://127.0.0.1:8788"
    )]
    pub(crate) api_url: String,

    #[arg(
        long,
        env = "ZENE_CLOUD_WORKER_TOKEN",
        default_value = "dev-worker-token"
    )]
    pub(crate) worker_token: String,

    #[arg(long, env = "ZENE_CLOUD_WORKER_ID")]
    pub(crate) worker_id: Option<String>,

    #[arg(
        long,
        env = "ZENE_CLOUD_WORKSPACE_ROOT",
        default_value = "./data/workspaces"
    )]
    pub(crate) workspace_root: PathBuf,

    #[arg(long, env = "ZENE_BIN")]
    pub(crate) zene_bin: Option<PathBuf>,

    /// Pass `--yolo` to real `zene acp` (auto-approve tools locally).
    #[arg(long, env = "ZENE_CLOUD_ACP_YOLO", default_value_t = false)]
    pub(crate) acp_yolo: bool,

    /// Idle seconds after a prompt returns before ending the ACP session (follow-ups reset idle).
    #[arg(long, env = "ZENE_CLOUD_ACP_IDLE_SECS", default_value_t = 600)]
    pub(crate) acp_idle_secs: u64,

    /// Allow in-process MockAgent when no zene binary is available.
    #[arg(long, env = "ZENE_CLOUD_ALLOW_MOCK", default_value_t = false)]
    pub(crate) allow_mock: bool,

    #[arg(long, default_value_t = 2)]
    pub(crate) poll_seconds: u64,

    /// Call push/PR endpoints after commit (mock Git Broker by default).
    #[arg(long, env = "ZENE_CLOUD_PUSH_PR", default_value_t = false)]
    pub(crate) push_pr: bool,

    /// Run as process supervisor (spawn/scale executor children). Mutually exclusive with executor loop.
    #[arg(long, env = "ZENE_CLOUD_WORKER_SUPERVISOR", default_value_t = false)]
    pub(crate) supervisor: bool,

    /// Always-on idle claimer processes under supervisor mode.
    #[arg(long, env = "ZENE_CLOUD_WORKER_MIN_WARM", default_value_t = 1)]
    pub(crate) min_warm: u64,

    /// Max concurrent provisioning/starting/cloning/running runs (supervisor).
    #[arg(long, env = "ZENE_CLOUD_WORKER_MAX_ACTIVE", default_value_t = 4)]
    pub(crate) max_active: u64,

    /// Max concurrent waiting_for_user/approval warm holds (supervisor).
    #[arg(long, env = "ZENE_CLOUD_WORKER_MAX_HOLD", default_value_t = 8)]
    pub(crate) max_hold: u64,

    /// Supervisor scale loop interval.
    #[arg(
        long,
        env = "ZENE_CLOUD_WORKER_SCALE_INTERVAL_MS",
        default_value_t = 1000
    )]
    pub(crate) scale_interval_ms: u64,

    /// Forward to zene acp for inference-gateway delta assembly (optional).
    #[arg(long, env = "ZENE_INFERENCE_GATEWAY_URL")]
    pub(crate) inference_gateway_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunOutcome {
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

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let cli = Cli::parse();
    if cli.supervisor {
        return supervisor::run_supervisor(cli).await;
    }
    run_executor(cli).await
}

async fn run_executor(cli: Cli) -> Result<()> {
    let worker_id = cli
        .worker_id
        .clone()
        .unwrap_or_else(|| format!("worker-{}", &Uuid::new_v4().to_string()[..8]));
    std::fs::create_dir_all(&cli.workspace_root)?;
    // Local API must not go through HTTP(S)_PROXY (common on developer machines);
    // otherwise claim/heartbeat get 502 from the proxy and runs stay queued forever.
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .context("build HTTP client")?;
    let zene_bin = resolve_zene_bin(cli.zene_bin.clone());
    let has_process_llm = has_process_llm_credentials();
    if zene_bin.is_some() && !has_process_llm {
        info!("no process-level LLM credentials; runs will use per-user BYOK when configured");
    }
    if let Some(path) = &zene_bin {
        info!(
            agent_mode = "real",
            path = %path.display(),
            yolo = cli.acp_yolo,
            idle_secs = cli.acp_idle_secs,
            "using real zene acp"
        );
    } else if cli.allow_mock {
        warn!(
            agent_mode = "mock",
            "zene binary missing; using MockAgent (ZENE_CLOUD_ALLOW_MOCK=1)"
        );
    } else {
        bail!(
            "zene binary not found and ZENE_CLOUD_ALLOW_MOCK is disabled; set ZENE_BIN or build zene"
        );
    }

    let shutdown = Arc::new(AtomicBool::new(false));
    spawn_shutdown_listener(shutdown.clone());

    info!(%worker_id, api = %cli.api_url, "zene-cloud-worker executor started");
    loop {
        if shutdown.load(Ordering::SeqCst) {
            info!(%worker_id, "executor shutting down (idle)");
            break;
        }
        match claim_run(&client, &cli, &worker_id).await {
            Ok(Some(claimed)) => {
                if let Err(err) = execute_run(
                    &client,
                    &cli,
                    &worker_id,
                    &claimed,
                    zene_bin.as_ref(),
                    shutdown.clone(),
                )
                .await
                {
                    if err.to_string().contains("stale_attempt") {
                        warn!(error = %err, run_id = %claimed.run.id, "run lost attempt fence; stopping without overwriting replacement");
                    } else if is_recoverable_worker_error(&err, &claimed) {
                        warn!(error = %err, run_id = %claimed.run.id, "recoverable worker error; re-queuing run");
                        let _ = set_status(
                            &client,
                            &cli,
                            claimed.run.id,
                            &worker_fence(&claimed, &worker_id),
                            RunStatus::Queued,
                            None,
                            Some(err.to_string()),
                        )
                        .await;
                    } else {
                        error!(error = %err, run_id = %claimed.run.id, "run failed");
                        let _ = set_status(
                            &client,
                            &cli,
                            claimed.run.id,
                            &worker_fence(&claimed, &worker_id),
                            RunStatus::Failed,
                            None,
                            Some(err.to_string()),
                        )
                        .await;
                    }
                }
                if shutdown.load(Ordering::SeqCst) {
                    info!(%worker_id, "executor shutting down after run");
                    break;
                }
            }
            Ok(None) => {
                sleep_interruptible(Duration::from_secs(cli.poll_seconds), &shutdown).await;
            }
            Err(err) => {
                warn!(error = %err, "claim failed");
                sleep_interruptible(Duration::from_secs(cli.poll_seconds), &shutdown).await;
            }
        }
    }
    Ok(())
}

pub(crate) fn spawn_shutdown_listener(shutdown: Arc<AtomicBool>) {
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm = match signal(SignalKind::terminate()) {
                Ok(s) => s,
                Err(err) => {
                    warn!(error = %err, "failed to install SIGTERM handler");
                    return;
                }
            };
            let mut sigint = match signal(SignalKind::interrupt()) {
                Ok(s) => s,
                Err(err) => {
                    warn!(error = %err, "failed to install SIGINT handler");
                    return;
                }
            };
            tokio::select! {
                _ = sigterm.recv() => {
                    info!("received SIGTERM");
                    shutdown.store(true, Ordering::SeqCst);
                }
                _ = sigint.recv() => {
                    info!("received SIGINT");
                    shutdown.store(true, Ordering::SeqCst);
                }
            }
        }
        #[cfg(not(unix))]
        {
            if let Err(err) = tokio::signal::ctrl_c().await {
                warn!(error = %err, "failed to install ctrl_c handler");
                return;
            }
            info!("received ctrl_c");
            shutdown.store(true, Ordering::SeqCst);
        }
    });
}

pub(crate) async fn sleep_interruptible(total: Duration, shutdown: &AtomicBool) {
    let step = Duration::from_millis(200);
    let mut left = total;
    while left > Duration::ZERO {
        if shutdown.load(Ordering::SeqCst) {
            return;
        }
        let slice = step.min(left);
        tokio::time::sleep(slice).await;
        left = left.saturating_sub(slice);
    }
}

async fn claim_run(
    client: &reqwest::Client,
    cli: &Cli,
    worker_id: &str,
) -> Result<Option<ClaimedRun>> {
    let response = client
        .post(format!("{}/internal/v1/runs/claim", cli.api_url))
        .bearer_auth(&cli.worker_token)
        .json(&WorkerClaimRequest {
            worker_id: worker_id.to_string(),
            workspace_root: cli.workspace_root.display().to_string(),
        })
        .send()
        .await?
        .error_for_status()?;
    Ok(response.json().await?)
}

fn worker_fence(claimed: &ClaimedRun, worker_id: &str) -> WorkerFence {
    WorkerFence {
        attempt_id: claimed.attempt_id,
        generation: claimed.generation,
        worker_id: worker_id.to_string(),
    }
}

async fn execute_run(
    client: &reqwest::Client,
    cli: &Cli,
    worker_id: &str,
    claimed: &ClaimedRun,
    zene_bin: Option<&PathBuf>,
    shutdown: Arc<AtomicBool>,
) -> Result<()> {
    let run_id = claimed.run.id;
    info!(%run_id, workspace = %claimed.workspace_dir, "claimed run");
    let fence = worker_fence(claimed, worker_id);
    set_status(client, cli, run_id, &fence, RunStatus::Starting, None, None).await?;

    let workspace = PathBuf::from(&claimed.workspace_dir);
    std::fs::create_dir_all(&workspace)?;
    let hb = heartbeat_loop(
        client.clone(),
        cli.api_url.clone(),
        cli.worker_token.clone(),
        run_id,
        fence.clone(),
    );

    let workspace_exists = workspace_ready(&workspace).await;
    let clone_auth = fetch_clone_auth(client, cli, run_id).await.ok();
    if workspace_exists {
        info!(path = %workspace.display(), "resuming this run's checkout");
        set_status(client, cli, run_id, &fence, RunStatus::Running, None, None).await?;
    } else {
        let auth = clone_auth
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("clone-auth failed"))?;
        let cache = cli
            .workspace_root
            .join(".repo-cache")
            .join(claimed.run.repository_id.to_string());
        let need_network_clone = !auth.mock && !bare_cache_ready(&cache).await;
        if need_network_clone {
            set_status(client, cli, run_id, &fence, RunStatus::Cloning, None, None).await?;
        }
        prepare_workspace(&cli.workspace_root, &workspace, auth).await?;
        set_status(client, cli, run_id, &fence, RunStatus::Running, None, None).await?;
    }
    let mut github_refresh: Option<tokio::task::JoinHandle<()>> = None;
    let github_for_acp = if let Some(auth) = clone_auth.as_ref().filter(|a| !a.mock) {
        match github_auth::install(&workspace, auth).await {
            Ok(_) => {
                let dir = github_auth::auth_dir(&workspace);
                github_refresh = Some(github_token_refresh_loop(
                    client.clone(),
                    cli.clone(),
                    run_id,
                    dir.clone(),
                ));
                Some((
                    dir,
                    auth.token.clone(),
                    github_auth::github_repo_slug(&auth.clone_url),
                ))
            }
            Err(err) => {
                warn!(error = %err, "failed to install GitHub credentials for agent");
                None
            }
        }
    } else {
        None
    };

    let abort_bg = || {
        hb.abort();
        if let Some(h) = github_refresh.as_ref() {
            h.abort();
        }
    };

    let result = if let Some(bin) = zene_bin {
        info!(run_id = %run_id, agent_mode = "real", "starting agent");
        run_with_real_acp(
            client,
            cli,
            claimed,
            &fence,
            &workspace,
            &cli.workspace_root,
            bin,
            github_for_acp.as_ref(),
            shutdown.clone(),
        )
        .await
    } else if cli.allow_mock {
        info!(run_id = %run_id, agent_mode = "mock", "starting agent");
        run_with_mock(
            client,
            cli,
            claimed,
            &fence,
            &workspace,
            &cli.workspace_root,
            shutdown.clone(),
        )
        .await
    } else {
        bail!("zene binary not found and mock agent is disabled")
    };

    match result {
        Ok(RunOutcome::Completed) => {
            if shutdown.load(Ordering::SeqCst) {
                abort_bg();
                return Ok(());
            }
            if shutdown.load(Ordering::SeqCst) {
                abort_bg();
                return Ok(());
            }
            set_status(
                client,
                cli,
                run_id,
                &fence,
                RunStatus::Completed,
                None,
                None,
            )
            .await?;
            abort_bg();
            Ok(())
        }
        Ok(RunOutcome::Cancelled | RunOutcome::Shutdown) => {
            abort_bg();
            Ok(())
        }
        Ok(RunOutcome::RuntimeInterrupted) => {
            abort_bg();
            bail!("ACP runtime interrupted before completion")
        }
        Err(err) => {
            abort_bg();
            Err(err)
        }
    }
}

async fn fetch_clone_auth(
    client: &reqwest::Client,
    cli: &Cli,
    run_id: Uuid,
) -> Result<CloneAuthResponse> {
    const MAX_ATTEMPTS: u32 = 4;
    let url = format!("{}/internal/v1/runs/{run_id}/clone-auth", cli.api_url);
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 0..MAX_ATTEMPTS {
        if attempt > 0 {
            let backoff = Duration::from_secs(1_u64 << attempt.min(3));
            tokio::time::sleep(backoff).await;
        }
        match client.get(&url).bearer_auth(&cli.worker_token).send().await {
            Ok(response) if response.status().is_success() => {
                return Ok(response.json().await.context("decode clone-auth")?);
            }
            Ok(response) => {
                last_err = Some(anyhow::anyhow!(
                    "clone-auth HTTP {}: {}",
                    response.status(),
                    response.text().await.unwrap_or_default()
                ));
            }
            Err(err) => {
                last_err = Some(err.into());
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("clone-auth failed"))).context("clone-auth")
}

fn is_recoverable_worker_error(err: &anyhow::Error, claimed: &ClaimedRun) -> bool {
    let msg = err.to_string();
    let setup_error = msg.contains("clone-auth")
        || msg.contains("git clone")
        || msg.contains("git bare clone")
        || msg.contains("prepare mock git workspace");
    if !setup_error {
        return false;
    }
    claimed.resume_session_id.is_some()
        || std::path::Path::new(&claimed.workspace_dir)
            .join(".git")
            .exists()
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

async fn run_runtime_session<R: RuntimeClient + 'static>(
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

const MOCK_HOLD_IDLE: Duration = Duration::from_secs(3);

async fn run_with_mock(
    client: &reqwest::Client,
    cli: &Cli,
    claimed: &ClaimedRun,
    fence: &WorkerFence,
    workspace: &Path,
    outbox_root: &Path,
    shutdown: Arc<AtomicBool>,
) -> Result<RunOutcome> {
    let outbox = EventOutbox::open(outbox_root, claimed.run.id).await?;
    outbox
        .flush(
            client,
            &cli.api_url,
            &cli.worker_token,
            claimed.run.id,
            fence,
        )
        .await
        .context("flush recovered event outbox")?;
    let runtime = Arc::new(MockRuntimeClient::connect(workspace));
    run_runtime_session(
        runtime,
        client,
        cli,
        claimed,
        fence,
        outbox,
        shutdown,
        MOCK_HOLD_IDLE,
        None,
        false,
        claimed.resume_without_prompt,
    )
    .await
}

async fn fetch_llm_auth(
    client: &reqwest::Client,
    cli: &Cli,
    run_id: Uuid,
) -> Result<Option<LlmAuthResponse>> {
    let response = client
        .get(format!(
            "{}/internal/v1/runs/{run_id}/llm-auth",
            cli.api_url
        ))
        .bearer_auth(&cli.worker_token)
        .send()
        .await
        .context("llm-auth request")?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let response = response.error_for_status().context("llm-auth")?;
    Ok(Some(response.json().await?))
}

fn llm_env_from_auth(auth: &LlmAuthResponse) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("ZENE_PROVIDER".into(), auth.provider.clone());
    env.insert("ZENE_API_KEY".into(), auth.api_key.clone());
    env.insert("ZENE_BASE_URL".into(), auth.base_url.clone());
    let model = auth.model.trim();
    if !model.is_empty() && model != "default" {
        env.insert("ZENE_MODEL".into(), model.to_string());
    }
    env
}

fn inject_run_max_turns(env: &mut HashMap<String, String>, max_turns: u32) {
    env.insert("ZENE_MAX_TURNS".into(), max_turns.to_string());
}

fn inject_run_context(
    env: &mut HashMap<String, String>,
    run_id: Uuid,
    api_url: &str,
    worker_token: &str,
) {
    env.insert("ZENE_RUN_ID".into(), run_id.to_string());
    env.insert(
        "ZENE_CLOUD_API_URL".into(),
        api_url.trim_end_matches('/').to_string(),
    );
    env.insert("ZENE_CLOUD_WORKER_TOKEN".into(), worker_token.to_string());
}

fn inject_inference_gateway(env: &mut HashMap<String, String>, url: Option<&str>) {
    let url = url
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            std::env::var("ZENE_INFERENCE_GATEWAY_URL")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });
    if let Some(url) = url {
        env.insert("ZENE_INFERENCE_GATEWAY_URL".into(), url);
    }
}

fn has_process_llm_credentials() -> bool {
    std::env::var("ZENE_API_KEY").is_ok()
        || std::env::var("OPENAI_API_KEY").is_ok()
        || std::env::var("ANTHROPIC_API_KEY").is_ok()
        || std::env::var("ZENE_BASE_URL").is_ok()
}

async fn run_with_real_acp(
    client: &reqwest::Client,
    cli: &Cli,
    claimed: &ClaimedRun,
    fence: &WorkerFence,
    workspace: &Path,
    outbox_root: &Path,
    zene_bin: &Path,
    github_for_acp: Option<&(PathBuf, Option<String>, Option<String>)>,
    shutdown: Arc<AtomicBool>,
) -> Result<RunOutcome> {
    let run_id = claimed.run.id;
    let outbox = EventOutbox::open(outbox_root, run_id).await?;
    outbox
        .flush(client, &cli.api_url, &cli.worker_token, run_id, fence)
        .await
        .context("flush recovered event outbox")?;
    let yolo = cli.acp_yolo
        || claimed.run.permission_mode == PermissionMode::Yolo
        || std::env::var("ZENE_YOLO").ok().as_deref() == Some("1");

    let mut llm_env = match fetch_llm_auth(client, cli, run_id).await {
        Ok(Some(auth)) => {
            info!(
                run_id = %run_id,
                base_url = %auth.base_url,
                model = %auth.model,
                "using user BYOK llm settings"
            );
            llm_env_from_auth(&auth)
        }
        Ok(None) => {
            if has_process_llm_credentials() {
                info!(run_id = %run_id, "no user llm settings; inheriting worker env");
                HashMap::new()
            } else {
                bail!(
                    "no LLM credentials: configure API key and base URL in Settings (BYOK), \
                     or set ZENE_API_KEY / ZENE_BASE_URL on the worker"
                );
            }
        }
        Err(err) => {
            if has_process_llm_credentials() {
                warn!(run_id = %run_id, error = %err, "llm-auth failed; inheriting worker env");
                HashMap::new()
            } else {
                bail!(
                    "llm-auth failed and no process-level LLM credentials: {err}; \
                     configure Settings → LLM before starting an agent"
                );
            }
        }
    };
    inject_run_max_turns(&mut llm_env, claimed.run.max_turns);
    inject_run_context(&mut llm_env, run_id, &cli.api_url, &cli.worker_token);
    inject_inference_gateway(&mut llm_env, cli.inference_gateway_url.as_deref());
    if let Some((dir, token, repo)) = github_for_acp {
        github_auth::inject_env(&mut llm_env, dir, token.as_deref(), repo.as_deref());
    }
    info!(
        run_id = %run_id,
        max_turns = claimed.run.max_turns,
        inference_gateway = cli.inference_gateway_url.is_some(),
        "injected ZENE_MAX_TURNS, ZENE_RUN_ID, and optional inference gateway for acp"
    );

    let runtime = Arc::new(
        AcpRuntimeClient::connect_with_session(
            zene_bin,
            workspace,
            yolo,
            &llm_env,
            claimed.resume_session_id.as_deref(),
        )
        .await
        .context("connect runtime client")?,
    );
    run_runtime_session(
        runtime,
        client,
        cli,
        claimed,
        fence,
        outbox,
        shutdown,
        Duration::from_secs(cli.acp_idle_secs.max(1)),
        Some(&llm_env),
        true,
        claimed.resume_without_prompt,
    )
    .await
}

struct ResolvedPermission {
    decision: ApprovalDecision,
    option_id: Option<String>,
    answer: Option<String>,
}

fn permission_outcome(approval: &ApprovalRequest) -> ResolvedPermission {
    ResolvedPermission {
        decision: approval.decision.unwrap_or(ApprovalDecision::AllowOnce),
        option_id: approval
            .payload
            .get("optionId")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        answer: approval
            .payload
            .get("answer")
            .and_then(|value| value.as_str())
            .map(str::to_string),
    }
}

async fn resolve_permission(
    client: &reqwest::Client,
    api_url: &str,
    token: &str,
    run_id: Uuid,
    request_key: &str,
    kind: ApprovalKind,
    allowed_decisions: Vec<ApprovalDecision>,
    payload: &ApprovalEventPayload,
) -> Result<ResolvedPermission> {
    let body = CreateApprovalRequest {
        request_key: request_key.to_string(),
        kind,
        risk: ApprovalRisk::Medium,
        payload: payload.clone(),
        allowed_decisions,
        expires_at: None,
    };
    let created: ApprovalRequest = client
        .post(format!("{api_url}/internal/v1/runs/{run_id}/approvals"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await?
        .error_for_status()
        .context("create approval")?
        .json()
        .await?;

    let approval_id = created.id;
    if created.status != ApprovalStatus::Pending {
        return Ok(permission_outcome(&created));
    }

    // Poll until resolved.
    for _ in 0..600 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let approval: ApprovalRequest = client
            .get(format!(
                "{api_url}/internal/v1/runs/{run_id}/approvals/{approval_id}"
            ))
            .bearer_auth(token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if approval.status != ApprovalStatus::Pending {
            return Ok(permission_outcome(&approval));
        }
    }
    bail!("approval {approval_id} timed out")
}

async fn persist_runtime_session(
    client: &reqwest::Client,
    cli: &Cli,
    run_id: Uuid,
    fence: &WorkerFence,
    session_id: &str,
) -> Result<()> {
    let request = WorkerSessionRequest {
        session_id: session_id.to_string(),
        fence: Some(fence.clone()),
    };
    client
        .post(format!(
            "{}/internal/v1/runs/{run_id}/runtime-session",
            cli.api_url
        ))
        .bearer_auth(&cli.worker_token)
        .json(&request)
        .send()
        .await?
        .error_for_status()
        .context("persist runtime session")?;
    Ok(())
}

async fn ack_command(
    client: &reqwest::Client,
    api_url: &str,
    token: &str,
    run_id: Uuid,
    fence: &WorkerFence,
    message_id: Uuid,
) -> Result<()> {
    let request = WorkerCommandAckRequest {
        message_id,
        fence: fence.clone(),
    };
    client
        .post(format!("{api_url}/internal/v1/runs/{run_id}/commands/ack"))
        .bearer_auth(token)
        .json(&request)
        .send()
        .await?
        .error_for_status()
        .context("ack worker command")?;
    Ok(())
}

async fn fetch_commands(
    client: &reqwest::Client,
    api_url: &str,
    token: &str,
    run_id: Uuid,
    fence: &WorkerFence,
) -> Result<WorkerCommandsResponse> {
    let response: WorkerCommandsResponse = client
        .get(format!(
            "{api_url}/internal/v1/runs/{run_id}/commands?attemptId={}&generation={}&workerId={}",
            fence.attempt_id, fence.generation, fence.worker_id
        ))
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(response)
}

async fn take_pending_mode(
    client: &reqwest::Client,
    cli: &Cli,
    run_id: Uuid,
) -> Result<Option<String>> {
    let resp = client
        .post(format!(
            "{}/internal/v1/runs/{run_id}/mode/take",
            cli.api_url
        ))
        .bearer_auth(&cli.worker_token)
        .send()
        .await?
        .error_for_status()?
        .json::<serde_json::Value>()
        .await?;
    Ok(resp
        .get("modeId")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty()))
}

async fn set_pending_mode(
    client: &reqwest::Client,
    api_url: &str,
    token: &str,
    run_id: Uuid,
    mode_id: &str,
) -> Result<()> {
    client
        .post(format!("{api_url}/internal/v1/runs/{run_id}/mode"))
        .bearer_auth(token)
        .json(&serde_json::json!({ "modeId": mode_id }))
        .send()
        .await?
        .error_for_status()
        .context("set pending mode")?;
    Ok(())
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

fn github_token_refresh_loop(
    client: reqwest::Client,
    cli: Cli,
    run_id: Uuid,
    dir: PathBuf,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(10 * 60)).await;
            match fetch_clone_auth(&client, &cli, run_id).await {
                Ok(auth) if !auth.mock => {
                    if let Some(token) = auth.token.as_deref().filter(|t| !t.is_empty()) {
                        if let Err(err) = github_auth::write_token_file(&dir, token).await {
                            warn!(run_id = %run_id, error = %err, "failed to refresh GitHub token file");
                        } else {
                            info!(run_id = %run_id, "refreshed GitHub installation token");
                        }
                    }
                }
                Ok(_) => {}
                Err(err) => {
                    warn!(run_id = %run_id, error = %err, "clone-auth refresh failed");
                }
            }
        }
    })
}

fn heartbeat_loop(
    client: reqwest::Client,
    api_url: String,
    token: String,
    run_id: Uuid,
    fence: WorkerFence,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            for attempt in 0..3_u32 {
                let ok = client
                    .post(format!("{api_url}/internal/v1/runs/{run_id}/heartbeat"))
                    .bearer_auth(&token)
                    .json(&fence)
                    .send()
                    .await
                    .map(|response| response.status().is_success())
                    .unwrap_or(false);
                if ok {
                    break;
                }
                if attempt < 2 {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
            tokio::time::sleep(Duration::from_secs(15)).await;
        }
    })
}

#[cfg(test)]
use event_outbox::event_file_key;
use event_outbox::EventOutbox;

async fn deliver_event(
    outbox: &EventOutbox,
    client: &reqwest::Client,
    api_url: &str,
    token: &str,
    run_id: Uuid,
    event: WorkerEventRequest,
    fence: &WorkerFence,
) -> Result<()> {
    outbox.enqueue(&event).await?;
    outbox.flush(client, api_url, token, run_id, fence).await
}

async fn post_event_raw(
    client: &reqwest::Client,
    api_url: &str,
    token: &str,
    run_id: Uuid,
    mut event: WorkerEventRequest,
    fence: &WorkerFence,
) -> Result<()> {
    event.fence = Some(fence.clone());
    let url = format!("{api_url}/internal/v1/runs/{run_id}/events");
    let mut delay = Duration::from_millis(100);
    for attempt in 0..4 {
        let response = client
            .post(&url)
            .bearer_auth(token)
            .json(&event)
            .send()
            .await;
        match response {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(response) => {
                let status = response.status();
                let retryable = status == reqwest::StatusCode::REQUEST_TIMEOUT
                    || status == reqwest::StatusCode::TOO_MANY_REQUESTS
                    || status.is_server_error();
                if !retryable || attempt == 3 {
                    bail!("post event rejected with HTTP {status}");
                }
            }
            Err(err) if attempt == 3 => return Err(err).context("post event request"),
            Err(_) => {}
        }
        tokio::time::sleep(delay).await;
        delay = delay.saturating_mul(2);
    }
    unreachable!("event retry loop returns on success or final failure")
}

async fn set_status(
    client: &reqwest::Client,
    cli: &Cli,
    run_id: Uuid,
    fence: &WorkerFence,
    status: RunStatus,
    head_sha: Option<String>,
    failure_code: Option<String>,
) -> Result<()> {
    post_run_status(
        client,
        &cli.api_url,
        &cli.worker_token,
        run_id,
        fence,
        status,
        head_sha,
        failure_code,
    )
    .await
}

async fn set_status_raw(
    client: &reqwest::Client,
    api_url: &str,
    token: &str,
    run_id: Uuid,
    fence: &WorkerFence,
    status: RunStatus,
) -> Result<()> {
    post_run_status(client, api_url, token, run_id, fence, status, None, None).await
}

async fn post_run_status(
    client: &reqwest::Client,
    api_url: &str,
    token: &str,
    run_id: Uuid,
    fence: &WorkerFence,
    status: RunStatus,
    head_sha: Option<String>,
    failure_code: Option<String>,
) -> Result<()> {
    let body = WorkerStatusRequest {
        status,
        head_sha,
        failure_code,
        fence: Some(fence.clone()),
    };
    client
        .post(format!("{api_url}/internal/v1/runs/{run_id}/status"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await?
        .error_for_status()
        .context("set status")?;
    Ok(())
}

#[cfg(test)]
mod title_tests {
    use super::{
        event_file_key, idle_hold_elapsed, outcome_for_hold, EventOutbox, HoldExit, RunOutcome,
    };
    use crate::git::{git_command, prepare_workspace, workspace_ready};
    use crate::title::{
        chat_completions_url, format_title_focus, sanitize_run_title, title_echoes_source,
        title_from_chat_response, title_is_user_locked, title_looks_like_question,
        title_needs_rewrite,
    };
    use futures::StreamExt;
    use serde_json::json;
    use std::future::IntoFuture;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use uuid::Uuid;
    use zene_cloud_domain::{CloneAuthResponse, RunEventKind, WorkerEventRequest, WorkerFence};

    #[test]
    fn completions_url_appends_path() {
        assert_eq!(
            chat_completions_url("https://api.deepseek.com/v1"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_url("https://open.bigmodel.cn/api/paas/v4/"),
            "https://open.bigmodel.cn/api/paas/v4/chat/completions"
        );
        assert_eq!(
            chat_completions_url("https://api.openai.com/v1/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn sanitize_strips_wrapping() {
        assert_eq!(sanitize_run_title("  \"项目总结\"  "), "项目总结");
        assert_eq!(sanitize_run_title("# Fix login bug\nmore"), "Fix login bug");
    }

    #[test]
    fn title_rejects_prompt_echo_and_questions() {
        assert!(title_echoes_source(
            "sglang 目前性能怎么样",
            &["sglang 目前性能怎么样"]
        ));
        assert!(title_echoes_source(
            "SGLang 目前性能怎么样？",
            &["sglang 目前性能怎么样"]
        ));
        assert!(!title_echoes_source(
            "SGLang 性能分析",
            &["sglang 目前性能怎么样"]
        ));
        let long = "请用 Rust 逐步分析多线程 Tokio 异步队列可能出现的死锁根因，并给出推导证明与完整的重构代码";
        assert!(title_echoes_source(
            "请用 Rust 逐步分析多线程 Tokio 异步队列可…",
            &[long]
        ));
        assert!(title_needs_rewrite(
            "请用 Rust 逐步分析多线程 Tokio 异步队列",
            &[long]
        ));
        assert!(title_looks_like_question("sglang 目前性能怎么样"));
        assert!(title_needs_rewrite(
            "sglang 目前性能怎么样",
            &["sglang 目前性能怎么样"]
        ));
        assert!(!title_needs_rewrite(
            "SGLang 性能分析",
            &["sglang 目前性能怎么样"]
        ));
    }

    #[test]
    fn title_from_chat_reads_string_or_parts() {
        assert_eq!(
            title_from_chat_response(&json!({
                "choices": [{ "message": { "content": "SGLang 性能分析" } }]
            })),
            "SGLang 性能分析"
        );
        assert_eq!(
            title_from_chat_response(&json!({
                "choices": [{ "message": { "content": [{ "type": "text", "text": "SGLang 性能分析" }] } }]
            })),
            "SGLang 性能分析"
        );
    }

    #[test]
    fn title_focus_includes_recent_follow_ups() {
        let focus = format_title_focus("检查一下项目", &["服务器不动，继续用".into()]);
        assert!(focus.contains("Original task:\n检查一下项目"));
        assert!(focus.contains("1. 服务器不动，继续用"));
    }

    #[test]
    fn user_rename_locks_auto_title() {
        assert!(!title_is_user_locked("检查一下项目", None, "检查一下项目"));
        assert!(title_is_user_locked("我改的标题", None, "检查一下项目"));
        assert!(!title_is_user_locked(
            "SSH 优化服务器",
            Some("SSH 优化服务器"),
            "检查一下项目"
        ));
        assert!(title_is_user_locked(
            "手动标题",
            Some("SSH 优化服务器"),
            "检查一下项目"
        ));
    }

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

    #[test]
    fn event_file_key_is_stable_and_safe() {
        let key = event_file_key("acp/event/1");
        assert_eq!(key.len(), 32);
        assert!(!key.contains('/'));
        assert_eq!(event_file_key("acp/event/1"), key);
        assert_eq!(event_file_key(&"x".repeat(100_000)).len(), 32);
    }

    fn event(source_event_id: &str) -> WorkerEventRequest {
        WorkerEventRequest {
            source_event_id: source_event_id.into(),
            cursor: Some(7),
            event_type: RunEventKind::Acp,
            payload: json!({"ok": true}),
            fence: None,
        }
    }

    #[tokio::test]
    async fn event_outbox_survives_reopen_without_tmp_files() {
        let root = std::env::temp_dir().join(format!("zene-worker-outbox-{}", Uuid::new_v4()));
        let run_id = Uuid::new_v4();
        let outbox = EventOutbox::open(&root, run_id).await.unwrap();
        let event = event("event-1");
        outbox.enqueue(&event).await.unwrap();
        outbox.enqueue(&event).await.unwrap();
        assert_eq!(outbox.stats().await.unwrap().0, 1);
        let orphan = outbox.dir.join(".orphan.json.tmp-crashed");
        tokio::fs::write(&orphan, b"partial").await.unwrap();

        let reopened = EventOutbox::open(&root, run_id).await.unwrap();
        let path = reopened.event_path("event-1");
        let stored: WorkerEventRequest =
            serde_json::from_slice(&tokio::fs::read(&path).await.unwrap()).unwrap();
        assert_eq!(stored.source_event_id, "event-1");
        assert_eq!(stored.cursor, Some(7));
        assert!(!orphan.exists());
        let mut entries = tokio::fs::read_dir(&reopened.dir).await.unwrap();
        let mut event_files = Vec::new();
        while let Some(entry) = entries.next_entry().await.unwrap() {
            if entry.path().extension().and_then(|ext| ext.to_str()) == Some("json") {
                event_files.push(entry.path());
            }
        }
        assert_eq!(event_files.len(), 1);

        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn event_outbox_concurrent_same_event_is_idempotent() {
        let root = std::env::temp_dir().join(format!("zene-worker-outbox-{}", Uuid::new_v4()));
        let outbox = EventOutbox::open(&root, Uuid::new_v4()).await.unwrap();
        let event = event("concurrent-event");
        let (left, right) = tokio::join!(outbox.enqueue(&event), outbox.enqueue(&event));
        left.unwrap();
        right.unwrap();
        assert_eq!(
            outbox.stats().await.unwrap(),
            (
                1,
                outbox
                    .event_path("concurrent-event")
                    .metadata()
                    .unwrap()
                    .len()
            )
        );
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn event_outbox_rejects_source_id_collision() {
        let root = std::env::temp_dir().join(format!("zene-worker-outbox-{}", Uuid::new_v4()));
        let outbox = EventOutbox::open(&root, Uuid::new_v4()).await.unwrap();
        let first = event("first");
        let second = event("second");
        tokio::fs::write(
            outbox.event_path("first"),
            serde_json::to_vec(&second).unwrap(),
        )
        .await
        .unwrap();
        let error = outbox
            .enqueue(&first)
            .await
            .expect_err("collision must fail");
        assert!(error.to_string().contains("collision"));
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn event_outbox_retries_transient_http_failure_before_acknowledging() {
        let root = std::env::temp_dir().join(format!("zene-worker-outbox-{}", Uuid::new_v4()));
        let run_id = Uuid::new_v4();
        let outbox = EventOutbox::open(&root, run_id).await.unwrap();
        outbox.enqueue(&event("retry-event")).await.unwrap();

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for response in [
                b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".as_slice(),
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".as_slice(),
            ] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buffer = [0_u8; 4096];
                let _ = stream.read(&mut buffer).await.unwrap();
                stream.write_all(response).await.unwrap();
            }
        });
        let fence = WorkerFence {
            attempt_id: Uuid::new_v4(),
            generation: 1,
            worker_id: "worker-retry".into(),
        };
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            outbox.flush(
                &reqwest::Client::new(),
                &format!("http://{address}"),
                "worker-token",
                run_id,
                &fence,
            ),
        )
        .await
        .expect("retrying flush should complete")
        .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), server)
            .await
            .expect("retry server should complete")
            .unwrap();
        assert_eq!(outbox.stats().await.unwrap(), (0, 0));
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn event_outbox_retains_event_after_non_retryable_http_failure() {
        let root = std::env::temp_dir().join(format!("zene-worker-outbox-{}", Uuid::new_v4()));
        let run_id = Uuid::new_v4();
        let outbox = EventOutbox::open(&root, run_id).await.unwrap();
        let queued = event("rejected-event");
        outbox.enqueue(&queued).await.unwrap();

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer).await.unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let fence = WorkerFence {
            attempt_id: Uuid::new_v4(),
            generation: 1,
            worker_id: "worker-reject".into(),
        };
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            outbox.flush(
                &reqwest::Client::new(),
                &format!("http://{address}"),
                "worker-token",
                run_id,
                &fence,
            ),
        )
        .await
        .expect("non-retryable flush should complete")
        .expect_err("HTTP 400 must be surfaced");
        assert!(error.to_string().contains("400"));
        tokio::time::timeout(std::time::Duration::from_secs(5), server)
            .await
            .expect("reject server should complete")
            .unwrap();
        assert_eq!(outbox.stats().await.unwrap().0, 1);
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn event_outbox_reconnects_to_real_api_and_sse_replays_after_replacement() {
        use zene_cloud_api::{router, AppState};
        use zene_cloud_db::Db;
        use zene_cloud_domain::{
            CreateRepositoryRequest, CreateRunRequest, PermissionMode, RegisterRequest, RunEvent,
            RunStatus, UpdateLlmSettingsRequest, WorkerClaimRequest,
        };
        use zene_cloud_github::{GithubClient, GithubConfig};

        let db = Db::connect("sqlite::memory:").await.unwrap();
        db.migrate().await.unwrap();
        let worker_token = "real-api-worker-token";
        db.ensure_dev_worker_token(worker_token).await.unwrap();
        let root = std::env::temp_dir().join(format!("zene-worker-api-{}", Uuid::new_v4()));
        let workspace_root = root.join("workspaces");
        tokio::fs::create_dir_all(&workspace_root).await.unwrap();
        let state = AppState::new(
            db.clone(),
            worker_token.into(),
            GithubClient::new(GithubConfig::live_default()),
            workspace_root.clone(),
            "http://127.0.0.1".into(),
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let api_task = tokio::spawn(axum::serve(listener, router(state.clone())).into_future());
        let api_url = format!("http://{address}");
        let client = reqwest::Client::new();

        let auth: zene_cloud_domain::AuthResponse = client
            .post(format!("{api_url}/api/v1/auth/register"))
            .json(&RegisterRequest {
                email: "worker-reconnect@example.com".into(),
                password: "password123".into(),
                display_name: "Worker Reconnect".into(),
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        let repo: zene_cloud_domain::Repository = client
            .post(format!("{api_url}/api/v1/repositories"))
            .bearer_auth(&auth.token)
            .json(&CreateRepositoryRequest {
                owner: "worker".into(),
                name: "reconnect".into(),
                default_branch: "main".into(),
                clone_url: None,
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        db.upsert_user_llm_settings(
            auth.user.id,
            UpdateLlmSettingsRequest {
                provider_id: "test".into(),
                base_url: "https://llm.invalid".into(),
                default_model: "default".into(),
                models: vec!["default".into()],
                api_key: Some("test-key".into()),
            },
        )
        .await
        .unwrap();
        let run: zene_cloud_domain::Run = client
            .post(format!("{api_url}/api/v1/runs"))
            .bearer_auth(&auth.token)
            .json(&CreateRunRequest {
                repository_id: repo.id,
                prompt: "worker reconnect".into(),
                base_ref: Some("main".into()),
                model: "default".into(),
                permission_mode: PermissionMode::Default,
                max_turns: 10,
                mode_id: None,
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();

        let claim: zene_cloud_domain::ClaimedRun = client
            .post(format!("{api_url}/internal/v1/runs/claim"))
            .bearer_auth(worker_token)
            .json(&WorkerClaimRequest {
                worker_id: "worker-1".into(),
                workspace_root: workspace_root.to_string_lossy().into(),
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(claim.run.id, run.id);
        let first_root = root.join("first-worker");
        let first_outbox = EventOutbox::open(&first_root, run.id).await.unwrap();
        let first_event = WorkerEventRequest {
            source_event_id: "real-provider-event-1".into(),
            cursor: Some(21),
            event_type: RunEventKind::Runtime,
            payload: json!({"marker": "first"}),
            fence: None,
        };
        first_outbox.enqueue(&first_event).await.unwrap();
        drop(first_outbox);

        db.update_run_status(run.id, RunStatus::Failed, None, Some("worker_lost".into()))
            .await
            .unwrap();
        db.update_run_status(run.id, RunStatus::Queued, None, None)
            .await
            .unwrap();
        let replacement: zene_cloud_domain::ClaimedRun = client
            .post(format!("{api_url}/internal/v1/runs/claim"))
            .bearer_auth(worker_token)
            .json(&WorkerClaimRequest {
                worker_id: "worker-2".into(),
                workspace_root: workspace_root.to_string_lossy().into(),
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        let second_fence = WorkerFence {
            attempt_id: replacement.attempt_id,
            generation: replacement.generation,
            worker_id: "worker-2".into(),
        };
        let replacement_outbox = EventOutbox::open(&first_root, run.id).await.unwrap();
        replacement_outbox
            .flush(&client, &api_url, worker_token, run.id, &second_fence)
            .await
            .unwrap();
        assert_eq!(replacement_outbox.stats().await.unwrap(), (0, 0));

        let first: RunEvent = db
            .events_after(run.id, 0)
            .await
            .unwrap()
            .into_iter()
            .find(|event| event.event_type == "runtime")
            .unwrap();
        let second_event = WorkerEventRequest {
            source_event_id: "real-provider-event-2".into(),
            cursor: Some(22),
            event_type: RunEventKind::Runtime,
            payload: json!({"marker": "second"}),
            fence: None,
        };
        replacement_outbox.enqueue(&second_event).await.unwrap();
        replacement_outbox
            .flush(&client, &api_url, worker_token, run.id, &second_fence)
            .await
            .unwrap();

        let response = client
            .get(format!("{api_url}/api/v1/runs/{}/events/stream", run.id))
            .bearer_auth(&auth.token)
            .header("Last-Event-ID", first.seq.to_string())
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let mut stream = response.bytes_stream();
        let mut sse = String::new();
        while !sse.contains("second") {
            let chunk = tokio::time::timeout(Duration::from_secs(3), stream.next())
                .await
                .expect("SSE replay timeout")
                .expect("SSE closed")
                .unwrap();
            sse.push_str(&String::from_utf8_lossy(&chunk));
        }
        assert!(sse.contains("second"));
        assert!(!sse.contains("first"));

        api_task.abort();
        let _ = api_task.await;
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn event_outbox_reopen_flushes_pending_event_and_removes_acknowledged_file() {
        let root = std::env::temp_dir().join(format!("zene-worker-outbox-{}", Uuid::new_v4()));
        let run_id = Uuid::new_v4();
        let first_worker = EventOutbox::open(&root, run_id).await.unwrap();
        first_worker.enqueue(&event("restart-event")).await.unwrap();
        drop(first_worker);

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            let body_start = loop {
                let read = stream.read(&mut buffer).await.unwrap();
                assert!(read > 0, "client closed before sending request");
                request.extend_from_slice(&buffer[..read]);
                if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    break position + 4;
                }
            };
            let headers = String::from_utf8_lossy(&request[..body_start]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.strip_prefix("Content-Length:")
                        .or_else(|| line.strip_prefix("content-length:"))
                })
                .and_then(|value| value.trim().parse::<usize>().ok())
                .expect("event POST should include content length");
            while request.len() < body_start + content_length {
                let read = stream.read(&mut buffer).await.unwrap();
                assert!(read > 0, "client closed before sending request body");
                request.extend_from_slice(&buffer[..read]);
            }
            let payload: WorkerEventRequest =
                serde_json::from_slice(&request[body_start..body_start + content_length]).unwrap();
            assert_eq!(payload.source_event_id, "restart-event");
            assert_eq!(payload.cursor, Some(7));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
                .await
                .unwrap();
        });

        let reopened = EventOutbox::open(&root, run_id).await.unwrap();
        let fence = WorkerFence {
            attempt_id: Uuid::new_v4(),
            generation: 2,
            worker_id: "replacement-worker".into(),
        };
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            reopened.flush(
                &reqwest::Client::new(),
                &format!("http://{address}"),
                "worker-token",
                run_id,
                &fence,
            ),
        )
        .await
        .expect("outbox flush should complete")
        .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), server)
            .await
            .expect("mock server should complete")
            .unwrap();
        assert_eq!(reopened.stats().await.unwrap(), (0, 0));
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn prepare_workspace_uses_git_worktree_from_cache() {
        let root = std::env::temp_dir().join(format!("zene-wt-test-{}", Uuid::new_v4()));
        let repo_id = Uuid::new_v4();
        let cache = root.join(".repo-cache").join(repo_id.to_string());
        std::fs::create_dir_all(&cache).unwrap();

        // Initialize a dummy bare repo
        let _ = git_command()
            .args(["-C", &cache.display().to_string(), "init", "--bare"])
            .status()
            .await
            .unwrap();
        // Create an initial commit in a temp worktree to have a valid HEAD
        let init_wt = root.join("init-wt");
        std::fs::create_dir_all(&init_wt).unwrap();
        let _ = git_command()
            .args(["-C", &init_wt.display().to_string(), "init"])
            .status()
            .await
            .unwrap();
        tokio::fs::write(init_wt.join("hello.txt"), "hello")
            .await
            .unwrap();
        let _ = git_command()
            .args(["-C", &init_wt.display().to_string(), "add", "."])
            .status()
            .await
            .unwrap();
        let _ = git_command()
            .args([
                "-C",
                &init_wt.display().to_string(),
                "-c",
                "user.email=test@test.com",
                "-c",
                "user.name=Test",
                "commit",
                "-m",
                "initial commit",
            ])
            .status()
            .await
            .unwrap();
        let _ = git_command()
            .args([
                "-C",
                &init_wt.display().to_string(),
                "push",
                &cache.display().to_string(),
                "HEAD:refs/heads/main",
            ])
            .status()
            .await
            .unwrap();
        let _ = git_command()
            .args([
                "-C",
                &cache.display().to_string(),
                "symbolic-ref",
                "HEAD",
                "refs/heads/main",
            ])
            .status()
            .await
            .unwrap();

        let auth = CloneAuthResponse {
            run_id: Uuid::new_v4(),
            repository_id: repo_id,
            clone_url: "https://example.com/test.git".into(),
            token: None,
            username: None,
            base_ref: "main".into(),
            head_branch: "zene/test-branch".into(),
            mock: false,
        };

        let ws = root.join("ws").join(Uuid::new_v4().to_string());
        prepare_workspace(&root, &ws, &auth).await.unwrap();
        assert!(ws.join("hello.txt").exists());
        assert!(workspace_ready(&ws).await);

        let _ = tokio::fs::remove_dir_all(root).await;
    }
}
