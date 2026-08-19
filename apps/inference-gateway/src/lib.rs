pub mod session_config;
pub mod session_http;
pub mod smartgate;

pub use session_config::SessionRuntimeConfig;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use axum::{
    body::Body,
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use session_config::{open_session_store, spawn_purge_task};
use session_http::session_routes;
use smartgate::{apply_smartgate_upstream_metadata, resolve_upstream_kind, UpstreamKind};
use tower_http::trace::TraceLayer;
use unigateway_sdk::core::{
    pool::ProviderPool, retry::LoadBalancingStrategy, Endpoint, EndpointCapabilities, ModelPolicy,
    ProviderKind, ProxyChatRequest, SecretString, UniGatewayEngine,
};
use unigateway_sdk::host::{
    dispatch_request_with_middleware, HostContext, HostDispatchOutcome, HostDispatchTarget,
    HostMiddleware, HostProtocol, HostRequest, PoolHost, PoolLookupOutcome, PoolLookupResult,
};
use unigateway_sdk::protocol::{
    openai_payload_to_chat_request, ProtocolHttpResponse, ProtocolResponseBody,
};
use unigateway_session::{DeltaAssemblyMiddleware, SessionHttpConfig, SessionKey};
use zene_llm::{
    BODY_ZENE_CONTEXT, HEADER_CONTEXT_DELIVERY, HEADER_CONTEXT_EPOCH, HEADER_PREFIX_HASH,
    HEADER_SESSION_ID, HEADER_TAIL_START, SESSION_GATEWAY_FIELD,
};

pub const DEFAULT_POOL_ID: &str = "zene-upstream";
pub const SESSION_NAMESPACE: &str = "zene";

#[derive(Clone)]
pub struct GatewayState {
    pub engine: Arc<UniGatewayEngine>,
    pub pool: ProviderPool,
    pub default_model: String,
    pub pool_host: Arc<StaticPoolHost>,
    pub middleware: Arc<HostMiddleware>,
    pub upstream_kind: UpstreamKind,
}

pub struct StaticPoolHost {
    pub pool: ProviderPool,
}

impl PoolHost for StaticPoolHost {
    fn pool_for_service<'a>(
        &'a self,
        _service_id: &'a str,
    ) -> unigateway_sdk::host::HostFuture<'a, PoolLookupResult<PoolLookupOutcome>> {
        let pool = self.pool.clone();
        Box::pin(async move { Ok(PoolLookupOutcome::found(pool)) })
    }
}

#[derive(Clone)]
pub struct GatewayOptions {
    pub upstream_url: String,
    pub upstream_api_key: String,
    pub default_model: String,
    pub session_config: SessionRuntimeConfig,
}

impl GatewayOptions {
    pub fn from_env() -> Self {
        Self {
            upstream_url: std::env::var("ZENE_UPSTREAM_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".into())
                .trim()
                .trim_end_matches('/')
                .to_string(),
            upstream_api_key: std::env::var("ZENE_UPSTREAM_API_KEY")
                .or_else(|_| std::env::var("OPENAI_API_KEY"))
                .unwrap_or_default(),
            default_model: std::env::var("ZENE_UPSTREAM_MODEL")
                .unwrap_or_else(|_| "gpt-4o-mini".to_string()),
            session_config: SessionRuntimeConfig::from_env(),
        }
    }
}

