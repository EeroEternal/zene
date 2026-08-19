use std::time::Duration;

use anyhow::{bail, Context, Result};
use uuid::Uuid;
use zene_cloud_domain::{
    ApprovalDecision, ApprovalEventPayload, ApprovalKind, ApprovalRequest, ApprovalRisk,
    ApprovalStatus, ClaimedRun, CloneAuthResponse, CreateApprovalRequest, LlmAuthResponse,
    RunStatus, WorkerClaimRequest, WorkerCommandAckRequest, WorkerCommandsResponse,
    WorkerEventRequest, WorkerFence, WorkerSessionRequest, WorkerStatusRequest,
};

use crate::event_outbox::EventOutbox;
use crate::Cli;

pub(crate) struct ResolvedPermission {
    pub decision: ApprovalDecision,
    pub option_id: Option<String>,
    pub answer: Option<String>,
}

pub(crate) async fn claim_run(
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

pub(crate) async fn fetch_clone_auth(
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

pub(crate) async fn fetch_llm_auth(
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

pub(crate) async fn persist_runtime_session(
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

pub(crate) async fn ack_command(
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

pub(crate) async fn fetch_commands(
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

pub(crate) async fn take_pending_mode(
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

pub(crate) async fn set_pending_mode(
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

pub(crate) fn permission_outcome(approval: &ApprovalRequest) -> ResolvedPermission {
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

pub(crate) async fn resolve_permission(
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

pub(crate) fn heartbeat_loop(
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

pub(crate) async fn deliver_event(
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

pub(crate) async fn post_event_raw(
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

pub(crate) async fn set_status(
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

pub(crate) async fn set_status_raw(
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
