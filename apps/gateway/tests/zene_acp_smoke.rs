//! End-to-end smoke: gateway ↔ real `zene acp` ↔ mock OpenAI-compatible LLM.
//!
//! Builds `zene` on demand when `target/debug/zene` is missing.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};
use tempfile::tempdir;
use zene_gateway::agent::AgentManager;
use zene_gateway::auth::AuthState;
use zene_gateway::http::{self, AppState};
use zene_gateway::lease::LeaseManager;

#[derive(Clone)]
struct MockLlmState {
    turns: Arc<AtomicUsize>,
}

async fn chat_completions(
    State(state): State<MockLlmState>,
    Json(body): Json<Value>,
) -> Response {
    let stream = body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let messages = body
        .get("messages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let has_tool = messages.iter().any(|m| m.get("role") == Some(&json!("tool")));
    let turn = state.turns.fetch_add(1, Ordering::SeqCst);

    if stream {
        let payload = if !has_tool && turn == 0 {
            json!({
                "id": "chatcmpl-mock-1",
                "object": "chat.completion.chunk",
                "model": "mock-model",
                "choices": [{
                    "index": 0,
                    "delta": {
                        "role": "assistant",
                        "tool_calls": [{
                            "index": 0,
                            "id": "call_1",
                            "type": "function",
                            "function": {
                                "name": "Write",
                                "arguments": "{\"path\":\"hello_zene.txt\",\"content\":\"Hello from gateway smoke\\n\"}"
                            }
                        }]
                    },
                    "finish_reason": null
                }]
            })
        } else {
            json!({
                "id": "chatcmpl-mock-2",
                "object": "chat.completion.chunk",
                "model": "mock-model",
                "choices": [{
                    "index": 0,
                    "delta": {
                        "role": "assistant",
                        "content": "Done. Created hello_zene.txt."
                    },
                    "finish_reason": null
                }]
            })
        };
        let finish = json!({
            "id": "chatcmpl-mock-fin",
            "object": "chat.completion.chunk",
            "model": "mock-model",
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": if !has_tool && turn == 0 { "tool_calls" } else { "stop" }
            }],
            "usage": {
                "prompt_tokens": 20,
                "completion_tokens": 10,
                "total_tokens": 30
            }
        });
        let body = format!("data: {payload}\n\ndata: {finish}\n\ndata: [DONE]\n\n");
        (
            [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
            body,
        )
            .into_response()
    } else if !has_tool {
        Json(json!({
            "id": "chatcmpl-mock-1",
            "object": "chat.completion",
            "model": "mock-model",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "Write",
                            "arguments": "{\"path\":\"hello_zene.txt\",\"content\":\"Hello from gateway smoke\\n\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": { "prompt_tokens": 20, "completion_tokens": 10, "total_tokens": 30 }
        }))
        .into_response()
    } else {
        Json(json!({
            "id": "chatcmpl-mock-2",
            "object": "chat.completion",
            "model": "mock-model",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Done. Created hello_zene.txt."
                },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 40, "completion_tokens": 12, "total_tokens": 52 }
        }))
        .into_response()
    }
}

async fn start_mock_llm() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("llm bind");
    let port = listener.local_addr().unwrap().port();
    let app = Router::new()
        .route("/v1/chat/completions", post(chat_completions))
        .route("/chat/completions", post(chat_completions))
        .with_state(MockLlmState {
            turns: Arc::new(AtomicUsize::new(0)),
        });
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("llm serve");
    });
    port
}

fn ensure_zene_bin() -> PathBuf {
    if let Ok(path) = std::env::var("ZENE_BIN") {
        let path = PathBuf::from(path);
        assert!(path.exists(), "ZENE_BIN does not exist: {}", path.display());
        return path;
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidate = manifest.join("../../target/debug/zene");
    if candidate.exists() {
        return candidate.canonicalize().unwrap();
    }
    let status = Command::new("cargo")
        .args(["build", "-p", "zene-cli", "--bin", "zene"])
        .current_dir(manifest.join("../.."))
        .status()
        .expect("spawn cargo build zene");
    assert!(status.success(), "cargo build -p zene-cli failed");
    candidate
        .canonicalize()
        .expect("zene binary missing after build")
}

async fn wait_for_payload<F>(
    client: &reqwest::Client,
    addr: SocketAddr,
    token: &str,
    agent_id: &str,
    cursor: &mut u64,
    timeout: Duration,
    mut pred: F,
) -> Value
where
    F: FnMut(&Value) -> bool,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let url = format!(
            "http://{addr}/api/v1/agents/{agent_id}/events?cursor={cursor}&waitMs=1000&limit=100"
        );
        let res = client
            .get(&url)
            .header("X-Zene-Token", token)
            .send()
            .await
            .expect("events");
        let body: Value = res.json().await.expect("events json");
        for event in body["events"].as_array().cloned().unwrap_or_default() {
            *cursor = event["cursor"].as_u64().unwrap_or(*cursor);
            let payload = event["payload"].clone();
            if pred(&payload) {
                return payload;
            }
        }
        if tokio::time::Instant::now() > deadline {
            panic!("timed out waiting for payload; last cursor={cursor}");
        }
    }
}

