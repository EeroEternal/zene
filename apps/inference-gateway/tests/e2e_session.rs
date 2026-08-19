//! End-to-end: publish prefix → delta chat → upstream receives assembled messages.

use std::sync::{Arc, Mutex};

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    routing::post,
    Json, Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tower::{Service, ServiceExt};
use unigateway_session::FingerprintPolicy;
use zene_inference_gateway::{build_gateway, GatewayOptions, SessionRuntimeConfig};

type CapturedBody = Arc<Mutex<Option<Value>>>;

async fn mock_upstream_chat(
    State(captured): State<CapturedBody>,
    Json(body): Json<Value>,
) -> Json<Value> {
    *captured.lock().expect("lock") = Some(body);
    Json(json!({
        "id": "cmpl-test",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "ok" },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 2,
            "completion_tokens": 1,
            "total_tokens": 3
        }
    }))
}

#[tokio::test]
async fn publish_then_delta_chat_assembles_prefix_and_tail() {
    std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
    std::env::set_var("no_proxy", "127.0.0.1,localhost");

    let captured: CapturedBody = Arc::new(Mutex::new(None));
    let upstream = Router::new()
        .route("/v1/chat/completions", post(mock_upstream_chat))
        .with_state(captured.clone());

    let upstream_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind upstream");
    let upstream_addr = upstream_listener.local_addr().expect("upstream addr");
    tokio::spawn(async move {
        axum::serve(upstream_listener, upstream).await.ok();
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let probe = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("client")
        .post(format!("http://{upstream_addr}/v1/chat/completions"))
        .json(&json!({"model":"gpt-4o-mini","messages":[]}))
        .send()
        .await
        .expect("probe upstream");
    assert!(
        probe.status().is_success(),
        "mock upstream probe failed: {}",
        probe.status()
    );

    let session_config = SessionRuntimeConfig {
        using_redis: false,
        fingerprint_policy: FingerprintPolicy::Required,
        ..SessionRuntimeConfig::from_env()
    };
    let options = GatewayOptions {
        upstream_url: format!("http://{upstream_addr}/v1"),
        upstream_api_key: "sk-test".into(),
        default_model: "gpt-4o-mini".into(),
        session_config,
    };

    let mut app = build_gateway(options)
        .await
        .expect("build gateway")
        .into_service();

    let fingerprint = json!({ "algorithm": "zene-v1", "value": "deadbeef00000001" });

    let publish_resp = app
        .ready()
        .await
        .expect("service ready")
        .call(
            Request::builder()
                .method("POST")
                .uri("/v1/zene/sessions/run-e2e/publish")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "epoch": 1,
                        "message_count": 1,
                        "messages": [{ "role": "user", "content": "prefix" }],
                        "fingerprint": fingerprint,
                    })
                    .to_string(),
                ))
                .expect("publish request"),
        )
        .await
        .expect("publish response");
    assert_eq!(publish_resp.status(), StatusCode::NO_CONTENT);

    let chat_resp = app
        .ready()
        .await
        .expect("service ready")
        .call(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header("Authorization", "Bearer sk-test")
                .header("X-Zene-Session-Id", "run-e2e")
                .header("X-Zene-Context-Epoch", "1")
                .header("X-Zene-Context-Delivery", "delta")
                .header("X-Zene-Tail-Start", "1")
                .header("X-Zene-Prefix-Hash", "deadbeef00000001")
                .body(Body::from(
                    json!({
                        "model": "gpt-4o-mini",
                        "messages": [{ "role": "user", "content": "tail" }]
                    })
                    .to_string(),
                ))
                .expect("chat request"),
        )
        .await
        .expect("chat response");
    assert!(
        chat_resp.status().is_success(),
        "chat status {} body={}",
        chat_resp.status(),
        String::from_utf8_lossy(
            &chat_resp
                .into_body()
                .collect()
                .await
                .expect("read chat body")
                .to_bytes()
        )
    );

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let upstream_body = captured
        .lock()
        .expect("lock")
        .take()
        .expect("upstream should receive assembled request");
    let messages = upstream_body
        .get("messages")
        .and_then(Value::as_array)
        .expect("messages array");
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages[0].get("content").and_then(Value::as_str),
        Some("prefix")
    );
    assert_eq!(
        messages[1].get("content").and_then(Value::as_str),
        Some("tail")
    );
}
