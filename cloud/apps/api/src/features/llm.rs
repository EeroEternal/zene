use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use uuid::Uuid;
use zene_cloud_domain::{
    CreateLlmProviderRequest, LlmAuthResponse, LlmProviderView, LlmSettingsView,
    UpdateLlmProviderRequest, UpdateLlmSettingsRequest,
};

use crate::auth::{AuthUser, WorkerAuth};
use crate::error::AppError;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/settings/llm",
            get(get_llm_settings).put(update_llm_settings),
        )
        .route(
            "/api/v1/settings/llm/providers",
            get(list_llm_providers).post(create_llm_provider),
        )
        .route(
            "/api/v1/settings/llm/providers/{provider_id}",
            put(update_llm_provider).delete(delete_llm_provider),
        )
        .route(
            "/api/v1/settings/llm/providers/{provider_id}/test",
            post(test_llm_provider),
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
    let providers = state.db.list_user_llm_providers(user.id).await?;
    if let Some(def) = providers
        .iter()
        .find(|p| p.is_default)
        .or_else(|| providers.first())
    {
        let trimmed = def.api_key.trim();
        let has_api_key = !trimmed.is_empty();
        let api_key_hint = if has_api_key {
            let hint: String = trimmed
                .chars()
                .rev()
                .take(4)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            Some(format!("••••{hint}"))
        } else {
            None
        };
        // Aggregate all models across providers
        let mut all_models: Vec<String> = Vec::new();
        if !def.default_model.trim().is_empty() {
            all_models.push(def.default_model.trim().to_string());
        }
        for p in &providers {
            for m in &p.models {
                if !all_models.contains(m) {
                    all_models.push(m.clone());
                }
            }
        }
        return Ok(Json(LlmSettingsView {
            provider_id: def.provider_id.clone(),
            base_url: def.base_url.clone(),
            default_model: def.default_model.clone(),
            models: all_models,
            has_api_key,
            api_key_hint,
        }));
    }

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
    let saved = state
        .db
        .upsert_user_llm_settings(user.id, req.clone())
        .await?;

    // Also sync to a default provider in user_llm_providers
    let providers = state.db.list_user_llm_providers(user.id).await?;
    if let Some(first) = providers
        .iter()
        .find(|p| p.is_default)
        .or_else(|| providers.first())
    {
        let _ = state
            .db
            .update_user_llm_provider(
                user.id,
                first.id,
                UpdateLlmProviderRequest {
                    provider_id: Some(req.provider_id.clone()),
                    name: None,
                    base_url: Some(req.base_url.clone()),
                    default_model: Some(req.default_model.clone()),
                    models: Some(req.models.clone()),
                    api_key: req.api_key.clone(),
                    is_default: Some(true),
                },
            )
            .await;
    } else {
        let _ = state
            .db
            .create_user_llm_provider(
                user.id,
                CreateLlmProviderRequest {
                    provider_id: req.provider_id.clone(),
                    name: None,
                    base_url: req.base_url.clone(),
                    default_model: req.default_model.clone(),
                    models: req.models.clone(),
                    api_key: req.api_key.clone(),
                    is_default: true,
                },
            )
            .await;
    }

    Ok(Json(saved.to_view()))
}

async fn list_llm_providers(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let providers = state.db.list_user_llm_providers(user.id).await?;
    let views: Vec<LlmProviderView> = providers.into_iter().map(|p| p.to_view()).collect();
    Ok(Json(views))
}

async fn create_llm_provider(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<CreateLlmProviderRequest>,
) -> Result<impl IntoResponse, AppError> {
    if req.provider_id.trim().is_empty() {
        return Err(AppError::bad_request("providerId is required"));
    }
    if req.base_url.trim().is_empty() {
        return Err(AppError::bad_request("baseUrl is required"));
    }
    let provider = state.db.create_user_llm_provider(user.id, req).await?;
    Ok(Json(provider.to_view()))
}

async fn update_llm_provider(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(provider_id): Path<Uuid>,
    Json(req): Json<UpdateLlmProviderRequest>,
) -> Result<impl IntoResponse, AppError> {
    let provider = state
        .db
        .update_user_llm_provider(user.id, provider_id, req)
        .await?;
    Ok(Json(provider.to_view()))
}

async fn delete_llm_provider(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(provider_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    state
        .db
        .delete_user_llm_provider(user.id, provider_id)
        .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn test_llm_provider(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(provider_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let provider = state
        .db
        .get_user_llm_provider(user.id, provider_id)
        .await?
        .ok_or_else(|| AppError::not_found("provider not found"))?;

    let base_url = provider.base_url.trim().trim_end_matches('/');
    if base_url.is_empty() {
        return Err(AppError::bad_request("base URL is empty"));
    }

    let api_key = provider.api_key.trim();
    if api_key.is_empty() {
        return Err(AppError::bad_request("API key is empty"));
    }

    let model = if !provider.default_model.trim().is_empty() {
        provider.default_model.trim().to_string()
    } else {
        "gpt-4o-mini".into()
    };

    let chat_url = format!("{}/chat/completions", base_url);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| AppError::internal(e))?;

    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 1,
    });

    let response = client
        .post(&chat_url)
        .bearer_auth(api_key)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::bad_request(format!("Connection failed: {}", e)))?;

    let status = response.status();
    let resp_body = response.text().await.unwrap_or_default();

    if status.is_success() {
        Ok(Json(serde_json::json!({
            "ok": true,
            "message": format!("Connected — model '{}' responded (HTTP {})", model, status.as_u16()),
        })))
    } else if status.as_u16() == 404 {
        Ok(Json(serde_json::json!({
            "ok": false,
            "message": format!("Model '{}' not found at {} (HTTP 404). Check the model name and base URL.", model, chat_url),
        })))
    } else if status.as_u16() == 401 || status.as_u16() == 403 {
        Ok(Json(serde_json::json!({
            "ok": false,
            "message": format!("Authentication failed (HTTP {}). Check your API key.", status.as_u16()),
        })))
    } else {
        let snippet: String = resp_body.chars().take(200).collect();
        Ok(Json(serde_json::json!({
            "ok": false,
            "message": format!("HTTP {} — {}", status.as_u16(), snippet),
        })))
    }
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

    let provider = state
        .db
        .resolve_user_llm_provider_for_model(run.requested_by, &run.model)
        .await?
        .ok_or_else(|| AppError::not_found("llm provider settings not configured"))?;

    if provider.api_key.trim().is_empty() {
        return Err(AppError::not_found("llm api key not configured"));
    }
    if provider.base_url.trim().is_empty() {
        return Err(AppError::not_found("llm base url not configured"));
    }

    let model = {
        let m = run.model.trim();
        if !m.is_empty() && m != "default" {
            m.to_string()
        } else if !provider.default_model.trim().is_empty() {
            provider.default_model.trim().to_string()
        } else {
            "default".into()
        }
    };

    Ok(Json(LlmAuthResponse {
        api_key: provider.api_key,
        base_url: provider.base_url,
        model,
        provider: "openai".into(),
    }))
}
