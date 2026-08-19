//! Map Zene session context to SmartGate upstream headers/body hints.

use std::time::Duration;

use anyhow::Context;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::{debug, info, warn};
use unigateway_sdk::core::ProxyChatRequest;
use zene_llm::{
    sanitize_smartgate_session_id, BODY_ZENE_CONTEXT, HEADER_CONTEXT_DELIVERY,
    HEADER_CONTEXT_EPOCH, HEADER_PREFIX_HASH, HEADER_SESSION_ID, SESSION_GATEWAY_FIELD,
    SMARTGATE_HEADER_CONTEXT_DELIVERY, SMARTGATE_HEADER_CONTEXT_EPOCH,
    SMARTGATE_HEADER_PREFIX_HASH, SMARTGATE_HEADER_SESSION_ID,
};

const ENV_UPSTREAM_KIND: &str = "ZENE_UPSTREAM_KIND";
const CAPABILITIES_PATH: &str = "/zene/capabilities";
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Upstream LLM provider behind this inference gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamKind {
    Generic,
    SmartGate,
}

#[derive(Debug, Clone, Deserialize)]
struct SmartGateCapabilities {
    gateway: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    features: Vec<String>,
}

/// Resolve upstream kind: env override → capabilities probe → URL heuristic.
pub async fn resolve_upstream_kind(upstream_url: &str) -> UpstreamKind {
    if let Some(kind) = upstream_kind_from_env_override() {
        info!(?kind, "upstream kind from ZENE_UPSTREAM_KIND");
        return kind;
    }

    match probe_smartgate_capabilities(upstream_url).await {
        Ok(true) => {
            info!(
                upstream_url,
                "upstream SmartGate detected via capabilities probe"
            );
            UpstreamKind::SmartGate
        }
        Ok(false) => {
            let kind = upstream_kind_from_url_heuristic(upstream_url);
            debug!(
                upstream_url,
                ?kind,
                "capabilities probe did not identify SmartGate; using URL heuristic fallback"
            );
            kind
        }
        Err(err) => {
            let kind = upstream_kind_from_url_heuristic(upstream_url);
            warn!(
                upstream_url,
                ?kind,
                error = %err,
                "capabilities probe failed; using URL heuristic fallback"
            );
            kind
        }
    }
}

fn upstream_kind_from_env_override() -> Option<UpstreamKind> {
    match std::env::var(ENV_UPSTREAM_KIND)
        .ok()
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("smartgate") => Some(UpstreamKind::SmartGate),
        Some("generic") | Some("openai") => Some(UpstreamKind::Generic),
        _ => None,
    }
}

fn upstream_kind_from_url_heuristic(url: &str) -> UpstreamKind {
    let lower = url.to_ascii_lowercase();
    if lower.contains("smartgate") || lower.contains("xgate") {
        UpstreamKind::SmartGate
    } else {
        UpstreamKind::Generic
    }
}

pub fn capabilities_url(upstream_url: &str) -> String {
    format!(
        "{}{CAPABILITIES_PATH}",
        upstream_url.trim().trim_end_matches('/')
    )
}

fn is_smartgate_capabilities(body: &SmartGateCapabilities) -> bool {
    body.gateway.eq_ignore_ascii_case("smartgate")
}

async fn probe_smartgate_capabilities(upstream_url: &str) -> anyhow::Result<bool> {
    let url = capabilities_url(upstream_url);
    let client = Client::builder()
        .timeout(PROBE_TIMEOUT)
        .build()
        .context("build capabilities probe client")?;
    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    if !resp.status().is_success() {
        anyhow::bail!("capabilities probe returned {}", resp.status());
    }
    let body: SmartGateCapabilities = resp
        .json()
        .await
        .with_context(|| format!("parse capabilities JSON from {url}"))?;
    if is_smartgate_capabilities(&body) {
        info!(
            url,
            version = body.version.as_deref().unwrap_or("unknown"),
            features = ?body.features,
            "SmartGate capabilities probe succeeded"
        );
        Ok(true)
    } else {
        debug!(
            url,
            gateway = %body.gateway,
            "capabilities probe returned non-SmartGate gateway"
        );
        Ok(false)
    }
}

pub fn smartgate_forward_headers() -> Vec<String> {
    vec![
        SMARTGATE_HEADER_SESSION_ID.to_string(),
        SMARTGATE_HEADER_CONTEXT_EPOCH.to_string(),
        SMARTGATE_HEADER_CONTEXT_DELIVERY.to_string(),
        SMARTGATE_HEADER_PREFIX_HASH.to_string(),
    ]
}

