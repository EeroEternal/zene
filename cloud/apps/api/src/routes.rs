use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Redirect};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;
use zene_cloud_domain::{
    github_account_view, ClaimedRun, CreateApprovalRequest, CreatePullRequestBody,
    CreateRunRequest, DecideApprovalRequest, EmailLoginRequest, EmailLoginResponse, LoginRequest,
    MessageRole, PostMessageRequest, QueueStats, RegisterRequest, ResetPasswordRequest, RunStatus,
    SendVerificationCodeRequest, SendVerificationCodeResponse, SetRunModeRequest,
    UpdateRunRequest, WorkerClaimRequest, WorkerCommandAckRequest, WorkerCommandsResponse,
    WorkerEventRequest, WorkerFence, WorkerPullRequestRequest, WorkerPushRequest,
    WorkerSessionRequest, WorkerStatusRequest, WorkerTitleRequest,
};

use crate::auth::{AuthUser, WorkerAuth};
use crate::error::AppError;
use crate::state::AppState;
use crate::workspace;

pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(crate::features::llm::router())
        .merge(crate::features::repositories::router())
        .merge(crate::features::github::router())
        .route("/api/v1/health", get(health))
        .route("/api/v1/auth/register", post(register))
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/send-code", post(send_verification_code))
        .route("/api/v1/auth/reset-password", post(reset_password))
        .route("/api/v1/auth/email", post(request_email_login))
        .route("/api/v1/auth/email/callback", get(email_login_callback))
        .route("/api/v1/me", get(me))
        .route("/api/v1/runs", get(list_runs).post(create_run))
        .route(
            "/api/v1/runs/{run_id}",
            get(get_run).patch(update_run).delete(delete_run),
        )
        .route(
            "/api/v1/runs/{run_id}/messages",
            get(list_messages).post(post_message),
        )
        .route("/api/v1/runs/{run_id}/mode", post(set_run_mode))
        .route("/api/v1/runs/{run_id}/events", get(list_events))
        .route("/api/v1/runs/{run_id}/events/stream", get(stream_events))
        .route("/api/v1/runs/{run_id}/cancel", post(cancel_run))
        .route("/api/v1/runs/{run_id}/retry", post(retry_run))
        .route("/api/v1/runs/{run_id}/approvals", get(list_approvals))
        .route(
            "/api/v1/runs/{run_id}/approvals/{approval_id}/decide",
            post(decide_approval),
        )
        .route("/api/v1/runs/{run_id}/files", get(list_run_files))
        .route("/api/v1/runs/{run_id}/file", get(read_run_file))
        .route("/api/v1/runs/{run_id}/diff", get(run_diff))
        .route("/api/v1/runs/{run_id}/git/status", get(run_git_status))
        .route("/api/v1/runs/{run_id}/git/compare", get(run_git_compare))
        .route(
            "/api/v1/runs/{run_id}/git/compare/diff",
            get(run_git_compare_diff),
        )
        .route("/api/v1/runs/{run_id}/git/commits", get(run_git_commits))
        .route(
            "/api/v1/runs/{run_id}/pull-requests",
            get(list_run_prs).post(create_run_pr),
        )
        .route(
            "/api/v1/runs/{run_id}/pull-requests/{pr_id}/ready",
            post(mark_run_pr_ready),
        )
        .route(
            "/api/v1/runs/{run_id}/pull-requests/{pr_id}/merge",
            post(merge_run_pr),
        )
        .route("/api/v1/runs/{run_id}/git/push", post(user_push))
        .route("/internal/v1/runs/claim", post(claim_run))
        .route("/internal/v1/queue/stats", get(queue_stats))
        .route("/internal/v1/runs/{run_id}/heartbeat", post(heartbeat))
        .route(
            "/internal/v1/runs/{run_id}/runtime-session",
            post(worker_runtime_session),
        )
        .route(
            "/internal/v1/runs/{run_id}/acp-session",
            post(worker_runtime_session),
        )
        .route("/internal/v1/runs/{run_id}/events", post(worker_event))
        .route("/internal/v1/runs/{run_id}/status", post(worker_status))
        .route("/internal/v1/runs/{run_id}/title", post(worker_title))
        .route(
            "/internal/v1/runs/{run_id}/clone-auth",
            get(clone_auth).post(clone_auth),
        )
        .route("/internal/v1/runs/{run_id}/commands", get(worker_commands))
        .route(
            "/internal/v1/runs/{run_id}/commands/ack",
            post(worker_command_ack),
        )
        .route(
            "/internal/v1/runs/{run_id}/mode",
            post(worker_set_pending_mode),
        )
        .route(
            "/internal/v1/runs/{run_id}/mode/take",
            post(worker_take_pending_mode),
        )
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
    let github = state.github_client().await;
    Json(serde_json::json!({
        "ok": true,
        "service": "zene-cloud-api",
        "version": env!("CARGO_PKG_VERSION"),
        "githubMode": github.mode(),
    }))
}

