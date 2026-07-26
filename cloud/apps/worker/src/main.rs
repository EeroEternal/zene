mod supervisor;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::Parser;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;
use zene_cloud_acp_bridge::{
    resolve_zene_bin, AcpBridge, AcpEvent, BridgeMsg, MockAgent, MockMsg, PermissionDecision,
};
use zene_cloud_domain::{
    ApprovalRequest, ApprovalStatus, ClaimedRun, CloneAuthResponse, CreateApprovalRequest,
    LlmAuthResponse, RunStatus, WorkerCommand, WorkerCommandsResponse, WorkerEventRequest,
    WorkerStatusRequest, WorkerTitleRequest,
};

#[derive(Debug, Clone, Parser)]
#[command(name = "zene-cloud-worker")]
pub(crate) struct Cli {
    #[arg(long, env = "ZENE_CLOUD_API_URL", default_value = "http://127.0.0.1:8788")]
    pub(crate) api_url: String,

    #[arg(long, env = "ZENE_CLOUD_WORKER_TOKEN", default_value = "dev-worker-token")]
    pub(crate) worker_token: String,

    #[arg(long, env = "ZENE_CLOUD_WORKER_ID")]
    pub(crate) worker_id: Option<String>,

    #[arg(long, env = "ZENE_CLOUD_WORKSPACE_ROOT", default_value = "./data/workspaces")]
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
    #[arg(long, env = "ZENE_CLOUD_PUSH_PR", default_value_t = true)]
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
    #[arg(long, env = "ZENE_CLOUD_WORKER_SCALE_INTERVAL_MS", default_value_t = 1000)]
    pub(crate) scale_interval_ms: u64,
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
        warn!(agent_mode = "mock", "zene binary missing; using MockAgent (ZENE_CLOUD_ALLOW_MOCK=1)");
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
                    error!(error = %err, run_id = %claimed.run.id, "run failed");
                    let _ = set_status(
                        &client,
                        &cli,
                        claimed.run.id,
                        RunStatus::Failed,
                        None,
                        Some(err.to_string()),
                    )
                    .await;
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

fn spawn_shutdown_listener(shutdown: Arc<AtomicBool>) {
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm =
                match signal(SignalKind::terminate()) {
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

async fn sleep_interruptible(total: Duration, shutdown: &AtomicBool) {
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
        .json(&serde_json::json!({
            "workerId": worker_id,
            "workspaceRoot": cli.workspace_root.display().to_string()
        }))
        .send()
        .await?
        .error_for_status()?;
    Ok(response.json().await?)
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
    set_status(client, cli, run_id, RunStatus::Starting, None, None).await?;

    let workspace = PathBuf::from(&claimed.workspace_dir);
    std::fs::create_dir_all(&workspace)?;
    let hb = heartbeat_loop(
        client.clone(),
        cli.api_url.clone(),
        cli.worker_token.clone(),
        worker_id.to_string(),
        run_id,
    );

    // a) clone credentials
    let clone_auth = fetch_clone_auth(client, cli, run_id).await?;

    // b) clone or mock workspace
    set_status(client, cli, run_id, RunStatus::Cloning, None, None).await?;
    prepare_workspace(&cli.workspace_root, &workspace, &clone_auth).await?;

    set_status(client, cli, run_id, RunStatus::Running, None, None).await?;

    let result = if let Some(bin) = zene_bin {
        info!(run_id = %run_id, agent_mode = "real", "starting agent");
        run_with_real_acp(client, cli, claimed, &workspace, bin, shutdown).await
    } else if cli.allow_mock {
        info!(run_id = %run_id, agent_mode = "mock", "starting agent");
        run_with_mock(client, cli, claimed, &workspace).await
    } else {
        bail!("zene binary not found and mock agent is disabled")
    };

    match result {
        Ok(()) => {
            let head_sha = git_commit_all(&workspace, &claimed.run.title)
                .await
                .ok()
                .flatten();
            if cli.push_pr {
                let _ = post_push(client, cli, run_id).await;
                let _ = post_pull_request(client, cli, run_id, &claimed.run.title).await;
            }
            set_status(
                client,
                cli,
                run_id,
                RunStatus::Completed,
                head_sha,
                None,
            )
            .await?;
            hb.abort();
            Ok(())
        }
        Err(err) => {
            hb.abort();
            Err(err)
        }
    }
}

async fn fetch_clone_auth(
    client: &reqwest::Client,
    cli: &Cli,
    run_id: Uuid,
) -> Result<CloneAuthResponse> {
    let response = client
        .get(format!(
            "{}/internal/v1/runs/{run_id}/clone-auth",
            cli.api_url
        ))
        .bearer_auth(&cli.worker_token)
        .send()
        .await?
        .error_for_status()
        .context("clone-auth")?;
    Ok(response.json().await?)
}

async fn workspace_ready(workspace: &Path) -> bool {
    if !workspace.join(".git").exists() {
        return false;
    }
    // Partial/interrupted clones leave a .git dir with no usable checkout.
    match run_git_output(workspace, &["rev-parse", "--verify", "HEAD"]).await {
        Ok(sha) => !sha.trim().is_empty(),
        Err(_) => false,
    }
}

async fn bare_cache_ready(cache: &Path) -> bool {
    // A bare repo has HEAD at the root (no nested .git).
    if !cache.join("HEAD").exists() {
        return false;
    }
    match run_git_output(cache, &["rev-parse", "--verify", "HEAD"]).await {
        Ok(sha) => !sha.trim().is_empty(),
        Err(_) => false,
    }
}

fn git_command() -> Command {
    let mut cmd = Command::new("git");
    // Developer proxies often throttle or stall github.com clones.
    for key in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        cmd.env_remove(key);
    }
    cmd
}

fn authenticated_clone_url(auth: &CloneAuthResponse) -> String {
    let mut clone_url = auth.clone_url.clone();
    if let Some(token) = &auth.token {
        if let Some(rest) = clone_url.strip_prefix("https://") {
            let user = auth.username.as_deref().unwrap_or("x-access-token");
            clone_url = format!("https://{user}:{token}@{rest}");
        }
    }
    clone_url
}

async fn ensure_repo_cache(cache: &Path, auth: &CloneAuthResponse) -> Result<()> {
    let clone_url = authenticated_clone_url(auth);

    if bare_cache_ready(cache).await {
        info!(
            path = %cache.display(),
            repository_id = %auth.repository_id,
            "updating repo cache"
        );
        // Tokens are short-lived; refresh the remote URL before fetch.
        if let Err(err) = run_git(cache, &["remote", "set-url", "origin", &clone_url]).await {
            warn!(error = %err, "failed to set cache remote url; recloning");
            let _ = tokio::fs::remove_dir_all(cache).await;
        } else {
            let refspec = format!(
                "+refs/heads/{}:refs/heads/{}",
                auth.base_ref, auth.base_ref
            );
            match run_git(
                cache,
                &["fetch", "--depth", "1", "origin", &refspec],
            )
            .await
            {
                Ok(()) => return Ok(()),
                Err(err) => {
                    warn!(
                        error = %err,
                        path = %cache.display(),
                        "cache fetch failed; recloning"
                    );
                    let _ = tokio::fs::remove_dir_all(cache).await;
                }
            }
        }
    } else if cache.exists() {
        warn!(path = %cache.display(), "removing incomplete repo cache");
        let _ = tokio::fs::remove_dir_all(cache).await;
    }

    std::fs::create_dir_all(cache.parent().unwrap_or_else(|| Path::new(".")))?;
    info!(
        url = %auth.clone_url,
        path = %cache.display(),
        repository_id = %auth.repository_id,
        "cloning repository into cache (shallow bare)"
    );
    let status = git_command()
        .args([
            "clone",
            "--bare",
            "--depth",
            "1",
            "--single-branch",
            "--branch",
            &auth.base_ref,
            &clone_url,
            &cache.display().to_string(),
        ])
        .status()
        .await
        .context("git clone --bare")?;
    if !status.success() {
        bail!("git bare clone failed with {status}");
    }
    Ok(())
}

async fn prepare_workspace(
    workspace_root: &Path,
    workspace: &Path,
    auth: &CloneAuthResponse,
) -> Result<()> {
    if workspace_ready(workspace).await {
        info!(path = %workspace.display(), "workspace already initialized");
        return Ok(());
    }
    if workspace.exists() {
        warn!(path = %workspace.display(), "removing incomplete workspace before clone");
        let _ = tokio::fs::remove_dir_all(workspace).await;
    }

    if auth.mock {
        std::fs::create_dir_all(workspace)?;
        info!(path = %workspace.display(), "preparing mock git workspace");
        run_git(workspace, &["init"]).await?;
        if run_git(workspace, &["checkout", "-b", &auth.head_branch])
            .await
            .is_err()
        {
            // git init may already be on main/master
            let _ = run_git(workspace, &["branch", "-M", &auth.head_branch]).await;
        }
        tokio::fs::write(
            workspace.join("README.md"),
            format!(
                "# {}/{}\n\nMock workspace for Zene Cloud Phase 0.\n\nBase ref: `{}`\n",
                "repo", "workspace", auth.base_ref
            ),
        )
        .await?;
        tokio::fs::create_dir_all(workspace.join("src")).await?;
        tokio::fs::write(
            workspace.join("src/main.rs"),
            "fn main() {\n    println!(\"hello cloud\");\n}\n",
        )
        .await?;
        run_git(workspace, &["add", "."]).await?;
        run_git(
            workspace,
            &[
                "-c",
                "user.email=zene-cloud@localhost",
                "-c",
                "user.name=Zene Cloud",
                "commit",
                "-m",
                "chore: initial mock workspace",
            ],
        )
        .await?;
        return Ok(());
    }

    let cache = workspace_root
        .join(".repo-cache")
        .join(auth.repository_id.to_string());
    ensure_repo_cache(&cache, auth).await?;

    std::fs::create_dir_all(
        workspace
            .parent()
            .unwrap_or_else(|| Path::new(".")),
    )?;
    info!(
        cache = %cache.display(),
        path = %workspace.display(),
        "cloning workspace from local cache"
    );
    let status = git_command()
        .args([
            "clone",
            "--local",
            &cache.display().to_string(),
            &workspace.display().to_string(),
        ])
        .status()
        .await
        .context("git clone --local")?;
    if !status.success() {
        bail!("git local clone failed with {status}");
    }
    let _ = run_git(
        workspace,
        &["checkout", "-B", &auth.head_branch, &auth.base_ref],
    )
    .await;
    Ok(())
}

async fn run_with_mock(
    client: &reqwest::Client,
    cli: &Cli,
    claimed: &ClaimedRun,
    workspace: &Path,
) -> Result<()> {
    let run_id = claimed.run.id;
    let agent = MockAgent::new(workspace.to_path_buf());
    let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<MockMsg>();

    let client_bg = client.clone();
    let cli_api = cli.api_url.clone();
    let token = cli.worker_token.clone();
    let event_task = tokio::spawn(async move {
        while let Some(msg) = msg_rx.recv().await {
            match msg {
                MockMsg::Event(event) => {
                    let _ = post_event_raw(
                        &client_bg,
                        &cli_api,
                        &token,
                        run_id,
                        WorkerEventRequest {
                            source_event_id: event.source_event_id,
                            event_type: event.event_type,
                            payload: event.payload,
                        },
                    )
                    .await;
                }
                MockMsg::Permission {
                    request_key,
                    params,
                    respond,
                } => {
                    match resolve_permission(
                        &client_bg,
                        &cli_api,
                        &token,
                        run_id,
                        &request_key,
                        None,
                        "tool",
                        &params,
                    )
                    .await
                    {
                        Ok(decision) => {
                            let _ = respond.send(decision);
                        }
                        Err(err) => {
                            warn!(error = %err, "permission resolve failed");
                            let _ = respond.send(PermissionDecision {
                                option_id: "reject-once".into(),
                            });
                        }
                    }
                }
            }
        }
    });

    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancel_flag = cancelled.clone();
    let client_cmd = client.clone();
    let cli_cmd = cli.api_url.clone();
    let token_cmd = cli.worker_token.clone();
    let cmd_task = tokio::spawn(async move {
        loop {
            if let Ok(commands) = fetch_commands(&client_cmd, &cli_cmd, &token_cmd, run_id).await {
                for cmd in commands {
                    if cmd.kind == "cancel" {
                        cancel_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                        return;
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });

    let mut prompts = vec![claimed.run.prompt.clone()];
    // Drain any follow-ups already waiting.
    if let Ok(commands) = fetch_commands(client, &cli.api_url, &cli.worker_token, run_id).await {
        for cmd in commands {
            if cmd.kind == "cancel" {
                cmd_task.abort();
                event_task.abort();
                set_status(client, cli, run_id, RunStatus::Cancelled, None, None).await?;
                return Ok(());
            }
            if cmd.kind == "prompt" {
                if let Some(text) = cmd.text {
                    prompts.push(text);
                }
            }
        }
    }

    for prompt in prompts {
        if cancelled.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        agent.run_prompt(&prompt, msg_tx.clone()).await?;
    }

    // Keep listening briefly for follow-up prompts.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if cancelled.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        if let Ok(commands) = fetch_commands(client, &cli.api_url, &cli.worker_token, run_id).await {
            for cmd in commands {
                if cmd.kind == "cancel" {
                    cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
                    break;
                }
                if let Some(text) = cmd.text {
                    if cmd.kind == "prompt" {
                        agent.run_prompt(&text, msg_tx.clone()).await?;
                        // extend wait window a bit after each follow-up
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    drop(msg_tx);
    let _ = event_task.await;
    cmd_task.abort();

    if cancelled.load(std::sync::atomic::Ordering::SeqCst) {
        set_status(client, cli, run_id, RunStatus::Cancelled, None, None).await?;
    }
    Ok(())
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
    workspace: &Path,
    zene_bin: &Path,
    shutdown: Arc<AtomicBool>,
) -> Result<()> {
    let run_id = claimed.run.id;
    let yolo = cli.acp_yolo
        || claimed.run.permission_mode == "yolo"
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
    info!(
        run_id = %run_id,
        max_turns = claimed.run.max_turns,
        "injected ZENE_MAX_TURNS for acp"
    );

    let (bridge, mut msg_rx) = AcpBridge::spawn(zene_bin, workspace, yolo, &llm_env).await?;
    let bridge = std::sync::Arc::new(tokio::sync::Mutex::new(Some(bridge)));

    let (session_id, init_events) = {
        let mut guard = bridge.lock().await;
        let b = guard.as_mut().context("bridge missing")?;
        b.initialize_and_new_session(workspace).await?
    };
    for event in init_events {
        post_event(client, cli, run_id, event_to_req(event)).await?;
    }

    let client_bg = client.clone();
    let cli_api = cli.api_url.clone();
    let token = cli.worker_token.clone();
    let bridge_bg = bridge.clone();
    let pump = tokio::spawn(async move {
        while let Some(msg) = msg_rx.recv().await {
            match msg {
                BridgeMsg::Notification { raw, .. } => {
                    let event = AcpEvent::from_notification(&raw);
                    let _ = post_event_raw(
                        &client_bg,
                        &cli_api,
                        &token,
                        run_id,
                        event_to_req(event),
                    )
                    .await;
                }
                BridgeMsg::ReverseRequest { id, method, params } => {
                    let event = AcpEvent::from_reverse_request(&id, &method, &params);
                    let _ = post_event_raw(
                        &client_bg,
                        &cli_api,
                        &token,
                        run_id,
                        event_to_req(event),
                    )
                    .await;

                    if method == "session/request_permission" {
                        let request_key = params
                            .pointer("/toolCall/toolCallId")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| format!("perm-{}", Uuid::new_v4()));
                        let decision = match resolve_permission(
                            &client_bg,
                            &cli_api,
                            &token,
                            run_id,
                            &request_key,
                            Some(&id_to_string(&id)),
                            "permission",
                            &params,
                        )
                        .await
                        {
                            Ok(d) => d,
                            Err(err) => {
                                warn!(error = %err, "permission resolve failed");
                                PermissionDecision {
                                    option_id: "reject-once".into(),
                                }
                            }
                        };
                        let mut guard = bridge_bg.lock().await;
                        if let Some(b) = guard.as_mut() {
                            let _ = b.respond(&id, decision.to_result()).await;
                        }
                    } else {
                        // Unsupported reverse requests: reject.
                        let mut guard = bridge_bg.lock().await;
                        if let Some(b) = guard.as_mut() {
                            let _ = b
                                .respond_error(&id, -32601, &format!("unsupported: {method}"))
                                .await;
                        }
                    }
                }
            }
        }
    });

    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let last_activity = std::sync::Arc::new(tokio::sync::Mutex::new(tokio::time::Instant::now()));
    let session_for_cancel = session_id.clone();
    let bridge_cancel = bridge.clone();
    let cancel_flag = cancelled.clone();
    let activity_cmd = last_activity.clone();
    let client_cmd = client.clone();
    let cli_cmd = cli.api_url.clone();
    let token_cmd = cli.worker_token.clone();
    let cmd_task = tokio::spawn(async move {
        loop {
            match fetch_commands(&client_cmd, &cli_cmd, &token_cmd, run_id).await {
                Ok(commands) => {
                    for cmd in commands {
                        if cmd.kind == "cancel" {
                            cancel_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                            let mut guard = bridge_cancel.lock().await;
                            if let Some(b) = guard.as_mut() {
                                let _ = b.cancel(&session_for_cancel).await;
                            }
                            return;
                        }
                        if cmd.kind == "prompt" {
                            if let Some(text) = cmd.text {
                                {
                                    let mut ts = activity_cmd.lock().await;
                                    *ts = tokio::time::Instant::now();
                                }
                                let _ = set_status_raw(
                                    &client_cmd,
                                    &cli_cmd,
                                    &token_cmd,
                                    run_id,
                                    RunStatus::Running,
                                )
                                .await;
                                let mut guard = bridge_cancel.lock().await;
                                if let Some(b) = guard.as_mut() {
                                    if let Err(err) = b.prompt(&session_for_cancel, &text).await {
                                        warn!(error = %err, "follow-up prompt failed");
                                    }
                                }
                                {
                                    let mut ts = activity_cmd.lock().await;
                                    *ts = tokio::time::Instant::now();
                                }
                                if !cancel_flag.load(std::sync::atomic::Ordering::SeqCst) {
                                    let _ = set_status_raw(
                                        &client_cmd,
                                        &cli_cmd,
                                        &token_cmd,
                                        run_id,
                                        RunStatus::WaitingForUser,
                                    )
                                    .await;
                                }
                            }
                        }
                    }
                }
                Err(err) => warn!(error = %err, "commands poll failed"),
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });

    {
        let mut guard = bridge.lock().await;
        let b = guard.as_mut().context("bridge missing")?;
        b.prompt(&session_id, &claimed.run.prompt)
            .await
            .context("session/prompt")?;
    }
    {
        let mut ts = last_activity.lock().await;
        *ts = tokio::time::Instant::now();
    }
    // Turn finished — free the UI for follow-ups while ACP session stays warm.
    if !cancelled.load(std::sync::atomic::Ordering::SeqCst) {
        set_status(client, cli, run_id, RunStatus::WaitingForUser, None, None).await?;
        if let Err(err) =
            maybe_refresh_run_title(client, cli, run_id, &claimed.run.prompt, &llm_env).await
        {
            warn!(run_id = %run_id, error = %err, "run title refresh failed");
        }
    }

    // Keep the session alive for follow-ups until idle timeout, cancel, SIGTERM, or child exit.
    let idle = Duration::from_secs(cli.acp_idle_secs.max(1));
    loop {
        if cancelled.load(Ordering::SeqCst) {
            break;
        }
        if shutdown.load(Ordering::SeqCst) {
            info!(run_id = %run_id, "ending hold due to shutdown signal");
            break;
        }
        {
            let mut guard = bridge.lock().await;
            if let Some(b) = guard.as_mut() {
                if b.child_exited() {
                    info!(run_id = %run_id, "acp child exited");
                    break;
                }
            } else {
                break;
            }
        }
        let elapsed = {
            let ts = last_activity.lock().await;
            ts.elapsed()
        };
        if elapsed >= idle {
            info!(run_id = %run_id, idle_secs = cli.acp_idle_secs, "acp idle timeout");
            break;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }

    cmd_task.abort();
    {
        let mut guard = bridge.lock().await;
        if let Some(b) = guard.take() {
            let _ = b.kill().await;
        }
    }
    pump.abort();

    if cancelled.load(std::sync::atomic::Ordering::SeqCst) {
        set_status(client, cli, run_id, RunStatus::Cancelled, None, None).await?;
    }
    Ok(())
}

async fn resolve_permission(
    client: &reqwest::Client,
    api_url: &str,
    token: &str,
    run_id: Uuid,
    request_key: &str,
    jsonrpc_id: Option<&str>,
    kind: &str,
    payload: &serde_json::Value,
) -> Result<PermissionDecision> {
    let body = CreateApprovalRequest {
        request_key: request_key.to_string(),
        jsonrpc_id: jsonrpc_id.map(|s| s.to_string()),
        kind: kind.to_string(),
        risk: "medium".into(),
        payload: payload.clone(),
        allowed_decisions: vec![
            "allow-once".into(),
            "allow-always".into(),
            "reject-once".into(),
        ],
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
        return Ok(PermissionDecision {
            option_id: created
                .decision
                .unwrap_or_else(|| "allow-once".into()),
        });
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
            return Ok(PermissionDecision {
                option_id: approval
                    .decision
                    .unwrap_or_else(|| "allow-once".into()),
            });
        }
    }
    bail!("approval {approval_id} timed out")
}

async fn fetch_commands(
    client: &reqwest::Client,
    api_url: &str,
    token: &str,
    run_id: Uuid,
) -> Result<Vec<WorkerCommand>> {
    let response: WorkerCommandsResponse = client
        .get(format!("{api_url}/internal/v1/runs/{run_id}/commands"))
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(response.commands)
}

fn event_to_req(event: AcpEvent) -> WorkerEventRequest {
    WorkerEventRequest {
        source_event_id: event.source_event_id,
        event_type: event.event_type,
        payload: event.payload,
    }
}

fn id_to_string(id: &serde_json::Value) -> String {
    match id {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn heartbeat_loop(
    client: reqwest::Client,
    api_url: String,
    token: String,
    worker_id: String,
    run_id: Uuid,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let _ = client
                .post(format!("{api_url}/internal/v1/runs/{run_id}/heartbeat"))
                .bearer_auth(&token)
                .json(&serde_json::json!({
                    "workerId": worker_id,
                    "workspaceRoot": "."
                }))
                .send()
                .await;
            tokio::time::sleep(Duration::from_secs(20)).await;
        }
    })
}

async fn post_event(
    client: &reqwest::Client,
    cli: &Cli,
    run_id: Uuid,
    event: WorkerEventRequest,
) -> Result<()> {
    post_event_raw(
        client,
        &cli.api_url,
        &cli.worker_token,
        run_id,
        event,
    )
    .await
}

async fn post_event_raw(
    client: &reqwest::Client,
    api_url: &str,
    token: &str,
    run_id: Uuid,
    event: WorkerEventRequest,
) -> Result<()> {
    client
        .post(format!("{api_url}/internal/v1/runs/{run_id}/events"))
        .bearer_auth(token)
        .json(&event)
        .send()
        .await?
        .error_for_status()
        .context("post event")?;
    Ok(())
}

fn chat_completions_url(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else {
        // Presets already include the API root (/v1, /compatible-mode/v1, /paas/v4, …).
        format!("{base}/chat/completions")
    }
}

fn sanitize_run_title(raw: &str) -> String {
    let cleaned = raw
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`' || c == '「' || c == '」')
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .trim_start_matches(['#', '-', '*', ' '])
        .trim();
    cleaned.chars().take(56).collect()
}

async fn maybe_refresh_run_title(
    client: &reqwest::Client,
    cli: &Cli,
    run_id: Uuid,
    prompt: &str,
    llm_env: &HashMap<String, String>,
) -> Result<()> {
    let api_key = llm_env
        .get("ZENE_API_KEY")
        .cloned()
        .or_else(|| llm_env.get("OPENAI_API_KEY").cloned())
        .or_else(|| std::env::var("ZENE_API_KEY").ok())
        .or_else(|| std::env::var("OPENAI_API_KEY").ok())
        .unwrap_or_default();
    let base_url = llm_env
        .get("ZENE_BASE_URL")
        .cloned()
        .or_else(|| std::env::var("ZENE_BASE_URL").ok())
        .unwrap_or_else(|| "https://api.openai.com/v1".into());
    let model = llm_env
        .get("ZENE_MODEL")
        .cloned()
        .or_else(|| std::env::var("ZENE_MODEL").ok())
        .unwrap_or_else(|| "gpt-4o-mini".into());
    if api_key.trim().is_empty() {
        return Ok(());
    }
    let url = chat_completions_url(&base_url);
    let snippet: String = prompt.chars().take(800).collect();
    // DeepSeek V4 (and similar) enable thinking by default; with a tiny max_tokens budget
    // all tokens go to reasoning_content and message.content stays empty.
    let mut body = serde_json::json!({
        "model": model,
        "temperature": 0.2,
        "max_tokens": 64,
        "thinking": { "type": "disabled" },
        "messages": [
            {
                "role": "system",
                "content": "Return only a concise agent session title in the user's language. Max 8 words. No quotes or punctuation wrapping."
            },
            {
                "role": "user",
                "content": format!("Task:\n{snippet}")
            }
        ]
    });
    let mut resp = client
        .post(&url)
        .bearer_auth(&api_key)
        .json(&body)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .context("title llm request")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let err_body = resp.text().await.unwrap_or_default();
        // Providers that reject unknown `thinking` get one retry without it.
        let thinking_rejected = status.as_u16() == 400
            && (err_body.contains("thinking")
                || err_body.contains("unknown")
                || err_body.contains("Unrecognized"));
        if thinking_rejected {
            body.as_object_mut().map(|o| o.remove("thinking"));
            // Give thinking models enough room if disable isn't supported.
            body["max_tokens"] = serde_json::json!(256);
            resp = client
                .post(&url)
                .bearer_auth(&api_key)
                .json(&body)
                .timeout(Duration::from_secs(20))
                .send()
                .await
                .context("title llm retry")?;
        } else {
            warn!(%status, body = %err_body.chars().take(240).collect::<String>(), "title llm failed");
            return Ok(());
        }
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let err_body = resp.text().await.unwrap_or_default();
        warn!(%status, body = %err_body.chars().take(240).collect::<String>(), "title llm failed");
        return Ok(());
    }
    let value: serde_json::Value = resp.json().await.context("title llm json")?;
    let title = value
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .map(sanitize_run_title)
        .unwrap_or_default();
    if title.is_empty() {
        warn!(run_id = %run_id, "title llm returned empty content");
        return Ok(());
    }
    let req = WorkerTitleRequest { title: title.clone() };
    client
        .post(format!("{}/internal/v1/runs/{run_id}/title", cli.api_url))
        .bearer_auth(&cli.worker_token)
        .json(&req)
        .send()
        .await?
        .error_for_status()
        .context("post title")?;
    info!(run_id = %run_id, %title, "refreshed run title");
    Ok(())
}

async fn set_status(
    client: &reqwest::Client,
    cli: &Cli,
    run_id: Uuid,
    status: RunStatus,
    head_sha: Option<String>,
    failure_code: Option<String>,
) -> Result<()> {
    post_run_status(
        client,
        &cli.api_url,
        &cli.worker_token,
        run_id,
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
    status: RunStatus,
) -> Result<()> {
    post_run_status(client, api_url, token, run_id, status, None, None).await
}

async fn post_run_status(
    client: &reqwest::Client,
    api_url: &str,
    token: &str,
    run_id: Uuid,
    status: RunStatus,
    head_sha: Option<String>,
    failure_code: Option<String>,
) -> Result<()> {
    let body = WorkerStatusRequest {
        status,
        head_sha,
        failure_code,
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

async fn post_push(client: &reqwest::Client, cli: &Cli, run_id: Uuid) -> Result<()> {
    client
        .post(format!(
            "{}/internal/v1/runs/{run_id}/git/push",
            cli.api_url
        ))
        .bearer_auth(&cli.worker_token)
        .json(&serde_json::json!({ "force": false }))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn post_pull_request(
    client: &reqwest::Client,
    cli: &Cli,
    run_id: Uuid,
    title: &str,
) -> Result<()> {
    client
        .post(format!(
            "{}/internal/v1/runs/{run_id}/git/pull-request",
            cli.api_url
        ))
        .bearer_auth(&cli.worker_token)
        .json(&serde_json::json!({
            "title": title,
            "body": "Created by Zene Cloud worker",
            "draft": true
        }))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn git_commit_all(workspace: &Path, title: &str) -> Result<Option<String>> {
    let status = run_git_output(workspace, &["status", "--porcelain"]).await?;
    if status.trim().is_empty() {
        let sha = run_git_output(workspace, &["rev-parse", "HEAD"])
            .await
            .ok()
            .map(|s| s.trim().to_string());
        return Ok(sha);
    }
    run_git(workspace, &["add", "-A"]).await?;
    let msg = format!("zene: {}", title.chars().take(72).collect::<String>());
    run_git(
        workspace,
        &[
            "-c",
            "user.email=zene-cloud@localhost",
            "-c",
            "user.name=Zene Cloud",
            "commit",
            "-m",
            &msg,
        ],
    )
    .await?;
    let sha = run_git_output(workspace, &["rev-parse", "HEAD"])
        .await?
        .trim()
        .to_string();
    Ok(Some(sha))
}

async fn run_git(workspace: &Path, args: &[&str]) -> Result<()> {
    let output = git_command()
        .current_dir(workspace)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .with_context(|| format!("git {}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {} failed: {stderr}", args.join(" "));
    }
    Ok(())
}

async fn run_git_output(workspace: &Path, args: &[&str]) -> Result<String> {
    let output = git_command()
        .current_dir(workspace)
        .args(args)
        .output()
        .await
        .with_context(|| format!("git {}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {} failed: {stderr}", args.join(" "));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[allow(dead_code)]
async fn drain_stderr_lines(stderr: tokio::process::ChildStderr) {
    let mut reader = BufReader::new(stderr);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }
}

#[cfg(test)]
mod title_tests {
    use super::{chat_completions_url, sanitize_run_title};

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
}
