use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Redirect};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use zene_cloud_domain::{
    GithubInstallationStatus, GithubProviderConfigView, UpdateGithubProviderConfigRequest,
};

use crate::auth::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/settings/github",
            get(get_github_settings).put(update_github_settings),
        )
        .route("/api/v1/github/status", get(github_status))
        .route("/api/v1/github/connect/start", get(github_connect_start))
        .route(
            "/api/v1/github/install/callback",
            get(github_install_callback),
        )
        .route("/api/v1/github/oauth/start", get(github_oauth_start))
        .route("/api/v1/github/oauth/callback", get(github_oauth_callback))
        .route("/api/v1/github/sync", post(github_sync))
        .route("/api/v1/github/installations", get(list_installations))
        .route(
            "/api/v1/github/installations/{installation_id}/sync",
            post(sync_installation_repos),
        )
}

fn github_provider_view(github: &zene_cloud_github::GithubClient) -> GithubProviderConfigView {
    let cfg = github.config();
    GithubProviderConfigView {
        mode: cfg.mode,
        configured: cfg.is_app_configured(),
        client_id: cfg.client_id.clone(),
        has_client_secret: cfg.client_secret.as_ref().is_some_and(|s| !s.is_empty()),
        app_id: cfg.app_id.clone(),
        has_app_private_key: cfg
            .app_private_key_pem
            .as_ref()
            .is_some_and(|s| !s.is_empty()),
        app_slug: cfg.app_slug.clone(),
    }
}

async fn get_github_settings(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    state
        .reload_github_for_org(org.id)
        .await
        .map_err(AppError::from)?;
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
    let github = state
        .reload_github_for_org(org.id)
        .await
        .map_err(AppError::from)?;
    let account = state.db.get_github_account(user.id).await?;
    Ok(Json(serde_json::json!({
        "provider": github_provider_view(&github),
        "connected": account.is_some(),
        "account": account,
        "installUrl": github.install_url(),
        "redirectUri": format!("{}/api/v1/github/oauth/callback", state.public_base_url),
    })))
}

async fn github_status(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    let github = state
        .reload_github_for_org(org.id)
        .await
        .map_err(AppError::from)?;
    let account = state.db.get_github_account(user.id).await?;
    let installations = state.db.list_installations(org.id).await?;
    let connected = account.is_some() || !installations.is_empty();
    let display_login = account
        .as_ref()
        .map(|a| a.login.clone())
        .or_else(|| installations.first().map(|i| i.account_login.clone()));
    Ok(Json(serde_json::json!({
        "mode": github.mode(),
        "configured": github.config().is_app_configured(),
        "connected": connected,
        "account": account,
        "displayLogin": display_login,
        "installations": installations,
        "installUrl": github.install_url(),
        "setupUrl": format!("{}/api/v1/github/install/callback", state.public_base_url),
    })))
}

async fn github_connect_start(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    let github = state
        .reload_github_for_org(org.id)
        .await
        .map_err(AppError::from)?;
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
    let github = state
        .reload_github_for_org(org.id)
        .await
        .map_err(AppError::from)?;
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
            remote.account_type,
            GithubInstallationStatus::Active,
        )
        .await?;
    let listed = github
        .list_installation_repos(&remote.id)
        .await
        .map_err(AppError::from)?;
    state.db.sync_repos_from_github(org.id, &listed).await?;
    let _ = query.setup_action;
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
    let github = state
        .reload_github_for_org(org.id)
        .await
        .map_err(AppError::from)?;
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
    let github = state
        .reload_github_for_org(org.id)
        .await
        .map_err(AppError::from)?;
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

async fn github_sync(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let org = state.db.primary_org(user.id).await?;
    let github = state
        .reload_github_for_org(org.id)
        .await
        .map_err(AppError::from)?;

    let installations = state.db.list_installations(org.id).await?;
    if installations.is_empty() {
        return Err(AppError::bad_request(
            "connect GitHub before syncing repositories",
        ));
    }
    let mut synced_installations = Vec::new();
    let mut all_repos = Vec::new();
    for inst in installations {
        if let Ok(remote) = github.get_installation(&inst.installation_id).await {
            let installation = state
                .db
                .upsert_installation(
                    org.id,
                    &remote.id,
                    &remote.account_login,
                    remote.account_type,
                    GithubInstallationStatus::Active,
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
    state
        .reload_github_for_org(org.id)
        .await
        .map_err(AppError::from)?;
    let github = state.github_client().await;
    let listed = github
        .list_installation_repos(&installation_id)
        .await
        .map_err(AppError::from)?;
    let repos = state.db.sync_repos_from_github(org.id, &listed).await?;
    Ok(Json(repos))
}