async fn send_verification_code(
    State(state): State<AppState>,
    Json(req): Json<SendVerificationCodeRequest>,
) -> Result<impl IntoResponse, AppError> {
    let email = req.email.trim().to_lowercase();
    if email.is_empty() {
        return Err(AppError::bad_request("Email is required"));
    }
    let purpose = if req.purpose.trim().is_empty() {
        "register"
    } else {
        req.purpose.trim()
    };
    if purpose == "register" {
        if state.db.user_exists_by_email(&email).await? {
            return Err(AppError::conflict(
                "This email is already registered. Please sign in with your password.",
            ));
        }
    } else if purpose == "reset_password" {
        if !state.db.user_exists_by_email(&email).await? {
            return Err(AppError::not_found("No account found with this email."));
        }
    } else {
        return Err(AppError::bad_request("Invalid purpose"));
    }

    let code = state
        .db
        .create_email_verification_code(&email, purpose)
        .await?;
    let cfg = crate::email::EmailConfig::from_env();
    if cfg.configured() {
        if let Err(err) =
            crate::email::send_verification_code_email(&cfg, &email, &code, purpose).await
        {
            return Err(AppError::from(err));
        }
        return Ok(Json(SendVerificationCodeResponse {
            ok: true,
            code: None,
        }));
    }
    tracing::warn!(
        %email,
        %code,
        %purpose,
        "Email sending unconfigured (CLOUDFLARE_MAIL_TOKEN or RESEND_API_KEY unset); returning verification code for local use"
    );
    Ok(Json(SendVerificationCodeResponse {
        ok: true,
        code: Some(code),
    }))
}

async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<impl IntoResponse, AppError> {
    let email = req.email.trim().to_lowercase();
    if let Some(code) = req.code.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        state
            .db
            .verify_and_consume_verification_code(&email, code, "register")
            .await?;
    } else {
        let cfg = crate::email::EmailConfig::from_env();
        if cfg.configured() {
            return Err(AppError::bad_request(
                "Verification code is required for registration",
            ));
        }
    }
    Ok(Json(state.db.register(req).await?))
}

async fn reset_password(
    State(state): State<AppState>,
    Json(req): Json<ResetPasswordRequest>,
) -> Result<impl IntoResponse, AppError> {
    let email = req.email.trim().to_lowercase();
    state
        .db
        .verify_and_consume_verification_code(&email, &req.code, "reset_password")
        .await?;
    Ok(Json(state.db.reset_password(&email, &req.new_password).await?))
}

async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(state.db.login(req).await?))
}

async fn request_email_login(
    State(state): State<AppState>,
    Json(req): Json<EmailLoginRequest>,
) -> Result<impl IntoResponse, AppError> {
    let token = state.db.create_email_login_token(&req.email).await?;
    let login_url = format!(
        "{}/api/v1/auth/email/callback?token={token}",
        state.public_base_url.trim_end_matches('/')
    );
    let cfg = crate::email::EmailConfig::from_env();
    if cfg.configured() {
        if let Err(err) =
            crate::email::send_signin_email(&cfg, &req.email.trim().to_lowercase(), &login_url)
                .await
        {
            let _ = state.db.invalidate_email_login_tokens(&req.email).await;
            return Err(AppError::from(err));
        }
        return Ok(Json(EmailLoginResponse {
            ok: true,
            login_url: None,
        }));
    }
    tracing::warn!(
        email = %req.email.trim().to_lowercase(),
        %login_url,
        "Email sending unconfigured (CLOUDFLARE_MAIL_TOKEN or RESEND_API_KEY unset); returning sign-in URL for local use"
    );
    Ok(Json(EmailLoginResponse {
        ok: true,
        login_url: Some(login_url),
    }))
}

