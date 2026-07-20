use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};
use tempfile::tempdir;
use zene_gateway::agent::AgentManager;
use zene_gateway::auth::AuthState;
use zene_gateway::http::{self, AppState};

fn mock_acp_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_zene-gateway-mock-acp"))
}

async fn start_server(token: &str) -> (SocketAddr, reqwest::Client) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let agents = AgentManager::new(mock_acp_bin(), Vec::new());
    let state = AppState {
        auth: AuthState::new(token.to_string(), "127.0.0.1".into(), addr.port()),
        agents,
        started_at: chrono::Utc::now(),
        version: "test",
    };
    let app = http::router(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    let client = reqwest::Client::new();
    (addr, client)
}

fn auth_json(token: &str) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("X-Zene-Token", token.parse().unwrap());
    headers.insert("content-type", "application/json".parse().unwrap());
    headers
}

async fn wait_for_payload<F>(
    client: &reqwest::Client,
    addr: SocketAddr,
    token: &str,
    agent_id: &str,
    cursor: &mut u64,
    mut pred: F,
) -> Value
where
    F: FnMut(&Value) -> bool,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let url = format!(
            "http://{addr}/api/v1/agents/{agent_id}/events?cursor={cursor}&waitMs=500&limit=50"
        );
        let res = client
            .get(url)
            .header("X-Zene-Token", token)
            .send()
            .await
            .expect("events");
        assert!(res.status().is_success(), "events status {}", res.status());
        let body: Value = res.json().await.expect("events json");
        for event in body["events"].as_array().cloned().unwrap_or_default() {
            *cursor = event["cursor"].as_u64().unwrap_or(*cursor);
            let payload = event["payload"].clone();
            if pred(&payload) {
                return payload;
            }
        }
        if tokio::time::Instant::now() > deadline {
            panic!("timed out waiting for payload; cursor={cursor}");
        }
    }
}

