use zene_llm::{Message, ToolDefinition};

/// Heuristic token estimate: ~4 characters per token.
pub fn estimate_chars_as_tokens(text: &str) -> u32 {
    let chars = text.chars().count() as u32;
    chars.div_ceil(4)
}

pub fn estimate_message_tokens(message: &Message) -> u32 {
    let mut tokens = 4u32;

    if let Some(content) = &message.content {
        tokens += estimate_chars_as_tokens(content);
    }

    if let Some(tool_calls) = &message.tool_calls {
        for call in tool_calls {
            tokens += estimate_chars_as_tokens(&call.id);
            tokens += estimate_chars_as_tokens(&call.name);
            tokens += estimate_chars_as_tokens(&call.arguments);
        }
    }

    if let Some(tool_call_id) = &message.tool_call_id {
        tokens += estimate_chars_as_tokens(tool_call_id);
    }

    if let Some(name) = &message.name {
        tokens += estimate_chars_as_tokens(name);
    }

    tokens
}

pub fn estimate_messages_tokens(messages: &[Message]) -> u32 {
    messages.iter().map(estimate_message_tokens).sum()
}

pub fn estimate_tools_tokens(tools: &[ToolDefinition]) -> u32 {
    let json = serde_json::to_string(tools).unwrap_or_default();
    estimate_chars_as_tokens(&json)
}

pub fn estimate_request_tokens(messages: &[Message], tools: &[ToolDefinition]) -> u32 {
    estimate_messages_tokens(messages) + estimate_tools_tokens(tools)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zene_llm::ToolCall;

    #[test]
    fn chars_heuristic_uses_ceiling_division() {
        assert_eq!(estimate_chars_as_tokens(""), 0);
        assert_eq!(estimate_chars_as_tokens("abcd"), 1);
        assert_eq!(estimate_chars_as_tokens("abcde"), 2);
    }

    #[test]
    fn tool_message_includes_content() {
        let message = Message::tool_result("call_1", "Read", "file contents");
        assert!(estimate_message_tokens(&message) > 4);
    }

    #[test]
    fn assistant_with_tools_counts_arguments() {
        let message = Message::assistant_with_tools(
            None,
            vec![ToolCall {
                id: "call_1".into(),
                name: "Read".into(),
                arguments: r#"{"path":"a.rs"}"#.into(),
            }],
        );
        assert!(estimate_message_tokens(&message) > estimate_message_tokens(&Message::assistant("hi")));
    }

    #[test]
    fn tools_json_is_included() {
        let tools = vec![ToolDefinition {
            name: "Read".into(),
            description: "Read a file".into(),
            parameters: serde_json::json!({"type": "object"}),
        }];
        assert!(estimate_tools_tokens(&tools) > 0);
    }
}
