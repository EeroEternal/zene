use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tracing::{info, warn};
use uuid::Uuid;
use zene_cloud_domain::{ClaimedRun, LlmAuthResponse, PermissionMode, RunStatus, WorkerFence};
use zene_cloud_runtime_client::{AcpRuntimeClient, MockRuntimeClient};

use crate::api::{fetch_clone_auth, fetch_llm_auth, heartbeat_loop, set_status};
use crate::event_outbox::EventOutbox;
use crate::git::{bare_cache_ready, prepare_workspace, workspace_ready};
use crate::runtime::{run_runtime_session, RunOutcome};
use crate::Cli;

pub(crate) fn worker_fence(claimed: &ClaimedRun, worker_id: &str) -> WorkerFence {
    WorkerFence {
        attempt_id: claimed.attempt_id,
        generation: claimed.generation,
        worker_id: worker_id.to_string(),
    }
}

pub(crate) fn is_recoverable_worker_error(err: &anyhow::Error, claimed: &ClaimedRun) -> bool {
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

pub(crate) fn has_process_llm_credentials() -> bool {
    std::env::var("ZENE_API_KEY").is_ok()
        || std::env::var("OPENAI_API_KEY").is_ok()
        || std::env::var("ANTHROPIC_API_KEY").is_ok()
        || std::env::var("ZENE_BASE_URL").is_ok()
}

pub(crate) async fn execute_run(
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
        match crate::github_auth::install(&workspace, auth).await {
            Ok(_) => {
                let dir = crate::github_auth::auth_dir(&workspace);
                github_refresh = Some(github_token_refresh_loop(
                    client.clone(),
                    cli.clone(),
                    run_id,
                    dir.clone(),
                ));
                Some((
                    dir,
                    auth.token.clone(),
                    crate::github_auth::github_repo_slug(&auth.clone_url),
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

fn inject_cellz(env: &mut HashMap<String, String>, url: Option<&str>) {
    let url = url
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            std::env::var("CELLZ_URL")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });
    if let Some(url) = url {
        env.insert("CELLZ_URL".into(), url);
    }
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
    inject_cellz(&mut llm_env, cli.cellz_url.as_deref());
    if let Some((dir, token, repo)) = github_for_acp {
        crate::github_auth::inject_env(&mut llm_env, dir, token.as_deref(), repo.as_deref());
    }
    info!(
        run_id = %run_id,
        max_turns = claimed.run.max_turns,
        inference_gateway = cli.inference_gateway_url.is_some(),
        cellz = cli.cellz_url.is_some(),
        "injected ZENE_MAX_TURNS, ZENE_RUN_ID, optional inference gateway and cellz for acp"
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
                        if let Err(err) = crate::github_auth::write_token_file(&dir, token).await {
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
