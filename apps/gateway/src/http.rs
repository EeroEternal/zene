use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use crate::agent::AgentManager;
use crate::auth::{AuthState, ErrorBody};
use crate::event_journal::CursorExpired;
use crate::lease::{LeaseError, LeaseManager};
use crate::poll_guard::PollGuard;
use crate::static_page::INDEX_HTML;

const MAX_BODY_BYTES: usize = 1_048_576;

#[derive(Clone)]
pub struct AppState {
    pub auth: AuthState,
    pub agents: AgentManager,
    pub leases: LeaseManager,
    pub polls: PollGuard,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaseRequest {
    pub request_id: Option<String>,
    pub client_id: Option<String>,
    #[serde(default)]
    pub force: bool,
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
        .route("/api/v1/agents", get(list_agents).post(create_agent))
        .route("/api/v1/agents/{agent_id}/messages", post(post_messages))
        .route("/api/v1/agents/{agent_id}/events", get(get_events))
        .route("/api/v1/agents/{agent_id}/events/stream", get(stream_events))
        .route("/api/v1/agents/{agent_id}/health", get(agent_health))
        .route("/api/v1/agents/{agent_id}/restart", post(restart_agent))
        .route("/api/v1/agents/{agent_id}/attach", post(attach_agent))
        .route("/api/v1/agents/{agent_id}/lease", get(get_lease).post(acquire_lease))
        .route(
            "/api/v1/agents/{agent_id}/lease/heartbeat",
            post(heartbeat_lease),
        )
        .route("/api/v1/agents/{agent_id}/lease/release", post(release_lease))
        .route("/api/v1/agents/{agent_id}/terminals", get(list_terminals))
        .route(
            "/api/v1/agents/{agent_id}/terminals/{terminal_id}",
            get(get_terminal_output),
        )
        .route(
            "/api/v1/agents/{agent_id}/terminals/{terminal_id}/kill",
            post(kill_terminal),
        )
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(TraceLayer::new_for_http())
        .with_state(Arc::new(state))
}

async fn index() -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    (headers, Html(INDEX_HTML)).into_response()
}