#[tokio::test]
async fn gateway_with_real_zene_acp_writes_file() {
    let zene = ensure_zene_bin();
    let llm_port = start_mock_llm().await;
    let base_url = format!("http://127.0.0.1:{llm_port}/v1");

    let workspace = tempdir().unwrap();
    // Isolate session/config from the developer home.
    let home = tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".zene")).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let token = "smoke-token";
    let agents = AgentManager::new(
        zene,
        vec![
            "--yolo".into(),
            "--sandbox".into(),
            "off".into(),
            "acp".into(),
        ],
    )
    .with_env(vec![
        ("HOME".into(), home.path().display().to_string()),
        ("ZENE_API_KEY".into(), "mock-key".into()),
        ("OPENAI_API_KEY".into(), "mock-key".into()),
        ("ZENE_BASE_URL".into(), base_url),
        ("ZENE_MODEL".into(), "mock-model".into()),
        ("ZENE_PROVIDER".into(), "openai".into()),
        ("ZENE_SANDBOX".into(), "off".into()),
    ]);
    let state = AppState {
        auth: AuthState::new(token.into(), "127.0.0.1".into(), addr.port()),
        agents,
        leases: LeaseManager::new(),
        polls: zene_gateway::poll_guard::PollGuard::new(2),
        started_at: chrono::Utc::now(),
        version: "test",
    };
    let app = http::router(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::new();

    let created: Value = client
        .post(format!("http://{addr}/api/v1/agents"))
        .header("X-Zene-Token", token)
        .json(&json!({
            "requestId": "smoke-create",
            "workspace": workspace.path()
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let agent_id = created["agent"]["agentId"].as_str().unwrap().to_string();
    let mut cursor = 0u64;

    client
        .post(format!("http://{addr}/api/v1/agents/{agent_id}/messages"))
        .header("X-Zene-Token", token)
        .json(&json!({
            "requestId": "smoke-init",
            "messages": [{
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": 1,
                    "clientCapabilities": {},
                    "clientInfo": { "name": "smoke", "version": "0" }
                }
            }]
        }))
        .send()
        .await
        .unwrap();
    wait_for_payload(
        &client,
        addr,
        token,
        &agent_id,
        &mut cursor,
        Duration::from_secs(20),
        |p| p.get("id") == Some(&json!(1)) && p.get("result").is_some(),
    )
    .await;

    client
        .post(format!("http://{addr}/api/v1/agents/{agent_id}/messages"))
        .header("X-Zene-Token", token)
        .json(&json!({
            "requestId": "smoke-new",
            "messages": [{
                "jsonrpc": "2.0",
                "id": 2,
                "method": "session/new",
                "params": {
                    "cwd": workspace.path(),
                    "mcpServers": []
                }
            }]
        }))
        .send()
        .await
        .unwrap();
    let session = wait_for_payload(
        &client,
        addr,
        token,
        &agent_id,
        &mut cursor,
        Duration::from_secs(20),
        |p| p.get("id") == Some(&json!(2)) && p.get("result").is_some(),
    )
    .await;
    let session_id = session["result"]["sessionId"].as_str().unwrap().to_string();

    client
        .post(format!("http://{addr}/api/v1/agents/{agent_id}/messages"))
        .header("X-Zene-Token", token)
        .json(&json!({
            "requestId": "smoke-prompt",
            "messages": [{
                "jsonrpc": "2.0",
                "id": 3,
                "method": "session/prompt",
                "params": {
                    "sessionId": session_id,
                    "prompt": [{ "type": "text", "text": "Create hello_zene.txt" }]
                }
            }]
        }))
        .send()
        .await
        .unwrap();

    wait_for_payload(
        &client,
        addr,
        token,
        &agent_id,
        &mut cursor,
        Duration::from_secs(60),
        |p| {
            p.get("method") == Some(&json!("session/update"))
                && p["params"]["update"]["sessionUpdate"] == "tool_call"
                && p["params"]["update"]["title"]
                    .as_str()
                    .or_else(|| p["params"]["update"]["kind"].as_str())
                    .is_some()
        },
    )
    .await;

    wait_for_payload(
        &client,
        addr,
        token,
        &agent_id,
        &mut cursor,
        Duration::from_secs(60),
        |p| p.get("id") == Some(&json!(3)) && p.get("result").is_some(),
    )
    .await;

    let written = workspace.path().join("hello_zene.txt");
    assert_file_eventually(&written, Duration::from_secs(10));
    let content = std::fs::read_to_string(&written).unwrap();
    assert!(content.contains("Hello from gateway smoke"));
}

fn assert_file_eventually(path: &Path, timeout: Duration) {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if path.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("expected file {} was not created", path.display());
}