#[derive(Deserialize)]
struct EmailCallbackQuery {
    token: String,
}

async fn email_login_callback(
    State(state): State<AppState>,
    Query(query): Query<EmailCallbackQuery>,
) -> Result<impl IntoResponse, AppError> {
    match state.db.consume_email_login_token(&query.token).await {
        Ok(auth) => Ok(Redirect::temporary(&format!("/?auth={}", auth.token))),
        Err(err) => {
            tracing::warn!(error = %err, "email sign-in callback failed");
            Ok(Redirect::temporary("/?auth_error=invalid"))
        }
    }
}

async fn me(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    state
        .reload_github_for_org(org.id)
        .await
        .map_err(AppError::from)?;
    let github = state.github_client().await;
    let gh_account = state.db.get_github_account(user.id).await?;
    Ok(Json(serde_json::json!({
        "user": user,
        "organization": org,
        "github": gh_account.as_ref().map(github_account_view),
        "githubMode": github.mode(),
        "githubConfigured": github.config().is_app_configured(),
    })))
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
    let settings = state.db.get_user_llm_settings(user.id).await?;
    let ready = settings
        .as_ref()
        .is_some_and(|s| !s.api_key.trim().is_empty() && !s.base_url.trim().is_empty());
    if !ready {
        return Err(AppError::bad_request(
            "Configure LLM API key and base URL in Settings before starting an agent",
        ));
    }
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

async fn update_run(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
    Json(req): Json<UpdateRunRequest>,
) -> Result<impl IntoResponse, AppError> {
    let _ = authorize_run(&state, user.id, run_id).await?;
    Ok(Json(
        state
            .db
            .update_run_meta(run_id, req.title.as_deref(), req.archived)
            .await?,
    ))
}

async fn delete_run(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let _ = authorize_run(&state, user.id, run_id).await?;
    state.db.delete_run(run_id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
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
                MessageRole::User,
                &req.text,
                req.client_message_id.as_deref(),
            )
            .await?,
    ))
}

