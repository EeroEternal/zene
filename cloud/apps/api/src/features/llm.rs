use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use uuid::Uuid;
use zene_cloud_domain::{LlmAuthResponse, LlmSettingsView, UpdateLlmSettingsRequest};

use crate::auth::{AuthUser, WorkerAuth};
use crate::error::AppError;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/settings/llm",
            get(get_llm_settings).put(update_llm_settings),
        )
        .route("/internal/v1/runs/{run_id}/llm-auth", get(llm_auth))
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
