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

/// Friendly one-line label for ACP clients. Prefer intent over raw commands/paths;
/// the full command stays in `rawInput` for expand-to-inspect.
pub fn tool_title(name: &str, arguments: &str) -> String {
    let args: Value = serde_json::from_str(arguments).unwrap_or(Value::Null);
    let field = |key: &str| -> Option<&str> { args.get(key).and_then(|v| v.as_str()) };

    match name {
        "Read" => format!("Read {}", display_path(field("path").unwrap_or("file"))),
        "Write" => format!("Wrote {}", display_path(field("path").unwrap_or("file"))),
        "Edit" => format!("Edited {}", display_path(field("path").unwrap_or("file"))),
        "Bash" => bash_title(field("command").unwrap_or("")),
        "Grep" => format!(
            "Searched code for {}",
            truncate(field("pattern").or_else(|| field("regex")).unwrap_or("…"), 40)
        ),
        "Glob" => format!(
            "Found files matching {}",
            truncate(
                field("pattern")
                    .or_else(|| field("glob_pattern"))
                    .or_else(|| field("glob"))
                    .unwrap_or("*"),
                40
            )
        ),
        "WebSearch" => format!(
            "Searched the web for \"{}\"",
            truncate(field("query").unwrap_or("…"), 40)
        ),
        "FetchUrl" => "Fetched a web page".to_string(),
        "TodoWrite" => "Updated todos".to_string(),
        "Task" => "Ran a subtask".to_string(),
        "TaskOutput" => "Checked task output".to_string(),
        "AskUser" => "Asked a question".to_string(),
        "EnterPlanMode" => "Entered plan mode".to_string(),
        "ExitPlanMode" => "Exited plan mode".to_string(),
        "Skill" => format!(
            "Used skill {}",
            field("skill").or_else(|| field("name")).unwrap_or("skill")
        ),
        _ => {
            if name.starts_with("mcp__") {
                format!("Used {}", name.rsplit("__").next().unwrap_or(name))
            } else {
                format!("Used {name}")
            }
        }
    }
}

fn bash_title(command: &str) -> String {
    let first = command
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .replace('\t', " ");
    let collapsed = first.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return "Ran a command".to_string();
    }
    let cmd = strip_leading_cd(&collapsed);
    bash_intent(cmd)
}

