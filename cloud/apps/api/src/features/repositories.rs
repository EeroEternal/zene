use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use uuid::Uuid;
use zene_cloud_domain::{CreateRepositoryRequest, GithubBranchSummary};

use crate::auth::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/repositories", get(list_repos).post(create_repo))
        .route(
            "/api/v1/repositories/{repository_id}/branches",
            get(list_repo_branches),
        )
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
