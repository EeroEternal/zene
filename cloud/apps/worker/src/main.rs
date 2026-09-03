mod api;
mod event_outbox;
mod execute;
pub(crate) mod git;
mod github_auth;
mod runtime;
mod supervisor;
pub(crate) mod title;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use api::{claim_run, set_status};
use clap::Parser;
use execute::{
    execute_run, has_process_llm_credentials, is_recoverable_worker_error, worker_fence,
};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;
use zene_cloud_acp_bridge::resolve_zene_bin;
use zene_cloud_domain::RunStatus;

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

    /// Optional cellz state daemon URL (e.g. http://127.0.0.1:8080)
    #[arg(long, env = "CELLZ_URL")]
    pub(crate) cellz_url: Option<String>,

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

    // Wait for API to be ready
    let health_url = format!("{}/api/v1/health", cli.api_url);
    let mut api_ready = false;
    for attempt in 0..20 {
        match client.get(&health_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                api_ready = true;
                break;
            }
            _ => {
                if attempt == 0 {
                    info!("Waiting for API server...");
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }
    if !api_ready {
        warn!("API server not ready after 10s; proceeding anyway");
    }

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