async fn bootstrap(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(json!({
        "gatewayVersion": state.version,
        "apiVersion": "v1",
        "acpTransport": "stdio-ndjson",
        "transports": {
            "longPolling": true,
            "sse": true,
            "shortPolling": true,
            "websocket": false
        },
        "auth": {
            "header": "X-Zene-Token",
            "query": "token",
            "clientIdHeader": "X-Zene-Client-Id"
        },
        "features": {
            "controllerLease": true,
            "terminalHost": true,
            "planPanel": true,
            "todoPanel": true,
            "journalPersist": state.agents.store().is_some(),
            "agentRestart": true,
            "agentAttach": state.agents.store().is_some()
        },
        "limits": {
            "maxPostBodyBytes": MAX_BODY_BYTES,
            "maxMessagesPerPost": 100,
            "defaultWaitMs": 25_000,
            "maxWaitMs": 30_000,
            "maxEventsPerPoll": 200,
            "maxConcurrentPollsPerAgent": 2,
            "leaseTtlMs": 30_000
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

async fn list_agents(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = state.auth.authorize(&headers, None) {
        return auth_error(status);
    }
    let live = state.agents.list_live().await;
    match state.agents.list_persisted().await {
        Ok(persisted) => Json(json!({
            "agents": live,
            "persisted": persisted,
            "meta": meta(),
        }))
        .into_response(),
        Err(err) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "list_agents_failed",
            err.to_string(),
            true,
        ),
    }
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

    let client_id = client_id_from_headers(&headers);
    if let Err(err) = state
        .leases
        .authorize_write(&agent_id, client_id.as_deref())
        .await
    {
        return lease_error(err);
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
            let msg = err.to_string();
            if msg.contains("exceeds max size") {
                return json_error(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "message_too_large",
                    msg,
                    false,
                );
            }
            let retryable = msg.contains("not running");
            json_error(
                StatusCode::CONFLICT,
                "agent_not_running",
                msg,
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
    if let Err(status) = state.auth.authorize(&headers, query.token.as_deref()) {
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
    let Some(_permit) = state.polls.try_acquire(&agent_id).await else {
        return json_error(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_polls",
            "too many concurrent event consumers for this agent",
            true,
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
        }) => cursor_expired(oldest_cursor, latest_cursor),
    }
}

async fn stream_events(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Query(query): Query<EventsQuery>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = state.auth.authorize(&headers, query.token.as_deref()) {
        return auth_error(status);
    }
    if state.agents.get(&agent_id).await.is_none() {
        return json_error(
            StatusCode::NOT_FOUND,
            "agent_not_found",
            "unknown agentId",
            false,
        );
    }
    let Some(journal) = state.agents.journal(&agent_id).await else {
        return json_error(
            StatusCode::NOT_FOUND,
            "agent_not_found",
            "unknown agentId",
            false,
        );
    };
    let Some(permit) = state.polls.try_acquire(&agent_id).await else {
        return json_error(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_polls",
            "too many concurrent event consumers for this agent",
            true,
        );
    };

    let mut cursor = query.cursor.unwrap_or(0);
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(32);
    tokio::spawn(async move {
        let _permit = permit;
        loop {
            match journal
                .wait_for_events(cursor, 100, Duration::from_secs(20))
                .await
            {
                Ok((events, next_cursor, _)) => {
                    if events.is_empty() {
                        let event = Event::default().comment("keepalive");
                        if tx.send(Ok(event)).await.is_err() {
                            break;
                        }
                        continue;
                    }
                    for item in events {
                        cursor = item.cursor;
                        let data = match serde_json::to_string(&item) {
                            Ok(s) => s,
                            Err(_) => continue,
                        };
                        let event = Event::default()
                            .id(item.cursor.to_string())
                            .event("acp")
                            .data(data);
                        if tx.send(Ok(event)).await.is_err() {
                            return;
                        }
                    }
                    // Keep cursor aligned with journal nextCursor after a batch.
                    cursor = cursor.max(next_cursor);
                }
                Err(CursorExpired {
                    oldest_cursor,
                    latest_cursor,
                }) => {
                    let data = json!({
                        "error": "cursor_expired",
                        "oldestCursor": oldest_cursor,
                        "latestCursor": latest_cursor,
                        "recovery": "reload_session"
                    })
                    .to_string();
                    let _ = tx
                        .send(Ok(Event::default().event("cursor_expired").data(data)))
                        .await;
                    break;
                }
            }
        }
    });

    let stream = ReceiverStream::new(rx);
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
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
            "lease": state.leases.status(&agent_id).await,
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

async fn restart_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = state.auth.authorize(&headers, None) {
        return auth_error(status);
    }
    let client_id = client_id_from_headers(&headers);
    if let Err(err) = state
        .leases
        .authorize_write(&agent_id, client_id.as_deref())
        .await
    {
        return lease_error(err);
    }
    match state.agents.restart(&agent_id).await {
        Ok(agent) => Json(json!({
            "agent": agent,
            "meta": meta(),
        }))
        .into_response(),
        Err(err) => json_error(
            StatusCode::CONFLICT,
            "agent_restart_failed",
            err.to_string(),
            true,
        ),
    }
}

async fn attach_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = state.auth.authorize(&headers, None) {
        return auth_error(status);
    }
    match state.agents.attach(&agent_id).await {
        Ok(agent) => Json(json!({
            "agent": agent,
            "meta": meta(),
        }))
        .into_response(),
        Err(err) => json_error(
            StatusCode::BAD_REQUEST,
            "agent_attach_failed",
            err.to_string(),
            true,
        ),
    }
}

async fn get_lease(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = state.auth.authorize(&headers, None) {
        return auth_error(status);
    }
    if state.agents.get(&agent_id).await.is_none() {
        return json_error(
            StatusCode::NOT_FOUND,
            "agent_not_found",
            "unknown agentId",
            false,
        );
    }
    Json(json!({
        "lease": state.leases.status(&agent_id).await,
        "meta": meta(),
    }))
    .into_response()
}

async fn acquire_lease(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<LeaseRequest>,
) -> Response {
    if let Err(status) = state.auth.authorize(&headers, None) {
        return auth_error(status);
    }
    if state.agents.get(&agent_id).await.is_none() {
        return json_error(
            StatusCode::NOT_FOUND,
            "agent_not_found",
            "unknown agentId",
            false,
        );
    }
    let client_id = body
        .client_id
        .or_else(|| client_id_from_headers(&headers));
    match state.leases.acquire(&agent_id, client_id, body.force).await {
        Ok(lease) => Json(json!({ "lease": lease, "meta": meta() })).into_response(),
        Err(err) => lease_error(err),
    }
}

async fn heartbeat_lease(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<LeaseRequest>,
) -> Response {
    if let Err(status) = state.auth.authorize(&headers, None) {
        return auth_error(status);
    }
    let Some(client_id) = body
        .client_id
        .or_else(|| client_id_from_headers(&headers))
    else {
        return json_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "clientId is required",
            false,
        );
    };
    match state.leases.heartbeat(&agent_id, &client_id).await {
        Ok(lease) => Json(json!({ "lease": lease, "meta": meta() })).into_response(),
        Err(err) => lease_error(err),
    }
}

async fn release_lease(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<LeaseRequest>,
) -> Response {
    if let Err(status) = state.auth.authorize(&headers, None) {
        return auth_error(status);
    }
    let Some(client_id) = body
        .client_id
        .or_else(|| client_id_from_headers(&headers))
    else {
        return json_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "clientId is required",
            false,
        );
    };
    match state.leases.release(&agent_id, &client_id).await {
        Ok(()) => Json(json!({ "released": true, "meta": meta() })).into_response(),
        Err(err) => lease_error(err),
    }
}

fn client_id_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-zene-client-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
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

fn lease_error(err: LeaseError) -> Response {
    match err {
        LeaseError::HeldBy {
            client_id,
            expires_in_ms,
        } => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "lease_held",
                "message": format!("controller lease held by {client_id}"),
                "holderClientId": client_id,
                "expiresInMs": expires_in_ms,
                "retryable": true,
                "traceId": Uuid::new_v4().to_string(),
            })),
        )
            .into_response(),
        LeaseError::NotHeld => json_error(
            StatusCode::CONFLICT,
            "lease_not_held",
            "no active controller lease for this client",
            true,
        ),
    }
}

