use std::path::{Path, PathBuf};
use std::process::Stdio;
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
    RunStatus, WorkerCommand, WorkerCommandsResponse, WorkerEventRequest, WorkerStatusRequest,
};

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

    /// Pass `--yolo` to real `zene acp` (auto-approve tools locally).
    #[arg(long, env = "ZENE_CLOUD_ACP_YOLO", default_value_t = false)]
    acp_yolo: bool,

    #[arg(long, default_value_t = 2)]
    poll_seconds: u64,

    /// Call push/PR endpoints after commit (mock Git Broker by default).
    #[arg(long, env = "ZENE_CLOUD_PUSH_PR", default_value_t = true)]
    push_pr: bool,
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
    let mut zene_bin = resolve_zene_bin(cli.zene_bin.clone());
    let has_llm = std::env::var("ZENE_API_KEY").is_ok()
        || std::env::var("OPENAI_API_KEY").is_ok()
        || std::env::var("ANTHROPIC_API_KEY").is_ok()
        || std::env::var("ZENE_BASE_URL").is_ok();
    if zene_bin.is_some() && !has_llm {
        warn!("zene binary found but no LLM credentials/base URL; falling back to mock agent");
        zene_bin = None;
    }
    if let Some(path) = &zene_bin {
        info!(path = %path.display(), yolo = cli.acp_yolo, "using real zene acp");
    } else {
        warn!("using mock agent");
    }

    info!(%worker_id, api = %cli.api_url, "zene-cloud-worker started");
    loop {
        match claim_run(&client, &cli, &worker_id).await {
            Ok(Some(claimed)) => {
                if let Err(err) =
                    execute_run(&client, &cli, &worker_id, &claimed, zene_bin.as_ref()).await
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
    prepare_workspace(&workspace, &clone_auth).await?;

    set_status(client, cli, run_id, RunStatus::Running, None, None).await?;

    let result = if let Some(bin) = zene_bin {
        run_with_real_acp(client, cli, claimed, &workspace, bin).await
    } else {
        run_with_mock(client, cli, claimed, &workspace).await
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

async fn prepare_workspace(workspace: &Path, auth: &CloneAuthResponse) -> Result<()> {
    if workspace.join(".git").exists() {
        info!(path = %workspace.display(), "workspace already initialized");
        return Ok(());
    }

    if auth.mock {
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

    info!(url = %auth.clone_url, "cloning repository");
    if workspace.exists() {
        let _ = tokio::fs::remove_dir_all(workspace).await;
    }
    std::fs::create_dir_all(
        workspace
            .parent()
            .unwrap_or_else(|| Path::new(".")),
    )?;

    let mut clone_url = auth.clone_url.clone();
    if let Some(token) = &auth.token {
        if let Some(rest) = clone_url.strip_prefix("https://") {
            let user = auth.username.as_deref().unwrap_or("x-access-token");
            clone_url = format!("https://{user}:{token}@{rest}");
        }
    }

    let status = Command::new("git")
        .args([
            "clone",
            "--branch",
            &auth.base_ref,
            "--single-branch",
            &clone_url,
            &workspace.display().to_string(),
        ])
        .status()
        .await
        .context("git clone")?;
    if !status.success() {
        bail!("git clone failed with {status}");
    }
    let _ = run_git(workspace, &["checkout", "-B", &auth.head_branch]).await;
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

async fn run_with_real_acp(
    client: &reqwest::Client,
    cli: &Cli,
    claimed: &ClaimedRun,
    workspace: &Path,
    zene_bin: &Path,
) -> Result<()> {
    let run_id = claimed.run.id;
    let yolo = cli.acp_yolo
        || claimed.run.permission_mode == "yolo"
        || std::env::var("ZENE_YOLO").ok().as_deref() == Some("1");

    let (bridge, mut msg_rx) = AcpBridge::spawn(zene_bin, workspace, yolo).await?;
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
    let session_for_cancel = session_id.clone();
    let bridge_cancel = bridge.clone();
    let cancel_flag = cancelled.clone();
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
                                let mut guard = bridge_cancel.lock().await;
                                if let Some(b) = guard.as_mut() {
                                    if let Err(err) = b.prompt(&session_for_cancel, &text).await {
                                        warn!(error = %err, "follow-up prompt failed");
                                    }
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

    // After the main prompt returns, wait briefly for follow-ups / cancel.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        if cancelled.load(std::sync::atomic::Ordering::SeqCst) {
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
    let output = Command::new("git")
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
    let output = Command::new("git")
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
