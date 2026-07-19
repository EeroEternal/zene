//! Build ACP `session/update` payloads from engine events and session history.

use serde_json::{json, Value};
use zene_llm::{Message, Role};

/// Map a built-in / MCP tool name to an ACP tool kind.
pub fn tool_kind(name: &str) -> &'static str {
    match name {
        "Read" | "Grep" | "Glob" => "read",
        "Write" | "Edit" => "edit",
        "Bash" | "Task" | "TaskOutput" => "execute",
        "FetchUrl" | "WebSearch" => "fetch",
        "TodoWrite" | "AskUser" | "EnterPlanMode" | "ExitPlanMode" | "Skill" => "think",
        _ if name.starts_with("mcp__") => "execute",
        _ => "other",
    }
}

pub fn tool_title(name: &str, arguments: &str) -> String {
    let preview = truncate(arguments, 80);
    if preview.is_empty() {
        format!("Run tool `{name}`")
    } else {
        format!("{name}({preview})")
    }
}

fn truncate(input: &str, max: usize) -> String {
    let trimmed = input.trim();
    if trimmed.chars().count() <= max {
        trimmed.to_string()
    } else {
        format!("{}...", trimmed.chars().take(max).collect::<String>())
    }
}

pub fn agent_message_chunk(text: &str) -> Value {
    json!({
        "sessionUpdate": "agent_message_chunk",
        "content": { "type": "text", "text": text }
    })
}

pub fn user_message_chunk(text: &str) -> Value {
    json!({
        "sessionUpdate": "user_message_chunk",
        "content": { "type": "text", "text": text }
    })
}

pub fn tool_call_update(
    tool_call_id: &str,
    name: &str,
    arguments: &str,
    status: &str,
) -> Value {
    let mut raw_input = serde_json::from_str::<Value>(arguments).unwrap_or_else(|_| {
        json!({ "raw": arguments })
    });
    if raw_input.is_null() {
        raw_input = json!({ "raw": arguments });
    }
    json!({
        "sessionUpdate": "tool_call",
        "toolCallId": tool_call_id,
        "title": tool_title(name, arguments),
        "kind": tool_kind(name),
        "status": status,
        "rawInput": raw_input,
    })
}

pub fn tool_call_result_update(
    tool_call_id: &str,
    content: &str,
    is_error: bool,
) -> Value {
    let status = if is_error { "failed" } else { "completed" };
    let text = if content.len() > 8_000 {
        format!("{}…", &content[..8_000])
    } else {
        content.to_string()
    };
    json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": tool_call_id,
        "status": status,
        "content": [{
            "type": "content",
            "content": { "type": "text", "text": text }
        }],
        "rawOutput": { "text": text, "isError": is_error },
    })
}

/// Convert TodoWrite arguments into an ACP `plan` update, if parseable.
pub fn plan_from_todo_arguments(arguments: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(arguments).ok()?;
    let todos = value.get("todos")?.as_array()?;
    let entries: Vec<Value> = todos
        .iter()
        .filter_map(|todo| {
            let content = todo.get("content")?.as_str()?;
            let status = match todo.get("status").and_then(Value::as_str).unwrap_or("pending") {
                "in_progress" | "in-progress" => "in_progress",
                "completed" | "done" => "completed",
                _ => "pending",
            };
            Some(json!({
                "content": content,
                "status": status,
                "priority": "medium",
            }))
        })
        .collect();
    if entries.is_empty() {
        return None;
    }
    Some(json!({
        "sessionUpdate": "plan",
        "entries": entries,
    }))
}

/// Replay persisted session messages as ACP `session/update` payloads.
pub fn replay_updates_from_messages(messages: &[Message]) -> Vec<Value> {
    let mut updates = Vec::new();
    for message in messages {
        match message.role {
            Role::System => {}
            Role::User => {
                if let Some(text) = message.content.as_deref() {
                    if !text.trim().is_empty() {
                        updates.push(user_message_chunk(text));
                    }
                }
            }
            Role::Assistant => {
                if let Some(text) = message.content.as_deref() {
                    if !text.trim().is_empty() {
                        updates.push(agent_message_chunk(text));
                    }
                }
                if let Some(calls) = &message.tool_calls {
                    for call in calls {
                        updates.push(tool_call_update(
                            &call.id,
                            &call.name,
                            &call.arguments,
                            "pending",
                        ));
                        if call.name == "TodoWrite" {
                            if let Some(plan) = plan_from_todo_arguments(&call.arguments) {
                                updates.push(plan);
                            }
                        }
                    }
                }
            }
            Role::Tool => {
                let id = message
                    .tool_call_id
                    .as_deref()
                    .unwrap_or("unknown");
                let content = message.content.as_deref().unwrap_or("");
                let is_error = message.is_error.unwrap_or(false);
                updates.push(tool_call_result_update(id, content, is_error));
            }
        }
    }
    updates
}

#[cfg(test)]
mod tests {
    use super::*;
    use zene_llm::ToolCall;

    #[test]
    fn maps_tool_kinds() {
        assert_eq!(tool_kind("Read"), "read");
        assert_eq!(tool_kind("Edit"), "edit");
        assert_eq!(tool_kind("Bash"), "execute");
        assert_eq!(tool_kind("TodoWrite"), "think");
        assert_eq!(tool_kind("mcp__git__status"), "execute");
    }

    #[test]
    fn builds_plan_from_todo_args() {
        let plan = plan_from_todo_arguments(
            r#"{"todos":[{"id":"1","content":"Ship ACP","status":"in_progress"}]}"#,
        )
        .expect("plan");
        assert_eq!(plan["sessionUpdate"], "plan");
        assert_eq!(plan["entries"][0]["content"], "Ship ACP");
        assert_eq!(plan["entries"][0]["status"], "in_progress");
    }

    #[test]
    fn replays_user_assistant_and_tools() {
        let messages = vec![
            Message::system("sys"),
            Message::user("hello"),
            Message::assistant_with_tools(
                Some("working".into()),
                vec![ToolCall {
                    id: "c1".into(),
                    name: "Read".into(),
                    arguments: r#"{"path":"a.rs"}"#.into(),
                }],
            ),
            Message::tool_result("c1", "Read", "fn main() {}"),
            Message::assistant("done"),
        ];
        let updates = replay_updates_from_messages(&messages);
        assert_eq!(updates[0]["sessionUpdate"], "user_message_chunk");
        assert_eq!(updates[1]["sessionUpdate"], "agent_message_chunk");
        assert_eq!(updates[2]["sessionUpdate"], "tool_call");
        assert_eq!(updates[2]["toolCallId"], "c1");
        assert_eq!(updates[3]["sessionUpdate"], "tool_call_update");
        assert_eq!(updates[3]["status"], "completed");
        assert_eq!(updates[4]["sessionUpdate"], "agent_message_chunk");
    }
}