fn strip_leading_cd(cmd: &str) -> &str {
    cmd.strip_prefix("cd ")
        .and_then(|rest| {
            rest.find(" && ")
                .map(|i| rest[i + 4..].trim())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or(cmd)
}

/// Map shell to a short intent phrase — never echo the raw command line.
fn bash_intent(cmd: &str) -> String {
    let lower = cmd.to_ascii_lowercase();
    let head = lower.split_whitespace().next().unwrap_or("");
    let rest = lower.split_whitespace().skip(1).collect::<Vec<_>>().join(" ");

    match head {
        "mkdir" | "mktemp" => "Created directories".into(),
        "rm" | "rmdir" => "Removed files".into(),
        "cp" | "mv" | "install" => "Moved or copied files".into(),
        "touch" => "Created files".into(),
        "chmod" | "chown" => "Changed file permissions".into(),
        "ls" | "tree" | "find" | "du" | "stat" | "file" => "Listed files".into(),
        "cat" | "head" | "tail" | "less" | "more" | "bat" => "Inspected file contents".into(),
        "rg" | "grep" | "ag" | "ack" => "Searched files".into(),
        "curl" | "wget" | "http" => "Fetched from the network".into(),
        "echo" | "printf" => "Printed output".into(),
        "which" | "type" | "command" | "whereis" => "Checked available tools".into(),
        "uname" | "sw_vers" | "sysctl" => "Checked system info".into(),
        "pwd" => "Checked working directory".into(),
        "export" | "unset" | "env" | "printenv" => "Updated environment".into(),
        "cargo" => cargo_intent(&rest),
        "rustc" | "rustup" => "Checked Rust toolchain".into(),
        "npm" | "pnpm" | "yarn" | "bun" => node_intent(head, &rest),
        "python" | "python3" | "pip" | "pip3" | "uv" => "Ran Python tooling".into(),
        "go" => "Ran Go tooling".into(),
        "make" | "cmake" | "ninja" => "Ran a build".into(),
        "docker" | "podman" => "Ran a container command".into(),
        "gh" => gh_intent(&rest),
        "git" => git_intent(&rest),
        "tar" | "zip" | "unzip" | "gzip" => "Archived files".into(),
        "sed" | "awk" | "cut" | "sort" | "uniq" | "tr" | "jq" | "xargs" => {
            "Processed text".into()
        }
        "test" | "[" => "Ran a shell check".into(),
        "bash" | "sh" | "zsh" => "Ran a shell script".into(),
        _ if lower.contains("&&") || lower.contains(';') || lower.contains('|') => {
            "Ran a shell script".into()
        }
        _ => "Ran a command".into(),
    }
}

fn cargo_intent(rest: &str) -> String {
    let sub = rest.split_whitespace().next().unwrap_or("");
    match sub {
        "build" | "b" => "Built with Cargo".into(),
        "check" | "c" | "clippy" => "Checked with Cargo".into(),
        "test" | "t" => "Ran Cargo tests".into(),
        "run" | "r" => "Ran a Cargo binary".into(),
        "init" | "new" => "Created a Cargo project".into(),
        "add" => "Added a Cargo dependency".into(),
        "fmt" => "Formatted Rust code".into(),
        "update" | "fetch" => "Updated Cargo dependencies".into(),
        "--version" | "-V" | "version" => "Checked Cargo version".into(),
        _ => "Ran Cargo".into(),
    }
}

fn node_intent(bin: &str, rest: &str) -> String {
    let sub = rest.split_whitespace().next().unwrap_or("");
    match sub {
        "install" | "i" | "ci" | "add" => format!("Installed {bin} packages"),
        "run" | "test" | "build" | "start" | "dev" => format!("Ran {bin} {sub}"),
        _ => format!("Ran {bin}"),
    }
}

fn git_intent(rest: &str) -> String {
    let sub = rest.split_whitespace().next().unwrap_or("");
    match sub {
        "status" | "diff" | "log" | "show" | "blame" => "Inspected git state".into(),
        "branch" | "switch" | "checkout" => "Changed git branch".into(),
        "clone" | "fetch" | "pull" => "Synced git remotes".into(),
        "push" => "Pushed to remote".into(),
        "add" | "commit" | "stash" => "Recorded git changes".into(),
        "merge" | "rebase" | "cherry-pick" => "Merged git history".into(),
        "remote" | "tag" => "Updated git refs".into(),
        _ => "Ran a git command".into(),
    }
}

fn gh_intent(rest: &str) -> String {
    let sub = rest.split_whitespace().next().unwrap_or("");
    match sub {
        "pr" => "Checked pull requests".into(),
        "issue" => "Checked issues".into(),
        "api" | "repo" => "Queried GitHub".into(),
        _ => "Ran GitHub CLI".into(),
    }
}

/// Prefer a short relative/basename path (hide workspace UUID prefixes).
fn display_path(path: &str) -> String {
    let p = path.trim().replace('\\', "/");
    let stripped = if let Some(idx) = p.find("/workspaces/") {
        let after = &p[idx + "/workspaces/".len()..];
        after
            .find('/')
            .map(|i| after[i + 1..].to_string())
            .unwrap_or_else(|| after.to_string())
    } else {
        p.clone()
    };
    let shown = if stripped.is_empty() { p } else { stripped };
    // Keep last 2 segments when still long.
    if shown.chars().count() > 42 {
        let parts: Vec<&str> = shown.split('/').filter(|s| !s.is_empty()).collect();
        if parts.len() >= 2 {
            return truncate(&format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1]), 42);
        }
    }
    truncate(&shown, 42)
}

fn truncate(input: &str, max: usize) -> String {
    let trimmed = input.trim();
    if trimmed.chars().count() <= max {
        trimmed.to_string()
    } else {
        format!("{}…", trimmed.chars().take(max).collect::<String>())
    }
}

