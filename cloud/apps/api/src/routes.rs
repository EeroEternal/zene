use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Redirect};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;
use zene_cloud_domain::{
    ClaimedRun, CreateApprovalRequest, CreatePullRequestBody, CreateRepositoryRequest,
    CreateRunRequest, DecideApprovalRequest, LoginRequest, PostMessageRequest, RegisterRequest,
    RunStatus, WorkerCommandsResponse, WorkerEventRequest, WorkerPullRequestRequest,
    WorkerPushRequest, WorkerStatusRequest,
};

use crate::auth::{AuthUser, WorkerAuth};
use crate::error::AppError;
use crate::state::AppState;
use crate::workspace;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/auth/register", post(register))
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/me", get(me))
        .route("/api/v1/github/status", get(github_status))
        .route("/api/v1/github/oauth/start", get(github_oauth_start))
        .route("/api/v1/github/oauth/callback", get(github_oauth_callback))
        .route("/api/v1/github/mock/connect", post(github_mock_connect))
        .route("/api/v1/github/installations", get(list_installations))
        .route("/api/v1/github/installations/mock", post(github_mock_install))
        .route(
            "/api/v1/github/installations/{installation_id}/sync",
            post(sync_installation_repos),
        )
        .route("/api/v1/repositories", get(list_repos).post(create_repo))
        .route("/api/v1/runs", get(list_runs).post(create_run))
        .route("/api/v1/runs/{run_id}", get(get_run))
        .route(
            "/api/v1/runs/{run_id}/messages",
            get(list_messages).post(post_message),
        )
        .route("/api/v1/runs/{run_id}/events", get(list_events))
        .route("/api/v1/runs/{run_id}/events/stream", get(stream_events))
        .route("/api/v1/runs/{run_id}/cancel", post(cancel_run))
        .route("/api/v1/runs/{run_id}/approvals", get(list_approvals))
        .route(
            "/api/v1/runs/{run_id}/approvals/{approval_id}/decide",
            post(decide_approval),
        )
        .route("/api/v1/runs/{run_id}/files", get(list_run_files))
        .route("/api/v1/runs/{run_id}/file", get(read_run_file))
        .route("/api/v1/runs/{run_id}/diff", get(run_diff))
        .route(
            "/api/v1/runs/{run_id}/pull-requests",
            get(list_run_prs).post(create_run_pr),
        )
        .route("/api/v1/runs/{run_id}/git/push", post(user_push))
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

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    Json(serde_json::json!({
        "ok": true,
        "service": "zene-cloud-api",
        "version": env!("CARGO_PKG_VERSION"),
        "githubMode": format!("{:?}", state.github.mode()).to_lowercase(),
    }))
}

async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(state.db.register(req).await?))
}

async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(state.db.login(req).await?))
}

async fn me(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    let github = state.db.get_github_account(user.id).await?;
    Ok(Json(serde_json::json!({
        "user": user,
        "organization": org,
        "github": github,
        "githubMode": format!("{:?}", state.github.mode()).to_lowercase(),
    })))
}

async fn github_status(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let account = state.db.get_github_account(user.id).await?;
    let org = state.db.primary_org(user.id).await?;
    let installations = state.db.list_installations(org.id).await?;
    Ok(Json(serde_json::json!({
        "mode": format!("{:?}", state.github.mode()).to_lowercase(),
        "connected": account.is_some(),
        "account": account,
        "installations": installations,
    })))
}

async fn github_oauth_start(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let redirect = format!("{}/api/v1/github/oauth/callback", state.public_base_url);
    let (url, oauth_state) = state
        .github
        .begin_oauth(Some(redirect.clone()))
        .map_err(AppError::from)?;
    state
        .db
        .save_oauth_state(&oauth_state, Some(user.id), Some("/"), 900)
        .await?;
    if state.github.is_mock() {
        // In mock mode, return URL that UI can open or we auto-complete via mock/connect.
        return Ok(Json(serde_json::json!({
            "authorizeUrl": url,
            "state": oauth_state,
            "mode": "mock",
            "hint": "mock mode: call POST /api/v1/github/mock/connect instead of browser OAuth"
        })));
    }
    Ok(Json(serde_json::json!({
        "authorizeUrl": url,
        "state": oauth_state,
        "mode": "live"
    })))
}

#[derive(Debug, Deserialize)]
struct OauthCallbackQuery {
    code: Option<String>,
    state: Option<String>,
}

