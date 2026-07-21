use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use zene_cloud_domain::User;

use crate::error::AppError;
use crate::state::AppState;

pub struct AuthUser(pub User);

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts).ok_or_else(|| AppError::unauthorized("missing token"))?;
        let user = state
            .db
            .user_from_token(&token)
            .await
            .map_err(AppError::from)?
            .ok_or_else(|| AppError::unauthorized("invalid session"))?;
        Ok(AuthUser(user))
    }
}

pub struct WorkerAuth;

impl FromRequestParts<AppState> for WorkerAuth {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts).ok_or_else(|| AppError::unauthorized("missing token"))?;
        if token == state.worker_token
            || state
                .db
                .verify_worker_token(&token)
                .await
                .map_err(AppError::from)?
        {
            return Ok(WorkerAuth);
        }
        Err(AppError::unauthorized("invalid worker token"))
    }
}

fn bearer_token(parts: &Parts) -> Option<String> {
    let value = parts.headers.get(axum::http::header::AUTHORIZATION)?;
    let value = value.to_str().ok()?;
    value
        .strip_prefix("Bearer ")
        .map(|v| v.to_string())
        .or_else(|| {
            parts
                .headers
                .get("x-zene-cloud-token")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.to_string())
        })
}