pub async fn build_gateway(options: GatewayOptions) -> anyhow::Result<Router> {
    let engine = UniGatewayEngine::builder()
        .with_builtin_http_drivers()
        .build()
        .context("build UniGatewayEngine")?;

    let upstream_kind = resolve_upstream_kind(&options.upstream_url).await;
    tracing::info!(?upstream_kind, upstream = %options.upstream_url, "resolved upstream kind");
    let forward_metadata_as_headers = match upstream_kind {
        UpstreamKind::SmartGate => Some(smartgate::smartgate_forward_headers()),
        UpstreamKind::Generic => None,
    };

    let endpoint = Endpoint {
        endpoint_id: "zene-upstream".to_string(),
        provider_name: Some("upstream".to_string()),
        source_endpoint_id: Some("upstream".to_string()),
        provider_family: Some("openai".to_string()),
        provider_kind: ProviderKind::OpenAiCompatible,
        driver_id: "openai-compatible".to_string(),
        base_url: options.upstream_url.clone(),
        api_key: SecretString::new(options.upstream_api_key),
        model_policy: ModelPolicy {
            default_model: Some(options.default_model.clone()),
            model_mapping: HashMap::new(),
        },
        enabled: true,
        max_concurrency: None,
        capabilities: EndpointCapabilities::default(),
        metadata: HashMap::new(),
        forward_metadata_as_headers: forward_metadata_as_headers.clone(),
    };

    let pool = ProviderPool {
        pool_id: DEFAULT_POOL_ID.to_string(),
        endpoints: vec![endpoint],
        load_balancing: LoadBalancingStrategy::RoundRobin,
        retry_policy: Default::default(),
        metadata: HashMap::new(),
        forward_metadata_as_headers,
    };

    engine.upsert_pool(pool.clone()).await?;

    let session_store = open_session_store(&options.session_config)?;
    if let Some(interval) = options.session_config.purge_interval {
        spawn_purge_task(session_store.clone(), interval);
    }

    let middleware_config = options
        .session_config
        .middleware_config(Arc::new(|_host, ctx| {
            SessionKey::new(SESSION_NAMESPACE, ctx.session_id.clone())
        }));
    let delta = Arc::new(DeltaAssemblyMiddleware::with_store(
        session_store.clone(),
        middleware_config,
    ));
    let middleware = Arc::new(HostMiddleware::new().with_request(delta));

    let state = GatewayState {
        engine: Arc::new(engine),
        pool_host: Arc::new(StaticPoolHost { pool: pool.clone() }),
        pool,
        default_model: options.default_model,
        middleware,
        upstream_kind,
    };

    let sessions = session_routes(
        session_store,
        SessionHttpConfig {
            path_prefix: "/v1/zene".to_string(),
            namespace: SESSION_NAMESPACE.to_string(),
        },
    );

    let chat = Router::new()
        .route("/v1/chat/completions", post(chat_completions))
        .route("/healthz", get(healthz))
        .with_state(state);

    Ok(Router::new()
        .merge(sessions)
        .merge(chat)
        .layer(TraceLayer::new_for_http()))
}

async fn healthz() -> impl IntoResponse {
    Json(json!({ "ok": true }))
}

pub async fn chat_completions(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    body: axum::Json<Value>,
) -> Result<Response, GatewayError> {
    let mut request =
        openai_payload_to_chat_request(&body, &state.default_model).map_err(GatewayError::parse)?;
    inject_session_context_from_headers(&headers, &mut request);
    if state.upstream_kind == UpstreamKind::SmartGate {
        apply_smartgate_upstream_metadata(&mut request);
    }

    let mut pool = state.pool.clone();
    apply_client_authorization(&headers, &mut pool);

    let context = HostContext::from_parts(&state.engine, state.pool_host.as_ref());
    let outcome = dispatch_request_with_middleware(
        &context,
        HostDispatchTarget::Pool(pool),
        HostProtocol::OpenAiChat,
        None,
        HostRequest::Chat(request),
        Some(state.middleware.as_ref()),
    )
    .await
    .map_err(GatewayError::host)?;

    let HostDispatchOutcome::Response(response) = outcome else {
        return Err(GatewayError::pool_not_found());
    };

    Ok(protocol_response_to_axum(response))
}

/// Forward BYOK bearer token from the client to the upstream pool for this request.
fn apply_client_authorization(headers: &HeaderMap, pool: &mut ProviderPool) {
    let Some(auth) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return;
    };
    let Some(token) = auth.strip_prefix("Bearer ").map(str::trim) else {
        return;
    };
    if token.is_empty() {
        return;
    }
    for endpoint in &mut pool.endpoints {
        endpoint.api_key = SecretString::new(token.to_string());
    }
}