async fn github_oauth_callback(
    State(state): State<AppState>,
    Query(query): Query<OauthCallbackQuery>,
) -> Result<impl IntoResponse, AppError> {
    let state_key = query
        .state
        .ok_or_else(|| AppError::bad_request("missing state"))?;
    let code = query
        .code
        .ok_or_else(|| AppError::bad_request("missing code"))?;
    let saved = state
        .db
        .take_oauth_state(&state_key)
        .await?
        .ok_or_else(|| AppError::bad_request("invalid oauth state"))?;
    let user_id = saved
        .user_id
        .ok_or_else(|| AppError::unauthorized("oauth state has no user"))?;
    let redirect = format!("{}/api/v1/github/oauth/callback", state.public_base_url);
    let tokens = state
        .github
        .exchange_oauth_code(&code, Some(redirect))
        .await
        .map_err(AppError::from)?;
    let gh_user = state
        .github
        .get_user(&tokens.access_token)
        .await
        .map_err(AppError::from)?;
    state
        .db
        .upsert_github_account(
            user_id,
            &gh_user.id,
            &gh_user.login,
            &tokens.access_token,
            &tokens.token_type,
            tokens.scope.as_deref(),
        )
        .await?;
    Ok(Redirect::temporary("/?github=connected"))
}

async fn github_mock_connect(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<impl IntoResponse, AppError> {
    if !state.github.is_mock() {
        return Err(AppError::conflict("mock connect only available in mock mode"));
    }
    let tokens = state
        .github
        .exchange_oauth_code("mock-code", None)
        .await
        .map_err(AppError::from)?;
    let gh_user = state
        .github
        .get_user(&tokens.access_token)
        .await
        .map_err(AppError::from)?;
    let account = state
        .db
        .upsert_github_account(
            user.id,
            &gh_user.id,
            &gh_user.login,
            &tokens.access_token,
            "bearer",
            Some("repo,read:user"),
        )
        .await?;
    let org = state.db.primary_org(user.id).await?;
    let installation = state
        .db
        .upsert_installation(
            org.id,
            "10001",
            "mock-org",
            "Organization",
            "active",
        )
        .await?;
    let listed = state
        .github
        .list_installation_repos("10001")
        .await
        .map_err(AppError::from)?;
    let repos = state.db.sync_repos_from_github(org.id, &listed).await?;
    Ok(Json(serde_json::json!({
        "account": account,
        "installation": installation,
        "repositories": repos,
    })))
}

async fn list_installations(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    Ok(Json(state.db.list_installations(org.id).await?))
}

async fn github_mock_install(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    let installation = state
        .db
        .upsert_installation(org.id, "10001", "mock-org", "Organization", "active")
        .await?;
    Ok(Json(installation))
}

async fn sync_installation_repos(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(installation_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    let installation = state
        .db
        .get_installation_by_provider_id(&installation_id)
        .await?
        .ok_or_else(|| AppError::not_found("installation not found"))?;
    if installation.organization_id != org.id {
        return Err(AppError::forbidden("installation not in organization"));
    }
    let listed = state
        .github
        .list_installation_repos(&installation_id)
        .await
        .map_err(AppError::from)?;
    let repos = state.db.sync_repos_from_github(org.id, &listed).await?;
    Ok(Json(repos))
}

async fn list_repos(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    Ok(Json(state.db.list_repositories(org.id).await?))
}

async fn create_repo(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<CreateRepositoryRequest>,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    Ok(Json(state.db.create_repository(org.id, req).await?))
}

async fn list_runs(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    Ok(Json(state.db.list_runs(org.id).await?))
}

async fn create_run(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<CreateRunRequest>,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    Ok(Json(state.db.create_run(org.id, user.id, req).await?))
}

async fn get_run(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(authorize_run(&state, user.id, run_id).await?))
}

async fn list_messages(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let _ = authorize_run(&state, user.id, run_id).await?;
    Ok(Json(state.db.list_messages(run_id).await?))
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
    Ok(Json(
        state
            .db
            .add_message(
                run_id,
                Some(user.id),
                "user",
                &req.text,
                req.client_message_id.as_deref(),
            )
            .await?,
    ))
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
                        if tx.send(Ok(Event::default().comment("ping"))).await.is_err() {
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
    Ok(Json(
        state
            .db
            .update_run_status(run_id, RunStatus::Cancelled, None, Some("user_cancelled".into()))
            .await?,
    ))
}

async fn list_approvals(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let _ = authorize_run(&state, user.id, run_id).await?;
    Ok(Json(state.db.list_pending_approvals(run_id).await?))
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
    Ok(Json(
        state
            .db
            .decide_approval(approval_id, &req.decision, Some(&user.id.to_string()))
            .await?,
    ))
}

async fn list_run_files(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let _ = authorize_run(&state, user.id, run_id).await?;
    let root = state.workspace_root.join(run_id.to_string());
    Ok(Json(workspace::list_files(&root, 500).map_err(AppError::from)?))
}

#[derive(Debug, Deserialize)]
struct FileQuery {
    path: String,
}

async fn read_run_file(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
    Query(query): Query<FileQuery>,
) -> Result<impl IntoResponse, AppError> {
    let _ = authorize_run(&state, user.id, run_id).await?;
    let root = state.workspace_root.join(run_id.to_string());
    Ok(Json(
        workspace::read_file(&root, &query.path, 200_000).map_err(AppError::from)?,
    ))
}

async fn run_diff(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let _ = authorize_run(&state, user.id, run_id).await?;
    let root = state.workspace_root.join(run_id.to_string());
    let diff = workspace::git_diff(&root).await.map_err(AppError::from)?;
    Ok(Json(serde_json::json!({ "diff": diff })))
}

async fn list_run_prs(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let _ = authorize_run(&state, user.id, run_id).await?;
    Ok(Json(state.db.list_pull_requests_for_run(run_id).await?))
}

async fn create_run_pr(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
    Json(req): Json<CreatePullRequestBody>,
) -> Result<impl IntoResponse, AppError> {
    let run = authorize_run(&state, user.id, run_id).await?;
    let pr = state
        .git_broker
        .create_draft_pr(&run, req)
        .await
        .map_err(AppError::from)?;
    Ok(Json(pr))
}

async fn user_push(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let run = authorize_run(&state, user.id, run_id).await?;
    let root = state.workspace_root.join(run_id.to_string());
    let bundle = create_bundle(&root).await.map_err(AppError::from)?;
    let expected = run.head_sha.clone().unwrap_or_else(|| "HEAD".into());
    let result = state
        .git_broker
        .accept_bundle_and_push(
            &run,
            &bundle,
            Some(expected.as_str()),
            &format!("push-{}", Uuid::new_v4()),
        )
        .await
        .map_err(AppError::from)?;
    Ok(Json(result))
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
    Ok(Json(
        state
            .db
            .append_event(
                run_id,
                0,
                Some(&req.source_event_id),
                &req.event_type,
                req.payload,
            )
            .await?,
    ))
}

async fn worker_status(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
    Json(req): Json<WorkerStatusRequest>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(
        state
            .db
            .update_run_status(run_id, req.status, req.head_sha, req.failure_code)
            .await?,
    ))
}

async fn clone_auth(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let run = state
        .db
        .get_run(run_id)
        .await?
        .ok_or_else(|| AppError::not_found("run not found"))?;
    let token = state
        .git_broker
        .issue_read_clone_token(&run)
        .await
        .map_err(AppError::from)?;
    let mock = token.mode == "mock" || state.github.is_mock();
    Ok(Json(zene_cloud_domain::CloneAuthResponse {
        run_id: run.id,
        repository_id: run.repository_id,
        clone_url: token.clone_url,
        username: Some("x-access-token".into()),
        token: Some(token.token),
        base_ref: run.base_ref,
        head_branch: run.head_branch,
        mock,
    }))
}

async fn worker_commands(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(WorkerCommandsResponse {
        commands: state.db.poll_worker_commands(run_id).await?,
    }))
}

