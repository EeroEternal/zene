use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;
use zene_cloud_domain::{
    ClaimedRun, CreateApprovalRequest, CreateRepositoryRequest, CreateRunRequest, DecideApprovalRequest,
    LoginRequest, PostMessageRequest, RegisterRequest, RunStatus, WorkerCommandsResponse,
    WorkerEventRequest, WorkerPullRequestRequest, WorkerPushRequest, WorkerStatusRequest,
};

use crate::auth::{AuthUser, WorkerAuth};
use crate::error::AppError;
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/auth/register", post(register))
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/me", get(me))
        .route("/api/v1/repositories", get(list_repos).post(create_repo))
        .route("/api/v1/runs", get(list_runs).post(create_run))
        .route("/api/v1/runs/{run_id}", get(get_run))
        .route("/api/v1/runs/{run_id}/messages", get(list_messages).post(post_message))
        .route("/api/v1/runs/{run_id}/events", get(list_events))
        .route("/api/v1/runs/{run_id}/events/stream", get(stream_events))
        .route("/api/v1/runs/{run_id}/cancel", post(cancel_run))
        .route(
            "/api/v1/runs/{run_id}/approvals/{approval_id}/decide",
            post(decide_approval),
        )
        .route("/internal/v1/runs/claim", post(claim_run))
        .route("/internal/v1/runs/{run_id}/heartbeat", post(heartbeat))
        .route("/internal/v1/runs/{run_id}/events", post(worker_event))
        .route("/internal/v1/runs/{run_id}/status", post(worker_status))
        .route(
            "/internal/v1/runs/{run_id}/clone-auth",
            get(clone_auth).post(clone_auth),
        )
        .route("/internal/v1/runs/{run_id}/commands", get(worker_commands))
        .route(
            "/internal/v1/runs/{run_id}/approvals",
            post(create_approval),
        )
        .route(
            "/internal/v1/runs/{run_id}/approvals/{approval_id}",
            get(get_approval),
        )
        .route("/internal/v1/runs/{run_id}/git/push", post(worker_push))
        .route(
            "/internal/v1/runs/{run_id}/git/pull-request",
            post(worker_pull_request),
        )
        .with_state(state)
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "ok": true,
        "service": "zene-cloud-api",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<impl IntoResponse, AppError> {
    let auth = state.db.register(req).await?;
    Ok(Json(auth))
}

async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<impl IntoResponse, AppError> {
    let auth = state.db.login(req).await?;
    Ok(Json(auth))
}

async fn me(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    Ok(Json(serde_json::json!({
        "user": user,
        "organization": org
    })))
}

async fn list_repos(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    let repos = state.db.list_repositories(org.id).await?;
    Ok(Json(repos))
}

async fn create_repo(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<CreateRepositoryRequest>,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    let repo = state.db.create_repository(org.id, req).await?;
    Ok(Json(repo))
}

async fn list_runs(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    let runs = state.db.list_runs(org.id).await?;
    Ok(Json(runs))
}

async fn create_run(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<CreateRunRequest>,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    let run = state.db.create_run(org.id, user.id, req).await?;
    Ok(Json(run))
}

async fn get_run(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let run = authorize_run(&state, user.id, run_id).await?;
    Ok(Json(run))
}

async fn list_messages(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let _ = authorize_run(&state, user.id, run_id).await?;
    let messages = state.db.list_messages(run_id).await?;
    Ok(Json(messages))
}

