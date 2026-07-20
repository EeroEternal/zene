use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};
use tempfile::tempdir;
use zene_gateway::agent::AgentManager;
use zene_gateway::auth::AuthState;
use zene_gateway::http::{self, AppState};
use zene_gateway::lease::LeaseManager;
use zene_gateway::poll_guard::PollGuard;
use zene_gateway::store::DataStore;

fn mock_acp_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_zene-gateway-mock-acp"))
}

async fn start_server_with(
    token: &str,
    store: Option<DataStore>,
    max_polls: usize,
) -> (SocketAddr, reqwest::Client, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let mut agents = AgentManager::new(mock_acp_bin(), Vec::new());
    if let Some(store) = store {
        agents = agents.with_store(store);
    }
    let state = AppState {
        auth: AuthState::new(token.to_string(), "127.0.0.1".into(), addr.port()),
        agents,
        leases: LeaseManager::new(),
        polls: PollGuard::new(max_polls),
        started_at: chrono::Utc::now(),
        version: "test",
    };
    let app = http::router(state);
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    let client = reqwest::Client::new();
    (addr, client, handle)
}

async fn start_server(token: &str) -> (SocketAddr, reqwest::Client) {
    let (addr, client, _handle) = start_server_with(token, None, 2).await;
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
async fn bootstrap_advertises_sse_and_lease() {
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
    assert_eq!(boot["transports"]["sse"], true);
    assert_eq!(boot["transports"]["websocket"], false);
    assert_eq!(boot["features"]["controllerLease"], true);
    assert_eq!(boot["features"]["terminalHost"], true);
    assert_eq!(boot["features"]["planPanel"], true);
    assert_eq!(boot["features"]["agentRestart"], true);
    assert_eq!(boot["limits"]["maxConcurrentPollsPerAgent"], 2);
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

#[tokio::test]
async fn sse_streams_acp_events() {
    let token = "secret";
    let (addr, client) = start_server(token).await;
    let dir = tempdir().unwrap();

    let created: Value = client
        .post(format!("http://{addr}/api/v1/agents"))
        .headers(auth_json(token))
        .json(&json!({
            "requestId": "sse-1",
            "workspace": dir.path()
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let agent_id = created["agent"]["agentId"].as_str().unwrap().to_string();

    let mut sse = client
        .get(format!(
            "http://{addr}/api/v1/agents/{agent_id}/events/stream?cursor=0&token={token}"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(
        sse.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
        "text/event-stream"
    );

    client
        .post(format!("http://{addr}/api/v1/agents/{agent_id}/messages"))
        .headers(auth_json(token))
        .json(&json!({
            "requestId": "sse-init",
            "messages": [{
                "jsonrpc": "2.0",
                "id": 7,
                "method": "initialize",
                "params": {
                    "protocolVersion": 1,
                    "clientCapabilities": {},
                    "clientInfo": { "name": "sse", "version": "0" }
                }
            }]
        }))
        .send()
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut buf = String::new();
    while tokio::time::Instant::now() < deadline {
        if let Some(chunk) = sse.chunk().await.unwrap() {
            buf.push_str(&String::from_utf8_lossy(&chunk));
            if buf.contains("\"id\":7") || buf.contains("\"id\": 7") {
                return;
            }
        }
    }
    panic!("SSE did not deliver initialize result; buf={buf}");
}

#[tokio::test]
async fn terminal_host_handles_acp_terminal_roundtrip() {
    let token = "secret";
    let (addr, client) = start_server(token).await;
    let dir = tempdir().unwrap();

    let created: Value = client
        .post(format!("http://{addr}/api/v1/agents"))
        .headers(auth_json(token))
        .json(&json!({
            "requestId": "term-create",
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

    client
        .post(format!("http://{addr}/api/v1/agents/{agent_id}/messages"))
        .headers(auth_json(token))
        .json(&json!({
            "requestId": "term-init",
            "messages": [{
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": 1,
                    "clientCapabilities": { "terminal": true },
                    "clientInfo": { "name": "term-test", "version": "0" }
                }
            }]
        }))
        .send()
        .await
        .unwrap();
    wait_for_payload(&client, addr, token, &agent_id, &mut cursor, |p| {
        p.get("id") == Some(&json!(1))
    })
    .await;

    client
        .post(format!("http://{addr}/api/v1/agents/{agent_id}/messages"))
        .headers(auth_json(token))
        .json(&json!({
            "requestId": "term-new",
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
        p.get("id") == Some(&json!(2))
    })
    .await;
    let session_id = session["result"]["sessionId"].as_str().unwrap().to_string();

    client
        .post(format!("http://{addr}/api/v1/agents/{agent_id}/messages"))
        .headers(auth_json(token))
        .json(&json!({
            "requestId": "term-prompt",
            "messages": [{
                "jsonrpc": "2.0",
                "id": 3,
                "method": "session/prompt",
                "params": {
                    "sessionId": session_id,
                    "prompt": [{ "type": "text", "text": "TERMINAL_PING" }]
                }
            }]
        }))
        .send()
        .await
        .unwrap();

    let created_event = wait_for_payload(&client, addr, token, &agent_id, &mut cursor, |p| {
        p.get("type") == Some(&json!("gateway.terminal")) && p.get("kind") == Some(&json!("created"))
    })
    .await;
    let terminal_id = created_event["terminalId"].as_str().unwrap().to_string();

    let assistant = wait_for_payload(&client, addr, token, &agent_id, &mut cursor, |p| {
        p.get("method") == Some(&json!("session/update"))
            && p["params"]["update"]["sessionUpdate"] == "agent_message_chunk"
            && p["params"]["update"]["content"]["text"]
                .as_str()
                .is_some_and(|t| t.contains("gateway-terminal-ok"))
    })
    .await;
    assert!(assistant["params"]["update"]["content"]["text"]
        .as_str()
        .unwrap()
        .contains("gateway-terminal-ok"));

    let listed: Value = client
        .get(format!("http://{addr}/api/v1/agents/{agent_id}/terminals"))
        .header("X-Zene-Token", token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(listed["terminals"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["terminalId"] == terminal_id));
}

#[tokio::test]
async fn controller_lease_blocks_other_client_writes() {
    let token = "secret";
    let (addr, client) = start_server(token).await;
    let dir = tempdir().unwrap();

    let created: Value = client
        .post(format!("http://{addr}/api/v1/agents"))
        .headers(auth_json(token))
        .json(&json!({
            "requestId": "lease-1",
            "workspace": dir.path()
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let agent_id = created["agent"]["agentId"].as_str().unwrap().to_string();

    let lease: Value = client
        .post(format!("http://{addr}/api/v1/agents/{agent_id}/lease"))
        .headers(auth_json(token))
        .json(&json!({ "clientId": "owner", "force": false }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(lease["lease"]["clientId"], "owner");

    let denied = client
        .post(format!("http://{addr}/api/v1/agents/{agent_id}/messages"))
        .headers(auth_json(token))
        .header("X-Zene-Client-Id", "other")
        .json(&json!({
            "requestId": "blocked",
            "messages": [{ "jsonrpc": "2.0", "method": "session/cancel", "params": {} }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), reqwest::StatusCode::CONFLICT);

    let allowed = client
        .post(format!("http://{addr}/api/v1/agents/{agent_id}/messages"))
        .headers(auth_json(token))
        .header("X-Zene-Client-Id", "owner")
        .json(&json!({
            "requestId": "ok-write",
            "messages": [{ "jsonrpc": "2.0", "method": "session/cancel", "params": {} }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(allowed.status(), reqwest::StatusCode::ACCEPTED);

    let stolen: Value = client
        .post(format!("http://{addr}/api/v1/agents/{agent_id}/lease"))
        .headers(auth_json(token))
        .json(&json!({ "clientId": "other", "force": true }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(stolen["lease"]["clientId"], "other");
}

#[tokio::test]
async fn persists_journal_and_supports_attach_after_gateway_restart() {
    let data = tempdir().unwrap();
    let store = DataStore::new(data.path().to_path_buf()).unwrap();
    let token = "secret";
    let (addr, client, handle) = start_server_with(token, Some(store.clone()), 2).await;
    let workspace = tempdir().unwrap();

    let created: Value = client
        .post(format!("http://{addr}/api/v1/agents"))
        .headers(auth_json(token))
        .json(&json!({
            "requestId": "persist-1",
            "workspace": workspace.path()
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let agent_id = created["agent"]["agentId"].as_str().unwrap().to_string();
    assert!(created["agent"]["journalPath"].as_str().is_some());

    let mut cursor = 0u64;
    let _ = wait_for_payload(&client, addr, token, &agent_id, &mut cursor, |p| {
        p["kind"] == "agent_started"
    })
    .await;

    let listed: Value = client
        .get(format!("http://{addr}/api/v1/agents"))
        .header("X-Zene-Token", token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed["persisted"].as_array().unwrap().len(), 1);
    assert_eq!(listed["persisted"][0]["agentId"], agent_id);

    // Simulate gateway process restart: stop first process, reopen same data dir.
    handle.abort();
    let (addr2, client2, _handle2) = start_server_with(token, Some(store), 2).await;
    let attached: Value = client2
        .post(format!("http://{addr2}/api/v1/agents/{agent_id}/attach"))
        .headers(auth_json(token))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(attached["agent"]["agentId"], agent_id);

    let events: Value = client2
        .get(format!(
            "http://{addr2}/api/v1/agents/{agent_id}/events?cursor=0&waitMs=0&limit=50"
        ))
        .header("X-Zene-Token", token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let kinds: Vec<&str> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["payload"]["kind"].as_str())
        .collect();
    assert!(kinds.contains(&"agent_started"));
    assert!(kinds.iter().any(|k| *k == "agent_attach"));
}

#[tokio::test]
async fn restart_keeps_journal_and_respawns_child() {
    let token = "secret";
    let (addr, client) = start_server(token).await;
    let workspace = tempdir().unwrap();

    let created: Value = client
        .post(format!("http://{addr}/api/v1/agents"))
        .headers(auth_json(token))
        .json(&json!({
            "requestId": "restart-1",
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
    let _ = wait_for_payload(&client, addr, token, &agent_id, &mut cursor, |p| {
        p["kind"] == "agent_started"
    })
    .await;

    let restarted: Value = client
        .post(format!("http://{addr}/api/v1/agents/{agent_id}/restart"))
        .headers(auth_json(token))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(restarted["agent"]["agentId"], agent_id);
    assert_eq!(restarted["agent"]["state"], "running");

    let payload = wait_for_payload(&client, addr, token, &agent_id, &mut cursor, |p| {
        p["kind"] == "agent_restarted" || p["kind"] == "agent_restarting"
    })
    .await;
    assert!(
        payload["kind"] == "agent_restarted" || payload["kind"] == "agent_restarting",
        "{payload}"
    );
}

#[tokio::test]
async fn rejects_too_many_concurrent_polls() {
    let token = "secret";
    let (addr, client, _handle) = start_server_with(token, None, 1).await;
    let workspace = tempdir().unwrap();

    let created: Value = client
        .post(format!("http://{addr}/api/v1/agents"))
        .headers(auth_json(token))
        .json(&json!({
            "requestId": "poll-1",
            "workspace": workspace.path()
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let agent_id = created["agent"]["agentId"].as_str().unwrap().to_string();

    // Drain existing events so the next long-poll actually waits and holds a permit.
    let mut cursor = 0u64;
    let _ = wait_for_payload(&client, addr, token, &agent_id, &mut cursor, |p| {
        p["kind"] == "agent_started"
    })
    .await;
    let drain: Value = client
        .get(format!(
            "http://{addr}/api/v1/agents/{agent_id}/events?cursor={cursor}&waitMs=0&limit=50"
        ))
        .header("X-Zene-Token", token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    cursor = drain["nextCursor"].as_u64().unwrap_or(cursor);

    let url = format!(
        "http://{addr}/api/v1/agents/{agent_id}/events?cursor={cursor}&waitMs=3000&limit=1"
    );
    let client_hang = client.clone();
    let token_hang = token.to_string();
    let hang = tokio::spawn(async move {
        client_hang
            .get(url)
            .header("X-Zene-Token", token_hang)
            .send()
            .await
    });
    tokio::time::sleep(Duration::from_millis(150)).await;

    let denied = client
        .get(format!(
            "http://{addr}/api/v1/agents/{agent_id}/events?cursor={cursor}&waitMs=0&limit=1"
        ))
        .header("X-Zene-Token", token)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);
    let body: Value = denied.json().await.unwrap();
    assert_eq!(body["error"], "too_many_polls");

    let _ = hang.await;
}

#[tokio::test]
async fn rejects_oversized_message_payload() {
    let token = "secret";
    let (addr, client) = start_server(token).await;
    let workspace = tempdir().unwrap();

    let created: Value = client
        .post(format!("http://{addr}/api/v1/agents"))
        .headers(auth_json(token))
        .json(&json!({
            "requestId": "big-1",
            "workspace": workspace.path()
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let agent_id = created["agent"]["agentId"].as_str().unwrap().to_string();

    let huge = "x".repeat(1_100_000);
    let denied = client
        .post(format!("http://{addr}/api/v1/agents/{agent_id}/messages"))
        .headers(auth_json(token))
        .json(&json!({
            "requestId": "big-msg",
            "messages": [{
                "jsonrpc": "2.0",
                "method": "session/prompt",
                "params": { "text": huge }
            }]
        }))
        .send()
        .await
        .unwrap();
    // Body limit (1 MiB) or per-message size check both reject oversized posts.
    assert!(
        denied.status() == reqwest::StatusCode::PAYLOAD_TOO_LARGE
            || denied.status() == reqwest::StatusCode::BAD_REQUEST,
        "unexpected status {}",
        denied.status()
    );
}