pub fn inject_session_context_from_headers(headers: &HeaderMap, request: &mut ProxyChatRequest) {
    if request.gateway_fields.contains_key(SESSION_GATEWAY_FIELD)
        || request.gateway_fields.contains_key(BODY_ZENE_CONTEXT)
    {
        return;
    }
    let Some(session_id) = header_str(headers, HEADER_SESSION_ID) else {
        return;
    };
    let epoch = header_str(headers, HEADER_CONTEXT_EPOCH)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let delivery = header_str(headers, HEADER_CONTEXT_DELIVERY).unwrap_or_else(|| "full".into());
    let mut ctx = json!({
        "session_id": session_id,
        "epoch": epoch,
        "delivery": delivery,
    });
    if let Some(hash) = header_str(headers, HEADER_PREFIX_HASH) {
        ctx["prefix_hash"] = json!(hash);
        ctx["fingerprint"] = json!({
            "algorithm": "zene-v1",
            "value": hash,
        });
    }
    if let Some(tail_start) = header_str(headers, HEADER_TAIL_START) {
        if let Ok(n) = tail_start.parse::<u64>() {
            ctx["tail_start"] = json!(n);
        }
    }
    request
        .gateway_fields
        .insert(SESSION_GATEWAY_FIELD.to_string(), ctx);
}

fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

fn protocol_response_to_axum(response: ProtocolHttpResponse) -> Response {
    let (status, body) = response.into_parts();
    match body {
        ProtocolResponseBody::Json(value) => (status, axum::Json(value)).into_response(),
        ProtocolResponseBody::ServerSentEvents(stream) => Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(Body::from_stream(stream))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    }
}

#[derive(Debug)]
pub struct GatewayError {
    status: StatusCode,
    message: String,
}

impl GatewayError {
    fn parse(error: impl Into<anyhow::Error>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: error.into().to_string(),
        }
    }

    fn host(error: unigateway_sdk::host::HostError) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
        }
    }

    fn pool_not_found() -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "provider pool not found".to_string(),
        }
    }
}

impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        (
            self.status,
            axum::Json(json!({
                "error": { "message": self.message, "type": "gateway_error" }
            })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn injects_session_context_from_zene_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(HEADER_SESSION_ID, HeaderValue::from_static("run-1"));
        headers.insert(HEADER_CONTEXT_EPOCH, HeaderValue::from_static("2"));
        headers.insert(HEADER_CONTEXT_DELIVERY, HeaderValue::from_static("delta"));
        headers.insert(HEADER_TAIL_START, HeaderValue::from_static("5"));
        headers.insert(HEADER_PREFIX_HASH, HeaderValue::from_static("abc123"));
        let mut request = ProxyChatRequest {
            model: "gpt-4o-mini".to_string(),
            messages: vec![],
            temperature: None,
            top_p: None,
            top_k: None,
            max_tokens: None,
            stop_sequences: None,
            stream: false,
            system: None,
            tools: None,
            tool_choice: None,
            raw_messages: None,
            gateway_fields: HashMap::new(),
            extra: HashMap::new(),
            metadata: HashMap::new(),
        };
        inject_session_context_from_headers(&headers, &mut request);
        let ctx = request
            .gateway_fields
            .get(SESSION_GATEWAY_FIELD)
            .expect("session context");
        assert_eq!(ctx["session_id"], "run-1");
        assert_eq!(ctx["epoch"], 2);
        assert_eq!(ctx["delivery"], "delta");
        assert_eq!(ctx["tail_start"], 5);
        assert_eq!(ctx["fingerprint"]["algorithm"], "zene-v1");
        assert_eq!(ctx["fingerprint"]["value"], "abc123");
    }
}
