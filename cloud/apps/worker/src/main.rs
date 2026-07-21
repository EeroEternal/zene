use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;
use zene_cloud_acp_bridge::{resolve_zene_bin, MockAgent};
use zene_cloud_domain::{ClaimedRun, RunStatus, WorkerEventRequest, WorkerStatusRequest};

#[derive(Debug, Parser)]
#[command(name = "zene-cloud-worker")]
struct Cli {
    #[arg(long, env = "ZENE_CLOUD_API_URL", default_value = "http://127.0.0.1:8788")]
    api_url: String,

    #[arg(long, env = "ZENE_CLOUD_WORKER_TOKEN", default_value = "dev-worker-token")]
    worker_token: String,

    #[arg(long, env = "ZENE_CLOUD_WORKER_ID")]
    worker_id: Option<String>,

    #[arg(long, env = "ZENE_CLOUD_WORKSPACE_ROOT", default_value = "./data/workspaces")]
    workspace_root: PathBuf,

    #[arg(long, env = "ZENE_BIN")]
    zene_bin: Option<PathBuf>,

    #[arg(long, default_value_t = 2)]
    poll_seconds: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let cli = Cli::parse();
    let worker_id = cli
        .worker_id
        .clone()
        .unwrap_or_else(|| format!("worker-{}", &Uuid::new_v4().to_string()[..8]));
    std::fs::create_dir_all(&cli.workspace_root)?;
    let client = reqwest::Client::new();
    let zene_bin = resolve_zene_bin(cli.zene_bin.clone());
    if let Some(path) = &zene_bin {
        info!(path = %path.display(), "using real zene acp");
    } else {
        warn!("zene binary not found; using Phase 0 mock agent");
    }

    info!(%worker_id, api = %cli.api_url, "zene-cloud-worker started");
    loop {
        match claim_run(&client, &cli, &worker_id).await {
            Ok(Some(claimed)) => {
                if let Err(err) = execute_run(&client, &cli, &worker_id, &claimed, zene_bin.as_ref()).await
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
            }
            Ok(None) => {
                tokio::time::sleep(Duration::from_secs(cli.poll_seconds)).await;
            }
            Err(err) => {
                warn!(error = %err, "claim failed");
                tokio::time::sleep(Duration::from_secs(cli.poll_seconds)).await;
            }
        }
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
) -> Result<()> {
    let run_id = claimed.run.id;
    info!(%run_id, workspace = %claimed.workspace_dir, "claimed run");
    set_status(
        client,
        cli,
        run_id,
        RunStatus::Starting,
        None,
        None,
    )
    .await?;

    let workspace = PathBuf::from(&claimed.workspace_dir);
    std::fs::create_dir_all(&workspace)?;
    let hb = heartbeat_loop(client.clone(), cli.api_url.clone(), cli.worker_token.clone(), worker_id.to_string(), run_id);

    set_status(client, cli, run_id, RunStatus::Running, None, None).await?;

    if zene_bin.is_some() {
        // Real ACP integration is scaffolded; Phase 0 defaults to mock to keep local demos reliable.
        warn!("real zene acp selected; Phase 0 still executes mock prompt path for stability");
    }

    let agent = MockAgent::new(workspace);
    let events = agent.run_prompt(&claimed.run.prompt).await?;
    for event in events {
        post_event(
            client,
            cli,
            run_id,
            WorkerEventRequest {
                source_event_id: event.source_event_id,
                event_type: event.event_type,
                payload: event.payload,
            },
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    set_status(client, cli, run_id, RunStatus::Completed, None, None).await?;
    hb.abort();
    Ok(())
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
    client
        .post(format!(
            "{}/internal/v1/runs/{run_id}/events",
            cli.api_url
        ))
        .bearer_auth(&cli.worker_token)
        .json(&event)
        .send()
        .await?
        .error_for_status()
        .context("post event")?;
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
    let body = WorkerStatusRequest {
        status,
        head_sha,
        failure_code,
    };
    client
        .post(format!(
            "{}/internal/v1/runs/{run_id}/status",
            cli.api_url
        ))
        .bearer_auth(&cli.worker_token)
        .json(&body)
        .send()
        .await?
        .error_for_status()
        .context("set status")?;
    Ok(())
}
