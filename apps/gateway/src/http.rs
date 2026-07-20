use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use crate::agent::AgentManager;
use crate::auth::{AuthState, ErrorBody};
use crate::event_journal::CursorExpired;
use crate::static_page::INDEX_HTML;

#[derive(Clone)]
pub struct AppState {
    pub auth: AuthState,
    pub agents: AgentManager,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub version: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAgentRequest {
    pub request_id: String,
    pub workspace: PathBuf,
    #[serde(default)]
    pub sandbox_profile: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostMessagesRequest {
    pub request_id: String,
    pub messages: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventsQuery {
    pub cursor: Option<u64>,
    pub wait_ms: Option<u64>,
    pub limit: Option<usize>,
    pub token: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApiMeta {
    server_time: chrono::DateTime<chrono::Utc>,
    trace_id: String,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/v1/bootstrap", get(bootstrap))
        .route("/api/v1/health", get(health))
        .route("/api/v1/agents", post(create_agent))
        .route("/api/v1/agents/{agent_id}/messages", post(post_messages))
        .route("/api/v1/agents/{agent_id}/events", get(get_events))
        .route("/api/v1/agents/{agent_id}/health", get(agent_health))
        .layer(TraceLayer::new_for_http())
        .with_state(Arc::new(state))
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn bootstrap(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(json!({
        "gatewayVersion": state.version,
        "apiVersion": "v1",
        "acpTransport": "stdio-ndjson",
        "transports": {
            "longPolling": true,
            "sse": false,
            "shortPolling": true,
            "websocket": false
        },
        "auth": {
            "header": "X-Zene-Token",
            "query": "token"
        },
        "limits": {
            "maxPostBodyBytes": 1_048_576,
            "maxMessagesPerPost": 100,
            "defaultWaitMs": 25_000,
            "maxWaitMs": 30_000,
            "maxEventsPerPoll": 200
        },
        "bind": {
            "host": state.auth.bind_host,
            "port": state.auth.port
        },
        "serverTime": chrono::Utc::now(),
        "startedAt": state.started_at,
    }))
}

async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(json!({
        "ok": true,
        "gatewayVersion": state.version,
        "uptimeMs": (chrono::Utc::now() - state.started_at).num_milliseconds().max(0),
        "serverTime": chrono::Utc::now(),
    }))
}

async fn create_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CreateAgentRequest>,
) -> Response {
    if let Err(status) = state.auth.authorize(&headers, None) {
        return auth_error(status);
    }
    if body.request_id.trim().is_empty() {
        return json_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "requestId is required",
            false,
        );
    }
    if let Some((status, cached)) = state.auth.recall_idempotent(&body.request_id).await {
        return (status, Json(cached)).into_response();
    }

    // sandbox_profile reserved for later; phase A passes workspace cwd only.
    let _ = body.sandbox_profile;
    match state.agents.create(body.workspace).await {
        Ok(info) => {
            let payload = json!({
                "agent": info,
                "meta": meta(),
            });
            state
                .auth
                .store_idempotent(body.request_id, StatusCode::OK, payload.clone())
                .await;
            (StatusCode::OK, Json(payload)).into_response()
        }
        Err(err) => json_error(
            StatusCode::BAD_REQUEST,
            "agent_create_failed",
            err.to_string(),
            true,
        ),
    }
}

async fn post_messages(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<PostMessagesRequest>,
) -> Response {
    if let Err(status) = state.auth.authorize(&headers, None) {
        return auth_error(status);
    }
    if body.request_id.trim().is_empty() {
        return json_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "requestId is required",
            false,
        );
    }
    if body.messages.is_empty() {
        return json_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "messages must not be empty",
            false,
        );
    }
    if body.messages.len() > 100 {
        return json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too_many_messages",
            "max 100 messages per POST",
            false,
        );
    }
    if let Some((status, cached)) = state.auth.recall_idempotent(&body.request_id).await {
        return (status, Json(cached)).into_response();
    }
    if state.agents.get(&agent_id).await.is_none() {
        return json_error(
            StatusCode::NOT_FOUND,
            "agent_not_found",
            "unknown agentId",
            false,
        );
    }

    match state.agents.write_messages(&agent_id, &body.messages).await {
        Ok(accepted) => {
            let payload = json!({
                "accepted": accepted,
                "agentId": agent_id,
                "meta": meta(),
            });
            state
                .auth
                .store_idempotent(body.request_id, StatusCode::ACCEPTED, payload.clone())
                .await;
            (StatusCode::ACCEPTED, Json(payload)).into_response()
        }
        Err(err) => {
            let retryable = err.to_string().contains("not running");
            json_error(
                StatusCode::CONFLICT,
                "agent_not_running",
                err.to_string(),
                retryable,
            )
        }
    }
}

async fn get_events(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Query(query): Query<EventsQuery>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = state
        .auth
        .authorize(&headers, query.token.as_deref())
    {
        return auth_error(status);
    }
    let Some(info) = state.agents.get(&agent_id).await else {
        return json_error(
            StatusCode::NOT_FOUND,
            "agent_not_found",
            "unknown agentId",
            false,
        );
    };
    let Some(journal) = state.agents.journal(&agent_id).await else {
        return json_error(
            StatusCode::NOT_FOUND,
            "agent_not_found",
            "unknown agentId",
            false,
        );
    };

    let cursor = query.cursor.unwrap_or(0);
    let limit = query.limit.unwrap_or(200).clamp(1, 200);
    let wait_ms = query.wait_ms.unwrap_or(25_000).min(30_000);
    let wait = Duration::from_millis(wait_ms);

    match journal.wait_for_events(cursor, limit, wait).await {
        Ok((events, next_cursor, has_more)) => {
            let payload = json!({
                "events": events,
                "nextCursor": next_cursor,
                "hasMore": has_more,
                "agentState": info.state,
                "meta": meta(),
            });
            (StatusCode::OK, Json(payload)).into_response()
        }
        Err(CursorExpired {
            oldest_cursor,
            latest_cursor,
        }) => {
            let mut body = ErrorBody::new(
                "cursor_expired",
                "event cursor is older than the retained journal",
                false,
            );
            body.oldest_cursor = Some(oldest_cursor);
            body.latest_cursor = Some(latest_cursor);
            body.recovery = Some("reload_session".into());
            (StatusCode::CONFLICT, Json(body)).into_response()
        }
    }
}

async fn agent_health(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = state.auth.authorize(&headers, None) {
        return auth_error(status);
    }
    match state.agents.health(&agent_id).await {
        Some(health) => Json(json!({
            "agent": health,
            "meta": meta(),
        }))
        .into_response(),
        None => json_error(
            StatusCode::NOT_FOUND,
            "agent_not_found",
            "unknown agentId",
            false,
        ),
    }
}

fn meta() -> ApiMeta {
    ApiMeta {
        server_time: chrono::Utc::now(),
        trace_id: Uuid::new_v4().to_string(),
    }
}

fn auth_error(status: StatusCode) -> Response {
    let (error, message) = match status {
        StatusCode::UNAUTHORIZED => ("unauthorized", "missing or invalid X-Zene-Token"),
        StatusCode::FORBIDDEN => ("forbidden_origin", "Origin is not allowed"),
        _ => ("unauthorized", "authentication failed"),
    };
    json_error(status, error, message, false)
}

fn json_error(
    status: StatusCode,
    error: &str,
    message: impl Into<String>,
    retryable: bool,
) -> Response {
    (
        status,
        Json(ErrorBody::new(error, message, retryable)),
    )
        .into_response()
}

