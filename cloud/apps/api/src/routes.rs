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
    ClaimedRun, CreateApprovalRequest, CreatePullRequestBody, CreateRepositoryRequest,
    CreateRunRequest, DecideApprovalRequest, GithubBranchSummary, GithubProviderConfigView,
    LlmAuthResponse, LlmSettingsView, LoginRequest, PostMessageRequest, QueueStats, RegisterRequest,
    RunStatus, UpdateGithubProviderConfigRequest, UpdateLlmSettingsRequest, UpdateRunRequest,
    WorkerCommandsResponse, WorkerEventRequest, WorkerFence, WorkerFenceRequest,
    WorkerAcpSessionRequest,
    WorkerPullRequestRequest, WorkerPushRequest, WorkerStatusRequest, WorkerTitleRequest,
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
        .route("/api/v1/settings/github", get(get_github_settings).put(update_github_settings))
        .route("/api/v1/settings/llm", get(get_llm_settings).put(update_llm_settings))
        .route("/api/v1/github/status", get(github_status))
        .route("/api/v1/github/connect/start", get(github_connect_start))
        .route("/api/v1/github/install/callback", get(github_install_callback))
        .route("/api/v1/github/oauth/start", get(github_oauth_start))
        .route("/api/v1/github/oauth/callback", get(github_oauth_callback))
        .route("/api/v1/github/mock/connect", post(github_mock_connect))
        .route("/api/v1/github/sync", post(github_sync))
        .route("/api/v1/github/installations", get(list_installations))
        .route("/api/v1/github/installations/mock", post(github_mock_install))
        .route(
            "/api/v1/github/installations/{installation_id}/sync",
            post(sync_installation_repos),
        )
        .route("/api/v1/repositories", get(list_repos).post(create_repo))
        .route(
            "/api/v1/repositories/{repository_id}/branches",
            get(list_repo_branches),
        )
        .route("/api/v1/runs", get(list_runs).post(create_run))
        .route(
            "/api/v1/runs/{run_id}",
            get(get_run).patch(update_run).delete(delete_run),
        )
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
        .route("/api/v1/runs/{run_id}/git/push", post(user_push))
        .route("/internal/v1/runs/claim", post(claim_run))
        .route("/internal/v1/queue/stats", get(queue_stats))
        .route("/internal/v1/runs/{run_id}/heartbeat", post(heartbeat))
        .route("/internal/v1/runs/{run_id}/acp-session", post(worker_acp_session))
        .route("/internal/v1/runs/{run_id}/events", post(worker_event))
        .route("/internal/v1/runs/{run_id}/status", post(worker_status))
        .route("/internal/v1/runs/{run_id}/title", post(worker_title))
        .route(
            "/internal/v1/runs/{run_id}/clone-auth",
            get(clone_auth).post(clone_auth),
        )
        .route("/internal/v1/runs/{run_id}/llm-auth", get(llm_auth))
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
    let github = state.github_client().await;
    Json(serde_json::json!({
        "ok": true,
        "service": "zene-cloud-api",
        "version": env!("CARGO_PKG_VERSION"),
        "githubMode": format!("{:?}", github.mode()).to_lowercase(),
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
    state.reload_github_for_org(org.id).await.map_err(AppError::from)?;
    let github = state.github_client().await;
    let gh_account = state.db.get_github_account(user.id).await?;
    Ok(Json(serde_json::json!({
        "user": user,
        "organization": org,
        "github": gh_account,
        "githubMode": format!("{:?}", github.mode()).to_lowercase(),
        "githubConfigured": github.config().is_app_configured(),
    })))
}

fn github_provider_view(
    github: &zene_cloud_github::GithubClient,
) -> GithubProviderConfigView {
    let cfg = github.config();
    GithubProviderConfigView {
        mode: cfg.mode,
        configured: cfg.is_app_configured(),
        client_id: cfg.client_id.clone(),
        has_client_secret: cfg.client_secret.as_ref().is_some_and(|s| !s.is_empty()),
        app_id: cfg.app_id.clone(),
        has_app_private_key: cfg.app_private_key_pem.as_ref().is_some_and(|s| !s.is_empty()),
        app_slug: cfg.app_slug.clone(),
    }
}

async fn get_github_settings(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    state.reload_github_for_org(org.id).await.map_err(AppError::from)?;
    let github = state.github_client().await;
    let account = state.db.get_github_account(user.id).await?;
    let view = github_provider_view(&github);
    Ok(Json(serde_json::json!({
        "provider": view,
        "connected": account.is_some(),
        "account": account,
        "installUrl": github.install_url(),
        "redirectUri": format!("{}/api/v1/github/oauth/callback", state.public_base_url),
    })))
}

async fn update_github_settings(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<UpdateGithubProviderConfigRequest>,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    state.db.upsert_github_provider_config(org.id, req).await?;
    let github = state.reload_github_for_org(org.id).await.map_err(AppError::from)?;
    let account = state.db.get_github_account(user.id).await?;
    Ok(Json(serde_json::json!({
        "provider": github_provider_view(&github),
        "connected": account.is_some(),
        "account": account,
        "installUrl": github.install_url(),
        "redirectUri": format!("{}/api/v1/github/oauth/callback", state.public_base_url),
    })))
}

fn empty_llm_settings_view() -> LlmSettingsView {
    LlmSettingsView {
        provider_id: "custom".into(),
        base_url: String::new(),
        default_model: String::new(),
        models: Vec::new(),
        has_api_key: false,
        api_key_hint: None,
    }
}

async fn get_llm_settings(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let settings = state.db.get_user_llm_settings(user.id).await?;
    Ok(Json(
        settings
            .map(|s| s.to_view())
            .unwrap_or_else(empty_llm_settings_view),
    ))
}

async fn update_llm_settings(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<UpdateLlmSettingsRequest>,
) -> Result<impl IntoResponse, AppError> {
    if req.provider_id.trim().is_empty() {
        return Err(AppError::bad_request("providerId is required"));
    }
    let saved = state.db.upsert_user_llm_settings(user.id, req).await?;
    Ok(Json(saved.to_view()))
}

async fn llm_auth(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let run = state
        .db
        .get_run(run_id)
        .await?
        .ok_or_else(|| AppError::not_found("run not found"))?;
    let settings = state
        .db
        .get_user_llm_settings(run.requested_by)
        .await?
        .ok_or_else(|| AppError::not_found("llm settings not configured"))?;
    if settings.api_key.trim().is_empty() {
        return Err(AppError::not_found("llm api key not configured"));
    }
    if settings.base_url.trim().is_empty() {
        return Err(AppError::not_found("llm base url not configured"));
    }
    let model = {
        let m = run.model.trim();
        if !m.is_empty() && m != "default" {
            m.to_string()
        } else if !settings.default_model.trim().is_empty() {
            settings.default_model.trim().to_string()
        } else {
            "default".into()
        }
    };
    Ok(Json(LlmAuthResponse {
        api_key: settings.api_key,
        base_url: settings.base_url,
        model,
        provider: "openai".into(),
    }))
}

async fn github_status(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    state.reload_github_for_org(org.id).await.map_err(AppError::from)?;
    let github = state.github_client().await;
    let account = state.db.get_github_account(user.id).await?;
    let installations = state.db.list_installations(org.id).await?;
    let live_installations: Vec<_> = installations
        .iter()
        .filter(|i| !is_mock_installation(i))
        .collect();
    let connected = if github.is_mock() {
        account.is_some() || !installations.is_empty()
    } else {
        account.is_some() || !live_installations.is_empty()
    };
    let display_login = account
        .as_ref()
        .map(|a| a.login.clone())
        .or_else(|| {
            live_installations
                .first()
                .map(|i| i.account_login.clone())
        })
        .or_else(|| installations.first().map(|i| i.account_login.clone()));
    Ok(Json(serde_json::json!({
        "mode": format!("{:?}", github.mode()).to_lowercase(),
        "configured": github.config().is_app_configured(),
        "connected": connected,
        "account": account,
        "displayLogin": display_login,
        "installations": installations,
        "installUrl": github.install_url(),
        "setupUrl": format!("{}/api/v1/github/install/callback", state.public_base_url),
    })))
}

fn is_mock_installation(inst: &zene_cloud_domain::GithubInstallation) -> bool {
    inst.installation_id == "10001" || inst.account_login == "mock-org"
}

async fn github_connect_start(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    state.reload_github_for_org(org.id).await.map_err(AppError::from)?;
    let github = state.github_client().await;
    if github.is_mock() {
        return Ok(Json(serde_json::json!({
            "mode": "mock",
            "hint": "mock mode: call POST /api/v1/github/mock/connect instead"
        })));
    }
    if !github.config().is_app_configured() {
        return Err(AppError::bad_request(
            "GitHub App is not configured on this server (set GITHUB_APP_ID, GITHUB_APP_PRIVATE_KEY_PATH, GITHUB_APP_SLUG)",
        ));
    }
    let oauth_state = zene_cloud_github::new_oauth_state();
    state
        .db
        .save_oauth_state(&oauth_state, Some(user.id), Some("/"), 900)
        .await?;
    let install_url = github
        .install_url_with_state(&oauth_state)
        .ok_or_else(|| AppError::bad_request("GitHub App slug is not configured"))?;
    Ok(Json(serde_json::json!({
        "installUrl": install_url,
        "mode": "live"
    })))
}

#[derive(Debug, Deserialize)]
struct InstallCallbackQuery {
    installation_id: Option<String>,
    setup_action: Option<String>,
    state: Option<String>,
}

async fn github_install_callback(
    State(state): State<AppState>,
    Query(query): Query<InstallCallbackQuery>,
) -> Result<impl IntoResponse, AppError> {
    let installation_id = query
        .installation_id
        .ok_or_else(|| AppError::bad_request("missing installation_id"))?;
    let state_key = query
        .state
        .ok_or_else(|| AppError::bad_request("missing state"))?;
    let saved = state
        .db
        .take_oauth_state(&state_key)
        .await?
        .ok_or_else(|| AppError::bad_request("invalid or expired state"))?;
    let user_id = saved
        .user_id
        .ok_or_else(|| AppError::unauthorized("install state has no user"))?;
    let org = state.db.primary_org(user_id).await?;
    state.reload_github_for_org(org.id).await.map_err(AppError::from)?;
    let github = state.github_client().await;
    let remote = github
        .get_installation(&installation_id)
        .await
        .map_err(AppError::from)?;
    state
        .db
        .upsert_installation(
            org.id,
            &remote.id,
            &remote.account_login,
            &remote.account_type,
            "active",
        )
        .await?;
    if !github.is_mock() {
        state.db.purge_mock_github_data(org.id).await?;
    }
    let listed = github
        .list_installation_repos(&remote.id)
        .await
        .map_err(AppError::from)?;
    state.db.sync_repos_from_github(org.id, &listed).await?;
    let _ = query.setup_action;
    // Absolute redirect so the popup lands on the configured public origin
    // (important when UI and API use different localhost ports).
    Ok(Redirect::temporary(&format!(
        "{}/?github=connected",
        state.public_base_url.trim_end_matches('/')
    )))
}

async fn github_oauth_start(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    state.reload_github_for_org(org.id).await.map_err(AppError::from)?;
    let github = state.github_client().await;
    if github.is_mock() {
        return Ok(Json(serde_json::json!({
            "mode": "mock",
            "hint": "mock mode: call POST /api/v1/github/mock/connect instead of browser OAuth"
        })));
    }
    if !github.config().is_configured() {
        return Err(AppError::bad_request(
            "Configure GitHub OAuth Client ID and Secret in Settings first",
        ));
    }
    let redirect = format!("{}/api/v1/github/oauth/callback", state.public_base_url);
    let (url, oauth_state) = github
        .begin_oauth(Some(redirect.clone()))
        .map_err(AppError::from)?;
    state
        .db
        .save_oauth_state(&oauth_state, Some(user.id), Some("/"), 900)
        .await?;
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
    let org = state.db.primary_org(user_id).await?;
    state.reload_github_for_org(org.id).await.map_err(AppError::from)?;
    let github = state.github_client().await;
    let redirect = format!("{}/api/v1/github/oauth/callback", state.public_base_url);
    let tokens = github
        .exchange_oauth_code(&code, Some(redirect))
        .await
        .map_err(AppError::from)?;
    let gh_user = github
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
    let org = state.db.primary_org(user.id).await?;
    state.reload_github_for_org(org.id).await.map_err(AppError::from)?;
    let github = state.github_client().await;
    if !github.is_mock() {
        return Err(AppError::conflict("mock connect only available in mock mode"));
    }
    let tokens = github
        .exchange_oauth_code("mock-code", None)
        .await
        .map_err(AppError::from)?;
    let gh_user = github
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
    let listed = github
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

async fn github_sync(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    state.reload_github_for_org(org.id).await.map_err(AppError::from)?;
    let github = state.github_client().await;
    if github.is_mock() {
        let tokens = github
            .exchange_oauth_code("mock-code", None)
            .await
            .map_err(AppError::from)?;
        let gh_user = github
            .get_user(&tokens.access_token)
            .await
            .map_err(AppError::from)?;
        state
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
        let installation = state
            .db
            .upsert_installation(org.id, "10001", "mock-org", "Organization", "active")
            .await?;
        let listed = github
            .list_installation_repos("10001")
            .await
            .map_err(AppError::from)?;
        let repos = state.db.sync_repos_from_github(org.id, &listed).await?;
        return Ok(Json(serde_json::json!({
            "installations": vec![installation],
            "repositories": repos,
        })));
    }

    // App install flow stores installations on the org; OAuth account is optional.
    // Re-sync repos for installations already linked to this org (do not pull every
    // installation of the GitHub App — that would cross tenants).
    let installations = state.db.list_installations(org.id).await?;
    let live: Vec<_> = installations
        .into_iter()
        .filter(|i| !is_mock_installation(i))
        .collect();
    if live.is_empty() {
        return Err(AppError::bad_request(
            "connect GitHub before syncing repositories",
        ));
    }
    let mut synced_installations = Vec::new();
    let mut all_repos = Vec::new();
    for inst in live {
        // Refresh account metadata from GitHub when possible.
        if let Ok(remote) = github.get_installation(&inst.installation_id).await {
            let installation = state
                .db
                .upsert_installation(
                    org.id,
                    &remote.id,
                    &remote.account_login,
                    &remote.account_type,
                    "active",
                )
                .await?;
            synced_installations.push(installation);
        } else {
            synced_installations.push(inst.clone());
        }
        let listed = github
            .list_installation_repos(&inst.installation_id)
            .await
            .map_err(AppError::from)?;
        let repos = state.db.sync_repos_from_github(org.id, &listed).await?;
        all_repos.extend(repos);
    }
    Ok(Json(serde_json::json!({
        "installations": synced_installations,
        "repositories": all_repos,
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
    state.reload_github_for_org(org.id).await.map_err(AppError::from)?;
    let github = state.github_client().await;
    let listed = github
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

async fn list_repo_branches(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(repository_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    let repo = state
        .db
        .get_repository(repository_id)
        .await?
        .ok_or_else(|| AppError::not_found("repository not found"))?;
    if repo.organization_id != org.id {
        return Err(AppError::forbidden("repository not in organization"));
    }
    let installation_id = repo
        .installation_id
        .as_deref()
        .ok_or_else(|| AppError::bad_request("repository has no GitHub installation"))?;
    state
        .reload_github_for_org(org.id)
        .await
        .map_err(AppError::from)?;
    let github = state.github_client().await;
    let names = github
        .list_repo_branches(installation_id, &repo.owner, &repo.name)
        .await
        .map_err(AppError::from)?;
    let branches = names
        .into_iter()
        .map(|name| GithubBranchSummary {
            default: name == repo.default_branch,
            name,
        })
        .collect::<Vec<_>>();
    Ok(Json(branches))
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
        None => state.db.events_after(run_id, query.after_seq.unwrap_or(0)).await?,
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
    let mut after = last_event_id
        .or(query.after_seq)
        .unwrap_or(0);
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
    let _ = authorize_run(&state, user.id, run_id).await?;
    let root = state.workspace_root.join(run_id.to_string());
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
    let _ = authorize_run(&state, user.id, run_id).await?;
    let root = state.workspace_root.join(run_id.to_string());
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
    let root = state.workspace_root.join(run_id.to_string());
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
    let root = state.workspace_root.join(run_id.to_string());
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
    let root = state.workspace_root.join(run_id.to_string());
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
    let git_broker = state.git_broker().await;
    let pr = git_broker
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
    let git_broker = state.git_broker().await;
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
    Ok(Json(claimed.map(|(run, attempt_id, generation, resume_session_id, workspace_dir)| {
        ClaimedRun {
            run,
            attempt_id,
            generation,
            resume_session_id,
            workspace_dir,
        }
    })))
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
    Json(req): Json<WorkerFenceRequest>,
) -> Result<impl IntoResponse, AppError> {
    let fence = WorkerFence {
        attempt_id: req.attempt_id,
        generation: req.generation,
        worker_id: req.worker_id,
    };
    state.db.heartbeat_fenced(run_id, &fence).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn worker_acp_session(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
    Json(req): Json<WorkerAcpSessionRequest>,
) -> Result<impl IntoResponse, AppError> {
    let fence = req
        .fence
        .ok_or_else(|| AppError::bad_request("worker fence is required"))?;
    state
        .db
        .set_acp_session_id_fenced(run_id, &fence, &req.session_id)
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
    let fence = req
        .fence
        .ok_or_else(|| AppError::bad_request("worker fence is required"))?;
    Ok(Json(
        state
            .db
            .update_run_status_fenced(
                run_id,
                &fence,
                req.status,
                req.head_sha,
                req.failure_code,
            )
            .await?,
    ))
}

async fn worker_title(
    State(state): State<AppState>,
    _worker: WorkerAuth,
    Path(run_id): Path<Uuid>,
    Json(req): Json<WorkerTitleRequest>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(
        state
            .db
            .update_run_meta(run_id, Some(req.title.trim()), None)
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
    let git_broker = state.git_broker().await;
    let token = git_broker
        .issue_read_clone_token(&run)
        .await
        .map_err(AppError::from)?;
    let github = state.github_client().await;
    let mock = token.mode == "mock" || github.is_mock();
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
    Query(req): Query<WorkerFenceRequest>,
) -> Result<impl IntoResponse, AppError> {
    let fence = WorkerFence {
        attempt_id: req.attempt_id,
        generation: req.generation,
        worker_id: req.worker_id,
    };
    Ok(Json(WorkerCommandsResponse {
        commands: state.db.poll_worker_commands_fenced(run_id, &fence).await?,
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
    let git_broker = state.git_broker().await;
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
    let git_broker = state.git_broker().await;
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