async fn set_run_mode(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
    Json(req): Json<SetRunModeRequest>,
) -> Result<impl IntoResponse, AppError> {
    let run = authorize_run(&state, user.id, run_id).await?;
    if !run.status.accepts_messages() {
        return Err(AppError::conflict(format!(
            "run status {} does not accept mode changes",
            run.status.as_str()
        )));
    }
    state.db.set_pending_mode(run_id, &req.mode_id).await?;
    Ok(Json(
        serde_json::json!({ "ok": true, "modeId": req.mode_id.trim() }),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventsQuery {
    after_seq: Option<i64>,
    after_cursor: Option<u64>,
}

async fn list_events(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
    Query(query): Query<EventsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let _ = authorize_run(&state, user.id, run_id).await?;
    let events = match query.after_cursor {
        Some(cursor) => state.db.events_after_cursor(run_id, cursor).await?,
        None => {
            state
                .db
                .events_after(run_id, query.after_seq.unwrap_or(0))
                .await?
        }
    };
    Ok(Json(serde_json::json!({
        "events": events,
        "nextSeq": events.last().map(|e| e.seq).unwrap_or(query.after_seq.unwrap_or(0)),
        "nextCursor": events.iter().filter_map(|e| e.cursor).max().or(query.after_cursor)
    })))
}

async fn stream_events(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
    headers: HeaderMap,
    Query(query): Query<EventsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let _ = authorize_run(&state, user.id, run_id).await?;
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<i64>().ok());
    let mut after = last_event_id.or(query.after_seq).unwrap_or(0);
    let mut after_cursor = query.after_cursor;
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(32);
    tokio::spawn(async move {
        loop {
            let result = match after_cursor {
                Some(cursor) => state.db.events_after_cursor(run_id, cursor).await,
                None => state.db.events_after(run_id, after).await,
            };
            match result {
                Ok(events) => {
                    if events.is_empty() {
                        if tx.send(Ok(Event::default().comment("ping"))).await.is_err() {
                            break;
                        }
                    } else {
                        // Provider cursor is only needed to establish the
                        // initial canonical position. Continue by seq so
                        // mixed platform/runtime events are never skipped.
                        after_cursor = None;
                        for event in events {
                            after = event.seq;
                            // `seq` is the canonical stream cursor. Do not
                            // switch to provider cursor mode after observing a
                            // partially annotated event stream.
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
            .update_run_status(
                run_id,
                RunStatus::Cancelled,
                None,
                Some("user_cancelled".into()),
            )
            .await?,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RetryRunRequest {
    text: Option<String>,
}

async fn retry_run(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
    Json(req): Json<RetryRunRequest>,
) -> Result<impl IntoResponse, AppError> {
    let run = authorize_run(&state, user.id, run_id).await?;
    if !matches!(
        run.status,
        RunStatus::Failed | RunStatus::TimedOut | RunStatus::Cancelled
    ) {
        return Err(AppError::conflict(format!(
            "run status {} cannot be retried",
            run.status.as_str()
        )));
    }
    let follow_up = req.text.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if let Some(text) = follow_up {
        state
            .db
            .add_message(run_id, Some(user.id), MessageRole::User, text, None)
            .await?;
    }
    Ok(Json(
        state
            .db
            .user_retry_run(run_id, follow_up)
            .await
            .map_err(AppError::from)?,
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
        && !approval
            .allowed_decisions
            .iter()
            .any(|d| d == &req.decision)
    {
        return Err(AppError::conflict(format!(
            "decision {} not allowed",
            req.decision.as_str()
        )));
    }
    Ok(Json(
        state
            .db
            .decide_approval_with_outcome(
                approval_id,
                req.decision,
                Some(&user.id.to_string()),
                req.option_id.as_deref(),
                req.answer.as_deref(),
            )
            .await?,
    ))
}

async fn list_run_files(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let run = authorize_run(&state, user.id, run_id).await?;
    let root = state.run_checkout_dir(&run);
    Ok(Json(
        workspace::list_files(&root, 500).map_err(AppError::from)?,
    ))
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
    let run = authorize_run(&state, user.id, run_id).await?;
    let root = state.run_checkout_dir(&run);
    Ok(Json(
        workspace::read_file(&root, &query.path, 200_000).map_err(AppError::from)?,
    ))
}

#[derive(Debug, Deserialize)]
struct DiffQuery {
    path: Option<String>,
}

async fn run_diff(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
    Query(query): Query<DiffQuery>,
) -> Result<impl IntoResponse, AppError> {
    let run = authorize_run(&state, user.id, run_id).await?;
    let root = state.run_checkout_dir(&run);
    let diff = workspace::git_diff(&root, query.path.as_deref())
        .await
        .map_err(AppError::from)?;
    Ok(Json(serde_json::json!({ "diff": diff })))
}

async fn run_git_status(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let run = authorize_run(&state, user.id, run_id).await?;
    let root = state.run_checkout_dir(&run);
    Ok(Json(
        workspace::git_status(&root).await.map_err(AppError::from)?,
    ))
}

async fn run_git_compare(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let run = authorize_run(&state, user.id, run_id).await?;
    let root = state.run_checkout_dir(&run);
    Ok(Json(
        workspace::git_compare(&root, &run.base_ref)
            .await
            .map_err(AppError::from)?,
    ))
}

async fn run_git_compare_diff(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
    Query(query): Query<DiffQuery>,
) -> Result<impl IntoResponse, AppError> {
    let run = authorize_run(&state, user.id, run_id).await?;
    let root = state.run_checkout_dir(&run);
    let path = query
        .path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .ok_or_else(|| AppError::bad_request("path is required"))?;
    let diff = workspace::git_compare_diff(&root, &run.base_ref, path)
        .await
        .map_err(AppError::from)?;
    Ok(Json(serde_json::json!({ "diff": diff })))
}

async fn run_git_commits(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let run = authorize_run(&state, user.id, run_id).await?;
    let root = state.run_checkout_dir(&run);
    Ok(Json(
        workspace::git_commits(&root, &run.base_ref, 50)
            .await
            .map_err(AppError::from)?,
    ))
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
    let git_broker = state
        .git_broker_for_org(run.organization_id)
        .await
        .map_err(AppError::from)?;
    let pr = git_broker
        .create_draft_pr(&run, req)
        .await
        .map_err(AppError::from)?;
    Ok(Json(pr))
}

async fn mark_run_pr_ready(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((run_id, pr_id)): Path<(Uuid, Uuid)>,
) -> Result<impl IntoResponse, AppError> {
    let run = authorize_run(&state, user.id, run_id).await?;
    let pr = state
        .db
        .get_pull_request(pr_id)
        .await?
        .filter(|pr| pr.run_id == run_id)
        .ok_or_else(|| AppError::not_found("pull request not found"))?;
    let git_broker = state
        .git_broker_for_org(run.organization_id)
        .await
        .map_err(AppError::from)?;
    let updated = git_broker
        .mark_pr_ready(&run, &pr)
        .await
        .map_err(AppError::from)?;
    Ok(Json(updated))
}

async fn merge_run_pr(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((run_id, pr_id)): Path<(Uuid, Uuid)>,
) -> Result<impl IntoResponse, AppError> {
    let run = authorize_run(&state, user.id, run_id).await?;
    let pr = state
        .db
        .get_pull_request(pr_id)
        .await?
        .filter(|pr| pr.run_id == run_id)
        .ok_or_else(|| AppError::not_found("pull request not found"))?;
    let git_broker = state
        .git_broker_for_org(run.organization_id)
        .await
        .map_err(AppError::from)?;
    let updated = git_broker
        .merge_pr(&run, &pr)
        .await
        .map_err(AppError::from)?;
    let root = state.run_checkout_dir(&run);
    if root.join(".git").exists() {
        let base_ref = &run.base_ref;
        let refspec = format!("+refs/heads/{base_ref}:refs/remotes/origin/{base_ref}");
        let _ = tokio::process::Command::new("git")
            .args(["fetch", "origin", &refspec])
            .current_dir(&root)
            .output()
            .await;
    }
    Ok(Json(updated))
}

async fn user_push(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let run = authorize_run(&state, user.id, run_id).await?;
    let root = state.run_checkout_dir(&run);
    let commit_msg = format!("zene: {}", run.title.chars().take(72).collect::<String>());
    workspace::commit_worktree_if_dirty(&root, &commit_msg)
        .await
        .map_err(AppError::from)?;
    if !workspace::branch_has_commits_ahead(&root, &run.base_ref)
        .await
        .map_err(AppError::from)?
    {
        return Err(AppError::bad_request(
            "no commits ahead of base branch; commit your changes before pushing",
        ));
    }
    let bundle = create_bundle(&root).await.map_err(AppError::from)?;
    let expected = run.head_sha.clone().unwrap_or_else(|| "HEAD".into());
    let git_broker = state
        .git_broker_for_org(run.organization_id)
        .await
        .map_err(AppError::from)?;
    let result = git_broker
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

async fn claim_run(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Json(req): Json<WorkerClaimRequest>,
) -> Result<impl IntoResponse, AppError> {
    let claimed = state
        .db
        .claim_next_run(&req.worker_id, std::path::Path::new(&req.workspace_root))
        .await?;
    Ok(Json(claimed.map(
        |(run, attempt_id, generation, resume_session_id, workspace_dir, resume_without_prompt)| {
            ClaimedRun {
                run,
                attempt_id,
                generation,
                resume_session_id,
                resume_without_prompt,
                workspace_dir,
            }
        },
    )))
}

async fn queue_stats(
    State(state): State<AppState>,
    _worker: WorkerAuth,
) -> Result<Json<QueueStats>, AppError> {
    Ok(Json(state.db.queue_stats().await?))
}

async fn heartbeat(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
    Json(req): Json<WorkerFence>,
) -> Result<impl IntoResponse, AppError> {
    state.db.heartbeat_fenced(run_id, &req).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn worker_runtime_session(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
    Json(req): Json<WorkerSessionRequest>,
) -> Result<impl IntoResponse, AppError> {
    let fence = req
        .fence
        .ok_or_else(|| AppError::bad_request("worker fence is required"))?;
    state
        .db
        .set_runtime_session_id_fenced(run_id, &fence, &req.session_id)
        .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn worker_event(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
    Json(req): Json<WorkerEventRequest>,
) -> Result<impl IntoResponse, AppError> {
    let fence = req
        .fence
        .ok_or_else(|| AppError::bad_request("worker fence is required"))?;
    Ok(Json(
        state
            .db
            .append_event_fenced_with_cursor(
                run_id,
                &fence,
                Some(&req.source_event_id),
                req.cursor,
                req.event_type,
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
    let fence = req
        .fence
        .ok_or_else(|| AppError::bad_request("worker fence is required"))?;
    Ok(Json(
        state
            .db
            .update_run_status_fenced(run_id, &fence, req.status, req.head_sha, req.failure_code)
            .await?,
    ))
}

async fn worker_title(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
    Json(req): Json<WorkerTitleRequest>,
) -> Result<impl IntoResponse, AppError> {
    let updated = match req.fence {
        Some(fence) => {
            state
                .db
                .update_run_title_fenced(run_id, &fence, req.title.trim())
                .await?
        }
        None => {
            state
                .db
                .update_run_title_legacy(run_id, req.title.trim())
                .await?
        }
    };
    Ok(Json(updated))
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
    if let Some(cached) = state.db.get_cached_clone_credentials(run_id).await? {
        return Ok(Json(cached));
    }
    let git_broker = state
        .git_broker_for_org(run.organization_id)
        .await
        .map_err(AppError::from)?;
    let token = git_broker
        .issue_read_clone_token(&run)
        .await
        .map_err(AppError::from)?;
    let response = zene_cloud_domain::CloneAuthResponse {
        run_id: run.id,
        repository_id: run.repository_id,
        clone_url: token.clone_url,
        username: Some("x-access-token".into()),
        token: Some(token.token),
        base_ref: run.base_ref,
        head_branch: run.head_branch,
        mock: false,
    };
    state.db.store_clone_credentials(&response).await?;
    Ok(Json(response))
}

async fn worker_commands(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
    Query(req): Query<WorkerFence>,
) -> Result<impl IntoResponse, AppError> {
    let commands = state.db.poll_worker_commands_fenced(run_id, &req).await?;
    let mode_id = state.db.take_pending_mode(run_id).await?;
    let title = state
        .db
        .get_run(run_id)
        .await?
        .map(|run| run.title)
        .filter(|title| !title.trim().is_empty());
    Ok(Json(WorkerCommandsResponse {
        commands,
        mode_id,
        title,
    }))
}

async fn worker_set_pending_mode(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
    Json(req): Json<SetRunModeRequest>,
) -> Result<impl IntoResponse, AppError> {
    state.db.set_pending_mode(run_id, &req.mode_id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn worker_take_pending_mode(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let mode_id = state.db.take_pending_mode(run_id).await?;
    Ok(Json(serde_json::json!({ "modeId": mode_id })))
}

async fn worker_command_ack(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
    Json(req): Json<WorkerCommandAckRequest>,
) -> Result<impl IntoResponse, AppError> {
    state
        .db
        .ack_worker_command_fenced(run_id, &req.fence, req.message_id)
        .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
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
    let root = state.run_checkout_dir(&run);
    let commit_msg = format!("zene: {}", run.title.chars().take(72).collect::<String>());
    let committed_head = workspace::commit_worktree_if_dirty(&root, &commit_msg)
        .await
        .map_err(AppError::from)?;
    if !workspace::branch_has_commits_ahead(&root, &run.base_ref)
        .await
        .map_err(AppError::from)?
    {
        return Err(AppError::bad_request(
            "no commits ahead of base branch; nothing to push",
        ));
    }
    let bundle = create_bundle(&root).await.map_err(AppError::from)?;
    let expected = committed_head
        .or_else(|| run.head_sha.clone())
        .unwrap_or_else(|| "HEAD".into());
    let key = req
        .idempotency_key
        .unwrap_or_else(|| format!("push-{run_id}-{expected}"));
    let git_broker = state
        .git_broker_for_org(run.organization_id)
        .await
        .map_err(AppError::from)?;
    Ok(Json(
        git_broker
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
    let git_broker = state
        .git_broker_for_org(run.organization_id)
        .await
        .map_err(AppError::from)?;
    Ok(Json(
        git_broker
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

#[cfg(test)]
mod reconnect_replay_tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::{IntoResponse, Response};
    use futures::StreamExt;
    use serde::de::DeserializeOwned;
    use serde_json::json;
    use std::path::PathBuf;
    use std::time::Duration;
    use uuid::Uuid;
    use zene_cloud_db::Db;
    use zene_cloud_domain::{
        AuthResponse, ClaimedRun, CreateRepositoryRequest, CreateRunRequest, PermissionMode,
        RegisterRequest, Run, RunEvent, RunEventKind, RunStatus, UpdateLlmSettingsRequest,
        WorkerEventRequest, WorkerFence,
    };
    use zene_cloud_github::{GithubClient, GithubConfig};

    async fn decode_json<T: DeserializeOwned>(response: Response) -> T {
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            panic!(
                "invalid JSON response: {error}; body={}",
                String::from_utf8_lossy(&bytes)
            )
        })
    }

    async fn read_sse_until(response: Response, marker: &str) -> String {
        assert_eq!(response.status(), StatusCode::OK);
        let mut stream = response.into_body().into_data_stream();
        let mut data = String::new();
        loop {
            let chunk = tokio::time::timeout(Duration::from_secs(3), stream.next())
                .await
                .expect("SSE replay did not arrive")
                .expect("SSE stream closed before replay")
                .expect("SSE body chunk failed");
            data.push_str(&String::from_utf8_lossy(&chunk));
            if data.contains(marker) {
                return data;
            }
        }
    }

    async fn worker_event_response(
        state: &AppState,
        run_id: Uuid,
        fence: WorkerFence,
        source_event_id: &str,
        cursor: u64,
        marker: &str,
    ) -> RunEvent {
        decode_json(
            worker_event(
                State(state.clone()),
                WorkerAuth,
                Path(run_id),
                Json(WorkerEventRequest {
                    source_event_id: source_event_id.into(),
                    cursor: Some(cursor),
                    event_type: RunEventKind::Runtime,
                    payload: json!({ "marker": marker }),
                    fence: Some(fence),
                }),
            )
            .await
            .unwrap()
            .into_response(),
        )
        .await
    }

    #[tokio::test]
    async fn cloud_api_sse_reconnect_replays_after_replacement_worker_without_duplicates() {
        let db = Db::connect("sqlite::memory:").await.unwrap();
        db.migrate().await.unwrap();
        let worker_token = "integration-worker-token";
        db.ensure_dev_worker_token(worker_token).await.unwrap();
        let workspace_root =
            std::env::temp_dir().join(format!("zene-api-replay-{}", Uuid::new_v4()));
        tokio::fs::create_dir_all(&workspace_root).await.unwrap();
        let state = AppState::new(
            db.clone(),
            worker_token.into(),
            GithubClient::new(GithubConfig::live_default()),
            workspace_root.clone(),
            "http://127.0.0.1:8788".into(),
        );
        let auth: AuthResponse = decode_json(
            register(
                State(state.clone()),
                Json(RegisterRequest {
                    email: "replay@example.com".into(),
                    password: "password123".into(),
                    display_name: "Replay Test".into(),
                    code: None,
                }),
            )
            .await
            .unwrap()
            .into_response(),
        )
        .await;
        let org = state.db.primary_org(auth.user.id).await.unwrap();
        let repo = state
            .db
            .create_repository(
                org.id,
                CreateRepositoryRequest {
                    owner: "replay".into(),
                    name: "demo".into(),
                    default_branch: "main".into(),
                    clone_url: None,
                },
            )
            .await
            .unwrap();
        state
            .db
            .upsert_user_llm_settings(
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
        let run: Run = decode_json(
            create_run(
                State(state.clone()),
                AuthUser(auth.user.clone()),
                Json(CreateRunRequest {
                    repository_id: repo.id,
                    prompt: "replay integration".into(),
                    base_ref: Some("main".into()),
                    model: "default".into(),
                    permission_mode: PermissionMode::Default,
                    max_turns: 10,
                    mode_id: None,
                }),
            )
            .await
            .unwrap()
            .into_response(),
        )
        .await;

        let claimed: ClaimedRun = decode_json(
            claim_run(
                State(state.clone()),
                WorkerAuth,
                Json(WorkerClaimRequest {
                    worker_id: "worker-1".into(),
                    workspace_root: workspace_root.to_string_lossy().into(),
                }),
            )
            .await
            .unwrap()
            .into_response(),
        )
        .await;
        assert_eq!(claimed.run.id, run.id);
        let first_fence = zene_cloud_domain::WorkerFence {
            attempt_id: claimed.attempt_id,
            generation: claimed.generation,
            worker_id: "worker-1".into(),
        };

        let first = worker_event_response(
            &state,
            run.id,
            first_fence.clone(),
            "provider-event-1",
            11,
            "event-1",
        )
        .await;
        let initial_sse = read_sse_until(
            stream_events(
                State(state.clone()),
                AuthUser(auth.user.clone()),
                Path(run.id),
                HeaderMap::new(),
                Query(EventsQuery {
                    after_seq: None,
                    after_cursor: None,
                }),
            )
            .await
            .unwrap()
            .into_response(),
            "event-1",
        )
        .await;
        assert!(initial_sse.contains(&format!("id: {}", first.seq)));

        db.update_run_status(run.id, RunStatus::Failed, None, Some("worker_lost".into()))
            .await
            .unwrap();
        db.update_run_status(run.id, RunStatus::Queued, None, None)
            .await
            .unwrap();
        let replacement: ClaimedRun = decode_json(
            claim_run(
                State(state.clone()),
                WorkerAuth,
                Json(WorkerClaimRequest {
                    worker_id: "worker-2".into(),
                    workspace_root: workspace_root.to_string_lossy().into(),
                }),
            )
            .await
            .unwrap()
            .into_response(),
        )
        .await;
        assert_eq!(replacement.resume_session_id, None);
        let second_fence = zene_cloud_domain::WorkerFence {
            attempt_id: replacement.attempt_id,
            generation: replacement.generation,
            worker_id: "worker-2".into(),
        };

        let replay = worker_event_response(
            &state,
            run.id,
            second_fence.clone(),
            "provider-event-1",
            99,
            "replayed-event-1",
        )
        .await;
        assert_eq!(replay.seq, first.seq);
        assert_eq!(replay.cursor, first.cursor);
        assert_eq!(replay.payload, first.payload);

        let second = worker_event_response(
            &state,
            run.id,
            second_fence,
            "provider-event-2",
            12,
            "event-2",
        )
        .await;
        assert!(second.seq > first.seq);
        let persisted = state.db.events_after(run.id, first.seq).await.unwrap();
        assert_eq!(
            persisted
                .iter()
                .filter(|event| event.payload == second.payload)
                .count(),
            1
        );

        let mut headers = HeaderMap::new();
        headers.insert("last-event-id", first.seq.to_string().parse().unwrap());
        let resumed_sse = read_sse_until(
            stream_events(
                State(state.clone()),
                AuthUser(auth.user),
                Path(run.id),
                headers,
                Query(EventsQuery {
                    after_seq: None,
                    after_cursor: None,
                }),
            )
            .await
            .unwrap()
            .into_response(),
            "event-2",
        )
        .await;
        assert!(resumed_sse.contains(&format!("id: {}", second.seq)));
        assert!(!resumed_sse.contains("event-1"));

        let _ = tokio::fs::remove_dir_all(PathBuf::from(workspace_root)).await;
    }
}