/// Translate Zene `_session_context` / `X-Zene-*` into SmartGate upstream metadata.
pub fn apply_smartgate_upstream_metadata(request: &mut ProxyChatRequest) {
    let Some(ctx) = session_context_value(request) else {
        return;
    };
    let Some(session_id_raw) = ctx.get("session_id").and_then(Value::as_str) else {
        return;
    };
    let session_id = sanitize_smartgate_session_id(session_id_raw);
    let epoch = ctx.get("epoch").and_then(Value::as_u64).unwrap_or(0);
    let delivery = ctx
        .get("delivery")
        .and_then(Value::as_str)
        .unwrap_or("full");

    request
        .metadata
        .insert(SMARTGATE_HEADER_SESSION_ID.to_string(), session_id.clone());
    request.metadata.insert(
        SMARTGATE_HEADER_CONTEXT_EPOCH.to_string(),
        epoch.to_string(),
    );
    request.metadata.insert(
        SMARTGATE_HEADER_CONTEXT_DELIVERY.to_string(),
        delivery.to_string(),
    );
    if let Some(hash) = ctx.get("prefix_hash").and_then(Value::as_str) {
        request
            .metadata
            .insert(SMARTGATE_HEADER_PREFIX_HASH.to_string(), hash.to_string());
    }
    // OpenAI-compatible fallback when an intermediary strips custom headers.
    request
        .extra
        .insert("user".to_string(), json!(session_id.clone()));

    // Strip Zene-only metadata so generic upstream drivers do not leak internal headers.
    for key in [
        HEADER_SESSION_ID,
        HEADER_CONTEXT_EPOCH,
        HEADER_CONTEXT_DELIVERY,
        HEADER_PREFIX_HASH,
    ] {
        request.metadata.remove(key);
    }
}

fn session_context_value(request: &ProxyChatRequest) -> Option<Value> {
    if let Some(ctx) = request.gateway_fields.get(SESSION_GATEWAY_FIELD) {
        return Some(ctx.clone());
    }
    if let Some(ctx) = request.gateway_fields.get(BODY_ZENE_CONTEXT) {
        return Some(ctx.clone());
    }
    let session_id = request.metadata.get(HEADER_SESSION_ID)?;
    let epoch = request
        .metadata
        .get(HEADER_CONTEXT_EPOCH)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let delivery = request
        .metadata
        .get(HEADER_CONTEXT_DELIVERY)
        .cloned()
        .unwrap_or_else(|| "full".to_string());
    let mut ctx = json!({
        "session_id": session_id,
        "epoch": epoch,
        "delivery": delivery,
    });
    if let Some(hash) = request.metadata.get(HEADER_PREFIX_HASH) {
        ctx["prefix_hash"] = json!(hash);
    }
    Some(ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn maps_session_context_to_smartgate_metadata() {
        let mut request = ProxyChatRequest {
            model: "fusion".into(),
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
            gateway_fields: HashMap::from([(
                SESSION_GATEWAY_FIELD.to_string(),
                json!({
                    "session_id": "run-abc!",
                    "epoch": 3,
                    "delivery": "full",
                    "prefix_hash": "a1b2c3d4e5f67890",
                }),
            )]),
            extra: HashMap::new(),
            metadata: HashMap::new(),
        };
        apply_smartgate_upstream_metadata(&mut request);
        assert_eq!(
            request.metadata.get(SMARTGATE_HEADER_SESSION_ID),
            Some(&"run-abc".to_string())
        );
        assert_eq!(
            request.metadata.get(SMARTGATE_HEADER_CONTEXT_EPOCH),
            Some(&"3".to_string())
        );
        assert_eq!(
            request.extra.get("user").and_then(Value::as_str),
            Some("run-abc")
        );
    }

    #[test]
    fn capabilities_url_appends_path_to_upstream_base() {
        assert_eq!(
            capabilities_url("https://api.xgate.sh/v1"),
            "https://api.xgate.sh/v1/zene/capabilities"
        );
    }

    #[test]
    fn recognizes_smartgate_capabilities_payload() {
        let body: SmartGateCapabilities = serde_json::from_value(json!({
            "gateway": "smartgate",
            "version": "0.2.0",
            "features": ["warming", "session_id"]
        }))
        .expect("capabilities");
        assert!(is_smartgate_capabilities(&body));
    }

    #[test]
    fn url_heuristic_detects_xgate() {
        assert_eq!(
            upstream_kind_from_url_heuristic("https://api.xgate.sh/v1"),
            UpstreamKind::SmartGate
        );
        assert_eq!(
            upstream_kind_from_url_heuristic("https://api.openai.com/v1"),
            UpstreamKind::Generic
        );
    }

    #[tokio::test]
    async fn probe_detects_smartgate_from_capabilities_endpoint() {
        use axum::{routing::get, Json, Router};
        use tokio::net::TcpListener;

        let app = Router::new().route(
            "/v1/zene/capabilities",
            get(|| async {
                Json(json!({
                    "gateway": "smartgate",
                    "version": "0.2.0",
                    "features": ["warming", "session_id"]
                }))
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });

        let upstream = format!("http://{addr}/v1");
        unsafe { std::env::remove_var(ENV_UPSTREAM_KIND) };
        let kind = resolve_upstream_kind(&upstream).await;
        assert_eq!(kind, UpstreamKind::SmartGate);
    }
}
