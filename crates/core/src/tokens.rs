use zene_llm::{Message, MessageKind, Role, ToolDefinition};

/// Per tool-call JSON/API framing beyond argument string length.
const TOOL_CALL_FRAME_TOKENS: u32 = 12;

/// Heuristic token estimator with configurable chars-per-token ratio.
#[derive(Debug, Clone, Copy)]
pub struct TokenEstimator {
    pub chars_per_token: f32,
}

impl Default for TokenEstimator {
    fn default() -> Self {
        Self {
            chars_per_token: 4.0,
        }
    }
}

impl TokenEstimator {
    pub fn new(chars_per_token: f32) -> Self {
        Self {
            chars_per_token: chars_per_token.max(1.0),
        }
    }

    pub fn estimate_chars_as_tokens(&self, text: &str) -> u32 {
        if text.is_empty() {
            return 0;
        }
        (text.chars().count() as f32 / self.chars_per_token).ceil() as u32
    }

    /// Role-specific framing overhead (JSON keys, role labels, separators).
    fn role_overhead(&self, role: Role, kind: Option<MessageKind>) -> u32 {
        let base = match role {
            Role::System => 8,
            Role::User => 4,
            Role::Assistant => 4,
            Role::Tool => 8,
        };
        if kind == Some(MessageKind::CompactionSummary) {
            base + 4
        } else {
            base
        }
    }

    /// JSON tool arguments: string length plus structural punctuation overhead.
    fn estimate_json_args_tokens(&self, json: &str) -> u32 {
        if json.is_empty() {
            return 0;
        }
        let content = self.estimate_chars_as_tokens(json);
        let structure = json
            .chars()
            .filter(|c| matches!(c, '{' | '}' | '[' | ']' | ':' | ',' | '"'))
            .count();
        content + (structure as f32 / self.chars_per_token).ceil() as u32
    }

    pub fn estimate_message_tokens(&self, message: &Message) -> u32 {
        let mut tokens = self.role_overhead(message.role, message.kind);

        if let Some(content) = &message.content {
            tokens += self.estimate_chars_as_tokens(content);
        }

        if let Some(tool_calls) = &message.tool_calls {
            for call in tool_calls {
                tokens += TOOL_CALL_FRAME_TOKENS;
                tokens += self.estimate_chars_as_tokens(&call.id);
                tokens += self.estimate_chars_as_tokens(&call.name);
                tokens += self.estimate_json_args_tokens(&call.arguments);
            }
        }

        if let Some(tool_call_id) = &message.tool_call_id {
            tokens += self.estimate_chars_as_tokens(tool_call_id);
        }

        if let Some(name) = &message.name {
            tokens += self.estimate_chars_as_tokens(name);
        }

        if message.is_error == Some(true) {
            tokens += 2;
        }

        tokens
    }

    pub fn estimate_messages_tokens(&self, messages: &[Message]) -> u32 {
        messages.iter().map(|m| self.estimate_message_tokens(m)).sum()
    }

    pub fn estimate_tools_tokens(&self, tools: &[ToolDefinition]) -> u32 {
        if tools.is_empty() {
            return 0;
        }
        let json = serde_json::to_string(tools).unwrap_or_default();
        self.estimate_chars_as_tokens(&json) + 4
    }

    pub fn estimate_request_tokens(&self, messages: &[Message], tools: &[ToolDefinition]) -> u32 {
        self.estimate_messages_tokens(messages) + self.estimate_tools_tokens(tools)
    }
}

/// Total estimated context size (messages + tool definitions) for compaction checks.
pub fn estimate_context(
    messages: &[Message],
    tools: &[ToolDefinition],
    estimator: &TokenEstimator,
) -> usize {
    estimator.estimate_request_tokens(messages, tools) as usize
}

/// Heuristic token estimate: ~4 characters per token (legacy default).
#[allow(dead_code)]
pub fn estimate_chars_as_tokens(text: &str) -> u32 {
    TokenEstimator::default().estimate_chars_as_tokens(text)
}

pub fn estimate_message_tokens(message: &Message) -> u32 {
    TokenEstimator::default().estimate_message_tokens(message)
}

#[allow(dead_code)]
pub fn estimate_messages_tokens(messages: &[Message]) -> u32 {
    TokenEstimator::default().estimate_messages_tokens(messages)
}

#[allow(dead_code)]
pub fn estimate_tools_tokens(tools: &[ToolDefinition]) -> u32 {
    TokenEstimator::default().estimate_tools_tokens(tools)
}

pub fn estimate_request_tokens(messages: &[Message], tools: &[ToolDefinition]) -> u32 {
    TokenEstimator::default().estimate_request_tokens(messages, tools)
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
    fn custom_ratio_increases_token_count() {
        let loose = TokenEstimator::new(8.0);
        let tight = TokenEstimator::new(2.0);
        let text = "abcdefgh";
        assert!(loose.estimate_chars_as_tokens(text) < tight.estimate_chars_as_tokens(text));
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

    #[test]
    fn estimate_context_matches_request_tokens() {
        let messages = vec![Message::user("hello")];
        let tools = vec![ToolDefinition {
            name: "Read".into(),
            description: "Read".into(),
            parameters: serde_json::json!({}),
        }];
        let estimator = TokenEstimator::default();
        assert_eq!(
            estimate_context(&messages, &tools, &estimator),
            estimator.estimate_request_tokens(&messages, &tools) as usize
        );
    }

    #[test]
    fn system_role_has_higher_overhead_than_user() {
        let estimator = TokenEstimator::default();
        let system = Message::system("hi");
        let user = Message::user("hi");
        assert!(estimator.estimate_message_tokens(&system) > estimator.estimate_message_tokens(&user));
    }

    #[test]
    fn json_args_include_structure_overhead() {
        let estimator = TokenEstimator::default();
        let plain = Message::assistant("hi");
        let with_tools = Message::assistant_with_tools(
            None,
            vec![ToolCall {
                id: "call_1".into(),
                name: "Read".into(),
                arguments: r#"{"path":"src/main.rs","offset":1,"limit":100}"#.into(),
            }],
        );
        assert!(
            estimator.estimate_message_tokens(&with_tools)
                > estimator.estimate_message_tokens(&plain) + 10
        );
    }

    #[test]
    fn compaction_summary_has_extra_overhead() {
        let estimator = TokenEstimator::default();
        let plain = Message::assistant("summary text");
        let summary = Message::compaction_summary("summary text");
        assert!(estimator.estimate_message_tokens(&summary) > estimator.estimate_message_tokens(&plain));
    }
}