fn cursor_expired(oldest_cursor: u64, latest_cursor: u64) -> Response {
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

async fn list_terminals(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = state.auth.authorize(&headers, None) {
        return auth_error(status);
    }
    match state.agents.list_terminals(&agent_id).await {
        Some(terminals) => Json(json!({
            "terminals": terminals,
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

#[derive(Debug, Deserialize)]
struct TerminalOutputQuery {
    offset: Option<usize>,
    token: Option<String>,
}

async fn get_terminal_output(
    State(state): State<Arc<AppState>>,
    Path((agent_id, terminal_id)): Path<(String, String)>,
    Query(query): Query<TerminalOutputQuery>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = state.auth.authorize(&headers, query.token.as_deref()) {
        return auth_error(status);
    }
    match state
        .agents
        .terminal_output(&agent_id, &terminal_id, query.offset.unwrap_or(0))
        .await
    {
        Ok((chunk, next_offset, exit_code)) => Json(json!({
            "terminalId": terminal_id,
            "chunk": chunk,
            "nextOffset": next_offset,
            "exitCode": exit_code,
            "meta": meta(),
        }))
        .into_response(),
        Err(err) => json_error(
            StatusCode::NOT_FOUND,
            "terminal_not_found",
            err.to_string(),
            false,
        ),
    }
}

async fn kill_terminal(
    State(state): State<Arc<AppState>>,
    Path((agent_id, terminal_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = state.auth.authorize(&headers, None) {
        return auth_error(status);
    }
    let client_id = client_id_from_headers(&headers);
    if let Err(err) = state
        .leases
        .authorize_write(&agent_id, client_id.as_deref())
        .await
    {
        return lease_error(err);
    }
    match state.agents.kill_terminal(&agent_id, &terminal_id).await {
        Ok(()) => Json(json!({
            "killed": true,
            "terminalId": terminal_id,
            "meta": meta(),
        }))
        .into_response(),
        Err(err) => json_error(
            StatusCode::NOT_FOUND,
            "terminal_not_found",
            err.to_string(),
            true,
        ),
    }
}

fn json_error(
    status: StatusCode,
    error: &str,
    message: impl Into<String>,
    retryable: bool,
) -> Response {
    (status, Json(ErrorBody::new(error, message, retryable))).into_response()
}