async fn post_message(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
    Json(req): Json<PostMessageRequest>,
) -> Result<impl IntoResponse, AppError> {
    let run = authorize_run(&state, user.id, run_id).await?;
    if !run.status.accepts_messages() {
        return Err(AppError::conflict(format!(
            "run status {} does not accept messages",
            run.status.as_str()
        )));
    }
    let message = state
        .db
        .add_message(
            run_id,
            Some(user.id),
            "user",
            &req.text,
            req.client_message_id.as_deref(),
        )
        .await?;
    Ok(Json(message))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventsQuery {
    after_seq: Option<i64>,
}

async fn list_events(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
    Query(query): Query<EventsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let _ = authorize_run(&state, user.id, run_id).await?;
    let events = state
        .db
        .events_after(run_id, query.after_seq.unwrap_or(0))
        .await?;
    Ok(Json(serde_json::json!({
        "events": events,
        "nextSeq": events.last().map(|e| e.seq).unwrap_or(query.after_seq.unwrap_or(0))
    })))
}

async fn stream_events(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
    Query(query): Query<EventsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let _ = authorize_run(&state, user.id, run_id).await?;
    let mut after = query.after_seq.unwrap_or(0);
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(32);
    tokio::spawn(async move {
        loop {
            match state.db.events_after(run_id, after).await {
                Ok(events) => {
                    if events.is_empty() {
                        if tx
                            .send(Ok(Event::default().comment("ping")))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    } else {
                        for event in events {
                            after = event.seq;
                            let data = serde_json::to_string(&event).unwrap_or_default();
                            if tx
                                .send(Ok(Event::default().id(event.seq.to_string()).data(data)))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                }
                Err(_) => break,
            }
            tokio::time::sleep(Duration::from_millis(800)).await;
        }
    });
    Ok(Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default()))
}

async fn cancel_run(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let run = authorize_run(&state, user.id, run_id).await?;
    if run.status.is_terminal() {
        return Ok(Json(run));
    }
    let run = state
        .db
        .update_run_status(run_id, RunStatus::Cancelled, None, Some("user_cancelled".into()))
        .await?;
    Ok(Json(run))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaimRequest {
    worker_id: String,
    workspace_root: String,
}

async fn claim_run(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Json(req): Json<ClaimRequest>,
) -> Result<impl IntoResponse, AppError> {
    let claimed = state
        .db
        .claim_next_run(&req.worker_id, std::path::Path::new(&req.workspace_root))
        .await?;
    Ok(Json(claimed.map(|(run, attempt_id, generation, workspace_dir)| {
        ClaimedRun {
            run,
            attempt_id,
            generation,
            workspace_dir,
        }
    })))
}

async fn heartbeat(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
    Json(req): Json<ClaimRequest>,
) -> Result<impl IntoResponse, AppError> {
    state.db.heartbeat(run_id, &req.worker_id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn worker_event(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
    Json(req): Json<WorkerEventRequest>,
) -> Result<impl IntoResponse, AppError> {
    let event = state
        .db
        .append_event(
            run_id,
            0,
            Some(&req.source_event_id),
            &req.event_type,
            req.payload,
        )
        .await?;
    Ok(Json(event))
}

async fn worker_status(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
    Json(req): Json<WorkerStatusRequest>,
) -> Result<impl IntoResponse, AppError> {
    let run = state
        .db
        .update_run_status(run_id, req.status, req.head_sha, req.failure_code)
        .await?;
    Ok(Json(run))
}

async fn clone_auth(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let auth = state.db.get_clone_auth(run_id).await?;
    Ok(Json(auth))
}

async fn worker_commands(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let commands = state.db.poll_worker_commands(run_id).await?;
    Ok(Json(WorkerCommandsResponse { commands }))
}

async fn create_approval(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
    Json(req): Json<CreateApprovalRequest>,
) -> Result<impl IntoResponse, AppError> {
    let approval = state.db.create_approval(run_id, req).await?;
    Ok(Json(approval))
}

async fn get_approval(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path((run_id, approval_id)): Path<(Uuid, Uuid)>,
) -> Result<impl IntoResponse, AppError> {
    let approval = state
        .db
        .get_approval(approval_id)
        .await?
        .ok_or_else(|| AppError::not_found("approval not found"))?;
    if approval.run_id != run_id {
        return Err(AppError::not_found("approval not found"));
    }
    Ok(Json(approval))
}

async fn decide_approval(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((run_id, approval_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<DecideApprovalRequest>,
) -> Result<impl IntoResponse, AppError> {
    let _ = authorize_run(&state, user.id, run_id).await?;
    let approval = state
        .db
        .get_approval(approval_id)
        .await?
        .ok_or_else(|| AppError::not_found("approval not found"))?;
    if approval.run_id != run_id {
        return Err(AppError::not_found("approval not found"));
    }
    if !approval.allowed_decisions.is_empty()
        && !approval.allowed_decisions.iter().any(|d| d == &req.decision)
    {
        return Err(AppError::conflict(format!(
            "decision {} not allowed",
            req.decision
        )));
    }
    let approval = state
        .db
        .decide_approval(approval_id, &req.decision, Some(&user.id.to_string()))
        .await?;
    Ok(Json(approval))
}

async fn worker_push(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
    Json(req): Json<WorkerPushRequest>,
) -> Result<impl IntoResponse, AppError> {
    let key = req
        .idempotency_key
        .unwrap_or_else(|| format!("push-{}", Uuid::new_v4()));
    let result = state
        .db
        .record_git_operation(
            run_id,
            "push",
            "skipped",
            &key,
            serde_json::json!({
                "skipped": true,
                "reason": "Phase 0 stub: GitHub push not configured",
                "force": req.force,
            }),
        )
        .await?;
    Ok(Json(result))
}

async fn worker_pull_request(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
    Json(req): Json<WorkerPullRequestRequest>,
) -> Result<impl IntoResponse, AppError> {
    let key = req
        .idempotency_key
        .unwrap_or_else(|| format!("pr-{}", Uuid::new_v4()));
    let result = state
        .db
        .record_git_operation(
            run_id,
            "pull_request",
            "skipped",
            &key,
            serde_json::json!({
                "skipped": true,
                "reason": "Phase 0 stub: GitHub PR not configured",
                "title": req.title,
                "body": req.body,
                "draft": req.draft,
            }),
        )
        .await?;
    Ok(Json(result))
}

async fn authorize_run(
    state: &AppState,
    user_id: Uuid,
    run_id: Uuid,
) -> Result<zene_cloud_domain::Run, AppError> {
    let org = state.db.primary_org(user_id).await?;
    let run = state
        .db
        .get_run(run_id)
        .await?
        .ok_or_else(|| AppError::not_found("run not found"))?;
    if run.organization_id != org.id {
        return Err(AppError::forbidden("run not in organization"));
    }
    Ok(run)
}