#[tokio::test]
async fn bootstrap_and_health_are_public() {
    let token = "test-token";
    let (addr, client) = start_server(token).await;

    let boot: Value = client
        .get(format!("http://{addr}/api/v1/bootstrap"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(boot["apiVersion"], "v1");
    assert_eq!(boot["transports"]["longPolling"], true);
    assert_eq!(boot["transports"]["websocket"], false);

    let health: Value = client
        .get(format!("http://{addr}/api/v1/health"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["ok"], true);
}

#[tokio::test]
async fn rejects_missing_token_and_bad_origin() {
    let token = "secret";
    let (addr, client) = start_server(token).await;
    let dir = tempdir().unwrap();

    let res = client
        .post(format!("http://{addr}/api/v1/agents"))
        .header("content-type", "application/json")
        .body(
            json!({
                "requestId": "r1",
                "workspace": dir.path()
            })
            .to_string(),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::UNAUTHORIZED);

    let res = client
        .post(format!("http://{addr}/api/v1/agents"))
        .headers(auth_json(token))
        .header("Origin", "http://evil.example")
        .body(
            json!({
                "requestId": "r2",
                "workspace": dir.path()
            })
            .to_string(),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn prompt_stream_and_permission_roundtrip() {
    let token = "secret";
    let (addr, client) = start_server(token).await;
    let dir = tempdir().unwrap();

    let created: Value = client
        .post(format!("http://{addr}/api/v1/agents"))
        .headers(auth_json(token))
        .json(&json!({
            "requestId": "create-1",
            "workspace": dir.path()
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let agent_id = created["agent"]["agentId"].as_str().unwrap().to_string();
    let mut cursor = 0u64;

    // idempotent recreate
    let created2: Value = client
        .post(format!("http://{addr}/api/v1/agents"))
        .headers(auth_json(token))
        .json(&json!({
            "requestId": "create-1",
            "workspace": dir.path()
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(created2["agent"]["agentId"], created["agent"]["agentId"]);

    let accepted = client
        .post(format!("http://{addr}/api/v1/agents/{agent_id}/messages"))
        .headers(auth_json(token))
        .json(&json!({
            "requestId": "m-init",
            "messages": [{
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": 1,
                    "clientCapabilities": {},
                    "clientInfo": { "name": "test", "version": "0" }
                }
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(accepted.status(), reqwest::StatusCode::ACCEPTED);

    let init = wait_for_payload(&client, addr, token, &agent_id, &mut cursor, |p| {
        p.get("id") == Some(&json!(1)) && p.get("result").is_some()
    })
    .await;
    assert_eq!(init["result"]["agentInfo"]["name"], "mock-acp");

    client
        .post(format!("http://{addr}/api/v1/agents/{agent_id}/messages"))
        .headers(auth_json(token))
        .json(&json!({
            "requestId": "m-new",
            "messages": [{
                "jsonrpc": "2.0",
                "id": 2,
                "method": "session/new",
                "params": { "cwd": dir.path(), "mcpServers": [] }
            }]
        }))
        .send()
        .await
        .unwrap();

    let session = wait_for_payload(&client, addr, token, &agent_id, &mut cursor, |p| {
        p.get("id") == Some(&json!(2)) && p.get("result").is_some()
    })
    .await;
    let session_id = session["result"]["sessionId"].as_str().unwrap().to_string();

    client
        .post(format!("http://{addr}/api/v1/agents/{agent_id}/messages"))
        .headers(auth_json(token))
        .json(&json!({
            "requestId": "m-prompt",
            "messages": [{
                "jsonrpc": "2.0",
                "id": 3,
                "method": "session/prompt",
                "params": {
                    "sessionId": session_id,
                    "prompt": [{ "type": "text", "text": "NEED_PERMISSION hello" }]
                }
            }]
        }))
        .send()
        .await
        .unwrap();

    let permission = wait_for_payload(&client, addr, token, &agent_id, &mut cursor, |p| {
        p.get("method") == Some(&json!("session/request_permission"))
    })
    .await;
    assert_eq!(permission["id"], 9001);

    client
        .post(format!("http://{addr}/api/v1/agents/{agent_id}/messages"))
        .headers(auth_json(token))
        .json(&json!({
            "requestId": "m-perm",
            "messages": [{
                "jsonrpc": "2.0",
                "id": 9001,
                "result": {
                    "outcome": { "outcome": "selected", "optionId": "allow-once" }
                }
            }]
        }))
        .send()
        .await
        .unwrap();

    let assistant = wait_for_payload(&client, addr, token, &agent_id, &mut cursor, |p| {
        p.get("method") == Some(&json!("session/update"))
            && p["params"]["update"]["sessionUpdate"] == "agent_message_chunk"
    })
    .await;
    assert!(assistant["params"]["update"]["content"]["text"]
        .as_str()
        .unwrap()
        .contains("NEED_PERMISSION hello"));

    let prompt_done = wait_for_payload(&client, addr, token, &agent_id, &mut cursor, |p| {
        p.get("id") == Some(&json!(3)) && p.get("result").is_some()
    })
    .await;
    assert_eq!(prompt_done["result"]["stopReason"], "end_turn");
}

#[tokio::test]
async fn long_poll_waits_then_returns_event() {
    let token = "secret";
    let (addr, client) = start_server(token).await;
    let dir = tempdir().unwrap();

    let created: Value = client
        .post(format!("http://{addr}/api/v1/agents"))
        .headers(auth_json(token))
        .json(&json!({
            "requestId": "c-wait",
            "workspace": dir.path()
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let agent_id = created["agent"]["agentId"].as_str().unwrap().to_string();

    // Drain the agent_started system event first.
    let mut cursor = 0u64;
    let _ = wait_for_payload(&client, addr, token, &agent_id, &mut cursor, |p| {
        p.get("type") == Some(&json!("gateway.system"))
    })
    .await;

    let poll = tokio::spawn({
        let client = client.clone();
        let token = token.to_string();
        let agent_id = agent_id.clone();
        async move {
            let url = format!(
                "http://{addr}/api/v1/agents/{agent_id}/events?cursor={cursor}&waitMs=2000&limit=20"
            );
            let started = std::time::Instant::now();
            let res = client
                .get(url)
                .header("X-Zene-Token", token)
                .send()
                .await
                .unwrap();
            let body: Value = res.json().await.unwrap();
            (started.elapsed(), body)
        }
    });

    tokio::time::sleep(Duration::from_millis(200)).await;
    client
        .post(format!("http://{addr}/api/v1/agents/{agent_id}/messages"))
        .headers(auth_json(token))
        .json(&json!({
            "requestId": "late-init",
            "messages": [{
                "jsonrpc": "2.0",
                "id": 42,
                "method": "initialize",
                "params": {
                    "protocolVersion": 1,
                    "clientCapabilities": {},
                    "clientInfo": { "name": "test", "version": "0" }
                }
            }]
        }))
        .send()
        .await
        .unwrap();

    let (elapsed, body) = poll.await.unwrap();
    assert!(elapsed >= Duration::from_millis(150));
    assert!(elapsed < Duration::from_secs(2));
    let events = body["events"].as_array().unwrap();
    assert!(events.iter().any(|e| e["payload"]["id"] == 42));
}