pub fn agent_message_chunk(text: &str) -> Value {
    json!({
        "sessionUpdate": "agent_message_chunk",
        "content": { "type": "text", "text": text }
    })
}

pub fn agent_thought_chunk(text: &str) -> Value {
    json!({
        "sessionUpdate": "agent_thought_chunk",
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

pub fn current_mode_update(mode_id: &str) -> Value {
    json!({
        "sessionUpdate": "current_mode_update",
        "modeId": mode_id,
    })
}

pub fn usage_update(
    used: u64,
    size: u64,
    prompt_tokens: u64,
    completion_tokens: u64,
    context_percent: u8,
    cached_tokens: Option<u64>,
    context_epoch: u64,
) -> Value {
    let mut meta = serde_json::Map::from_iter([
        ("promptTokens".into(), json!(prompt_tokens)),
        ("completionTokens".into(), json!(completion_tokens)),
        ("contextPercent".into(), json!(context_percent)),
        ("contextEpoch".into(), json!(context_epoch)),
    ]);
    if let Some(cached) = cached_tokens {
        meta.insert("cachedTokens".into(), json!(cached));
    }
    json!({
        "sessionUpdate": "usage_update",
        "used": used,
        "size": size,
        "_meta": Value::Object(meta),
    })
}

pub fn available_modes() -> Value {
    json!([
        {
            "id": "default",
            "name": "Default",
            "description": "Full tool access with permission prompts for gated tools"
        },
        {
            "id": "plan",
            "name": "Plan",
            "description": "Read-only exploration; ExitPlanMode required before edits"
        }
    ])
}

pub fn modes_state(current_mode_id: &str) -> Value {
    json!({
        "currentModeId": current_mode_id,
        "availableModes": available_modes(),
    })
}

pub fn available_commands_update() -> Value {
    json!({
        "sessionUpdate": "available_commands_update",
        "availableCommands": [
            {
                "name": "plan",
                "description": "Enter plan mode (read-only tools until ExitPlanMode)"
            },
            {
                "name": "compact",
                "description": "Compact conversation context"
            },
            {
                "name": "rewind",
                "description": "Rewind to a session checkpoint"
            }
        ]
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
    fn human_tool_titles() {
        assert_eq!(
            tool_title("Read", r#"{"path":"src/main.rs"}"#),
            "Read src/main.rs"
        );
        assert_eq!(
            tool_title("Write", r#"{"path":"Cargo.toml","contents":"[package]"}"#),
            "Wrote Cargo.toml"
        );
        assert_eq!(
            tool_title(
                "Write",
                r#"{"path":"/Users/x/cloud/data/workspaces/abc-123/crates/axon/src/lib.rs"}"#
            ),
            "Wrote crates/axon/src/lib.rs"
        );
        assert_eq!(
            tool_title("Bash", r#"{"command":"cd /tmp && cargo test -q"}"#),
            "Ran Cargo tests"
        );
        assert_eq!(
            tool_title("Bash", r#"{"command":"mkdir -p crates/foo/src"}"#),
            "Created directories"
        );
        assert_eq!(
            tool_title("Bash", r#"{"command":"ls -la && find . -name '*.rs'"}"#),
            "Listed files"
        );
        assert_eq!(
            tool_title("Grep", r#"{"pattern":"tool_title"}"#),
            "Searched code for tool_title"
        );
        assert_eq!(tool_title("TodoWrite", r#"{"todos":[]}"#), "Updated todos");
        assert!(!tool_title("Bash", r#"{"command":"ls -la /tmp/foo"}"#).contains('/'));
    }

    #[test]
    fn modes_and_commands_shapes() {
        let modes = modes_state("default");
        assert_eq!(modes["currentModeId"], "default");
        assert_eq!(modes["availableModes"].as_array().unwrap().len(), 2);
        assert_eq!(
            available_commands_update()["sessionUpdate"],
            "available_commands_update"
        );
        assert_eq!(current_mode_update("plan")["modeId"], "plan");
        assert_eq!(
            agent_thought_chunk("hmm")["sessionUpdate"],
            "agent_thought_chunk"
        );
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
