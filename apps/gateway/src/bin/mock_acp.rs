//! Minimal ACP stdio peer used by gateway integration tests.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};

use serde_json::{json, Value};

fn main() {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut stdout = io::stdout();
    let session_id = String::from("mock-session-1");
    let mut pending_responses: HashMap<String, Value> = HashMap::new();
    let mut line = String::new();
    let mut mode = "default".to_string();

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

        // Client JSON-RPC response.
        if msg.get("method").is_none() && msg.get("id").is_some() {
            if let Some(id) = msg.get("id") {
                pending_responses.insert(id_to_key(id), msg);
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
            "session/new" | "session/load" | "session/resume" => {
                write_msg(
                    &mut stdout,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "sessionId": session_id,
                            "modes": {
                                "currentModeId": mode,
                                "availableModes": [
                                    { "id": "default", "name": "Default" },
                                    { "id": "plan", "name": "Plan" }
                                ]
                            }
                        }
                    }),
                );
                write_msg(
                    &mut stdout,
                    json!({
                        "jsonrpc": "2.0",
                        "method": "session/update",
                        "params": {
                            "sessionId": session_id,
                            "update": {
                                "sessionUpdate": "current_mode_update",
                                "currentModeId": mode
                            }
                        }
                    }),
                );
            }
            "session/list" => {
                write_msg(
                    &mut stdout,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "sessions": [{
                                "sessionId": session_id,
                                "cwd": params.get("cwd").cloned().unwrap_or(json!(".")),
                                "title": "mock session",
                                "updatedAt": "2026-07-20T00:00:00Z"
                            }]
                        }
                    }),
                );
            }
            "session/close" => {
                write_msg(
                    &mut stdout,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {}
                    }),
                );
            }
            "session/set_mode" => {
                mode = params
                    .get("modeId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default")
                    .to_string();
                write_msg(
                    &mut stdout,
                    json!({
                        "jsonrpc": "2.0",
                        "method": "session/update",
                        "params": {
                            "sessionId": session_id,
                            "update": {
                                "sessionUpdate": "current_mode_update",
                                "currentModeId": mode
                            }
                        }
                    }),
                );
                write_msg(
                    &mut stdout,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {}
                    }),
                );
            }
            "session/prompt" => {
                handle_prompt(
                    &mut input,
                    &mut stdout,
                    &session_id,
                    &mut pending_responses,
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
    pending_responses: &mut HashMap<String, Value>,
    id: Option<Value>,
    params: &Value,
    line: &mut String,
) {
    let prompt_text = extract_prompt_text(params);
    let want_permission = prompt_text.contains("NEED_PERMISSION");
    let want_terminal = prompt_text.contains("TERMINAL_PING");
    let want_todo = prompt_text.contains("NEED_TODO");

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

    if want_todo {
        write_msg(
            stdout,
            json!({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {
                    "sessionId": session_id,
                    "update": {
                        "sessionUpdate": "tool_call",
                        "toolCallId": "todo_1",
                        "title": "TodoWrite",
                        "kind": "think",
                        "status": "pending",
                        "rawInput": {
                            "todos": [
                                { "id": "1", "content": "Ship plan panel", "status": "in_progress" },
                                { "id": "2", "content": "Wire terminal host", "status": "pending" }
                            ]
                        }
                    }
                }
            }),
        );
        write_msg(
            stdout,
            json!({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {
                    "sessionId": session_id,
                    "update": {
                        "sessionUpdate": "plan",
                        "entries": [
                            { "content": "Ship plan panel", "status": "in_progress", "priority": "high" },
                            { "content": "Wire terminal host", "status": "pending", "priority": "medium" }
                        ]
                    }
                }
            }),
        );
        write_msg(
            stdout,
            json!({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {
                    "sessionId": session_id,
                    "update": {
                        "sessionUpdate": "tool_call",
                        "toolCallId": "task_bg_1",
                        "title": "Task",
                        "kind": "execute",
                        "status": "in_progress",
                        "rawInput": { "description": "background check", "run_in_background": true }
                    }
                }
            }),
        );
    }

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
        let _ = wait_response(input, pending_responses, &perm_id.to_string(), line);
    }

    if want_terminal {
        let create_id = 9100;
        write_msg(
            stdout,
            json!({
                "jsonrpc": "2.0",
                "id": create_id,
                "method": "terminal/create",
                "params": {
                    "sessionId": session_id,
                    "command": "printf",
                    "args": ["gateway-terminal-ok"],
                    "cwd": ".",
                    "outputByteLimit": 4096
                }
            }),
        );
        let created = match wait_response(input, pending_responses, &create_id.to_string(), line) {
            Some(v) => v,
            None => return,
        };
        let terminal_id = created
            .pointer("/result/terminalId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let wait_id = 9101;
        write_msg(
            stdout,
            json!({
                "jsonrpc": "2.0",
                "id": wait_id,
                "method": "terminal/wait_for_exit",
                "params": { "sessionId": session_id, "terminalId": terminal_id }
            }),
        );
        let _ = wait_response(input, pending_responses, &wait_id.to_string(), line);

        let out_id = 9102;
        write_msg(
            stdout,
            json!({
                "jsonrpc": "2.0",
                "id": out_id,
                "method": "terminal/output",
                "params": { "sessionId": session_id, "terminalId": terminal_id }
            }),
        );
        let output = wait_response(input, pending_responses, &out_id.to_string(), line);
        let text = output
            .as_ref()
            .and_then(|v| v.pointer("/result/output"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let rel_id = 9103;
        write_msg(
            stdout,
            json!({
                "jsonrpc": "2.0",
                "id": rel_id,
                "method": "terminal/release",
                "params": { "sessionId": session_id, "terminalId": terminal_id }
            }),
        );
        let _ = wait_response(input, pending_responses, &rel_id.to_string(), line);

        write_msg(
            stdout,
            json!({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {
                    "sessionId": session_id,
                    "update": {
                        "sessionUpdate": "agent_message_chunk",
                        "content": { "type": "text", "text": format!("terminal:{text}") }
                    }
                }
            }),
        );
    } else {
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
    }

    write_msg(
        stdout,
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "stopReason": "end_turn" }
        }),
    );
}

fn wait_response(
    input: &mut impl BufRead,
    pending_responses: &mut HashMap<String, Value>,
    key: &str,
    line: &mut String,
) -> Option<Value> {
    while !pending_responses.contains_key(key) {
        line.clear();
        let n = match input.read_line(line) {
            Ok(0) => return None,
            Ok(n) => n,
            Err(_) => return None,
        };
        if n == 0 || line.trim().is_empty() {
            continue;
        }
        if let Ok(resp) = serde_json::from_str::<Value>(line.trim()) {
            if resp.get("method").is_none() {
                if let Some(rid) = resp.get("id") {
                    pending_responses.insert(id_to_key(rid), resp);
                }
            }
        }
    }
    pending_responses.remove(key)
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