async fn create_approval(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
    Json(req): Json<CreateApprovalRequest>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(state.db.create_approval(run_id, req).await?))
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

async fn worker_push(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
    Json(req): Json<WorkerPushRequest>,
) -> Result<impl IntoResponse, AppError> {
    let run = state
        .db
        .get_run(run_id)
        .await?
        .ok_or_else(|| AppError::not_found("run not found"))?;
    let root = state.workspace_root.join(run_id.to_string());
    let bundle = create_bundle(&root).await.map_err(AppError::from)?;
    let expected = run.head_sha.clone().unwrap_or_else(|| "HEAD".into());
    let key = req
        .idempotency_key
        .unwrap_or_else(|| format!("push-{}", Uuid::new_v4()));
    Ok(Json(
        state
            .git_broker
            .accept_bundle_and_push(&run, &bundle, Some(expected.as_str()), &key)
            .await
            .map_err(AppError::from)?,
    ))
}

async fn worker_pull_request(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
    Json(req): Json<WorkerPullRequestRequest>,
) -> Result<impl IntoResponse, AppError> {
    let run = state
        .db
        .get_run(run_id)
        .await?
        .ok_or_else(|| AppError::not_found("run not found"))?;
    let body = CreatePullRequestBody {
        title: req.title.unwrap_or_else(|| run.title.clone()),
        body: req.body,
        draft: req.draft,
        base_ref: Some(run.base_ref.clone()),
        head_ref: Some(run.head_branch.clone()),
    };
    Ok(Json(
        state
            .git_broker
            .create_draft_pr(&run, body)
            .await
            .map_err(AppError::from)?,
    ))
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

async fn create_bundle(root: &std::path::Path) -> anyhow::Result<Vec<u8>> {
    if !root.join(".git").exists() {
        return Ok(b"MOCK_BUNDLE".to_vec());
    }
    let bundle_path = root.join(".zene-bundle.bundle");
    let status = tokio::process::Command::new("git")
        .args(["bundle", "create", ".zene-bundle.bundle", "--all"])
        .current_dir(root)
        .status()
        .await?;
    if !status.success() {
        return Ok(format!("MOCK_BUNDLE:{}", root.display()).into_bytes());
    }
    let bytes = tokio::fs::read(&bundle_path).await.unwrap_or_default();
    let _ = tokio::fs::remove_file(&bundle_path).await;
    if bytes.is_empty() {
        Ok(b"MOCK_BUNDLE".to_vec())
    } else {
        Ok(bytes)
    }
}
