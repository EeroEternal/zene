//! Minimal ACP stdio peer used by gateway integration tests.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};

use serde_json::{json, Value};

fn main() {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut stdout = io::stdout();
    let session_id = String::from("mock-session-1");
    let mut pending_permission: HashMap<String, bool> = HashMap::new();
    let mut line = String::new();

    loop {
        line.clear();
        let n = match input.read_line(&mut line) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        if n == 0 || line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };

        // Client JSON-RPC response to our permission request.
        if msg.get("method").is_none() && msg.get("id").is_some() {
            if let Some(id) = msg.get("id") {
                pending_permission.insert(id_to_key(id), true);
            }
            continue;
        }

        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let id = msg.get("id").cloned();
        let params = msg.get("params").cloned().unwrap_or(json!({}));

        match method {
            "initialize" => {
                write_msg(
                    &mut stdout,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "protocolVersion": 1,
                            "agentCapabilities": {
                                "loadSession": true,
                                "promptCapabilities": {
                                    "image": false,
                                    "audio": false,
                                    "embeddedContext": true
                                }
                            },
                            "agentInfo": { "name": "mock-acp", "version": "0.0.1" }
                        }
                    }),
                );
            }
            "session/new" => {
                write_msg(
                    &mut stdout,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "sessionId": session_id,
                            "modes": {
                                "currentModeId": "default",
                                "availableModes": [
                                    { "id": "default", "name": "Default" },
                                    { "id": "plan", "name": "Plan" }
                                ]
                            }
                        }
                    }),
                );
            }
            "session/prompt" => {
                handle_prompt(
                    &mut input,
                    &mut stdout,
                    &session_id,
                    &mut pending_permission,
                    id,
                    &params,
                    &mut line,
                );
            }
            "session/cancel" => {}
            _ => {
                if let Some(id) = id {
                    write_msg(
                        &mut stdout,
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": {
                                "code": -32601,
                                "message": format!("Method not found: {method}")
                            }
                        }),
                    );
                }
            }
        }
    }
}

fn handle_prompt(
    input: &mut impl BufRead,
    stdout: &mut impl Write,
    session_id: &str,
    pending_permission: &mut HashMap<String, bool>,
    id: Option<Value>,
    params: &Value,
    line: &mut String,
) {
    let prompt_text = extract_prompt_text(params);
    let want_permission = prompt_text.contains("NEED_PERMISSION");

    write_msg(
        stdout,
        json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": session_id,
                "update": {
                    "sessionUpdate": "agent_thought_chunk",
                    "content": { "type": "text", "text": "thinking…" }
                }
            }
        }),
    );

    if want_permission {
        let perm_id = 9001;
        write_msg(
            stdout,
            json!({
                "jsonrpc": "2.0",
                "id": perm_id,
                "method": "session/request_permission",
                "params": {
                    "sessionId": session_id,
                    "toolCall": {
                        "toolCallId": "call_mock_1",
                        "title": "Bash",
                        "kind": "execute",
                        "status": "pending"
                    },
                    "options": [
                        {
                            "optionId": "allow-once",
                            "name": "Allow once",
                            "kind": "allow_once"
                        },
                        {
                            "optionId": "reject",
                            "name": "Reject",
                            "kind": "reject_once"
                        }
                    ]
                }
            }),
        );

        let key = perm_id.to_string();
        while !pending_permission.contains_key(&key) {
            line.clear();
            let n = match input.read_line(line) {
                Ok(0) => return,
                Ok(n) => n,
                Err(_) => return,
            };
            if n == 0 || line.trim().is_empty() {
                continue;
            }
            if let Ok(resp) = serde_json::from_str::<Value>(line.trim()) {
                if resp.get("method").is_none() {
                    if let Some(rid) = resp.get("id") {
                        pending_permission.insert(id_to_key(rid), true);
                    }
                }
            }
        }
    }

    write_msg(
        stdout,
        json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": session_id,
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": format!("echo:{prompt_text}") }
                }
            }
        }),
    );
    write_msg(
        stdout,
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "stopReason": "end_turn" }
        }),
    );
}

fn extract_prompt_text(params: &Value) -> String {
    params
        .get("prompt")
        .and_then(|p| p.as_array())
        .and_then(|arr| arr.first())
        .and_then(|block| block.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string()
}

fn id_to_key(id: &Value) -> String {
    match id {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn write_msg(stdout: &mut impl Write, value: Value) {
    let line = value.to_string();
    let _ = writeln!(stdout, "{line}");
    let _ = stdout.flush();
}
